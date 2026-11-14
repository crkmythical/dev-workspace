//! desktop-start — start the on-demand remote desktop(s). A port of
//! packages/cli/src/desktop-start.ts.
//!
//!   desktop-start [selkies|vnc]   (no arg = every installed stack, "both" mode)

use sdw_core::desktop::domain::DesktopTarget;
use sdw_core::desktop::layout::{DESKTOP_CACHE_DIR, SELKIES_HOME, VNC_HOME};
use sdw_core::shared::layout::WORKSPACE_MOUNT;
use sdw_core::workspace::workload::{WORKLOAD_GID, WORKLOAD_UID};
use sdw_infra::cmd;
use sdw_infra::desktop::{
    installed_stacks, is_desktop_running, parse_target, start_desktop, target_name,
    wait_for_desktop_stream,
};
use std::fs;
use std::os::unix::fs::{chown, PermissionsExt};

fn url_for(target: DesktopTarget) -> &'static str {
    match target {
        DesktopTarget::Selkies => "/desktop/",
        DesktopTarget::Vnc => "/vnc/",
    }
}

pub fn run(args: &[String]) -> i32 {
    if !cmd::ok("mountpoint", &["-q", WORKSPACE_MOUNT]) {
        eprintln!("ERROR: Vault not mounted. Run 'unlock-vault' first.");
        return 1;
    }

    let target = match args.first().map(String::as_str) {
        None => None,
        Some(a) => match parse_target(a) {
            Some(t) => Some(t),
            None => {
                eprintln!("ERROR: unknown target '{a}'. Use 'selkies' or 'vnc'.");
                return 1;
            }
        },
    };

    let installed = installed_stacks();
    if let Some(t) = target {
        if !installed.contains(&t) {
            eprintln!(
                "ERROR: stack '{}' is not installed in this image.",
                target_name(t)
            );
            return 1;
        }
    }
    let targets: Vec<DesktopTarget> = match target {
        Some(t) => vec![t],
        None => installed,
    };

    // The sessions run as the workload user: their runtime directory is theirs alone.
    let _ = fs::create_dir_all(DESKTOP_CACHE_DIR);
    let _ = fs::set_permissions(DESKTOP_CACHE_DIR, fs::Permissions::from_mode(0o700));
    let _ = chown(DESKTOP_CACHE_DIR, Some(WORKLOAD_UID), Some(WORKLOAD_GID));
    if targets.contains(&DesktopTarget::Selkies) {
        let _ = fs::create_dir_all(format!("{SELKIES_HOME}/.local/share/applications"));
    }
    if targets.contains(&DesktopTarget::Vnc) {
        let _ = fs::create_dir_all(format!("{VNC_HOME}/.local/share/applications"));
    }

    for target in targets {
        if is_desktop_running(target) {
            println!(
                "Desktop [{}] already running. Access at: {}",
                target_name(target),
                url_for(target)
            );
            continue;
        }
        println!("Starting desktop [{}]...", target_name(target));
        start_desktop(Some(target));
        if wait_for_desktop_stream(target, 30) {
            println!(
                "✓ Desktop [{}] ready. Access at: {}",
                target_name(target),
                url_for(target)
            );
        } else {
            eprintln!(
                "⚠ Desktop [{}] started but stream port not ready after 30s.",
                target_name(target)
            );
        }
    }
    0
}
