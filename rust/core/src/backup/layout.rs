//! Backup runtime state location — a port of packages/core/src/backup/layout.ts.

/// Runtime state directory (non-FUSE, container-local). Loss on restart is non-fatal.
/// `{RUN_DIR}/backup-state`.
pub const BACKUP_STATE_DIR: &str = "/var/run/backup-state";
