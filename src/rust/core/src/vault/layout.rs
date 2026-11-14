//! Where the workspace vault keeps its ciphertext, and vault-related runtime state — a port of
//! packages/core/src/vault/layout.ts. The two runtime paths derive from `RUN_DIR`; see
//! crate::shared::layout for why they are spelled as literals here.

/// Ciphertext of the workspace vault.
pub const VAULT_CIPHER_DIR: &str = "/vault/cipher";

/// Where vault-sync keeps its per-cycle state. `{RUN_DIR}/vault-sync`.
pub const VAULT_SYNC_STATE_DIR: &str = "/var/run/vault-sync";
