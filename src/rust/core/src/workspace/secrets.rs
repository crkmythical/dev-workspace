//! Secrets supplied as files instead of environment variables.
//!
//! The environment of a container is visible to everything that runs in it (`/proc/<pid>/environ`
//! of each child, `docker inspect`, crash reports). For each name in [`SECRET_ENV_VARS`] the value
//! may therefore also come from a file: `<NAME>_FILE` holds the path (Docker/Compose secrets are
//! mounted at `/run/secrets/<name>`), and only that path is in the environment.
//!
//! Precedence: a non-empty `<NAME>` wins, so a value left in `.env` is not silently ignored (the
//! secrets overlay blanks the variables it replaces). A `<NAME>_FILE` that cannot be read is an
//! error, never "unset": a missing PASSWORD would otherwise turn into a generated one.
//!
//! Pure: the environment and the file reader are passed in.

use std::fmt;

/// The variables that may be given as `<NAME>_FILE`.
pub const SECRET_ENV_VARS: &[&str] = &[
    "PASSWORD",
    "CLASH_SUBSCRIPTION_URL",
    "VAULT_GIT_REPO",
    "RESTIC_PASSWORD",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "PENTEST_VAULT_PASSWORD",
    "CROC_PASS",
];

pub fn is_secret_var(name: &str) -> bool {
    SECRET_ENV_VARS.contains(&name)
}

/// The variable naming the file: `PASSWORD` -> `PASSWORD_FILE`.
pub fn file_var(name: &str) -> String {
    format!("{name}_FILE")
}

/// `<NAME>_FILE` is set but its file cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFileError {
    pub var: String,
    pub path: String,
    pub reason: String,
}

impl fmt::Display for SecretFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}={}: cannot read the secret file ({})",
            self.var, self.path, self.reason
        )
    }
}

/// One trailing newline is an artefact of `echo secret > file`, not part of the secret.
fn strip_one_newline(s: &str) -> &str {
    s.strip_suffix("\r\n")
        .or_else(|| s.strip_suffix('\n'))
        .unwrap_or(s)
}

/// Resolve `name` from the environment or from `<name>_FILE`. `read` returns the file's text or the
/// reason it failed. `Ok(None)` means neither is set (or the file is empty).
pub fn resolve(
    name: &str,
    get: &dyn Fn(&str) -> Option<String>,
    read: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Option<String>, SecretFileError> {
    if let Some(v) = get(name).filter(|v| !v.is_empty()) {
        return Ok(Some(v));
    }
    let var = file_var(name);
    let Some(path) = get(&var).filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    match read(&path) {
        Ok(text) => Ok(Some(strip_one_newline(&text).to_string()).filter(|v| !v.is_empty())),
        Err(reason) => Err(SecretFileError { var, path, reason }),
    }
}

/// Every `<NAME>_FILE` that is in use (its variable is not set directly) and cannot be read.
pub fn unreadable_files(
    get: &dyn Fn(&str) -> Option<String>,
    read: &dyn Fn(&str) -> Result<String, String>,
) -> Vec<SecretFileError> {
    SECRET_ENV_VARS
        .iter()
        .filter_map(|name| resolve(name, get, read).err())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn files(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |p| m.get(p).cloned().ok_or_else(|| "No such file".to_string())
    }

    #[test]
    fn the_environment_variable_wins() {
        let get = env(&[
            ("PASSWORD", "from-env"),
            ("PASSWORD_FILE", "/run/secrets/p"),
        ]);
        let read = files(&[("/run/secrets/p", "from-file\n")]);
        assert_eq!(
            resolve("PASSWORD", &get, &read),
            Ok(Some("from-env".to_string()))
        );
    }

    #[test]
    fn an_empty_variable_falls_through_to_the_file() {
        // The compose overlay blanks PASSWORD so a stale .env value cannot shadow the secret.
        let get = env(&[("PASSWORD", ""), ("PASSWORD_FILE", "/run/secrets/p")]);
        let read = files(&[("/run/secrets/p", "from-file\n")]);
        assert_eq!(
            resolve("PASSWORD", &get, &read),
            Ok(Some("from-file".to_string()))
        );
    }

    #[test]
    fn only_one_trailing_newline_is_removed() {
        let get = env(&[("PASSWORD_FILE", "/p")]);
        assert_eq!(
            resolve("PASSWORD", &get, &files(&[("/p", "s3cret \n")])),
            Ok(Some("s3cret ".to_string()))
        );
        assert_eq!(
            resolve("PASSWORD", &get, &files(&[("/p", "a\r\n")])),
            Ok(Some("a".to_string()))
        );
        assert_eq!(
            resolve("PASSWORD", &get, &files(&[("/p", "a\n\n")])),
            Ok(Some("a\n".to_string()))
        );
    }

    #[test]
    fn neither_set_is_none_and_an_empty_file_is_none() {
        let read = files(&[("/empty", "\n")]);
        assert_eq!(resolve("PASSWORD", &env(&[]), &read), Ok(None));
        assert_eq!(
            resolve("PASSWORD", &env(&[("PASSWORD_FILE", "/empty")]), &read),
            Ok(None)
        );
    }

    #[test]
    fn an_unreadable_file_is_an_error_not_unset() {
        let get = env(&[("PASSWORD_FILE", "/run/secrets/missing")]);
        let err = resolve("PASSWORD", &get, &files(&[])).unwrap_err();
        assert_eq!(err.var, "PASSWORD_FILE");
        assert_eq!(err.path, "/run/secrets/missing");
        assert!(err.to_string().contains("cannot read the secret file"));
    }

    #[test]
    fn unreadable_files_lists_only_the_ones_in_use() {
        let get = env(&[
            ("PASSWORD_FILE", "/ok"),
            ("RESTIC_PASSWORD_FILE", "/gone"),
            // Set directly, so its file is never looked at.
            ("CROC_PASS", "x"),
            ("CROC_PASS_FILE", "/also-gone"),
        ]);
        let errs = unreadable_files(&get, &files(&[("/ok", "x")]));
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].var, "RESTIC_PASSWORD_FILE");
    }

    #[test]
    fn the_secret_set_is_the_documented_one() {
        for n in [
            "PASSWORD",
            "RESTIC_PASSWORD",
            "AWS_SECRET_ACCESS_KEY",
            "CLASH_SUBSCRIPTION_URL",
        ] {
            assert!(is_secret_var(n), "{n}");
        }
        assert!(!is_secret_var("TZ"));
        assert_eq!(file_var("PASSWORD"), "PASSWORD_FILE");
    }
}
