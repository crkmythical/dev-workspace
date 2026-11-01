//! Optional feature flags — a port of packages/core/src/workspace/features.ts. All OFF unless
//! explicitly enabled. A pure parser over an environment getter; the cli adapter supplies the
//! real `std::env`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureFlags {
    pub sync: bool,
    pub remote_destruct: bool,
    /// Enforce fail-closed egress with an nftables default-DROP filter (only loopback + Clash's
    /// own uid may leave the box). Needs CAP_NET_ADMIN and Clash running as its own uid, so it is
    /// opt-in; when on but unapplicable the boot fails closed rather than booting leaky.
    pub egress_enforce: bool,
}

/// The same truthy set as the TS side: 1 / true / yes / on, case-insensitive, trimmed.
pub fn is_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn parse_feature_flags<F: Fn(&str) -> Option<String>>(get: F) -> FeatureFlags {
    FeatureFlags {
        sync: is_on(get("SYNC_ENABLED").as_deref()),
        remote_destruct: is_on(get("REMOTE_DESTRUCT_ENABLED").as_deref()),
        egress_enforce: is_on(get("EGRESS_ENFORCE").as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_and_falsey() {
        for v in ["1", "true", "TRUE", "Yes", " on ", "On"] {
            assert!(is_on(Some(v)), "{v:?} should be on");
        }
        for v in ["", "0", "false", "no", "off", "enabled?", " "] {
            assert!(!is_on(Some(v)), "{v:?} should be off");
        }
        assert!(!is_on(None));
    }

    #[test]
    fn parse_from_env() {
        let f = parse_feature_flags(|k| (k == "SYNC_ENABLED").then(|| "1".to_string()));
        assert!(f.sync);
        assert!(!f.remote_destruct);

        let f =
            parse_feature_flags(|k| (k == "REMOTE_DESTRUCT_ENABLED").then(|| "yes".to_string()));
        assert!(!f.sync);
        assert!(f.remote_destruct);

        let f = parse_feature_flags(|_| None);
        assert!(!f.sync && !f.remote_destruct && !f.egress_enforce);

        let f = parse_feature_flags(|k| (k == "EGRESS_ENFORCE").then(|| "on".to_string()));
        assert!(f.egress_enforce && !f.sync && !f.remote_destruct);
    }
}
