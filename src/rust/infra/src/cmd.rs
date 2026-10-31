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
