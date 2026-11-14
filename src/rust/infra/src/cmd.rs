//! Thin wrappers over `std::process::Command` for the adapters. Every external tool the container
//! ships (gocryptfs, fusermount, mountpoint, supervisorctl, restic, git, ...) is driven through
//! one of these. Errors collapse to "did not succeed" because the adapters treat a missing or
//! failing tool the same way the TS `$\`...\`.nothrow()` did.

use std::io::Write;
use std::process::{Command, Stdio};

/// Run silently; true iff it exited 0. stdio is discarded.
pub fn ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run capturing stdout; returns (succeeded, stdout). stderr is discarded.
pub fn capture(program: &str, args: &[&str]) -> (bool, String) {
    match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ),
        Err(_) => (false, String::new()),
    }
}

/// Run feeding `input` to stdin; returns (succeeded, trimmed stderr). stdout is discarded.
pub fn run_stdin(program: &str, args: &[&str], input: &str) -> (bool, String) {
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return (false, e.to_string()),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
        // dropping stdin closes it, signalling EOF to the child
    }
    match child.wait_with_output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(e) => (false, e.to_string()),
    }
}

/// Run with the parent's stdio inherited (interactive passthrough); returns the exit code.
pub fn inherit(program: &str, args: &[&str]) -> i32 {
    match Command::new(program).args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("could not run {program}: {e}");
            1
        }
    }
}

/// Run silently in a working directory; true iff it exited 0. For tools with no `-C` equivalent.
pub fn ok_cwd(cwd: &str, program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run to completion, but give up after `limit`: the child is killed and `None` returned. For tools
/// that retry an unreachable backend for minutes on their own (restic keeps trying for up to 15), so
/// that a start-up step asking "is it there?" gets an answer in bounded time. stdout and stderr are
/// captured; stdin is closed.
pub fn output_within(
    command: &mut Command,
    limit: std::time::Duration,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut out = child.stdout.take().expect("stdout is piped");
    let mut err = child.stderr.take().expect("stderr is piped");
    // Drain both pipes on threads, or a chatty child fills a pipe and never exits.
    let out_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = out.read_to_end(&mut bytes);
        bytes
    });
    let err_thread = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = err.read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(std::process::Output {
                status,
                stdout: out_thread.join().unwrap_or_default(),
                stderr: err_thread.join().unwrap_or_default(),
            }));
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_quick_command_returns_its_output() {
        let out = output_within(
            Command::new("sh").args(["-c", "echo out; echo err >&2; exit 3"]),
            Duration::from_secs(10),
        )
        .unwrap()
        .expect("finished in time");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&out.stdout), "out\n");
        assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
    }

    #[test]
    fn a_command_that_runs_too_long_is_killed_and_reported_as_none() {
        let started = std::time::Instant::now();
        let out =
            output_within(Command::new("sleep").arg("30"), Duration::from_millis(200)).unwrap();
        assert!(out.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "it did not wait for the child"
        );
    }

    #[test]
    fn a_program_that_is_not_there_is_an_error() {
        assert!(output_within(
            &mut Command::new("no-such-program-sdw"),
            Duration::from_secs(1)
        )
        .is_err());
    }
}
