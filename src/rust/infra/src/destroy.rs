//! Destroy adapter: the shell side effects of destruction — a port of the effects half of
//! packages/infra/src/destroy.ts. Shared by the `self-destruct` CLI and the remote-destruct route.
//!
//! The TS `createDestroyDeps` is not ported: core's DestroyDeps borrows its ports, so the callers
//! build the deps from local adapters (GocryptfsVolume, SupervisorHolders, FileShredder, this) and
//! call destroy_workspace, rather than receiving an owned struct.

use crate::cmd;
use crate::shred::FileShredder;
use sdw_core::auth::session::SESSION_KEY_PATH;
use sdw_core::egress::layout::CLASH_CONFIG_PATH;
use sdw_core::shared::layout::CREDENTIALS_DIR;
use sdw_core::sync::layout::SYNC_PASSPHRASE_PATH;
use sdw_core::vault::destruct::{
    DESTRUCT_COMPLETED_PATH, DESTRUCT_KEY_HASH_PATH, DESTRUCT_STATE_DIR,
};
use sdw_core::vault::ports::{DestroyEffects, RemoteRepoResult, Shredder};
use sdw_core::workspace::workload::GENERATED_PASSWORD_PATH;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use zeroize::Zeroize;

#[derive(Default)]
pub struct DestroyOptions {
    /// Remote the vault ciphertext is replicated to; only needed to delete it (--remote).
    pub vault_repo: Option<String>,
}

pub struct ShellDestroyEffects {
    opts: DestroyOptions,
}

impl ShellDestroyEffects {
    pub fn new(opts: DestroyOptions) -> Self {
        ShellDestroyEffects { opts }
    }
}

fn github_repo_slug(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let candidate = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some((_, hierarchical)) = trimmed.split_once("://") {
        let (authority, path) = hierarchical.split_once('/')?;
        let host = authority.rsplit('@').next()?.split(':').next()?;
        if host != "github.com" {
            return None;
        }
        path
    } else {
        trimmed
    };
    let mut parts = candidate.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let safe = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    (parts.next().is_none() && safe(owner) && safe(repo)).then(|| format!("{owner}/{repo}"))
}

fn delete_github_repo(slug: &str, token: &str) -> bool {
    if token.is_empty()
        || token.len() > 512
        || token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'"' || byte == b'\\')
    {
        return false;
    }
    let config = format!(
        "silent\nshow-error\nfail\nrequest = \"DELETE\"\nurl = \"https://api.github.com/repos/{slug}\"\n\
         header = \"Accept: application/vnd.github+json\"\n\
         header = \"Authorization: Bearer {token}\"\n\
         header = \"X-GitHub-Api-Version: 2022-11-28\"\n"
    );
    let mut child = match Command::new("curl")
        .args(["--config", "-"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("http_proxy", "http://127.0.0.1:7890")
        .env("https_proxy", "http://127.0.0.1:7890")
        .env("all_proxy", "socks5h://127.0.0.1:7890")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    if child
        .stdin
        .take()
        .is_none_or(|mut stdin| stdin.write_all(config.as_bytes()).is_err())
    {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait().is_ok_and(|status| status.success())
}

impl DestroyEffects for ShellDestroyEffects {
    fn quiesce_services(&self) -> Result<(), String> {
        cmd::ok("supervisorctl", &["stop", "vault-sync-cron"]);
        Ok(())
    }

    fn delete_remote_repo(&self) -> Result<RemoteRepoResult, String> {
        let repo = match &self.opts.vault_repo {
            Some(r) => r,
            None => return Ok(RemoteRepoResult::SkippedNoRepo),
        };
        let Some(slug) = github_repo_slug(repo) else {
            return Ok(RemoteRepoResult::Failed);
        };
        let token_path = format!("{CREDENTIALS_DIR}/github-token");
        let mut token = match fs::read_to_string(&token_path) {
            Ok(token) if !token.trim().is_empty() => token,
            _ => return Ok(RemoteRepoResult::SkippedNoToken),
        };
        // Feed the credential to curl through a private stdin config. It never appears in argv,
        // the environment, a persistent gh config, stdout or stderr.
        let deleted = delete_github_repo(&slug, token.trim());
        token.zeroize();
        if deleted {
            Ok(RemoteRepoResult::Deleted)
        } else {
            Ok(RemoteRepoResult::Failed)
        }
    }

    fn shred_tree(&self, dir: &str) -> Result<(), String> {
        if !Path::new(dir).exists() {
            return Ok(());
        }
        let shredded = cmd::ok(
            "find",
            &[dir, "-type", "f", "-exec", "shred", "-n1", "-z", "{}", "+"],
        );
        // Everything inside, hidden entries too (a glob would skip the .git replication repo); the
        // directory itself is a volume root and stays.
        let deleted = cmd::ok("find", &[dir, "-mindepth", "1", "-delete"]);
        if shredded && deleted {
            Ok(())
        } else {
            // Be honest: the keys are already shredded (that phase verifies itself), but the
            // ciphertext was not provably overwritten/removed, so this phase is NOT a success and
            // the report's `complete` must reflect it.
            Err(format!(
                "shred_tree({dir}): shred ok={shredded}, delete ok={deleted}"
            ))
        }
    }

    fn scrub_credentials(&self) -> Result<(), String> {
        // Everything outside the vault that holds a secret: the Clash config (subscription URL), the
        // sync key source (the vault passphrase), a saved generated password, the session key.
        // (Every one is tried even if an earlier one fails: collect first, then judge.)
        let clash = [
            CLASH_CONFIG_PATH,
            SYNC_PASSPHRASE_PATH,
            GENERATED_PASSWORD_PATH,
            SESSION_KEY_PATH,
        ]
        .iter()
        .map(|path| FileShredder.shred(path, 3))
        .collect::<Vec<_>>()
        .into_iter()
        .all(|shredded| shredded);
        let creds = cmd::ok(
            "rm",
            &[
                "-rf",
                "/root/.ssh",
                "/root/.gitconfig",
                "/root/.git-credentials",
            ],
        );
        let hash = cmd::ok("rm", &["-f", DESTRUCT_KEY_HASH_PATH]);
        if clash && creds && hash {
            Ok(())
        } else {
            Err(format!(
                "scrub_credentials: clash={clash}, creds={creds}, hash={hash}"
            ))
        }
    }

    fn clear_shell_history(&self) -> Result<(), String> {
        let history = std::env::var("HOME")
            .map(|h| format!("{h}/.bash_history"))
            .unwrap_or_else(|_| "/root/.bash_history".to_string());
        cmd::ok("truncate", &["-s", "0", &history]);
        Ok(())
    }

    fn write_completion_marker(&self, iso_timestamp: &str) -> Result<(), String> {
        let _ = fs::create_dir_all(DESTRUCT_STATE_DIR);
        fs::write(DESTRUCT_COMPLETED_PATH, iso_timestamp).map_err(|e| e.to_string())
    }
}
