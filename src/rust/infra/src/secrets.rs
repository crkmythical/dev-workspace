//! The real environment and file system behind `sdw_core::workspace::secrets`: a secret comes from
//! its environment variable or from the file named by `<NAME>_FILE` (Docker/Compose secrets).

use sdw_core::workspace::secrets::{
    is_secret_var, resolve, unreadable_files, SecretFileError, SECRET_ENV_VARS,
};
use std::process::Command;

pub fn read_secret_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

fn env_get(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// `std::env::var` for an ordinary variable. For a secret one, also `<NAME>_FILE`. A file that
/// cannot be read is reported on stderr and reads as unset; the boot's preflight step has already
/// refused to start on it, so this only matters to commands run by hand.
pub fn env_or_secret(name: &str) -> Option<String> {
    if !is_secret_var(name) {
        return env_get(name);
    }
    match resolve(name, &env_get, &read_secret_file) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            None
        }
    }
}

/// `bytes` random bytes from the kernel as lower-case hex. Err when /dev/urandom cannot be read:
/// there is deliberately no weaker fallback, since the callers mint keys and passwords with it.
pub fn random_hex(bytes: usize) -> Result<String, String> {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .map_err(|e| format!("cannot read /dev/urandom: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// 64 hex characters as 32 bytes; None for anything else.
fn parse_hex_key(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(key)
}

/// Write `text` to `path` so that only the owner can ever read it: the directory is created 0700,
/// the file is written 0600 to a sibling temp file, then renamed over the target (a reader sees the
/// old or the new content, never half).
pub fn write_private_file(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|()| f.sync_all()))
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("cannot write {}: {e}", path.display())
        })
}

/// A random 32-byte key kept in `path` as 64 hex characters, created on first use with mode 0600
/// (its directory 0700). A file that is not such a key is replaced rather than trusted: a damaged
/// key must not become a predictable one. For keys that have to outlive a process, such as the
/// session-signing key under AUTH_SESSION_PERSIST.
pub fn persistent_key(path: &std::path::Path) -> Result<[u8; 32], String> {
    if let Some(key) = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_hex_key(text.trim()))
    {
        return Ok(key);
    }
    let hex = random_hex(32)?;
    let key = parse_hex_key(&hex).ok_or_else(|| "a fresh key did not parse".to_string())?;
    write_private_file(path, &format!("{hex}\n"))?;
    Ok(key)
}

/// Every `<NAME>_FILE` in use whose file cannot be read (empty = all good).
pub fn unreadable_secret_files() -> Vec<SecretFileError> {
    unreadable_files(&env_get, &read_secret_file)
}

/// Hand the named secrets to a child through ITS environment. A child that reads a secret from
/// its environment (restic, croc) then gets it even when ours only holds a `<NAME>_FILE` path.
pub fn pass_secrets(command: &mut Command, names: &[&str]) {
    for name in names {
        debug_assert!(
            SECRET_ENV_VARS.contains(name),
            "{name} is not a secret variable"
        );
        if let Some(value) = env_or_secret(name) {
            command.env(name, value);
        }
    }
}

/// The variables restic reads for the repository password and the S3 credentials.
pub const RESTIC_SECRETS: &[&str] = &[
    "RESTIC_PASSWORD",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_file_reaches_the_child_environment_and_the_variable_wins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("restic");
        std::fs::write(&file, "from-file\n").unwrap();
        // Only this test touches these names, so the process environment is safe to set here.
        std::env::set_var("RESTIC_PASSWORD_FILE", &file);
        std::env::remove_var("RESTIC_PASSWORD");
        assert_eq!(
            env_or_secret("RESTIC_PASSWORD").as_deref(),
            Some("from-file")
        );
        let mut c = Command::new("true");
        pass_secrets(&mut c, &["RESTIC_PASSWORD"]);
        let seen: Vec<_> = c.get_envs().collect();
        assert!(seen
            .iter()
            .any(|(k, v)| *k == "RESTIC_PASSWORD" && v.is_some_and(|v| v == "from-file")));

        std::env::set_var("RESTIC_PASSWORD", "from-env");
        assert_eq!(
            env_or_secret("RESTIC_PASSWORD").as_deref(),
            Some("from-env")
        );
        std::env::remove_var("RESTIC_PASSWORD");

        std::env::set_var("RESTIC_PASSWORD_FILE", dir.path().join("missing"));
        assert!(unreadable_secret_files()
            .iter()
            .any(|e| e.var == "RESTIC_PASSWORD_FILE"));
        std::env::remove_var("RESTIC_PASSWORD_FILE");
    }

    #[test]
    fn random_hex_is_hex_of_the_requested_length_and_not_constant() {
        let a = random_hex(32).unwrap();
        let b = random_hex(32).unwrap();
        assert_eq!(a.len(), 64);
        assert!(a
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
        assert_eq!(random_hex(0).unwrap(), "");
    }

    #[test]
    fn a_persistent_key_is_created_private_and_then_reused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/session.key");
        let first = persistent_key(&path).unwrap();
        assert_eq!(
            persistent_key(&path).unwrap(),
            first,
            "the same key on the next start"
        );
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim().len(), 64);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn a_damaged_key_file_is_replaced_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.key");
        for damaged in ["", "short", &"zz".repeat(32), &"ab".repeat(33)] {
            std::fs::write(&path, damaged).unwrap();
            let key = persistent_key(&path).unwrap();
            assert_eq!(persistent_key(&path).unwrap(), key, "{damaged:?}");
            assert_ne!(key, [0u8; 32]);
        }
    }

    #[test]
    fn hex_keys_parse_exactly() {
        assert_eq!(parse_hex_key(&"00".repeat(32)), Some([0u8; 32]));
        assert_eq!(parse_hex_key(&"ff".repeat(32)), Some([255u8; 32]));
        assert_eq!(parse_hex_key(&"0".repeat(63)), None);
        assert_eq!(parse_hex_key(&"g0".repeat(32)), None);
        assert_eq!(parse_hex_key("é".repeat(32).as_str()), None);
    }

    #[test]
    fn ordinary_variables_do_not_look_for_a_file() {
        std::env::set_var("SDW_TEST_PLAIN_FILE", "/nonexistent");
        std::env::remove_var("SDW_TEST_PLAIN");
        assert_eq!(env_or_secret("SDW_TEST_PLAIN"), None);
        std::env::remove_var("SDW_TEST_PLAIN_FILE");
    }
}
