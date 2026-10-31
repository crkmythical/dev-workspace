//! argv[0] / subcommand routing for the multi-call binary.

/// The binary's own name. When invoked as itself (`sdw <command>`) the command is argv[1];
/// invoked through a command-named symlink, argv[0]'s basename is the command.
const BIN_NAME: &str = "sdw";

/// The commands that get a PATH symlink (the TS `bin` map). entrypoint / backup-watcher /
/// vault-sync are invoked as `sdw <name>` by their callers (entrypoint.sh, supervisord, cron), so
/// they are not symlinked.
const BIN_COMMANDS: &[&str] = &[
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
    println!(
        "Installed {} command symlinks in {dir} -> {}",
        BIN_COMMANDS.len(),
        exe.display()
    );
    0
}

/// Commands wired up so far. The rest of packages/cli lands here over the course of stage 3;
/// until then an unknown command is an error and the image keeps running Bun.
const COMMANDS: &[&str] = &[
    "backup-init",
    "backup-watcher",
    "desktop-install",
    "desktop-start",
    "desktop-stop",
    "destruct-passphrase",
    "doctor",
    "entrypoint",
    "env-example",
    "init-pentest",
    "init-vault",
    "lock-pentest",
    "lock-vault",
    "pentest",
    "pentest-bootstrap",
    "pull",
    "push",
    "self-destruct",
    "setup",
    "sync-service",
    "unlock-pentest",
    "unlock-vault",
    "vault-sync",
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
    dispatch(&command, &rest)
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn dispatch(command: &str, args: &[String]) -> i32 {
    match command {
        "backup-init" => commands::backup_init::run(args),
        "backup-watcher" => commands::backup_watcher::run(args),
        "desktop-install" => commands::desktop_install::run(args),
        "desktop-start" => commands::desktop_start::run(args),
        "desktop-stop" => commands::desktop_stop::run(args),
        "destruct-passphrase" => commands::destruct_passphrase::run(args),
        "doctor" => commands::doctor::run(args),
        "entrypoint" => commands::entrypoint::run(args),
        "env-example" => commands::env_example::run(args),
        "install" => install_symlinks(args.first().map(String::as_str).unwrap_or("/usr/local/bin")),
        "init-pentest" => commands::init_pentest::run(args),
        "init-vault" => commands::init_vault::run(args),
        "lock-pentest" => commands::lock_pentest::run(args),
        "lock-vault" => commands::lock_vault::run(args),
        "pentest" => commands::pentest_enter::run(args),
        "pentest-bootstrap" => commands::pentest_bootstrap::run(args),
        "push" => commands::push::run(args),
        "pull" => commands::pull::run(args),
        "self-destruct" => commands::self_destruct::run(args),
        "setup" => commands::setup::run(args),
        "sync-service" => commands::sync_service::run(args),
        "unlock-pentest" => commands::unlock_pentest::run(args),
        "unlock-vault" => commands::unlock_vault::run(args),
        "vault-sync" => commands::vault_sync::run(args),
        other => {
            eprintln!("sdw: unknown command '{other}'");
            eprint_usage();
            2
        }
    }
}

use crate::commands;

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
}
