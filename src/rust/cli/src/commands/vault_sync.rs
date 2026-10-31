//! vault-sync — periodic vault-ciphertext sync to a git remote. A port of
//! packages/cli/src/vault-sync.ts. Run by cron every 30 min. Cron carries no environment, so this
//! does not read VAULT_GIT_REPO: "has an origin" (set up at boot) is what turns replication on.

use sdw_core::shared::failure::FailureTracker;
use sdw_core::shared::layout::WORKSPACE_MOUNT;
use sdw_core::vault::domain::{plan_sync, SkipReason, SyncPlan, SyncState, SYNC_FAILURE_THRESHOLD};
use sdw_core::vault::layout::{VAULT_CIPHER_DIR, VAULT_SYNC_STATE_DIR};
use sdw_infra::cmd;
use sdw_infra::failure_tracker::FileFailureTracker;
use sdw_infra::replication::{
    commit_staged, has_unpushed_commits, origin_of, push_to_origin, stage_changes,
};
use sdw_infra::time::now_iso;

const NAME: &str = "Vault";
const EMAIL: &str = "vault@local";

fn fail(tracker: &mut FileFailureTracker, msg: &str) -> i32 {
    tracker.record_failure();
    eprintln!("vault-sync failed: {msg}");
    1
}

pub fn run(_args: &[String]) -> i32 {
    let mut tracker = FileFailureTracker::new(
        VAULT_SYNC_STATE_DIR,
        SYNC_FAILURE_THRESHOLD,
        "sync-failure.md",
        |n| format!("# ⚠ Vault Sync Failure\n\nFailed {n} times. Run `doctor` to diagnose.\n"),
    );

    let has_remote = origin_of(VAULT_CIPHER_DIR).is_some();
    let mounted = cmd::ok("mountpoint", &["-q", WORKSPACE_MOUNT]);

    // Nowhere to push to (replication is opt-in), or the vault is locked: nothing to do.
    let gate = plan_sync(SyncState {
        has_remote,
        mounted,
        has_staged_changes: false,
        has_unpushed_commits: false,
    });
    if let SyncPlan::Skip { reason, .. } = gate {
        if reason != SkipReason::NoChanges {
            return 0;
        }
    }

    let plan = plan_sync(SyncState {
        has_remote,
        mounted,
        has_staged_changes: stage_changes(VAULT_CIPHER_DIR),
        has_unpushed_commits: has_unpushed_commits(VAULT_CIPHER_DIR),
    });
    match plan {
        SyncPlan::Skip { .. } => 0, // quiet cycle
        SyncPlan::CommitAndPush => {
            let message = format!("auto-sync {}", now_iso());
            if !commit_staged(VAULT_CIPHER_DIR, &message, NAME, EMAIL) {
                return fail(&mut tracker, "commit failed");
            }
            if !push_to_origin(VAULT_CIPHER_DIR, NAME, EMAIL) {
                return fail(&mut tracker, "push failed");
            }
            tracker.record_success();
            0
        }
        SyncPlan::PushOnly => {
            // A commit whose push failed earlier is still waiting.
            if !push_to_origin(VAULT_CIPHER_DIR, NAME, EMAIL) {
                return fail(&mut tracker, "push failed");
            }
            tracker.record_success();
            0
        }
    }
}
