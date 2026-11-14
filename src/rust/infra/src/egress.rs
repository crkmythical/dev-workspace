//! Egress adapters — a port of packages/infra/src/egress.ts. The Clash/mihomo implementation of
//! EgressPorts, a quick live observation for `doctor`, and the two places the start-up result is
//! left (a status file and a note inside the vault). What counts as confirmed/degraded/down is
//! decided in sdw_core::egress.
//!
//! The controller is plain HTTP on localhost behind a bearer secret; infra::clash speaks to it over
//! a raw socket (and the JSON is parsed with serde_json) rather than pulling an HTTP/TLS client
//! into the static binary. Traffic THROUGH the proxy (the probes) is still curl's job.

use crate::clash::{config_is_ready, Controller, QUICK, REFRESH};
use crate::cmd;
use crate::notifications::{remove_note, write_note};
use crate::time::now_iso;
use sdw_core::egress::allowlist::AllowList;
use sdw_core::egress::domain::{
    egress_notice, parse_egress_report, serialize_egress_report, EgressObservation, EgressReport,
};
use sdw_core::egress::firewall::{egress_ruleset, ssh_proxy_config, EGRESS_TABLE};
use sdw_core::egress::firewall_iptables::{egress_rules_v4, egress_rules_v6};
use sdw_core::egress::layout::{
    CLASH_BIN, CLASH_CONFIG_PATH, CLASH_DIR, CLASH_HTTP_PORT, CLASH_PROVIDER_NAME, CLASH_USER,
    EGRESS_NOTIFICATION_PATH, EGRESS_PROBE_URL, EGRESS_STATUS_PATH, SSH_PROXY_CONF_PATH,
};
use sdw_core::egress::ports::EgressPorts;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Something accepts TCP connections on 127.0.0.1:`port`.
pub fn port_accepts(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
}

fn proxy_url() -> String {
    format!("http://127.0.0.1:{CLASH_HTTP_PORT}")
}

/// EgressPorts backed by Clash. `log` narrates the bring-up; `clash_uid` is Some under enforced
/// egress, so Clash is spawned as its own uid (the only uid the firewall lets egress directly).
pub struct ClashEgressPorts<L: Fn(&str)> {
    log: L,
    clash_uid: Option<u32>,
}

pub fn clash_egress_ports<L: Fn(&str)>(log: L, clash_uid: Option<u32>) -> ClashEgressPorts<L> {
    ClashEgressPorts { log, clash_uid }
}

impl<L: Fn(&str)> EgressPorts for ClashEgressPorts<L> {
    fn start_proxy(&self) -> Result<(), String> {
        // A daemon: it must outlive this process, so spawn detached and do not wait on it. As the
        // clash user whenever the image has one (always now; Some(uid) says it was found), so the
        // firewall under EGRESS_ENFORCE can single out its traffic. The binary carries
        // cap_net_bind_service (set in the image), so it can still bind the DNS port 53.
        if !config_is_ready() {
            return Err(format!(
                "there is no rendered config at {CLASH_CONFIG_PATH}; not starting Clash (left to \
                 itself it writes a default config and runs as an open DIRECT proxy)"
            ));
        }
        let mut command = clash_command(self.clash_uid.is_some());
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        // Reap it when it exits. Dropped without a wait, a crashed Clash stays a zombie that
        // `ps` still lists, and nothing would ever restart it.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn proxy_listening(&self) -> Result<bool, String> {
        Ok(port_accepts(CLASH_HTTP_PORT))
    }

    fn provider_nodes(&self) -> Result<Option<i64>, String> {
        Ok(provider_node_count())
    }

    fn refresh_provider(&self) -> Result<(), String> {
        let path = format!("/providers/proxies/{CLASH_PROVIDER_NAME}");
        match Controller::from_config().put(&path, None, REFRESH) {
            Ok(reply) if reply.ok() => Ok(()),
            Ok(reply) => Err(format!(
                "the refresh request to {path} was answered {}",
                reply.status
            )),
            Err(e) => Err(format!("the refresh request to {path} failed: {e}")),
        }
    }

    fn probe(&self) -> Result<bool, String> {
        Ok(cmd::ok(
            "curl",
            &[
                "-fsSL",
                "--proxy",
                &proxy_url(),
                "--max-time",
                "10",
                EGRESS_PROBE_URL,
                "-o",
                "/dev/null",
            ],
        ))
    }

    fn sleep(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    fn now(&self) -> String {
        now_iso()
    }

    fn log(&self, message: &str) {
        (self.log)(message);
    }
}

/// A fast look at the state right now (seconds, not the start-up's patient waits). It asks the SAME
/// question as the start-up probe, whether a request through the proxy reaches EGRESS_PROBE_URL, so
/// `doctor` and the egress monitor cannot contradict the boot result. (They used to fetch
/// gstatic's 204 page, which many networks reach without any proxy: on one, an empty subscription
/// read "confirmed" in `doctor` while the boot said "degraded".) A HEAD request keeps it cheap:
/// the monitor runs it every minute through a possibly metered node. `-f` makes an HTTP error from
/// the proxy count as a failed probe.
pub fn observe_egress() -> EgressObservation {
    let proxy_listening = port_accepts(CLASH_HTTP_PORT);
    if !proxy_listening {
        return EgressObservation {
            proxy_listening,
            probe_ok: false,
        };
    }
    let probe_ok = cmd::ok(
        "curl",
        &[
            "-sfI",
            "--connect-timeout",
            "3",
            "--max-time",
            "8",
            "-x",
            &proxy_url(),
            EGRESS_PROBE_URL,
            "-o",
            "/dev/null",
        ],
    );
    EgressObservation {
        proxy_listening,
        probe_ok,
    }
}

/// `curl <args>` as the clash user (uid, gid, no supplementary groups) when the image has one, which
/// is the one identity the egress firewall lets out directly: a probe made as root would be
/// dropped by our own filter and so say nothing about the network. Falls back to the current user
/// where there is no such account.
fn curl_as_clash(args: &[&str]) -> bool {
    let Some(clash_uid) = resolve_uid(CLASH_USER) else {
        return cmd::ok("curl", args);
    };
    // Already that user (the standalone client runs `proxy-only` as clash): there is nothing to
    // switch to, and `setpriv --clear-groups` would need CAP_SETGID, which it no longer has.
    if rustix::process::geteuid().as_raw() == clash_uid {
        return cmd::ok("curl", args);
    }
    let reuid = format!("--reuid={CLASH_USER}");
    let regid = format!("--regid={CLASH_USER}");
    let mut full: Vec<&str> = vec![&reuid, &regid, "--clear-groups", "curl"];
    full.extend_from_slice(args);
    cmd::ok("setpriv", &full)
}

/// Boot probe for DNS auto-detection: can we reach the GLOBAL DoH resolvers directly? A DoH query
/// is sent straight to 1.1.1.1 / 8.8.8.8 (both IPs, so no name to resolve first), bypassing the
/// Clash proxy env vars (--noproxy, since Clash may not be up yet). On a censored network the TLS
/// to them is reset and both fail fast, so the caller falls back to domestic resolvers. true as
/// soon as one responds.
///
/// It runs as the clash user, so the answer is the same whether or not the egress firewall is up
/// (`set-clash-dns` runs it long after the boot loaded the filter).
pub fn foreign_dns_reachable() -> bool {
    for ip in ["1.1.1.1", "8.8.8.8"] {
        let url = format!("https://{ip}/dns-query?name=www.cloudflare.com&type=A");
        if curl_as_clash(&[
            "-fsS",
            "--noproxy",
            "*",
            "--max-time",
            "3",
            "-H",
            "accept: application/dns-json",
            &url,
            "-o",
            "/dev/null",
        ]) {
            return true;
        }
    }
    false
}

/// Hot-reload Clash from its on-disk config via the controller API (a full config reload, so DNS
/// re-initializes — no dropped process). false when the controller is not reachable (Clash not up
/// yet); the next (re)start reads the new file regardless.
pub fn reload_clash() -> bool {
    let body = serde_json::json!({ "path": CLASH_CONFIG_PATH }).to_string();
    matches!(
        Controller::from_config().put("/configs?force=true", Some(&body), REFRESH),
        Ok(reply) if reply.ok()
    )
}

/// The numeric uid of a system user via `id -u`. None if the user does not exist.
pub fn resolve_uid(user: &str) -> Option<u32> {
    let (ok, out) = cmd::capture("id", &["-u", user]);
    if !ok {
        return None;
    }
    out.trim().parse::<u32>().ok()
}

/// The numeric primary gid of a system user via `id -g`. None if the user does not exist.
pub fn resolve_gid(user: &str) -> Option<u32> {
    let (ok, out) = cmd::capture("id", &["-g", user]);
    if !ok {
        return None;
    }
    out.trim().parse::<u32>().ok()
}

/// The command that runs Clash in its directory. As the clash user when `as_clash_user`: uid AND
/// gid, and no supplementary groups, through `setpriv`, exactly as `sdw clash-watcher` respawns it
/// (setting only the uid left the process in group root).
pub fn clash_command(as_clash_user: bool) -> Command {
    let mut command = if as_clash_user {
        let mut c = Command::new("setpriv");
        c.args([
            &format!("--reuid={CLASH_USER}"),
            &format!("--regid={CLASH_USER}"),
            "--clear-groups",
            CLASH_BIN,
        ]);
        c
    } else {
        Command::new(CLASH_BIN)
    };
    command.args(["-d", CLASH_DIR]);
    command
}

/// A Clash process is alive (a zombie, which `pgrep` would count, is not).
pub fn clash_running() -> bool {
    let (ok, out) = cmd::capture("ps", &["-C", "clash", "-o", "stat="]);
    ok && out
        .lines()
        .map(str::trim)
        .any(|state| !state.is_empty() && !state.starts_with('Z'))
}

/// Give the clash user its directory (it writes the provider cache there) and return its uid, or
/// None when the image has no such user, in which case Clash runs as root.
pub fn prepare_clash_dir() -> Option<u32> {
    let uid = resolve_uid(CLASH_USER);
    if uid.is_some() {
        cmd::ok(
            "chown",
            &["-R", &format!("{CLASH_USER}:{CLASH_USER}"), CLASH_DIR],
        );
    } else {
        eprintln!("WARNING: user '{CLASH_USER}' not found; starting Clash as root.");
    }
    uid
}

/// How many nodes the subscription provider has loaded, as the controller reports it. None when the
/// controller does not answer (Clash is down, or the secret does not match).
pub fn provider_node_count() -> Option<i64> {
    let path = format!("/providers/proxies/{CLASH_PROVIDER_NAME}");
    match Controller::from_config().get(&path, QUICK) {
        Ok(reply) if reply.ok() => count_provider_nodes(&reply.body),
        _ => None,
    }
}

/// The number of proxies in the controller's answer for a provider: the length of `proxies`,
/// whether it is an array or an object, and zero when it is neither. None when the body is not
/// JSON at all (the controller answered something else).
fn count_provider_nodes(body: &str) -> Option<i64> {
    let data: serde_json::Value = serde_json::from_str(body).ok()?;
    Some(match data.get("proxies") {
        Some(serde_json::Value::Array(a)) => a.len() as i64,
        Some(serde_json::Value::Object(o)) => o.len() as i64,
        _ => 0,
    })
}

/// Which packet filter actually loaded the egress rules. nftables is tried first and is what every
/// normal Linux kernel uses; iptables-legacy is the fallback for kernels whose nftables is broken
/// (Android AVF). Reported in the boot log so it is never a mystery which one is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallBackend {
    Nftables,
    IptablesLegacy,
}

impl FirewallBackend {
    pub fn label(self) -> &'static str {
        match self {
            FirewallBackend::Nftables => "nftables",
            FirewallBackend::IptablesLegacy => "iptables-legacy",
        }
    }
}

/// Install the fail-closed egress firewall (default-DROP OUTPUT; only loopback, the Clash uid,
/// established replies and the operator's allowlist leave the box). Needs CAP_NET_ADMIN.
///
/// Tries nftables first — the default on every kernel where it works, so normal Linux and the
/// Docker image behave exactly as before. Falls back to iptables-legacy where nftables is
/// non-functional although netfilter is present (Android's AVF guest kernel: `nft` returns "Invalid
/// argument", the x_tables path works). Each backend applies its ruleset atomically. Returns Err —
/// so the boot fails closed — only when NEITHER backend can load the filter, or when it could lock
/// IPv4 but not IPv6 (half-filtered would leak v6).
pub fn apply_egress_firewall(
    clash_uid: u32,
    allow: &AllowList,
) -> Result<FirewallBackend, String> {
    // 1. nftables: one `inet` table covering v4 and v6, applied atomically by `nft -f`.
    let (nft_ok, nft_err) = cmd::run_stdin("nft", &["-f", "-"], &egress_ruleset(clash_uid, allow));
    if nft_ok {
        return Ok(FirewallBackend::Nftables);
    }

    // 2. iptables-legacy fallback: IPv4, then IPv6 when the kernel has a v6 stack. Each program is
    //    committed in ONE atomic transaction by `*-restore`, so a re-apply never opens a window.
    let (v4_ok, v4_err) = cmd::run_stdin(
        "iptables-legacy-restore",
        &[],
        &egress_rules_v4(clash_uid, allow),
    );
    if !v4_ok {
        return Err(format!(
            "no usable firewall backend. nftables: {}; iptables-legacy: {}",
            nft_err.trim(),
            v4_err.trim()
        ));
    }
    if ipv6_present() {
        let (v6_ok, v6_err) = cmd::run_stdin(
            "ip6tables-legacy-restore",
            &[],
            &egress_rules_v6(clash_uid, allow),
        );
        if !v6_ok {
            return Err(format!(
                "iptables-legacy locked IPv4 but not IPv6 ({}); refusing to boot half-filtered \
                 (v6 traffic would leak directly)",
                v6_err.trim()
            ));
        }
    }
    Ok(FirewallBackend::IptablesLegacy)
}

/// Whether this kernel has an IPv6 stack. Absent = no v6 traffic to filter and no `ip6tables` to
/// run; present = the fallback must lock v6 too or fail closed.
fn ipv6_present() -> bool {
    Path::new("/proc/net/if_inet6").exists()
}

/// The egress firewall is loaded right now, under either backend: the nftables `inet` table is
/// present, or the iptables-legacy filter table has OUTPUT at policy DROP (what the fallback sets).
pub fn egress_firewall_loaded() -> bool {
    cmd::ok("nft", &["list", "table", "inet", EGRESS_TABLE]) || iptables_legacy_output_drops()
}

/// The iptables-legacy filter OUTPUT chain defaults to DROP — the fallback backend's signature.
fn iptables_legacy_output_drops() -> bool {
    let (ok, out) = cmd::capture("iptables-legacy", &["-S", "OUTPUT"]);
    ok && out.lines().any(|line| line.trim() == "-P OUTPUT DROP")
}

/// Make `ssh` use Clash (see `firewall::ssh_proxy_config`): it does not read the proxy variables,
/// so with the firewall on it would otherwise run into the DROP.
pub fn write_ssh_proxy_config() -> Result<(), String> {
    let path = Path::new(SSH_PROXY_CONF_PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, ssh_proxy_config()).map_err(|e| format!("{SSH_PROXY_CONF_PATH}: {e}"))
}

/// Take that configuration away again (the firewall is off, so `ssh` can connect directly).
pub fn remove_ssh_proxy_config() {
    let _ = std::fs::remove_file(SSH_PROXY_CONF_PATH);
}

/// Leave the start-up observation where `doctor` can read it back (container-local).
pub fn write_egress_status(report: &EgressReport) -> std::io::Result<()> {
    std::fs::write(EGRESS_STATUS_PATH, serialize_egress_report(report))
}

pub fn read_egress_status() -> Option<EgressReport> {
    let raw = std::fs::read_to_string(EGRESS_STATUS_PATH).ok()?;
    parse_egress_report(Some(&raw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeResult {
    Written,
    Removed,
    Unavailable,
}

/// Make the note inside the vault match the report: written while egress is not confirmed, removed
/// once it is. "unavailable" when the vault is not mounted.
pub fn sync_egress_notice(report: &EgressReport) -> NoticeResult {
    match egress_notice(report) {
        None => {
            remove_note(EGRESS_NOTIFICATION_PATH);
            NoticeResult::Removed
        }
        Some(note) => {
            if write_note(EGRESS_NOTIFICATION_PATH, &note) {
                NoticeResult::Written
            } else {
                NoticeResult::Unavailable
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_nodes_are_counted_from_either_shape() {
        assert_eq!(
            count_provider_nodes(r#"{"proxies":[{"name":"a"},{"name":"b"}]}"#),
            Some(2)
        );
        assert_eq!(
            count_provider_nodes(r#"{"proxies":{"a":{},"b":{},"c":{}}}"#),
            Some(3)
        );
        assert_eq!(count_provider_nodes(r#"{"proxies":[]}"#), Some(0));
        assert_eq!(count_provider_nodes(r#"{"name":"subscription"}"#), Some(0));
        assert_eq!(count_provider_nodes(r#"{"proxies":7}"#), Some(0));
        assert_eq!(count_provider_nodes("not json"), None);
        assert_eq!(count_provider_nodes(""), None);
    }

    #[test]
    fn clash_runs_as_its_user_with_no_supplementary_groups() {
        let as_user = clash_command(true);
        let args: Vec<_> = as_user
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(as_user.get_program(), "setpriv");
        assert_eq!(
            args,
            [
                "--reuid=clash",
                "--regid=clash",
                "--clear-groups",
                CLASH_BIN,
                "-d",
                CLASH_DIR
            ]
        );
        let as_root = clash_command(false);
        assert_eq!(as_root.get_program(), CLASH_BIN);
    }
}
