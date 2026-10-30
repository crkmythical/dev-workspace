//! Optional features, all OFF unless enabled in the environment — a port of
//! packages/core/src/workspace/features.ts.
//!
//!   SYNC_ENABLED=1             the browser sync page + its upload/download API.
//!   REMOTE_DESTRUCT_ENABLED=1  POST /sync/api/destruct.
//!
//! A feature that is off makes its route answer 404, indistinguishable from a
//! route that does not exist.

#[derive(Clone, Copy, Debug)]
pub struct Features {
    pub sync: bool,
    pub remote_destruct: bool,
}

impl Features {
    pub fn from_env() -> Self {
        Self {
            sync: is_on(std::env::var("SYNC_ENABLED").ok().as_deref()),
            remote_destruct: is_on(std::env::var("REMOTE_DESTRUCT_ENABLED").ok().as_deref()),
        }
    }
}

/// The same truthy set as the TS side: 1 / true / yes / on, case-insensitive.
fn is_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|s| s.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

#[cfg(test)]
mod tests {
    use super::is_on;

    #[test]
    fn truthy_values() {
        for v in ["1", "true", "TRUE", "Yes", " on ", "On"] {
            assert!(is_on(Some(v)), "{v:?} should be on");
        }
    }

    #[test]
    fn falsey_values() {
        for v in ["", "0", "false", "no", "off", "enabled?", " "] {
            assert!(!is_on(Some(v)), "{v:?} should be off");
        }
        assert!(!is_on(None));
    }
}
