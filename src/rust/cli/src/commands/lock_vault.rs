//! lock-vault — unmount the gocryptfs vault. A port of packages/cli/src/lock-vault.ts.
//!
//! Thin driving adapter over core's lock_vault(): desktops and the backup watcher are stopped
//! first (they hold fds on the mount), then the vault is unmounted; a busy unmount can be forced.

use crate::prompt::prompt_line;
use sdw_core::sync::layout::SYNC_PASSPHRASE_PATH;
use sdw_core::vault::ports::UnmountMode;
use sdw_core::vault::specs::workspace_vault;
use sdw_core::vault::usecases::{cancel_lock, lock_vault, LockDeps, LockResult};
use sdw_infra::gocryptfs::GocryptfsVolume;
use sdw_infra::holders::SupervisorHolders;
use sdw_infra::state::broadcast_state;

pub fn run(_args: &[String]) -> i32 {
    let volume = GocryptfsVolume;
    let holders = SupervisorHolders::new();
    let log = |m: &str| println!("{m}");
    let log_ref: &dyn Fn(&str) = &log;
    let deps = LockDeps {
        volume: &volume,
        holders: &holders,
        log: Some(log_ref),
    };

    let result = match lock_vault(&workspace_vault(), &deps, UnmountMode::Graceful) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: {e}");
            return 1;
        }
    };

    let mut forced = false;
    match result {
        LockResult::Noop => {
            println!("Vault is not currently mounted.");
            return 0;
        }
        LockResult::Rejected => {
            eprintln!("ERROR: This environment has been destroyed.");
            return 1;
        }
        LockResult::Busy => {
            eprintln!("Could not unmount cleanly (open file handles?).");
            let answer = prompt_line("Force unmount? [y/N]: ");
            if answer.map(|a| a.to_lowercase()).as_deref() != Some("y") {
                println!("Aborted.");
                // The vault stays mounted, so what the attempt stopped has to run again.
                let restarted = cancel_lock(&holders);
                if !restarted.is_empty() {
                    println!("Started again: {}.", restarted.join(", "));
                }
                return 1;
            }
            match lock_vault(&workspace_vault(), &deps, UnmountMode::Lazy) {
                Ok(LockResult::Locked) => {}
                _ => {
                    eprintln!("ERROR: Force unmount failed.");
                    return 1;
                }
            }
            forced = true;
            println!("Vault force-locked.");
        }
        LockResult::Locked => {
            println!("Vault locked.");
        }
    }

    // The sync key source is only valid while the vault is open.
    let _ = std::fs::remove_file(SYNC_PASSPHRASE_PATH);
    broadcast_state("locked");
    if forced {
        // A lazy unmount detaches at once, but whoever already had a file open keeps it.
        println!("Processes that still had files open can keep using them until they exit.");
        println!(
            "To cut them off now, restart the container (or: supervisorctl restart code-server)."
        );
    } else {
        println!("Plaintext data is no longer accessible.");
    }
    0
}
