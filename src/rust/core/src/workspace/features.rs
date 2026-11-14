//! Feature flags — a port of packages/core/src/workspace/features.ts. The sync page and the remote
//! self-destruct endpoint are OFF unless explicitly enabled. Egress enforcement is the opposite: ON
//! unless explicitly disabled, because a workspace that leaks its real address by default is not
//! the product. A pure parser over an environment getter; the cli adapter supplies the real
//! `std::env`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureFlags {
    pub sync: bool,
    pub remote_destruct: bool,
    /// Enforce fail-closed egress with an nftables default-DROP filter (only loopback + Clash's
    /// own uid + the operator's allowlist may leave the box). ON by default: unset, empty or any
    /// value that is not a clear "no" means on. Needs CAP_NET_ADMIN and Clash running as its own
    /// uid; when it cannot be applied the boot fails closed rather than booting leaky.
    pub egress_enforce: bool,
}

const ON_WORDS: &[&str] = &["1", "true", "yes", "on"];
const OFF_WORDS: &[&str] = &["0", "false", "no", "off"];

fn normalised(value: Option<&str>) -> Option<String> {
    value.map(|s| s.trim().to_ascii_lowercase())
}

/// The same truthy set as the TS side: 1 / true / yes / on, case-insensitive, trimmed.
pub fn is_on(value: Option<&str>) -> bool {
    normalised(value).is_some_and(|v| ON_WORDS.contains(&v.as_str()))
}

/// The explicit "no" of a switch that is on by default: 0 / false / no / off, case-insensitive,
/// trimmed. Anything else (unset, empty, a typo) is NOT off: for a safety switch, an unreadable
/// value must not be read as permission to turn the protection off.
pub fn is_off(value: Option<&str>) -> bool {
    normalised(value).is_some_and(|v| OFF_WORDS.contains(&v.as_str()))
}

/// Whether a switch value is one that [`is_on`] / [`is_off`] understand (an empty one means "the
/// default"). The caller warns about the rest.
pub fn is_recognised_switch(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    v.is_empty() || ON_WORDS.contains(&v.as_str()) || OFF_WORDS.contains(&v.as_str())
}

pub fn parse_feature_flags<F: Fn(&str) -> Option<String>>(get: F) -> FeatureFlags {
    FeatureFlags {
        sync: is_on(get("SYNC_ENABLED").as_deref()),
        remote_destruct: is_on(get("REMOTE_DESTRUCT_ENABLED").as_deref()),
        egress_enforce: !is_off(get("EGRESS_ENFORCE").as_deref()),
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
    fn only_a_clear_no_turns_a_default_on_switch_off() {
        for v in ["0", "false", "FALSE", "No", " off ", "Off"] {
            assert!(is_off(Some(v)), "{v:?} should be off");
        }
        // Unset, empty, yes, and every value that is merely unreadable leave the switch on.
        for v in [
            "", " ", "1", "true", "on", "disabled", "of", "none", "0x0", "2",
        ] {
            assert!(!is_off(Some(v)), "{v:?} must not turn it off");
        }
        assert!(!is_off(None));
    }

    #[test]
    fn recognised_switch_values() {
        for v in ["", " ", "1", "0", "True", "OFF", " no ", "yes", "on"] {
            assert!(is_recognised_switch(v), "{v:?}");
        }
        for v in ["disabled", "enabled", "2", "of", "y", "n", "maybe"] {
            assert!(!is_recognised_switch(v), "{v:?}");
        }
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

        // Nothing set: the opt-in features are off, the egress firewall is on.
        let f = parse_feature_flags(|_| None);
        assert!(!f.sync && !f.remote_destruct && f.egress_enforce);

        let f = parse_feature_flags(|k| (k == "EGRESS_ENFORCE").then(|| "on".to_string()));
        assert!(f.egress_enforce && !f.sync && !f.remote_destruct);

        for off in ["0", "false", "no", "off", "OFF"] {
            let f = parse_feature_flags(|k| (k == "EGRESS_ENFORCE").then(|| off.to_string()));
            assert!(!f.egress_enforce, "{off}");
        }
        // An empty value is the default, which is on; so is one that cannot be read.
        for on in ["", "disabled"] {
            let f = parse_feature_flags(|k| (k == "EGRESS_ENFORCE").then(|| on.to_string()));
            assert!(f.egress_enforce, "{on:?}");
        }
    }
}
