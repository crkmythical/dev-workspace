//! Destroy adapter: the shell side effects of destruction — a port of the effects half of
//! packages/infra/src/destroy.ts. Shared by the `self-destruct` CLI and the remote-destruct route.
//!
//! The TS `createDestroyDeps` is not ported: core's DestroyDeps borrows its ports, so the callers
//! build the deps from local adapters (GocryptfsVolume, SupervisorHolders, FileShredder, this) and
//! call destroy_workspace, rather than receiving an owned struct.

use crate::cmd;
use crate::shred::FileShredder;
use sdw_core::egress::layout::CLASH_CONFIG_PATH;
use sdw_core::shared::layout::CREDENTIALS_DIR;
use sdw_core::vault::destruct::{
    DESTRUCT_COMPLETED_PATH, DESTRUCT_KEY_HASH_PATH, DESTRUCT_STATE_DIR,
};
use sdw_core::vault::ports::{DestroyEffects, RemoteRepoResult, Shredder};
use std::fs;
use std::path::Path;

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
        if !Path::new(&format!("{CREDENTIALS_DIR}/github-token")).exists() {
            return Ok(RemoteRepoResult::SkippedNoToken);
        }
        if cmd::ok("gh", &["repo", "delete", "--yes", repo]) {
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
        let clash = FileShredder.shred(CLASH_CONFIG_PATH, 3);
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
