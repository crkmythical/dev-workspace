//! Replication of the vault's ciphertext to a git remote — a port of
//! packages/infra/src/replication.ts.
//!
//! The repository lives in the cipher directory, outside the mount, and holds only ciphertext and
//! gocryptfs.conf. The start-up makes sure it exists and points at VAULT_GIT_REPO
//! (ensure_cipher_repo); the cron job only commits and pushes (cron carries no env). Nothing here
//! prints: the remote URL may carry an access token.

use crate::cmd;
use std::fs;
use std::path::Path;

/// What git runs for ssh:// and git@host: remotes when nobody is there to answer a question.
pub const UNATTENDED_SSH: &str = "ssh -o StrictHostKeyChecking=accept-new -o BatchMode=yes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnsureResult {
    Created,
    RemoteUpdated,
    Unchanged,
}

fn git(dir: &str, args: &[&str]) -> (bool, String) {
    let mut full = vec!["-C", dir];
    full.extend_from_slice(args);
    cmd::capture("git", &full)
}

fn git_dir_exists(dir: &str) -> bool {
    Path::new(&format!("{dir}/.git")).exists()
}

/// The URL of `origin`, or None when there is no repository or it has no such remote.
pub fn origin_of(dir: &str) -> Option<String> {
    if !git_dir_exists(dir) {
        return None;
    }
    let (ok, out) = git(dir, &["remote", "get-url", "origin"]);
    if !ok {
        return None;
    }
    let url = out.trim();
    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// Make `dir` a git repository whose `origin` is `remote`. Idempotent; existing files are not
/// rewritten.
pub fn ensure_cipher_repo(
    dir: &str,
    remote: &str,
    gitignore: &str,
    gitattributes: &str,
) -> Result<EnsureResult, String> {
    let mut result = EnsureResult::Unchanged;

    if !git_dir_exists(dir) {
        if !git(dir, &["init", "-q", "-b", "main"]).0 {
            return Err("git init failed in the cipher directory".to_string());
        }
        result = EnsureResult::Created;
    }
    if !Path::new(&format!("{dir}/.gitignore")).exists() {
        fs::write(format!("{dir}/.gitignore"), gitignore).map_err(|e| e.to_string())?;
    }
    if !Path::new(&format!("{dir}/.gitattributes")).exists() {
        fs::write(format!("{dir}/.gitattributes"), gitattributes).map_err(|e| e.to_string())?;
    }

    // cron cannot answer ssh's "trust this host?"; accept-new pins the first key, BatchMode keeps a
    // passphrase-protected key from hanging the job. A command the user set is theirs and stays.
    if !git(dir, &["config", "--local", "--get", "core.sshCommand"]).0
        && !git(
            dir,
            &["config", "--local", "core.sshCommand", UNATTENDED_SSH],
        )
        .0
    {
        return Err("could not set core.sshCommand".to_string());
    }

    match origin_of(dir) {
        None => {
            if !git(dir, &["remote", "add", "origin", remote]).0 {
                return Err("could not add the origin remote".to_string());
            }
        }
        Some(current) if current != remote => {
            if !git(dir, &["remote", "set-url", "origin", remote]).0 {
                return Err("could not update the origin remote".to_string());
            }
            if result == EnsureResult::Unchanged {
                result = EnsureResult::RemoteUpdated;
            }
        }
        Some(_) => {}
    }
    Ok(result)
}

/// Stage everything. True when there is something to commit.
pub fn stage_changes(dir: &str) -> bool {
    git(dir, &["add", "-A"]);
    // `diff --cached --quiet` exits 1 (not success) when something is staged.
    !git(dir, &["diff", "--cached", "--quiet"]).0
}

/// Commits not on origin/main yet: all of them when never pushed, the ones after origin/main
/// otherwise. A push that failed leaves its commit here.
pub fn has_unpushed_commits(dir: &str) -> bool {
    if !git(dir, &["rev-parse", "--verify", "-q", "HEAD"]).0 {
        return false; // nothing committed yet
    }
    if !git(
        dir,
        &["rev-parse", "--verify", "-q", "refs/remotes/origin/main"],
    )
    .0
    {
        return true; // never pushed
    }
    let (_, ahead) = git(
        dir,
        &["rev-list", "--count", "refs/remotes/origin/main..HEAD"],
    );
    ahead.trim().parse::<i64>().unwrap_or(0) > 0
}

fn configured(dir: &str, key: &str, fallback: &str) -> String {
    let (ok, out) = git(dir, &["config", key]);
    let value = out.trim();
    if ok && !value.is_empty() {
        value.to_string()
    } else {
        fallback.to_string()
    }
}

/// `-c user.name=... -c user.email=...`, from git's own config or the fallback, so a container
/// without an identity does not fail with "who are you".
fn identity_args(dir: &str, fallback_name: &str, fallback_email: &str) -> Vec<String> {
    vec![
        "-c".to_string(),
        format!("user.name={}", configured(dir, "user.name", fallback_name)),
        "-c".to_string(),
        format!(
            "user.email={}",
            configured(dir, "user.email", fallback_email)
        ),
    ]
}

/// Commit what is staged.
pub fn commit_staged(dir: &str, message: &str, fallback_name: &str, fallback_email: &str) -> bool {
    let who = identity_args(dir, fallback_name, fallback_email);
    let mut args: Vec<&str> = vec!["-C", dir];
    args.extend(who.iter().map(String::as_str));
    args.extend(["commit", "-q", "-m", message]);
    cmd::capture("git", &args).0
}

/// Push to origin/main. If refused (another copy pushed first), rebase onto the remote once and
/// push again.
pub fn push_to_origin(dir: &str, fallback_name: &str, fallback_email: &str) -> bool {
    let push = || cmd::capture("git", &["-C", dir, "push", "-q", "-u", "origin", "main"]).0;
    if push() {
        return true;
    }
    let who = identity_args(dir, fallback_name, fallback_email);
    let mut args: Vec<&str> = vec!["-C", dir];
    args.extend(who.iter().map(String::as_str));
    args.extend(["pull", "--rebase", "-q", "origin", "main"]);
    if !cmd::capture("git", &args).0 {
        return false;
    }
    push()
}
