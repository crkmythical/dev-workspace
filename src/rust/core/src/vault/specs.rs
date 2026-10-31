//! Deployment default for the workspace vault — a port of packages/core/src/vault/specs.ts. Only
//! composition roots (cli / server / entrypoint) should reach for it; plugins that own further
//! vaults define their own (see crate::plugins).

use crate::shared::layout::WORKSPACE_MOUNT;
use crate::vault::domain::VaultSpec;
use crate::vault::layout::VAULT_CIPHER_DIR;

/// The main vault: id "workspace", mounted at WORKSPACE_MOUNT. `allow_other` stays false — only a
/// plugin volume holding a root file system needs it (see the pentest plugin).
pub fn workspace_vault() -> VaultSpec {
    VaultSpec {
        id: "workspace".to_string(),
        cipher_dir: VAULT_CIPHER_DIR.to_string(),
        mount_point: WORKSPACE_MOUNT.to_string(),
        allow_other: false,
    }
}
