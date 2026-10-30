//! unlock-vault — mount the gocryptfs vault at /workspace and run the post-unlock hooks. A port of
//! packages/cli/src/unlock-vault.ts. Thin driving adapter; the decision, mount and hooks live in
//! core + infra and are shared with the boot-time auto-unlock.

use crate::commands::system_env;
use crate::passphrase::read_passphrase;
use sdw_core::vault::domain::explain_unlock_failure;
use sdw_core::vault::ports::UnlockHook;
use sdw_core::vault::specs::workspace_vault;
use sdw_core::vault::usecases::{unlock_vault, UnlockDeps, UnlockRejectReason, UnlockResult};
use sdw_core::workspace::config::load_config;
use sdw_infra::gocryptfs::GocryptfsVolume;
use sdw_infra::unlock_hooks::{interactive_unlock_extras, workspace_unlock_hooks};

pub fn run(_args: &[String]) -> i32 {
    let config = load_config(system_env);
    let announce: fn(&str) = |m| println!("{m}");
    let mut hooks = workspace_unlock_hooks(config.features, announce);
    hooks.extend(interactive_unlock_extras(
        config.backup.restic_repository.is_some(),
    ));
    let hook_refs: Vec<&dyn UnlockHook> = hooks.iter().map(|h| h.as_ref()).collect();

    let volume = GocryptfsVolume;
    let result = unlock_vault(
        &workspace_vault(),
        || read_passphrase(None, "Vault passphrase: "),
        &UnlockDeps {
            volume: &volume,
            hooks: &hook_refs,
        },
    );

    match result {
        UnlockResult::Noop => {
            println!("Vault already unlocked.");
            0
        }
        UnlockResult::Rejected { reason } => {
            eprintln!(
                "{}",
                match reason {
                    UnlockRejectReason::NotInitialized =>
                        "ERROR: Vault not initialized. Run 'init-vault' first.",
                    UnlockRejectReason::EmptyPassphrase => "ERROR: Passphrase cannot be empty.",
                    UnlockRejectReason::Destroyed => "ERROR: This environment has been destroyed.",
                }
            );
            1
        }
        UnlockResult::Failed { detail } => {
            eprintln!("ERROR: Failed to unlock vault.");
            let trimmed = detail.trim();
            if !trimmed.is_empty() {
                eprintln!("  {}", trimmed.split('\n').collect::<Vec<_>>().join("\n  "));
            }
            let why = explain_unlock_failure(&detail, false);
            let lines = if why.is_empty() {
                vec!["Wrong passphrase, or the mount failed.".to_string()]
            } else {
                why
            };
            for line in lines {
                eprintln!("  {line}");
            }
            1
        }
        UnlockResult::Unlocked {
            warn_short_passphrase,
            hook_failures,
        } => {
            if warn_short_passphrase {
                eprintln!(
                    "⚠ WARNING: Passphrase is very short (<8 chars). Use 16+ for production."
                );
            }
            for failure in hook_failures {
                eprintln!("WARNING: post-unlock step failed: {failure}");
            }
            println!("\n✓ Vault unlocked. Workspace available at /workspace\n");
            0
        }
    }
}
