//! Desktop domain: the supervisord topology of the remote-desktop stacks and the start/stop
//! ordering. A port of packages/core/src/desktop/domain.ts. Pure (supervisorctl lives in infra).
//!
//! Stop order is always the exact reverse of start order, XFCE (the primary HOME-fd holder on the
//! gocryptfs mount) stopped first, so `fusermount -u /workspace` never hits EBUSY. Tests mirror
//! packages/core/tests/desktop-domain.test.ts.

pub const SELKIES_STREAM_PORT: u16 = 6080;
pub const VNC_STREAM_PORT: u16 = 6081;
/// Default KasmVNC geometry when DESKTOP_RESOLUTION is unset.
pub const DESKTOP_DEFAULT_RESOLUTION: &str = "1920x1080";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopTarget {
    Selkies,
    Vnc,
}

/// Program members per stack, in START order (group-qualified for supervisorctl). XFCE is always
/// last to start / first to stop.
const SELKIES_PROGRAMS: &[&str] = &[
    "desktop:desktop-xvfb",
    "desktop:desktop-audio",
    "desktop:desktop-selkies",
    "desktop:desktop-xfce",
];
const VNC_PROGRAMS: &[&str] = &["vnc:desktop-xvnc", "vnc:desktop-xfce-vnc"];

fn programs(target: DesktopTarget) -> &'static [&'static str] {
    match target {
        DesktopTarget::Selkies => SELKIES_PROGRAMS,
        DesktopTarget::Vnc => VNC_PROGRAMS,
    }
}

/// Which stacks an image contains, from the build-time marker file:
/// "both" → [selkies, vnc]; "kasmvnc" → [vnc]; anything else → legacy selkies-only.
pub fn parse_stack_marker(raw: Option<&str>) -> Vec<DesktopTarget> {
    match raw.map(str::trim) {
        Some("both") => vec![DesktopTarget::Selkies, DesktopTarget::Vnc],
        Some("kasmvnc") => vec![DesktopTarget::Vnc],
        _ => vec![DesktopTarget::Selkies],
    }
}

/// Supervisord group name for a stack.
pub fn desktop_group(target: DesktopTarget) -> &'static str {
    match target {
        DesktopTarget::Selkies => "desktop",
        DesktopTarget::Vnc => "vnc",
    }
}

/// WebSocket stream port for a stack.
pub fn desktop_stream_port(target: DesktopTarget) -> u16 {
    match target {
        DesktopTarget::Selkies => SELKIES_STREAM_PORT,
        DesktopTarget::Vnc => VNC_STREAM_PORT,
    }
}

/// Group-qualified name of the base program (X server) for a stack.
pub fn desktop_anchor(target: DesktopTarget) -> &'static str {
    programs(target)[0]
}

/// Start order for a stack (base/X server first).
pub fn desktop_start_order(target: DesktopTarget) -> &'static [&'static str] {
    programs(target)
}

/// Stop order for a stack: the exact reverse of the start order.
pub fn desktop_stop_order(target: DesktopTarget) -> Vec<&'static str> {
    programs(target).iter().rev().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STACKS: [DesktopTarget; 2] = [DesktopTarget::Selkies, DesktopTarget::Vnc];

    fn first_to_stop(target: DesktopTarget) -> &'static str {
        match target {
            DesktopTarget::Selkies => "desktop:desktop-xfce",
            DesktopTarget::Vnc => "vnc:desktop-xfce-vnc",
        }
    }
    fn program_count(target: DesktopTarget) -> usize {
        match target {
            DesktopTarget::Selkies => 4,
            DesktopTarget::Vnc => 2,
        }
    }

    #[test]
    fn fuse_safety_topology_for_both_stacks() {
        for stack in STACKS {
            // Stop order is the exact reverse of start order.
            let reversed: Vec<&str> = desktop_start_order(stack).iter().rev().copied().collect();
            assert_eq!(desktop_stop_order(stack), reversed);
            // XFCE (the primary HOME-fd holder) dies first.
            assert_eq!(desktop_stop_order(stack)[0], first_to_stop(stack));
            // Program counts.
            assert_eq!(desktop_start_order(stack).len(), program_count(stack));
            assert_eq!(desktop_stop_order(stack).len(), program_count(stack));
            // The anchor is the X server, first to start, inside the stack's own group.
            assert_eq!(desktop_anchor(stack), desktop_start_order(stack)[0]);
            assert!(desktop_anchor(stack).starts_with(&format!("{}:", desktop_group(stack))));
        }
    }

    #[test]
    fn stacks_are_independent() {
        let selkies: std::collections::HashSet<_> =
            desktop_start_order(DesktopTarget::Selkies).iter().collect();
        for p in desktop_start_order(DesktopTarget::Vnc) {
            assert!(!selkies.contains(p), "{p} should not be shared");
        }
        assert_ne!(
            desktop_group(DesktopTarget::Selkies),
            desktop_group(DesktopTarget::Vnc)
        );
        assert_ne!(
            desktop_stream_port(DesktopTarget::Selkies),
            desktop_stream_port(DesktopTarget::Vnc)
        );
    }

    #[test]
    fn parse_stack_marker_cases() {
        assert_eq!(
            parse_stack_marker(Some("both")),
            vec![DesktopTarget::Selkies, DesktopTarget::Vnc]
        );
        assert_eq!(
            parse_stack_marker(Some("selkies")),
            vec![DesktopTarget::Selkies]
        );
        assert_eq!(
            parse_stack_marker(Some("kasmvnc")),
            vec![DesktopTarget::Vnc]
        );
        assert_eq!(
            parse_stack_marker(Some("kasmvnc\n")),
            vec![DesktopTarget::Vnc]
        );
        assert_eq!(parse_stack_marker(None), vec![DesktopTarget::Selkies]);
        assert_eq!(parse_stack_marker(Some("")), vec![DesktopTarget::Selkies]);
        assert_eq!(
            parse_stack_marker(Some("nonsense")),
            vec![DesktopTarget::Selkies]
        );
    }
}
