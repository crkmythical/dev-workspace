//! Where the sync service lives and keeps its files — a port of packages/core/src/sync/layout.ts.
//! The sync-service crate consumes these directly (see its state.rs/main.rs). Paths deriving from
//! WORKSPACE_MOUNT/CREDENTIALS_DIR are literals (see crate::shared::layout).

pub const SYNC_SERVICE_PORT: u16 = 8081;

/// Where the image build puts the compiled SPA (served by the sync service under /sync/).
pub const SPA_DIST_DIR: &str = "/opt/workspace/packages/spa/dist";

/// Files received from the browser end up here. `{WORKSPACE_MOUNT}/shared`.
pub const SHARED_DIR: &str = "/workspace/shared";

/// In-flight chunked uploads (stale ones cleaned by image/etc/cron.d/cleanup). `{SHARED_DIR}/.uploads`.
pub const UPLOAD_TRACKING_DIR: &str = "/workspace/shared/.uploads";

/// The AEAD sync key is derived from this file. It lives inside the vault, so it is only readable
/// while the vault is mounted: its disappearance is the authoritative "vault locked" signal.
/// `{CREDENTIALS_DIR}/sync-passphrase`.
pub const SYNC_PASSPHRASE_PATH: &str = "/workspace/.credentials/sync-passphrase";
