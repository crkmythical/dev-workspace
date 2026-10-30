//! argv[0] / subcommand routing for the multi-call binary.

/// The binary's own name. When invoked as itself (`sdw <command>`) the command is argv[1];
/// invoked through a command-named symlink, argv[0]'s basename is the command.
const BIN_NAME: &str = "sdw";

/// Commands wired up so far. The rest of packages/cli lands here over the course of stage 3;
/// until then an unknown command is an error and the image keeps running Bun.
const COMMANDS: &[&str] = &[
    "destruct-passphrase",
    "lock-vault",
    "pull",
    "push",
    "self-destruct",
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
        "destruct-passphrase" => commands::destruct_passphrase::run(args),
        "lock-vault" => commands::lock_vault::run(args),
        "push" => commands::push::run(args),
        "pull" => commands::pull::run(args),
        "self-destruct" => commands::self_destruct::run(args),
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
