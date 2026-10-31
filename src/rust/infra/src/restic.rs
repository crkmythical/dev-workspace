//! restic adapter — a port of packages/infra/src/restic.ts. Asks a repository what state it is in;
//! the decision (init / warn / leave alone) is core's classify_repo_probe from restic's exit code
//! and message. Used by the start-up and by `backup-init`, so both treat the repo the same way.

use sdw_core::backup::domain::{classify_repo_probe, RepoProbe};
use sdw_core::egress::layout::CLASH_HTTP_PORT;
use std::process::{Command, Stdio};

pub struct RepoStatus {
    pub verdict: RepoProbe,
    /// What restic printed on stderr, for a diagnostic. May contain the repository URL.
    pub detail: String,
}

/// restic goes through the same proxy as everything else; the child inherits the rest of the env.
fn restic(args: &[&str]) -> Command {
    let proxy = format!("http://127.0.0.1:{CLASH_HTTP_PORT}");
    let mut c = Command::new("restic");
    c.args(args);
    for key in ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"] {
        c.env(key, &proxy);
    }
    c
}

/// `restic snapshots`, read-only (no lock), classified. Never panics.
pub fn probe_repository() -> RepoStatus {
    let output = restic(&["snapshots", "--no-lock"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            RepoStatus {
                verdict: classify_repo_probe(out.status.code().unwrap_or(-1), &stderr),
                detail: stderr.trim().to_string(),
            }
        }
        // restic is not there at all: not "missing", so nothing is initialized.
        Err(e) => RepoStatus {
            verdict: RepoProbe::Unreachable,
            detail: e.to_string(),
        },
    }
}

/// `restic init`. True on success (stdout/stderr inherited so the user sees progress).
pub fn init_repository() -> bool {
    restic(&["init"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
