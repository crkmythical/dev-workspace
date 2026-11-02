//! Egress adapters — a port of packages/infra/src/egress.ts. The Clash/mihomo implementation of
//! EgressPorts, a quick live observation for `doctor`, and the two places the start-up result is
//! left (a status file and a note inside the vault). What counts as confirmed/degraded/down is
//! decided in sdw_core::egress.
//!
//! The controller is plain HTTP on localhost, so it is driven with curl (and the JSON parsed with
//! serde_json) rather than pulling an HTTP/TLS client into the static binary.

use crate::cmd;
use crate::notifications::{remove_note, write_note};
use crate::time::now_iso;
use sdw_core::egress::domain::{
    egress_notice, parse_egress_report, serialize_egress_report, EgressObservation, EgressReport,
};
use sdw_core::egress::firewall::egress_ruleset;
use sdw_core::egress::layout::{
    CLASH_AUTO_GROUP, CLASH_BIN, CLASH_CONFIG_PATH, CLASH_CONTROLLER_PORT, CLASH_DIR,
    CLASH_HTTP_PORT, CLASH_PROVIDER_NAME, EGRESS_NOTIFICATION_PATH, EGRESS_PROBE_URL,
    EGRESS_STATUS_PATH,
};
use sdw_core::egress::ports::EgressPorts;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

fn port_accepts(port: u16) -> bool {
    cmd::ok("nc", &["-z", "127.0.0.1", &port.to_string()])
}

fn proxy_url() -> String {
    format!("http://127.0.0.1:{CLASH_HTTP_PORT}")
}

fn controller_url() -> String {
    format!("http://127.0.0.1:{CLASH_CONTROLLER_PORT}")
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
        // A daemon: it must outlive this process, so spawn detached and do not wait on it. Under
        // enforced egress, run it as the clash uid so the firewall can single out its traffic; the
        // clash binary carries cap_net_bind_service (set in the image), so it can still bind the
        // DNS port 53 as a non-root process.
        let mut command = Command::new(CLASH_BIN);
        command
            .args(["-d", CLASH_DIR])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(uid) = self.clash_uid {
            command.uid(uid);
        }
        command.spawn().map(|_child| ()).map_err(|e| e.to_string())
    }

    fn proxy_listening(&self) -> Result<bool, String> {
        Ok(port_accepts(CLASH_HTTP_PORT))
    }

    fn provider_nodes(&self) -> Result<Option<i64>, String> {
        let url = format!(
            "{}/providers/proxies/{CLASH_PROVIDER_NAME}",
            controller_url()
        );
        let (ok, body) = cmd::capture("curl", &["-fsS", &url]);
        if !ok {
            return Ok(None);
        }
        let data: serde_json::Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(_) => return Ok(None),
        };
        // Matches Object.keys(proxies ?? {}).length: an array or object, counted; else zero.
        let count = match data.get("proxies") {
            Some(serde_json::Value::Array(a)) => a.len(),
            Some(serde_json::Value::Object(o)) => o.len(),
            _ => 0,
        };
        Ok(Some(count as i64))
    }

    fn refresh_provider(&self) -> Result<(), String> {
        let url = format!(
            "{}/providers/proxies/{CLASH_PROVIDER_NAME}",
            controller_url()
        );
        if cmd::ok("curl", &["-fsS", "-X", "PUT", &url]) {
            Ok(())
        } else {
            Err(format!("refresh request to {url} failed"))
        }
    }

    fn select_auto_node(&self) -> Result<(), String> {
        let url = format!("{}/proxies/GLOBAL", controller_url());
        let body = format!("{{\"name\":\"{CLASH_AUTO_GROUP}\"}}");
        if cmd::ok(
            "curl",
            &[
                "-fsS",
                "-X",
                "PUT",
                &url,
                "-H",
                "Content-Type: application/json",
                "-d",
                &body,
            ],
        ) {
            Ok(())
        } else {
            Err(format!(
                "pointing GLOBAL at {CLASH_AUTO_GROUP} failed; traffic may stay DIRECT"
            ))
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

/// A fast look at the state right now (seconds, not the start-up's patient waits). `-f` makes an
/// HTTP error from the proxy count as a failed probe.
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
            "-sf",
            "--connect-timeout",
            "2",
            "--max-time",
            "5",
            "-x",
            &proxy_url(),
            "http://www.gstatic.com/generate_204",
            "-o",
            "/dev/null",
        ],
    );
    EgressObservation {
        proxy_listening,
        probe_ok,
    }
}

/// Boot probe for DNS auto-detection: can we reach the GLOBAL DoH resolvers directly? A DoH query
/// is sent straight to 1.1.1.1 / 8.8.8.8 (both IPs, so no name to resolve first), bypassing the
/// Clash proxy env vars (--noproxy, since Clash may not be up yet). On a censored network the TLS
/// to them is reset and both fail fast, so the caller falls back to domestic resolvers. true as
/// soon as one responds.
pub fn foreign_dns_reachable() -> bool {
    for ip in ["1.1.1.1", "8.8.8.8"] {
        let url = format!("https://{ip}/dns-query?name=www.cloudflare.com&type=A");
        if cmd::ok(
            "curl",
            &[
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
            ],
        ) {
            return true;
        }
    }
    false
}

/// Hot-reload Clash from its on-disk config via the controller API (a full config reload, so DNS
/// re-initializes — no dropped process). false when the controller is not reachable (Clash not up
/// yet); the next (re)start reads the new file regardless.
pub fn reload_clash() -> bool {
    let url = format!("{}/configs?force=true", controller_url());
    let body = format!("{{\"path\":\"{CLASH_CONFIG_PATH}\"}}");
    cmd::ok(
        "curl",
        &[
            "-fsS",
            "-X",
            "PUT",
            &url,
            "-H",
            "Content-Type: application/json",
            "-d",
            &body,
        ],
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

/// Install the fail-closed egress firewall (nftables default-DROP OUTPUT; only loopback, the Clash
/// uid, and established replies leave the box). Needs CAP_NET_ADMIN and `nft`; returns Err (so the
/// boot can fail closed) when either is missing.
pub fn apply_egress_firewall(clash_uid: u32) -> Result<(), String> {
    let (ok, stderr) = cmd::run_stdin("nft", &["-f", "-"], &egress_ruleset(clash_uid));
    if ok {
        Ok(())
    } else {
        Err(format!("nft load failed: {stderr}"))
    }
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
