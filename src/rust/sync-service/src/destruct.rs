//! The remote self-destruct trigger, behind a trait so the route's decision logic
//! (feature gate, rate limit, passphrase check) is unit-tested without ever
//! crossing the irreversible boundary.
//!
//! The real trigger keeps that boundary in ONE place during the migration: it
//! launches the existing `self-destruct` CLI, which shreds the vault keys, stops
//! every service and SIGTERMs PID 1. (The TS route destroyed in-process; moving it
//! behind the CLI means this daemon never holds the destruction logic itself. The
//! behaviour is the same and is proven by tests/smoke/checks/80-destruct.sh.)

use std::process::Command;

pub trait DestructTrigger: Send + Sync {
    /// Launch destruction. `Ok` means the (detached) destroyer is now running; `Err` means it
    /// could not even be started, so the caller must NOT report success.
    fn trigger(&self) -> Result<(), String>;
}

/// Production trigger: launch `self-destruct --force --remote`, detached. Not awaited — the CLI
/// terminates PID 1, which takes this process down with it. `--remote` so a remote destruct also
/// deletes the off-box ciphertext mirror (skipped cleanly when no VAULT_GIT_REPO / github-token
/// is configured); without it, the encrypted vault survived on the git remote.
pub struct SpawnSelfDestruct;

impl DestructTrigger for SpawnSelfDestruct {
    fn trigger(&self) -> Result<(), String> {
        Command::new("self-destruct")
            .arg("--force")
            .arg("--remote")
            .spawn()
            .map(|_child| ())
            .map_err(|e| format!("could not launch self-destruct: {e}"))
    }
}
