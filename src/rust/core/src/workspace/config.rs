//! Typed view of the user-facing environment — a port of packages/core/src/workspace/config.ts.
//! The one place that knows the variable names and defaults the code reads; composition roots call
//! load_config once. Pure: it takes an environment getter. Tests mirror
//! packages/core/tests/config.test.ts.

use crate::backup::domain::BackupGateEnv;
use crate::desktop::domain::DESKTOP_DEFAULT_RESOLUTION;
use crate::workspace::features::{parse_feature_flags, FeatureFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitConfig {
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    /// Remote that vault ciphertext is replicated to (HTTPS+token or SSH URL).
    pub vault_repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    /// Clash subscription URL. Empty when unset.
    pub clash_subscription_url: String,
    /// CLASH_DNS as the user set it, or None when unset. None means "let the workspace decide":
    /// auto-detect at boot, or a runtime override from `set-clash-dns`. The precedence lives in
    /// egress::domain::choose_clash_dns; this is just the raw env value.
    pub clash_dns: Option<String>,
    /// Master password shared by code-server, the desktops and the vault. None when unset.
    pub password: Option<String>,
    pub desktop_resolution: String,
    /// Host port docker-compose publishes the web UI on; only used to print correct URLs.
    pub host_port: String,
    pub git: GitConfig,
    pub backup: BackupGateEnv,
    pub features: FeatureFlags,
    /// EGRESS_ALLOW_CIDRS as the user set it (None when unset or empty). Parsed, and rejected if
    /// malformed, by the boot step that loads the egress firewall (core::egress::allowlist).
    pub egress_allow_cidrs: Option<String>,
}

/// What docker-compose publishes when TUNNEL_HOST_PORT is unset.
pub const DEFAULT_HOST_PORT: &str = "18080";

/// A port number as docker-compose would accept it (1..=65535, at most five digits); anything else
/// falls back to the default rather than mislead in a URL.
fn valid_host_port(raw: Option<String>) -> String {
    let port: u32 = match raw.as_deref().map(str::trim) {
        Some(r) if (1..=5).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_digit()) => {
            r.parse().unwrap_or(0)
        }
        _ => 0,
    };
    if (1..=65535).contains(&port) {
        port.to_string()
    } else {
        DEFAULT_HOST_PORT.to_string()
    }
}

pub fn load_config(get: impl Fn(&str) -> Option<String>) -> WorkspaceConfig {
    // An unset variable and an empty one are the same thing.
    let v = |name: &str| get(name).filter(|s| !s.is_empty());
    WorkspaceConfig {
        clash_subscription_url: v("CLASH_SUBSCRIPTION_URL").unwrap_or_default(),
        clash_dns: v("CLASH_DNS"),
        password: v("PASSWORD"),
        desktop_resolution: v("DESKTOP_RESOLUTION")
            .unwrap_or_else(|| DESKTOP_DEFAULT_RESOLUTION.to_string()),
        host_port: valid_host_port(v("TUNNEL_HOST_PORT")),
        git: GitConfig {
            user_name: v("GIT_USER_NAME"),
            user_email: v("GIT_USER_EMAIL"),
            vault_repo: v("VAULT_GIT_REPO"),
        },
        backup: BackupGateEnv {
            restic_repository: v("RESTIC_REPOSITORY"),
            restic_password: v("RESTIC_PASSWORD"),
            aws_access_key_id: v("AWS_ACCESS_KEY_ID"),
            aws_secret_access_key: v("AWS_SECRET_ACCESS_KEY"),
        },
        features: parse_feature_flags(&get),
        egress_allow_cidrs: v("EGRESS_ALLOW_CIDRS"),
    }
}

// --- startup validation (defense in depth behind scripts/entrypoint.sh) ---

/// Variables that MUST be present for the container to start. Empty: the workspace boots with
/// nothing set — code-server, the desktops and the vault are all local and need no network.
/// CLASH_SUBSCRIPTION_URL used to be here; it is now optional. An empty subscription leaves egress
/// closed (Clash loads no nodes, the fail-closed firewall keeps default-DROPping), which is a
/// supported state the banner and `doctor` explain, not a reason to refuse to start. The generic
/// check below stays in use so re-adding a hard requirement is a one-line, tested change.
pub const REQUIRED_ENV_VARS: &[&str] = &[];

/// The required env vars that are missing (absent or empty).
pub fn find_missing_required_env(
    get: impl Fn(&str) -> Option<String>,
    required: &[&str],
) -> Vec<String> {
    required
        .iter()
        .filter(|name| get(name).filter(|s| !s.is_empty()).is_none())
        .map(|s| s.to_string())
        .collect()
}

/// The error message naming at least one missing variable; None when nothing is missing.
pub fn env_validation_error(missing: &[String]) -> Option<String> {
    missing
        .first()
        .map(|m| format!("ERROR: Required env var {m} is not set."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// An environment getter from a list of pairs.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn defaults_on_empty_env() {
        assert_eq!(
            load_config(|_| None),
            WorkspaceConfig {
                clash_subscription_url: String::new(),
                clash_dns: None,
                password: None,
                desktop_resolution: "1920x1080".to_string(),
                host_port: "18080".to_string(),
                git: GitConfig {
                    user_name: None,
                    user_email: None,
                    vault_repo: None
                },
                backup: BackupGateEnv::default(),
                // The opt-in features are off; the egress firewall is on unless turned off.
                features: FeatureFlags {
                    sync: false,
                    remote_destruct: false,
                    egress_enforce: true
                },
                egress_allow_cidrs: None,
            }
        );
    }

    #[test]
    fn empty_value_is_unset() {
        let cfg = load_config(env(&[
            ("PASSWORD", ""),
            ("DESKTOP_RESOLUTION", ""),
            ("VAULT_GIT_REPO", ""),
            ("CLASH_DNS", ""),
        ]));
        assert_eq!(cfg.password, None);
        assert_eq!(cfg.desktop_resolution, "1920x1080");
        assert_eq!(cfg.git.vault_repo, None);
        // An empty CLASH_DNS counts as unset (None), so auto-detection decides at boot.
        assert_eq!(cfg.clash_dns, None);
    }

    #[test]
    fn reads_every_documented_variable() {
        let cfg = load_config(env(&[
            ("CLASH_SUBSCRIPTION_URL", "https://example.test/sub"),
            ("CLASH_DNS", "223.5.5.5,119.29.29.29"),
            ("PASSWORD", "s3cret"),
            ("DESKTOP_RESOLUTION", "1280x720"),
            ("GIT_USER_NAME", "Ada"),
            ("GIT_USER_EMAIL", "ada@example.test"),
            ("VAULT_GIT_REPO", "git@example.test:ada/vault.git"),
            ("RESTIC_REPOSITORY", "s3:https://s3.example.test/bucket"),
            ("RESTIC_PASSWORD", "r"),
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "key"),
            ("SYNC_ENABLED", "1"),
            ("REMOTE_DESTRUCT_ENABLED", "true"),
            ("EGRESS_ENFORCE", "1"),
            ("EGRESS_ALLOW_CIDRS", "10.0.0.0/8,192.168.1.5"),
        ]));
        assert_eq!(cfg.clash_subscription_url, "https://example.test/sub");
        assert_eq!(cfg.clash_dns.as_deref(), Some("223.5.5.5,119.29.29.29"));
        assert_eq!(cfg.password.as_deref(), Some("s3cret"));
        assert_eq!(cfg.desktop_resolution, "1280x720");
        assert_eq!(
            cfg.git,
            GitConfig {
                user_name: Some("Ada".to_string()),
                user_email: Some("ada@example.test".to_string()),
                vault_repo: Some("git@example.test:ada/vault.git".to_string()),
            }
        );
        assert_eq!(
            cfg.backup.restic_repository.as_deref(),
            Some("s3:https://s3.example.test/bucket")
        );
        assert_eq!(
            cfg.features,
            FeatureFlags {
                sync: true,
                remote_destruct: true,
                egress_enforce: true
            }
        );
        assert_eq!(
            cfg.egress_allow_cidrs.as_deref(),
            Some("10.0.0.0/8,192.168.1.5")
        );
    }

    #[test]
    fn the_egress_firewall_is_on_unless_switched_off() {
        let flag = |value: Option<&str>| {
            let pairs: Vec<(&str, &str)> =
                value.map(|v| ("EGRESS_ENFORCE", v)).into_iter().collect();
            load_config(env(&pairs)).features.egress_enforce
        };
        assert!(flag(None));
        assert!(flag(Some("")));
        assert!(flag(Some("1")));
        assert!(!flag(Some("0")));
        assert!(!flag(Some("false")));
        // Unreadable is not "off": a typo must not drop the protection.
        assert!(flag(Some("disabled")));
        // An empty allowlist is no allowlist.
        assert_eq!(
            load_config(env(&[("EGRESS_ALLOW_CIDRS", "")])).egress_allow_cidrs,
            None
        );
    }

    #[test]
    fn host_port_only_if_a_real_port() {
        assert_eq!(
            load_config(env(&[("TUNNEL_HOST_PORT", "18081")])).host_port,
            "18081"
        );
        assert_eq!(
            load_config(env(&[("TUNNEL_HOST_PORT", " 8080 ")])).host_port,
            "8080"
        );
        for bad in [
            "", "0", "65536", "99999", "abc", "80;rm", "-1", "1e3", "123456",
        ] {
            assert_eq!(
                load_config(env(&[("TUNNEL_HOST_PORT", bad)])).host_port,
                "18080",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn nothing_is_required_to_start() {
        // CLASH_SUBSCRIPTION_URL used to be mandatory; it is now optional, so the required set is
        // empty and an empty environment passes. Asserting it here makes re-introducing a hard
        // requirement a deliberate, visible change.
        assert!(REQUIRED_ENV_VARS.is_empty());
        let missing = find_missing_required_env(|_| None, REQUIRED_ENV_VARS);
        assert!(missing.is_empty());
        assert_eq!(env_validation_error(&missing), None);
    }

    // The required-env check itself stays in use (defense in depth, and so that re-adding a
    // required variable is one line); exercise it against a synthetic required list.
    const SAMPLE_REQUIRED: &[&str] = &["SAMPLE_REQUIRED"];

    #[test]
    fn present_but_empty_counts_as_missing() {
        let missing = find_missing_required_env(env(&[("SAMPLE_REQUIRED", "")]), SAMPLE_REQUIRED);
        assert_eq!(missing, ["SAMPLE_REQUIRED"]);
    }

    #[test]
    fn complete_set_passes() {
        let missing = find_missing_required_env(env(&[("SAMPLE_REQUIRED", "v")]), SAMPLE_REQUIRED);
        assert!(missing.is_empty());
        assert_eq!(env_validation_error(&missing), None);
    }

    proptest! {
        #[test]
        fn missing_required_is_complete_and_named(present in any::<bool>()) {
            let pairs: Vec<(&str, &str)> =
                if present { vec![("SAMPLE_REQUIRED", "v")] } else { vec![] };
            let missing = find_missing_required_env(env(&pairs), SAMPLE_REQUIRED);
            let error = env_validation_error(&missing);
            if present {
                prop_assert!(missing.is_empty());
                prop_assert!(error.is_none());
            } else {
                prop_assert_eq!(missing.len(), 1);
                prop_assert!(error.unwrap().contains("SAMPLE_REQUIRED"));
            }
        }
    }
}
