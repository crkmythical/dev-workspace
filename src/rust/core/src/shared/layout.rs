//! Container filesystem map shared by every context — a port of
//! packages/core/src/shared/layout.ts. Each context's own layout derives its paths from these
//! roots, so no absolute prefix is repeated and nothing outside a layout spells an absolute path.
//! (Rust `const` cannot concatenate other `const`s, so the derived paths below are written out as
//! literals, each annotated with the root it comes from.)

/// Plaintext FUSE mount of the workspace vault.
pub const WORKSPACE_MOUNT: &str = "/workspace";

/// Credentials stored inside the vault (ssh keys, tokens). `{WORKSPACE_MOUNT}/.credentials`.
pub const CREDENTIALS_DIR: &str = "/workspace/.credentials";

/// Notes for the user ("backup failing", "egress degraded"); inside the vault, so only readable
/// while unlocked. `{WORKSPACE_MOUNT}/.notifications`.
pub const NOTIFICATIONS_DIR: &str = "/workspace/.notifications";

/// Container-local runtime state (not persisted, not inside the vault).
pub const RUN_DIR: &str = "/var/run";
