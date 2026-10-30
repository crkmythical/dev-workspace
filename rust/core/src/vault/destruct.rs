//! Self-destruct: state paths, key-derivation parameters and the remote trigger's rate limit —
//! a port of packages/core/src/vault/destruct.ts.
//!
//! The sync-service crate still carries its own copy of the subset it needs (crypto/constants);
//! pointing it at this module is the remaining dedup step of the migration. The paths derive from
//! RUN_DIR; see crate::shared::layout for why they are spelled as literals.

/// Persisted on a named volume so it survives container recreation. `{RUN_DIR}/self-destruct`.
pub const DESTRUCT_STATE_DIR: &str = "/var/run/self-destruct";
pub const DESTRUCT_KEY_HASH_PATH: &str = "/var/run/self-destruct/key-hash";
/// Written last by a destroy. Its presence puts the container into inert mode (entrypoint.sh).
pub const DESTRUCT_COMPLETED_PATH: &str = "/var/run/self-destruct/completed";

// Key derivation (HKDF-SHA256), domain-separated from the sync key.
pub const DESTRUCT_KEY_LENGTH: usize = 32;
pub const DESTRUCT_HKDF_SALT: &str = "destruct-key-v1";
pub const DESTRUCT_HKDF_INFO: &str = "self-destruct";

// Remote trigger rate limit.
pub const DESTRUCT_RATE_LIMIT_WINDOW_MS: i64 = 60_000;
pub const DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS: u64 = 3;
/// Global backstop: absolute cap on destruct attempts per window across ALL clients. Bounds a
/// brute force that rotates X-Forwarded-For to defeat the per-IP budget (the per-IP key is
/// client-influenced; this cap is not).
pub const DESTRUCT_RATE_LIMIT_GLOBAL_MAX: u64 = 10;
pub const DESTRUCT_RESPONSE_DELAY_MS: u64 = 200;
