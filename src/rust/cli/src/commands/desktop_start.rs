//! desktop-start — start the on-demand remote desktop(s). A port of
//! packages/cli/src/desktop-start.ts.
//!
//!   desktop-start [selkies|vnc]   (no arg = every installed stack, "both" mode)

use sdw_core::desktop::domain::DesktopTarget;
use sdw_core::desktop::layout::{DESKTOP_CACHE_DIR, SELKIES_HOME, VNC_HOME};
use sdw_core::shared::layout::WORKSPACE_MOUNT;
use sdw_core::workspace::layout::CODE_SERVER_CONFIG_PATH;
use sdw_infra::cmd;
use sdw_infra::desktop::{
    ensure_kasm_passwd, installed_stacks, is_desktop_running, parse_target, start_desktop,
    target_name, wait_for_desktop_stream,
};
use std::fs;
use std::path::Path;

fn url_for(target: DesktopTarget) -> &'static str {
    match target {
        DesktopTarget::Selkies => "/desktop/",
        DesktopTarget::Vnc => "/vnc/",
    }
}

/// Read the master password from a code-server config (the runtime SSOT): the line `password: X`.
fn parse_code_server_password(content: &str) -> String {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("password:") {
            return rest.trim().to_string();
        }
    }
    String::new()
}

fn master_password_from_config() -> String {
    match fs::read_to_string(CODE_SERVER_CONFIG_PATH) {
        Ok(content) => parse_code_server_password(&content),
        Err(_) => String::new(),
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

    let _ = fs::create_dir_all(DESKTOP_CACHE_DIR);
    if targets.contains(&DesktopTarget::Selkies) {
        let _ = fs::create_dir_all(format!("{SELKIES_HOME}/.local/share/applications"));
    }
    if targets.contains(&DesktopTarget::Vnc) {
        let _ = fs::create_dir_all(format!("{VNC_HOME}/.local/share/applications"));
        if !Path::new(&format!("{VNC_HOME}/.kasmpasswd")).exists() {
            ensure_kasm_passwd(&master_password_from_config());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_code_server_password_line() {
        let config = "bind-addr: 127.0.0.1:8082\npassword: s3cret-pass\nauth: password\n";
        assert_eq!(parse_code_server_password(config), "s3cret-pass");
    }

    #[test]
    fn absent_password_is_empty() {
        assert_eq!(parse_code_server_password("auth: none\n"), "");
    }
}
