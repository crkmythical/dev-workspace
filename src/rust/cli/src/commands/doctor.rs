//! doctor — workspace health check. A port of packages/cli/src/doctor.ts. Pure orchestration over
//! core's health verdicts and the infra probes; prints a checklist and exits 1 on any failure.

use crate::commands::system_env;
use sdw_core::audit::{rfc3339_ms, summarize, AUDIT_LOG_PATH};
use sdw_core::backup::domain::{
    evaluate_backup_gate, plan_doctor_backup_status, DoctorBackupState, DoctorBackupStatus,
};
use sdw_core::backup::layout::BACKUP_STATE_DIR;
use sdw_core::desktop::domain::{desktop_stream_port, DesktopTarget};
use sdw_core::desktop::layout::{SELKIES_DISPLAY, SELKIES_HEALTH_PATH, VNC_DISPLAY};
use sdw_core::egress::domain::{egress_health, egress_status, Health};
use sdw_core::egress::layout::CLASH_HTTP_PORT;
use sdw_core::plugins::plugins;
use sdw_core::shared::layout::WORKSPACE_MOUNT;
use sdw_core::sync::layout::SYNC_SERVICE_PORT;
use sdw_core::vault::domain::VaultState;
use sdw_core::vault::layout::{VAULT_CIPHER_DIR, VAULT_SYNC_STATE_DIR};
use sdw_core::vault::ports::VolumeDriver;
use sdw_core::workspace::config::load_config;
use sdw_core::workspace::layout::{code_server_selected, CODE_SERVER_PORT, EDITOR_SUITE_PATH};
use sdw_infra::audit::read_recent_lines;
use sdw_infra::cmd;
use sdw_infra::desktop::{installed_stacks, is_any_desktop_running, is_desktop_running};
use sdw_infra::egress::{egress_firewall_loaded, observe_egress, read_egress_status};
use sdw_infra::gocryptfs::GocryptfsVolume;
use sdw_infra::time::{now_ms, parse_iso_to_ms};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    Warn,
    Fail,
}

struct Row {
    component: String,
    status: Status,
    detail: Option<String>,
}

impl Row {
    fn new(component: &str, status: Status, detail: Option<String>) -> Self {
        Row {
            component: component.to_string(),
            status,
            detail,
        }
    }
}

fn from_health(h: Health) -> Status {
    match h {
        Health::Ok => Status::Ok,
        Health::Warn => Status::Warn,
        Health::Fail => Status::Fail,
    }
}

fn du_size(dir: &str) -> String {
    let (_, out) = cmd::capture("du", &["-sh", dir]);
    out.split('\t').next().unwrap_or("").trim().to_string()
}

fn display_active(display: &str) -> bool {
    Command::new("xdpyinfo")
        .env("DISPLAY", display)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

struct StackMeta {
    x_label: &'static str,
    stream_label: &'static str,
    display: &'static str,
    /// A URL path the stream server answers on, when it has one: an open port only shows that
    /// something listens, this shows that the server itself is up.
    health_path: Option<&'static str>,
}

fn stack_meta(target: DesktopTarget) -> StackMeta {
    match target {
        DesktopTarget::Selkies => StackMeta {
            x_label: "Desktop X server (Xvfb)",
            stream_label: "Desktop stream (Selkies)",
            display: SELKIES_DISPLAY,
            health_path: Some(SELKIES_HEALTH_PATH),
        },
        DesktopTarget::Vnc => StackMeta {
            x_label: "VNC X/server (Xkasmvnc)",
            stream_label: "VNC stream (KasmVNC)",
            display: VNC_DISPLAY,
            health_path: None,
        },
    }
}

pub fn run(_args: &[String]) -> i32 {
    let config = load_config(system_env);
    let mut results: Vec<Row> = Vec::new();

    // 1. Egress — the LIVE proxy path, next to what the start-up probe saw.
    let health = egress_health(
        egress_status(observe_egress()),
        read_egress_status().as_ref(),
        CLASH_HTTP_PORT,
    );
    results.push(Row::new(
        "Egress (Clash proxy)",
        from_health(health.status),
        Some(health.detail),
    ));

    // 1b. The egress firewall: is the filter that stops direct egress really loaded?
    results.push(firewall_row(
        config.features.egress_enforce,
        egress_firewall_loaded(),
    ));

    // 2. Vault mount
    if cmd::ok("mountpoint", &["-q", WORKSPACE_MOUNT]) {
        results.push(Row::new(
            "Vault mount",
            Status::Ok,
            Some(format!("{WORKSPACE_MOUNT} mounted")),
        ));
    } else {
        results.push(Row::new(
            "Vault mount",
            Status::Warn,
            Some("not mounted — run unlock-vault".to_string()),
        ));
    }

    // 3. Optional browser editor
    let editor_marker = fs::read_to_string(EDITOR_SUITE_PATH).ok();
    if !code_server_selected(editor_marker.as_deref()) {
        results.push(Row::new(
            "code-server",
            Status::Ok,
            Some("not installed in this image profile".to_string()),
        ));
    } else {
        let cs_url = format!("http://127.0.0.1:{CODE_SERVER_PORT}/healthz");
        if cmd::ok(
            "curl",
            &["-s", "--connect-timeout", "2", &cs_url, "-o", "/dev/null"],
        ) {
            results.push(Row::new("code-server", Status::Ok, None));
        } else {
            results.push(Row::new(
                "code-server",
                Status::Fail,
                Some(format!("port {CODE_SERVER_PORT} not responding")),
            ));
        }
    }

    // 4. Sync service (optional)
    if config.features.sync {
        let sync_url = format!("http://127.0.0.1:{SYNC_SERVICE_PORT}/sync/api/doctor");
        if cmd::ok(
            "curl",
            &["-s", "--connect-timeout", "2", &sync_url, "-o", "/dev/null"],
        ) {
            results.push(Row::new("Sync service", Status::Ok, None));
        } else {
            results.push(Row::new(
                "Sync service",
                Status::Fail,
                Some(format!("port {SYNC_SERVICE_PORT} not responding")),
            ));
        }
    } else {
        results.push(Row::new(
            "Sync service",
            Status::Ok,
            Some("disabled (set SYNC_ENABLED=1 to enable)".to_string()),
        ));
    }

    // 5. Last vault sync
    let last_sync_file = format!("{VAULT_SYNC_STATE_DIR}/last-success");
    match read_trimmed(&last_sync_file) {
        Some(ts) => {
            let stale = parse_iso_to_ms(&ts).is_none_or(|ms| now_ms() - ms >= 7_200_000);
            if stale {
                results.push(Row::new(
                    "Vault sync",
                    Status::Warn,
                    Some(format!("last: {ts} (>2h ago)")),
                ));
            } else {
                results.push(Row::new(
                    "Vault sync",
                    Status::Ok,
                    Some(format!("last: {ts}")),
                ));
            }
        }
        None => results.push(Row::new(
            "Vault sync",
            Status::Warn,
            Some("no sync recorded".to_string()),
        )),
    }

    // 6. Disk usage
    if Path::new(VAULT_CIPHER_DIR).exists() {
        results.push(Row::new(
            "Vault disk",
            Status::Ok,
            Some(du_size(VAULT_CIPHER_DIR)),
        ));
    }

    // 7. Desktop(s), one report per installed stack.
    for target in installed_stacks() {
        let meta = stack_meta(target);
        if is_desktop_running(target) {
            results.push(Row::new(
                meta.x_label,
                Status::Ok,
                Some("running".to_string()),
            ));

            let port = desktop_stream_port(target);
            if cmd::ok("nc", &["-z", "127.0.0.1", &port.to_string()]) {
                results.push(Row::new(
                    meta.stream_label,
                    Status::Ok,
                    Some(format!("port {port} listening")),
                ));
            } else {
                results.push(Row::new(
                    meta.stream_label,
                    Status::Fail,
                    Some(format!("port {port} not responding")),
                ));
            }

            // The stream server itself (Selkies serves its client and API from the same port).
            if let Some(path) = meta.health_path {
                let url = format!("http://127.0.0.1:{port}{path}");
                if cmd::ok(
                    "curl",
                    &["-sf", "--connect-timeout", "2", &url, "-o", "/dev/null"],
                ) {
                    results.push(Row::new(
                        "Desktop client (HTTP)",
                        Status::Ok,
                        Some(format!("answers at {path}")),
                    ));
                } else {
                    results.push(Row::new(
                        "Desktop client (HTTP)",
                        Status::Warn,
                        Some(format!("port {port} is open but {path} does not answer")),
                    ));
                }
            }

            let display_label = format!("Desktop Display {}", meta.display);
            if display_active(meta.display) {
                results.push(Row::new(
                    &display_label,
                    Status::Ok,
                    Some("active".to_string()),
                ));
            } else {
                results.push(Row::new(
                    &display_label,
                    Status::Warn,
                    Some("not responding".to_string()),
                ));
            }
        } else {
            results.push(Row::new(
                &format!("Desktop [{}]", stack_name(target)),
                Status::Ok,
                Some("not started (optional)".to_string()),
            ));
        }
    }

    // Window-manager / panel health (shared XFCE binaries) — catches the "fake-alive" session.
    if is_any_desktop_running() {
        let wm_ok = cmd::ok("pgrep", &["-x", "xfwm4"]) && cmd::ok("pgrep", &["-x", "xfce4-panel"]);
        if wm_ok {
            results.push(Row::new(
                "Desktop session",
                Status::Ok,
                Some("xfwm4 + xfce4-panel running".to_string()),
            ));
        } else {
            results.push(Row::new(
                "Desktop session",
                Status::Warn,
                Some(
                    "xfwm4/xfce4-panel not running — restart: desktop-stop && desktop-start"
                        .to_string(),
                ),
            ));
        }
    }

    // 8a. Plugin vaults (optional extra encrypted volumes), one row each.
    for plugin in plugins() {
        for vault in &plugin.vaults {
            let component = format!("{} vault", capitalize(&vault.id));
            let state = GocryptfsVolume.state(vault);
            let (status, label) = plugin_vault_status(&state);
            let detail = if matches!(state, VaultState::Unlocked) {
                format!("mounted ({})", du_size(&vault.mount_point))
            } else {
                label.to_string()
            };
            results.push(Row::new(&component, status, Some(detail)));
        }
    }

    // 8b. Backup (S3/restic)
    let backup_gate = evaluate_backup_gate(&config.backup);
    let last_success = read_trimmed(&format!("{BACKUP_STATE_DIR}/last-success"));
    let failures: u32 = read_trimmed(&format!("{BACKUP_STATE_DIR}/consecutive-failures"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let backup_status = plan_doctor_backup_status(&DoctorBackupState {
        gate_result: backup_gate,
        last_success_ms: last_success.as_deref().and_then(parse_iso_to_ms),
        consecutive_failures: failures,
        now_ms: now_ms(),
    });
    let (backup_row_status, backup_detail) = match backup_status {
        DoctorBackupStatus::NotConfigured => (Status::Ok, "not configured (optional)".to_string()),
        DoctorBackupStatus::Ok => (
            Status::Ok,
            format!("last: {}", last_success.as_deref().unwrap_or("")),
        ),
        DoctorBackupStatus::Warn => (
            Status::Warn,
            match &last_success {
                Some(ts) => format!("last: {ts} (stale)"),
                None => "no backup recorded".to_string(),
            },
        ),
        DoctorBackupStatus::Fail => (Status::Fail, format!("{failures} consecutive failures")),
    };
    results.push(Row::new(
        "Backup (S3/restic)",
        backup_row_status,
        Some(backup_detail),
    ));

    // 9. Audit trail — who logged in or tried to, self-destruct calls, egress changes (24 h).
    results.push(audit_row(
        &read_recent_lines(Path::new(AUDIT_LOG_PATH), 1 << 20),
        now_ms(),
    ));

    // Output
    println!("=== Workspace Health Check ===");
    let mut has_failure = false;
    for row in &results {
        let icon = match row.status {
            Status::Ok => "✓",
            Status::Warn => "⚠",
            Status::Fail => "✗",
        };
        match &row.detail {
            Some(d) => println!("  {icon} {} — {d}", row.component),
            None => println!("  {icon} {}", row.component),
        }
        if row.status == Status::Fail {
            has_failure = true;
        }
    }
    println!(
        "{}",
        if has_failure {
            "\nSome checks failed."
        } else {
            "\nAll checks passed."
        }
    );

    if config.git.vault_repo.is_none() {
        println!("\n⚠ VAULT_GIT_REPO not configured — vault data is NOT backed up.");
        println!("  If Docker Desktop updates or volumes are lost, data is gone.");
        println!("  Set VAULT_GIT_REPO in .env to enable auto-backup.");
    }

    i32::from(has_failure)
}

/// The egress firewall as one row, from what was asked for (`EGRESS_ENFORCE`, on by default) and
/// what is actually loaded. Wanted but absent is a failure: the workspace believes its egress is
/// closed and it is open (root with NET_ADMIN deleted the table, or the boot never loaded it).
/// Off on purpose is a warning, because it is a state to know about, not one that is broken.
fn firewall_row(wanted: bool, loaded: bool) -> Row {
    let component = "Egress firewall";
    match (wanted, loaded) {
        (true, true) => Row::new(
            component,
            Status::Ok,
            Some("enforced (direct egress blocked)".to_string()),
        ),
        (true, false) => Row::new(
            component,
            Status::Fail,
            Some(
                "wanted but NOT loaded: direct egress is open — restart the container \
                 (docker compose restart), the boot loads it"
                    .to_string(),
            ),
        ),
        (false, false) => Row::new(
            component,
            Status::Warn,
            Some(
                "off (EGRESS_ENFORCE=0): a process that ignores the proxy variables connects \
                 directly"
                    .to_string(),
            ),
        ),
        (false, true) => Row::new(
            component,
            Status::Warn,
            Some("loaded although EGRESS_ENFORCE=0 here; it is gone after a restart".to_string()),
        ),
    }
}

/// The audit trail of the last 24 hours as one row: who logged in or tried to, and what the
/// self-destruct endpoint and the egress monitor saw. Quiet unless something looks like an attack
/// (ten wrong passwords, a throttled login, any self-destruct call) or a destruct actually fired.
fn audit_row(lines: &[String], now: i64) -> Row {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    let component = "Audit trail";
    if lines.is_empty() {
        return Row::new(component, Status::Ok, Some("no events recorded yet".into()));
    }
    let s = summarize(lines.iter().map(String::as_str), now, DAY_MS);
    let stamp = |ms: i64| rfc3339_ms(ms)[..19].replace('T', " ") + "Z";
    let mut detail = format!("24h: {} logins, {} failed", s.logins_ok, s.logins_failed);
    if s.logins_throttled > 0 {
        detail += &format!(", {} throttled", s.logins_throttled);
    }
    if s.access_ok > 0 {
        detail += &format!(", {} via Access", s.access_ok);
    }
    if let Some((t, ip)) = &s.last_failed_login {
        detail += &format!(
            "; last wrong password {} from {}",
            stamp(*t),
            ip.as_deref().unwrap_or("?")
        );
    }
    if s.destruct_attempts > 0 {
        detail += &format!(
            "; {} self-destruct call(s), {} fired",
            s.destruct_attempts, s.destruct_triggered
        );
    }
    if let Some((t, change)) = &s.last_egress {
        detail += &format!("; egress {change} at {}", stamp(*t));
    }
    let suspicious = s.logins_failed >= 10 || s.logins_throttled > 0 || s.destruct_attempts > 0;
    let status = if suspicious { Status::Warn } else { Status::Ok };
    if suspicious {
        detail += " (run `audit-log` for the details)";
    }
    Row::new(component, status, Some(detail))
}

fn stack_name(target: DesktopTarget) -> &'static str {
    match target {
        DesktopTarget::Selkies => "selkies",
        DesktopTarget::Vnc => "vnc",
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Health row for an optional plugin vault. It never FAILS (the vault is opt-in; a locked or
/// uninitialized one is a normal resting state), but it reports the TRUE state rather than lumping
/// a `Destroyed` vault in with "not initialized". The caller substitutes the mount size for the
/// Unlocked detail.
fn plugin_vault_status(state: &VaultState) -> (Status, &'static str) {
    match state {
        VaultState::Unlocked => (Status::Ok, "mounted"),
        VaultState::Locked => (Status::Ok, "locked (initialized)"),
        VaultState::Uninitialized => (Status::Ok, "not initialized (optional)"),
        VaultState::Destroyed => (Status::Warn, "destroyed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdw_core::audit::AuditEvent;

    #[test]
    fn plugin_vault_row_never_fails_and_distinguishes_destroyed() {
        // Optional vault: unlocked / locked / uninitialized are all fine, never a failure.
        assert_eq!(plugin_vault_status(&VaultState::Unlocked).0, Status::Ok);
        assert_eq!(plugin_vault_status(&VaultState::Locked).0, Status::Ok);
        assert_eq!(
            plugin_vault_status(&VaultState::Uninitialized).0,
            Status::Ok
        );
        // ...but a destroyed vault is a distinct, notable state, not "not initialized".
        assert_eq!(
            plugin_vault_status(&VaultState::Destroyed),
            (Status::Warn, "destroyed")
        );
    }

    #[test]
    fn the_firewall_row_fails_only_when_wanted_and_missing() {
        assert_eq!(firewall_row(true, true).status, Status::Ok);
        let missing = firewall_row(true, false);
        assert_eq!(missing.status, Status::Fail);
        assert!(missing.detail.unwrap().contains("direct egress is open"));
        // Off on purpose is a state to know about, not a broken workspace.
        let off = firewall_row(false, false);
        assert_eq!(off.status, Status::Warn);
        assert!(off.detail.unwrap().contains("EGRESS_ENFORCE=0"));
        assert_eq!(firewall_row(false, true).status, Status::Warn);
    }

    fn line(ts: i64, ev: &'static str, out: &'static str, ip: Option<&str>) -> String {
        AuditEvent::new(ts, ev, out)
            .with_opt("ip", ip)
            .to_json_line()
    }

    #[test]
    fn audit_row_is_quiet_when_nothing_happened_or_all_is_routine() {
        let now = 1_000_000_000;
        let empty = audit_row(&[], now);
        assert_eq!(empty.status, Status::Ok);
        assert!(empty.detail.unwrap().contains("no events"));

        let routine = vec![
            line(now - 10, "auth.login", "success", Some("203.0.113.1")),
            line(now - 20, "auth.login", "failure", Some("203.0.113.1")),
        ];
        let row = audit_row(&routine, now);
        assert_eq!(row.status, Status::Ok);
        let d = row.detail.unwrap();
        assert!(d.contains("1 logins") && d.contains("1 failed"), "{d}");
    }

    #[test]
    fn audit_row_warns_on_guessing_throttling_and_destruct_attempts() {
        let now = 1_000_000_000;
        let guessing: Vec<String> = (0..10)
            .map(|i| line(now - i, "auth.login", "failure", Some("198.51.100.9")))
            .collect();
        let row = audit_row(&guessing, now);
        assert_eq!(row.status, Status::Warn);
        let d = row.detail.unwrap();
        assert!(d.contains("198.51.100.9") && d.contains("10 failed"), "{d}");

        let throttled = vec![line(now - 5, "auth.login", "throttled", None)];
        assert_eq!(audit_row(&throttled, now).status, Status::Warn);

        let destruct = vec![line(now - 5, "destruct.attempt", "rejected", None)];
        let row = audit_row(&destruct, now);
        assert_eq!(row.status, Status::Warn);
        assert!(row.detail.unwrap().contains("self-destruct"));
    }

    #[test]
    fn audit_row_ignores_events_older_than_a_day() {
        let now = 10 * 86_400_000;
        let old: Vec<String> = (0..50)
            .map(|_| line(now - 2 * 86_400_000, "auth.login", "failure", None))
            .collect();
        assert_eq!(audit_row(&old, now).status, Status::Ok);
    }
}
