//! Desktop adapter: supervisorctl control of the remote-desktop process groups — a port of
//! packages/infra/src/desktop.ts. The topology and ordering rules are pure (crate sdw_core
//! desktop::domain); this drives supervisorctl in that order.

use crate::cmd;
use sdw_core::desktop::domain::{
    desktop_anchor, desktop_group, desktop_start_order, desktop_stop_order, desktop_stream_port,
    parse_stack_marker, DesktopTarget,
};
use sdw_core::desktop::layout::{DESKTOP_STACK_MARKER_PATH, VNC_HOME};
use std::thread::sleep;
use std::time::Duration;

/// The lowercase name of a stack, as used in CLI args and user-facing messages.
pub fn target_name(target: DesktopTarget) -> &'static str {
    match target {
        DesktopTarget::Selkies => "selkies",
        DesktopTarget::Vnc => "vnc",
    }
}

/// Parse a CLI target argument ("selkies" | "vnc").
pub fn parse_target(s: &str) -> Option<DesktopTarget> {
    match s {
        "selkies" => Some(DesktopTarget::Selkies),
        "vnc" => Some(DesktopTarget::Vnc),
        _ => None,
    }
}

/// Which stacks are installed in this image (from the build-time marker).
pub fn installed_stacks() -> Vec<DesktopTarget> {
    let raw = std::fs::read_to_string(DESKTOP_STACK_MARKER_PATH).ok();
    parse_stack_marker(raw.as_deref())
}

/// Resolve a caller target argument to the concrete list of stacks to act on.
fn resolve_targets(target: Option<DesktopTarget>) -> Vec<DesktopTarget> {
    let installed = installed_stacks();
    match target {
        None => installed,
        Some(t) if installed.contains(&t) => vec![t],
        Some(_) => vec![],
    }
}

/// Whether a specific stack's X server (base of the chain) is RUNNING.
pub fn is_desktop_running(target: DesktopTarget) -> bool {
    let (ok, out) = cmd::capture("supervisorctl", &["status", desktop_anchor(target)]);
    ok && out.contains("RUNNING")
}

/// Whether ANY installed stack is currently running.
pub fn is_any_desktop_running() -> bool {
    installed_stacks().into_iter().any(is_desktop_running)
}

/// Start one stack in dependency order. Caller handles readiness.
fn start_stack(target: DesktopTarget) {
    let order = desktop_start_order(target);
    cmd::ok("supervisorctl", &["start", order[0]]);
    sleep(Duration::from_millis(2000));
    for program in &order[1..] {
        cmd::ok("supervisorctl", &["start", program]);
    }
}

/// Start desktop(s). None starts every installed stack ("both" mode); Some starts only that one.
pub fn start_desktop(target: Option<DesktopTarget>) {
    for t in resolve_targets(target) {
        start_stack(t);
    }
}

/// Stop one stack (reverse dependency order). Safe when not running.
fn stop_stack(target: DesktopTarget) {
    let mut args = vec!["stop"];
    args.extend(desktop_stop_order(target));
    cmd::ok("supervisorctl", &args);
}

/// Stop desktop(s). None stops every installed stack; Some stops only that one.
pub fn stop_desktop(target: Option<DesktopTarget>) {
    for t in resolve_targets(target) {
        stop_stack(t);
    }
}

/// Stop ALL desktop stacks before a FUSE unmount (lock-vault / destroy). Critical for EBUSY
/// safety: every XFCE session holding a HOME fd on the gocryptfs mount must exit first.
pub fn stop_all_desktops() {
    for t in installed_stacks() {
        stop_stack(t);
    }
}

/// Names of any desktop programs still RUNNING across all groups (post-stop check).
pub fn running_desktop_programs() -> Vec<String> {
    let mut groups: Vec<&str> = Vec::new();
    for t in installed_stacks() {
        let g = desktop_group(t);
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    let mut running = Vec::new();
    for g in groups {
        let (_, out) = cmd::capture("supervisorctl", &["status", &format!("{g}:*")]);
        for line in out.lines() {
            if line.contains("RUNNING") {
                if let Some(name) = line.split_whitespace().next() {
                    running.push(name.to_string());
                }
            }
        }
    }
    running
}

/// Poll until a stack's WebSocket port accepts TCP connections, or time out.
pub fn wait_for_desktop_stream(target: DesktopTarget, timeout_sec: u32) -> bool {
    let port = desktop_stream_port(target).to_string();
    for _ in 0..timeout_sec {
        if cmd::ok("nc", &["-z", "127.0.0.1", &port]) {
            return true;
        }
        sleep(Duration::from_secs(1));
    }
    false
}

/// Write KasmVNC's native HTTP Basic Auth password file ($VNC_HOME/.kasmpasswd, user "user") from
/// the plaintext password. KasmVNC owns the /vnc/ auth end-to-end, so this must exist for the VNC
/// desktop to authenticate. Idempotent.
pub fn ensure_kasm_passwd(password: &str) {
    if password.is_empty() {
        return;
    }
    let kasmpasswd = format!("{VNC_HOME}/.kasmpasswd");
    cmd::run_stdin(
        "kasmvncpasswd",
        &["-u", "user", "-w", "-r", &kasmpasswd],
        &format!("{password}\n{password}\n"),
    );
    cmd::ok("chmod", &["0600", &kasmpasswd]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_name_and_parse_round_trip() {
        for t in [DesktopTarget::Selkies, DesktopTarget::Vnc] {
            assert_eq!(parse_target(target_name(t)), Some(t));
        }
        assert_eq!(parse_target("nonsense"), None);
    }
}
