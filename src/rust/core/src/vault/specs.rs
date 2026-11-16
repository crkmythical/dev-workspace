//! Deployment default for the workspace vault — a port of packages/core/src/vault/specs.ts. Only
//! composition roots (cli / server / entrypoint) should reach for it; plugins that own further
//! vaults define their own (see crate::plugins).

use crate::shared::layout::WORKSPACE_MOUNT;
use crate::vault::domain::VaultSpec;
use crate::vault::layout::VAULT_CIPHER_DIR;
use crate::workspace::workload::{WORKLOAD_GID, WORKLOAD_UID};
/// The main vault: id "workspace", mounted at WORKSPACE_MOUNT by root, but its files belong to the
/// workload user (`owner`), which `allow_other` lets in. Root-created bootstrap paths are also
/// explicitly chowned after unlock because some gocryptfs/distro combinations still enforce their
/// underlying root ownership during access checks. The mount point itself is mode 0700, so
/// any other user (Caddy, Clash, a service account) is kept out by the kernel (gocryptfs turns on
/// default_permissions together with allow_other).
pub fn workspace_vault() -> VaultSpec {
    VaultSpec {
        id: "workspace".to_string(),
        cipher_dir: VAULT_CIPHER_DIR.to_string(),
        mount_point: WORKSPACE_MOUNT.to_string(),
        allow_other: true,
        owner: Some((WORKLOAD_UID, WORKLOAD_GID)),
    }
}
