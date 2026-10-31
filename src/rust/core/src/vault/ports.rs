//! Vault ports: what the use cases need from the outside world — a port of
//! packages/core/src/vault/ports.ts.
//!
//! Implemented by adapters in the cli/infra layer; replaced by fakes in tests. The TS methods are
//! async because Bun I/O is; lock/unlock/destroy are strictly sequential, so the traits are sync
//! here (core stays runtime-free). Where a TS method could throw and the use case caught it, the
//! method returns `Result`; the ones the use cases never wrapped stay infallible.

use crate::vault::domain::{VaultSpec, VaultState};

/// How hard to try to release a mount: graceful (refuses when files are open), lazy (detach now,
/// clean up when the last handle closes), escalate (graceful, then kill users and retry, then lazy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmountMode {
    Graceful,
    Lazy,
    Escalate,
}

impl UnmountMode {
    pub fn as_str(self) -> &'static str {
        match self {
            UnmountMode::Graceful => "graceful",
            UnmountMode::Lazy => "lazy",
            UnmountMode::Escalate => "escalate",
        }
    }
}

/// The outcome of a mount attempt. `detail` carries gocryptfs's message on failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountOutcome {
    pub ok: bool,
    pub detail: Option<String>,
}

pub trait VolumeDriver {
    /// Observe the vault's current state.
    fn state(&self, spec: &VaultSpec) -> VaultState;
    fn init(&self, spec: &VaultSpec, passphrase: &str) -> bool;
    fn mount(&self, spec: &VaultSpec, passphrase: &str) -> MountOutcome;
    /// Returns Ok(true) when the mount point was released (or was not mounted). `report` hears of
    /// anything done to get there that the user would want to know (e.g. processes killed in
    /// "escalate" mode). An Err is a failure to even try (e.g. fusermount blew up).
    fn unmount(
        &self,
        spec: &VaultSpec,
        mode: UnmountMode,
        report: &dyn Fn(&str),
    ) -> Result<bool, String>;
}

/// Anything that may hold fds or inotify watches on the plaintext mount (desktop sessions, the
/// backup watcher, ...). They must all be stopped before an unmount, or it fails with EBUSY.
pub trait HolderControl {
    /// Idempotent. Does nothing when nothing is running.
    fn stop_all(&self);
    /// Start again what `stop_all` stopped, for a lock that then did not happen (the unmount was
    /// busy and the user declined to force it). Returns what it started, for the user to be told.
    /// A vault with nothing to restore keeps the default (empty).
    fn restore(&self) -> Vec<String> {
        Vec::new()
    }
}

pub trait Shredder {
    /// Irrecoverably remove a file. Returns true if the file is gone afterwards.
    fn shred(&self, path: &str, passes: u32) -> bool;
}

pub struct UnlockContext<'a> {
    pub spec: &'a VaultSpec,
    pub passphrase: &'a str,
}

/// A step that runs after a successful mount. A failing hook never undoes the unlock.
pub trait UnlockHook {
    fn name(&self) -> &str;
    fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRepoResult {
    Deleted,
    Failed,
    SkippedNoRepo,
    SkippedNoToken,
}

/// Side effects of destruction that are not volume, holder or shred operations. Each may fail; the
/// destroy loop records the failure and runs the next phase anyway.
pub trait DestroyEffects {
    /// Stop periodic replication (vault-sync cron).
    fn quiesce_services(&self) -> Result<(), String>;
    fn delete_remote_repo(&self) -> Result<RemoteRepoResult, String>;
    /// Shred every file under a directory, then remove what is left.
    fn shred_tree(&self, dir: &str) -> Result<(), String>;
    /// Proxy config, ssh keys, git credentials, remote-destruct key hash.
    fn scrub_credentials(&self) -> Result<(), String>;
    fn clear_shell_history(&self) -> Result<(), String>;
    /// Written last: its presence puts the container into inert mode on restart.
    fn write_completion_marker(&self, iso_timestamp: &str) -> Result<(), String>;
}
