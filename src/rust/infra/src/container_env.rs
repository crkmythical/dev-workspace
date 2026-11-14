//! The container's environment as it was given at start, for root tools reached through the bridge.
//!
//! A command the workload runs as root (`unlock-vault`, `doctor`, ...) goes through `doas`, which
//! resets the environment: the workload's own variables must not reach root, and doas cannot tell
//! which of them are harmless. But those tools are configured through the container's environment
//! (`RESTIC_*`, `CLASH_SUBSCRIPTION_URL`, the feature flags). PID 1 holds exactly that, root can read
//! it, and the workload cannot (`/proc/1/environ` is root's).

use sdw_core::workspace::workload::CONTAINER_ENV_SOURCE;

/// `NAME=value` entries separated by NUL, as /proc/<pid>/environ has them. Entries without a `=`
/// are ignored; the value may contain anything but NUL.
pub fn parse_environ(raw: &[u8]) -> Vec<(String, String)> {
    raw.split(|b| *b == 0)
        .filter_map(|entry| {
            let entry = String::from_utf8_lossy(entry);
            let (name, value) = entry.split_once('=')?;
            (!name.is_empty()).then(|| (name.to_string(), value.to_string()))
        })
        .collect()
}

/// Set every variable `path` holds that this process does not already have. What the caller was
/// started with (HOME, PATH, TERM from doas) wins. Returns how many were added.
pub fn adopt_from(path: &str) -> usize {
    let Ok(raw) = std::fs::read(path) else {
        return 0;
    };
    let mut added = 0;
    for (name, value) in parse_environ(&raw) {
        if std::env::var_os(&name).is_none() {
            std::env::set_var(&name, value);
            added += 1;
        }
    }
    added
}

/// [`adopt_from`] the container's start-up environment.
pub fn adopt() -> usize {
    adopt_from(CONTAINER_ENV_SOURCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environ_entries_are_split_on_nul_and_the_first_equals() {
        let parsed = parse_environ(b"A=1\0B=x=y\0EMPTY=\0NOEQUALS\0=novalue\0");
        assert_eq!(
            parsed,
            [
                ("A".to_string(), "1".to_string()),
                ("B".to_string(), "x=y".to_string()),
                ("EMPTY".to_string(), String::new()),
            ]
        );
        assert!(parse_environ(b"").is_empty());
    }

    #[test]
    fn adopting_fills_gaps_and_never_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("environ");
        std::fs::write(
            &file,
            b"SDW_TEST_ADOPT_NEW=from-container\0SDW_TEST_ADOPT_KEPT=from-container\0",
        )
        .unwrap();
        // Only this test touches these names, so the process environment is safe to set here.
        std::env::remove_var("SDW_TEST_ADOPT_NEW");
        std::env::set_var("SDW_TEST_ADOPT_KEPT", "from-caller");
        assert_eq!(adopt_from(file.to_str().unwrap()), 1);
        assert_eq!(
            std::env::var("SDW_TEST_ADOPT_NEW").unwrap(),
            "from-container"
        );
        assert_eq!(std::env::var("SDW_TEST_ADOPT_KEPT").unwrap(), "from-caller");
        std::env::remove_var("SDW_TEST_ADOPT_NEW");
        std::env::remove_var("SDW_TEST_ADOPT_KEPT");
        assert_eq!(adopt_from("/nonexistent/environ"), 0);
    }
}
