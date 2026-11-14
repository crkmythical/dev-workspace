//! `sdw workload [--home DIR] [--cwd DIR] [--] COMMAND [ARGS...]` — start COMMAND as the
//! unprivileged workload user, with the environment allowlist. What supervisord runs for code-server
//! and the desktop programs, and what the boot uses to run a user's tool installs.
//!
//! The process becomes COMMAND (exec), so supervisord supervises COMMAND itself and stops its
//! process group as before. Run as root only: it is the step that gives privileges up.

use sdw_core::workspace::workload::WORKLOAD_HOME;
use sdw_infra::workload::{is_root, workload_command};
use std::os::unix::process::CommandExt;

fn usage() -> i32 {
    eprintln!("usage: sdw workload [--home DIR] [--cwd DIR] [--] COMMAND [ARGS...]");
    2
}

pub fn run(args: &[String]) -> i32 {
    let mut home = WORKLOAD_HOME.to_string();
    let mut cwd: Option<String> = None;
    let mut rest = args;
    while let Some(first) = rest.first() {
        match first.as_str() {
            "--home" | "--cwd" if rest.len() >= 2 => {
                if first == "--home" {
                    home = rest[1].clone();
                } else {
                    cwd = Some(rest[1].clone());
                }
                rest = &rest[2..];
            }
            "--" => {
                rest = &rest[1..];
                break;
            }
            _ => break,
        }
    }
    let Some(program) = rest.first() else {
        return usage();
    };
    if !is_root() {
        eprintln!("workload: only root can start a process as another user");
        return 1;
    }
    let (mut command, refused) = workload_command(program, &rest[1..], &home, cwd.as_deref());
    for name in refused {
        eprintln!("workload: WORKLOAD_ENV names {name}, a workspace secret; it is not passed on");
    }
    let err = command.exec();
    eprintln!("workload: cannot start {program}: {err}");
    if err.kind() == std::io::ErrorKind::NotFound {
        127
    } else {
        126
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_command_is_a_usage_error() {
        assert_eq!(run(&[]), 2);
        assert_eq!(run(&["--home".to_string(), "/h".to_string()]), 2);
        assert_eq!(run(&["--".to_string()]), 2);
    }
}
