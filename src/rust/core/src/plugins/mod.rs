//! Plugin registry — a port of packages/core/src/plugins/index.ts. The optional features compiled
//! into this build. To drop a plugin, delete its line from `plugins()` and its folder; nothing
//! else in the source names it.
//!
//! `env_groups()` is the combined set rendered into `.env.example`: the core's groups
//! (crate::workspace::env_schema) first, then each plugin's.

pub mod pentest;
pub mod plugin;

pub use plugin::Plugin;

use crate::vault::domain::VaultSpec;
use crate::vault::specs::workspace_vault;
use crate::workspace::env_schema::{core_env_groups, EnvGroup};
use pentest::pentest_plugin;

/// The optional features compiled into this build.
pub fn plugins() -> Vec<Plugin> {
    vec![pentest_plugin()]
}

/// Every encrypted volume of this build: the workspace first, then each plugin's.
pub fn all_vaults() -> Vec<VaultSpec> {
    let mut vaults = vec![workspace_vault()];
    for plugin in plugins() {
        vaults.extend(plugin.vaults);
    }
    vaults
}

/// Every variable the user can set — the core's groups, then each plugin's — the input to
/// `.env.example`. Mirrors ENV_GROUPS in the TS plugins index.
pub fn env_groups() -> Vec<EnvGroup> {
    let mut groups = core_env_groups();
    for plugin in plugins() {
        groups.extend(plugin.env);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::pentest::pentest_vault;

    // Mirrors packages/core/tests/plugins.test.ts.

    #[test]
    fn every_plugin_has_its_own_id() {
        let ids: Vec<String> = plugins().into_iter().map(|p| p.id).collect();
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn all_vaults_is_the_workspace_first_then_every_plugins_volumes() {
        let vaults = all_vaults();
        assert_eq!(vaults[0], workspace_vault());
        let rest: Vec<VaultSpec> = plugins().into_iter().flat_map(|p| p.vaults).collect();
        assert_eq!(&vaults[1..], &rest[..]);
    }

    #[test]
    fn no_two_volumes_share_an_id_cipher_dir_or_mount_point() {
        // destroy shreds every volume's key and unmounts every mount point: a collision would
        // shred the wrong key or unmount the wrong thing.
        let vaults = all_vaults();
        let fields: [fn(&VaultSpec) -> String; 3] = [
            |v| v.id.clone(),
            |v| v.cipher_dir.clone(),
            |v| v.mount_point.clone(),
        ];
        for field in fields {
            let values: Vec<String> = vaults.iter().map(field).collect();
            let unique: std::collections::HashSet<&String> = values.iter().collect();
            assert_eq!(unique.len(), values.len());
        }
    }

    #[test]
    fn no_volume_lives_inside_anothers_directories() {
        fn inside(a: &str, b: &str) -> bool {
            a == b || a.starts_with(format!("{b}/").as_str())
        }
        let vaults = all_vaults();
        for a in &vaults {
            for b in &vaults {
                if a == b {
                    continue;
                }
                assert!(!inside(&a.mount_point, &b.mount_point));
                assert!(!inside(&a.cipher_dir, &b.cipher_dir));
                assert!(!inside(&a.cipher_dir, &b.mount_point));
                assert!(!inside(&a.mount_point, &b.cipher_dir));
            }
        }
    }

    #[test]
    fn a_plugin_cannot_claim_the_workspaces_id() {
        let workspace_id = workspace_vault().id;
        for plugin in plugins() {
            for vault in plugin.vaults {
                assert_ne!(vault.id, workspace_id);
            }
        }
    }

    // Mirrors the registration half of packages/core/tests/plugins/pentest.test.ts
    // (the empty-as-unset config behaviour is tested in pentest::config).

    #[test]
    fn pentest_is_registered_and_owns_one_volume_of_its_own() {
        assert!(plugins().contains(&pentest_plugin()));
        assert_eq!(pentest_plugin().vaults, vec![pentest_vault()]);
        assert!(all_vaults().contains(&pentest_vault()));
    }

    #[test]
    fn pentest_lets_other_users_use_its_volume() {
        // tor runs as debian-tor, apt downloads as _apt; FUSE turns every other uid away unless asked.
        assert!(pentest_vault().allow_other);
    }

    /// The committed `.env.example` is exactly what the generator produces — the drift guard (a
    /// port of the TS env-example --check). Regenerate with `make env-example` after a schema edit.
    #[test]
    fn env_example_is_up_to_date() {
        use crate::workspace::env_schema::render_env_example;
        // From src/rust/core/src/plugins/ the repo-root .env.example is five levels up.
        let committed = include_str!("../../../../../.env.example");
        assert_eq!(
            render_env_example(&env_groups()),
            committed,
            "`.env.example` is out of date — run `make env-example`"
        );
    }
}
