//! The plugin contract — a port of packages/core/src/plugins/plugin.ts.
//!
//! Deliberately tiny: a plugin contributes optional encrypted volumes and the env vars it reads.
//! Everything that must treat all volumes alike (destroy, stale-mount cleanup at boot, doctor)
//! iterates the registry (crate::plugins) instead of naming a plugin, which keeps the rest of the
//! code free of any plugin's name. Grow it only when a second plugin needs the same thing.

use crate::vault::domain::VaultSpec;
use crate::workspace::env_schema::EnvGroup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plugin {
    pub id: String,
    /// Extra encrypted volumes the plugin owns. They are destroyed with the workspace, detached
    /// when stale at boot, and reported by `doctor`.
    pub vaults: Vec<VaultSpec>,
    /// Environment variables the plugin reads: documented in the generated `.env.example`. An
    /// empty vector is the TS `env?` being absent.
    pub env: Vec<EnvGroup>,
}
