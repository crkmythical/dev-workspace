//! argv[0] / subcommand routing for the multi-call binary.

/// The binary's own name. When invoked as itself (`sdw <command>`) the command is argv[1];
/// invoked through a command-named symlink, argv[0]'s basename is the command.
const BIN_NAME: &str = "sdw";

/// The commands that get a PATH symlink (the TS `bin` map). entrypoint / backup-watcher /
/// vault-sync are invoked as `sdw <name>` by their callers (entrypoint.sh, supervisord, cron), so
/// they are not symlinked.
const BIN_COMMANDS: &[&str] = &[
    "audit-log",
    "backup-init",
    "desktop-install",
    "desktop-start",
    "desktop-stop",
    "destruct-passphrase",
    "doctor",
    "init-pentest",
    "init-vault",
    "lock-pentest",
    "lock-vault",
    "pentest",
    "pentest-bootstrap",
    "pull",
    "push",
    "self-destruct",
    "set-clash-dns",
    "setup",
    "unlock-pentest",
    "unlock-vault",
];

/// `sdw install [dir]` — symlink every bin command in `dir` (default /usr/local/bin) to this
/// binary, so each command name on PATH dispatches via argv[0]. Replaces install-bins.ts.
fn install_symlinks(dir: &str) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("install: cannot find own path: {e}");
            return 1;
        }
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("install: cannot create {dir}: {e}");
        return 1;
    }
    for name in BIN_COMMANDS {
        let link = std::path::Path::new(dir).join(name);
        let _ = std::fs::remove_file(&link);
        if let Err(e) = std::os::unix::fs::symlink(&exe, &link) {
            eprintln!("install: symlink {name} failed: {e}");
            return 1;
        }
    }
    // The bridge: the root-only entries `doas` is allowed to run (see core::workspace::workload).
    if let Err(e) = std::fs::create_dir_all(PRIV_BRIDGE_DIR) {
        eprintln!("install: cannot create {PRIV_BRIDGE_DIR}: {e}");
        return 1;
    }
    for verb in PRIVILEGED_VERBS {
        let link = std::path::Path::new(PRIV_BRIDGE_DIR).join(verb);
        let _ = std::fs::remove_file(&link);
        if let Err(e) = std::os::unix::fs::symlink(&exe, &link) {
            eprintln!("install: symlink {verb} in {PRIV_BRIDGE_DIR} failed: {e}");
            return 1;
        }
    }
    println!(
        "Installed {} command symlinks in {dir} and {} bridge entries in {PRIV_BRIDGE_DIR} -> {}",
        BIN_COMMANDS.len(),
        PRIVILEGED_VERBS.len(),
        exe.display()
    );
    0
}

/// Every command the multi-call binary dispatches (the full surface). BIN_COMMANDS above is the
/// subset that also gets a PATH symlink; the rest are invoked as `sdw <name>`. Unknown = usage error.
const COMMANDS: &[&str] = &[
    "audit-log",
    "backup-init",
    "backup-watcher",
    "clash-watcher",
    "desktop-install",
    "desktop-start",
    "desktop-stop",
    "destruct-passphrase",
    "doas-rules",
    "doctor",
    "egress-monitor",
    "entrypoint",
    "env-example",
    "init-pentest",
    "init-vault",
    "lock-pentest",
    "lock-vault",
    "pentest",
    "pentest-bootstrap",
    "proxy-only",
    "pull",
    "push",
    "self-destruct",
    "set-clash-dns",
    "setup",
    "sync-service",
    "unlock-pentest",
    "unlock-vault",
    "vault-sync",
    "workload",
];
pub fn run(args: Vec<String>) -> i32 {
    let arg0 = args.first().map(|s| basename(s)).unwrap_or_default();
    let (command, rest) = if arg0 == BIN_NAME || arg0.is_empty() {
        match args.get(1) {
            Some(c) => (c.clone(), args[2..].to_vec()),
            None => {
                eprint_usage();
                return 2;
            }
        }
    } else {
        (arg0.to_string(), args[1..].to_vec())
    };
    if is_root() {
        // Started through the bridge on behalf of the workload: doas gave this process a clean
        // environment, so pick up the configuration the container was started with.
        if args.first().is_some_and(|a| is_bridge_invocation(a)) {
            sdw_infra::container_env::adopt();
        }
        return dispatch(&command, &rest);
    }
    as_workload(&command, &rest)
}

/// Commands that run as whoever called them: they need no privilege and have nothing to bridge.
/// `env-example` prints a file. `proxy-only` renders a config into a directory the caller must own
/// and runs Clash: in the standalone client the caller is the `clash` user (the command starts as
/// root and runs itself again as that user); in the workspace image the workload (`dev`) simply
/// fails to write into `/etc/clash`.
const RUN_AS_WORKLOAD: &[&str] = &["env-example", "proxy-only"];

/// A command typed by the unprivileged workload: the ones that need root go through the bridge
/// (`doas` runs the same-named entry under PRIV_BRIDGE_DIR as root, with a clean environment, if
/// /etc/doas.conf lists it); the operator's commands are refused with a pointer to `docker exec`.
fn as_workload(command: &str, args: &[String]) -> i32 {
    if PRIVILEGED_VERBS.contains(&command) {
        let err = std::process::Command::new("doas")
            .arg(format!("{PRIV_BRIDGE_DIR}/{command}"))
            .args(args)
            .exec();
        eprintln!("{command}: cannot reach the privileged side: {err}");
        return 126;
    }
    if RUN_AS_WORKLOAD.contains(&command) {
        return dispatch(command, args);
    }
    eprintln!(
        "{command}: this one is for the operator, not for code running in the workspace.\n\
         Run it from outside: docker exec -it <container> {command}"
    );
    126
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn dispatch(command: &str, args: &[String]) -> i32 {
    match command {
        "audit-log" => commands::audit_log::run(args),
        "backup-init" => commands::backup_init::run(args),
        "backup-watcher" => commands::backup_watcher::run(args),
        "clash-watcher" => commands::clash_watcher::run(args),
        "desktop-install" => commands::desktop_install::run(args),
        "desktop-start" => commands::desktop_start::run(args),
        "desktop-stop" => commands::desktop_stop::run(args),
        "destruct-passphrase" => commands::destruct_passphrase::run(args),
        "doas-rules" => {
            print!("{}", doas_rules());
            0
        }
        "doctor" => commands::doctor::run(args),
        "egress-monitor" => commands::egress_monitor::run(args),
        "entrypoint" => commands::entrypoint::run(args),
        "env-example" => commands::env_example::run(args),
        "install" => install_symlinks(args.first().map(String::as_str).unwrap_or("/usr/local/bin")),
        "init-pentest" => commands::init_pentest::run(args),
        "init-vault" => commands::init_vault::run(args),
        "lock-pentest" => commands::lock_pentest::run(args),
        "lock-vault" => commands::lock_vault::run(args),
        "pentest" => commands::pentest_enter::run(args),
        "pentest-bootstrap" => commands::pentest_bootstrap::run(args),
        "proxy-only" => commands::proxy_only::run(args),
        "push" => commands::push::run(args),
        "pull" => commands::pull::run(args),
        "self-destruct" => commands::self_destruct::run(args),
        "set-clash-dns" => commands::clash_dns::run(args),
        "setup" => commands::setup::run(args),
        "sync-service" => commands::sync_service::run(args),
        "unlock-pentest" => commands::unlock_pentest::run(args),
        "unlock-vault" => commands::unlock_vault::run(args),
        "vault-sync" => commands::vault_sync::run(args),
        "workload" => commands::workload::run(args),
        other => {
            eprintln!("sdw: unknown command '{other}'");
            eprint_usage();
            2
        }
    }
}

use crate::commands;
use sdw_core::workspace::workload::{
    doas_rules, is_bridge_invocation, PRIVILEGED_VERBS, PRIV_BRIDGE_DIR,
};
use sdw_infra::workload::is_root;
use std::os::unix::process::CommandExt;

fn eprint_usage() {
    eprintln!("usage: sdw <command> [args]   (or invoke through a command-named symlink)");
    eprintln!("commands: {}", COMMANDS.join(", "));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_takes_the_last_path_segment() {
        assert_eq!(basename("/usr/local/bin/init-vault"), "init-vault");
        assert_eq!(basename("push"), "push");
        assert_eq!(basename("./sdw"), "sdw");
    }

    #[test]
    fn an_unknown_command_is_a_usage_error() {
        assert_eq!(dispatch("no-such-command", &[]), 2);
    }

    #[test]
    fn sdw_with_no_subcommand_is_a_usage_error() {
        assert_eq!(run(vec!["sdw".to_string()]), 2);
        assert_eq!(run(vec!["/usr/local/bin/sdw".to_string()]), 2);
    }

    #[test]
    fn a_caller_without_root_gets_the_commands_that_run_as_itself_and_is_refused_the_operators() {
        // proxy-only runs again as the clash user in the standalone client: it must reach its own
        // argument handling (a usage error, 2), not the refusal (126).
        assert_eq!(as_workload("proxy-only", &["--bogus".to_string()]), 2);
        // Network/configuration changes, arbitrary-path transfers, package installation, pentest
        // root access and destructive operations are all operator-only.
        for command in [
            "audit-log",
            "backup-init",
            "desktop-install",
            "destruct-passphrase",
            "entrypoint",
            "init-pentest",
            "init-vault",
            "lock-pentest",
            "pentest",
            "pentest-bootstrap",
            "pull",
            "push",
            "self-destruct",
            "set-clash-dns",
            "setup",
            "sync-service",
            "unlock-pentest",
            "no-such-command",
        ] {
            assert_eq!(as_workload(command, &[]), 126, "{command}");
        }
    }

    #[test]
    fn no_operator_command_is_reachable_without_root_by_a_side_door() {
        for command in RUN_AS_WORKLOAD {
            assert!(
                !PRIVILEGED_VERBS.contains(command),
                "{command} would be both bridged and run as the caller"
            );
        }
        for operator_only in [
            "self-destruct",
            "destruct-passphrase",
            "backup-init",
            "entrypoint",
        ] {
            assert!(!RUN_AS_WORKLOAD.contains(&operator_only), "{operator_only}");
        }
    }
}
