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
use sdw_core::egress::layout::{
    CLASH_AUTO_GROUP, CLASH_BIN, CLASH_CONTROLLER_PORT, CLASH_DIR, CLASH_HTTP_PORT,
    CLASH_PROVIDER_NAME, EGRESS_NOTIFICATION_PATH, EGRESS_PROBE_URL, EGRESS_STATUS_PATH,
};
use sdw_core::egress::ports::EgressPorts;
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

/// EgressPorts backed by Clash. `log` is where the bring-up narrates its progress.
pub struct ClashEgressPorts<L: Fn(&str)> {
    log: L,
}

pub fn clash_egress_ports<L: Fn(&str)>(log: L) -> ClashEgressPorts<L> {
    ClashEgressPorts { log }
}

impl<L: Fn(&str)> EgressPorts for ClashEgressPorts<L> {
    fn start_proxy(&self) -> Result<(), String> {
        // A daemon: it must outlive this process, so spawn detached and do not wait on it.
        Command::new(CLASH_BIN)
            .args(["-d", CLASH_DIR])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_child| ())
            .map_err(|e| e.to_string())
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
