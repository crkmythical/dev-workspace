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
    fn trigger(&self);
}

/// Production trigger: launch `self-destruct --force`, detached. Not awaited —
/// the CLI terminates PID 1, which takes this process down with it.
pub struct SpawnSelfDestruct;

impl DestructTrigger for SpawnSelfDestruct {
    fn trigger(&self) {
        if let Err(e) = Command::new("self-destruct").arg("--force").spawn() {
            eprintln!("remote destruct: could not launch self-destruct: {e}");
        }
    }
}
