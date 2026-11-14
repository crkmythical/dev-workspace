//! Where the sync service lives and keeps its files — a port of packages/core/src/sync/layout.ts.
//! The sync-service crate consumes these directly (see its state.rs/main.rs). Paths deriving from
//! WORKSPACE_MOUNT/CREDENTIALS_DIR are literals (see crate::shared::layout).

pub const SYNC_SERVICE_PORT: u16 = 8081;

/// Files received from the browser end up here. `{WORKSPACE_MOUNT}/shared`.
pub const SHARED_DIR: &str = "/workspace/shared";

/// In-flight chunked uploads (stale ones cleaned by image/etc/cron.d/cleanup). `{SHARED_DIR}/.uploads`.
pub const UPLOAD_TRACKING_DIR: &str = "/workspace/shared/.uploads";

/// The AEAD sync key is derived from this file, written at unlock and removed at lock, so its
/// presence is the authoritative "vault unlocked" signal. It holds the vault passphrase, so it is
/// NOT in the vault (where the unprivileged workload owns every file) but in the root-only runtime
/// directory. `{PRIVATE_RUN_DIR}/sync-passphrase`.
pub const SYNC_PASSPHRASE_PATH: &str = "/run/sdw/sync-passphrase";
