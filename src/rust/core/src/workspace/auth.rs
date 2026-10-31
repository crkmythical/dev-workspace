//! The master password: ONE secret shared by code-server, the desktops and the vault.
//!
//! A port of packages/core/src/workspace/auth.ts. Pure: the caller reads the existing config,
//! supplies a generator, and writes the result. Tests mirror the auth half of
//! packages/core/tests/auth-banner.test.ts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordSource {
    /// PASSWORD in the environment.
    Env,
    /// Nothing configured: a random one was made and must be shown to the user.
    Generated,
    /// Read back from an existing code-server config.
    Existing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterPassword {
    pub value: String,
    pub source: PasswordSource,
    /// The code-server config has to be (re)written for this password to take effect.
    pub write_config: bool,
}

/// Precedence: an explicit PASSWORD always wins (and is written out); otherwise the password
/// already in code-server's config is kept, so restarts do not change it; otherwise a new one is
/// generated and written. Errs when PASSWORD contains a line break (it goes onto one YAML line,
/// a command line and a prompt).
pub fn resolve_master_password(
    env_password: Option<&str>,
    existing_config: Option<&str>,
    generate: impl FnOnce() -> String,
) -> Result<MasterPassword, String> {
    if let Some(ep) = env_password.filter(|s| !s.is_empty()) {
        if ep.contains(['\r', '\n']) {
            return Err("PASSWORD must be a single line".to_string());
        }
        return Ok(MasterPassword {
            value: ep.to_string(),
            source: PasswordSource::Env,
            write_config: true,
        });
    }
    if let Some(existing) = existing_config.and_then(parse_code_server_password) {
        return Ok(MasterPassword {
            value: existing,
            source: PasswordSource::Existing,
            write_config: false,
        });
    }
    Ok(MasterPassword {
        value: generate(),
        source: PasswordSource::Generated,
        write_config: true,
    })
}

/// code-server's config.yaml. The password is a double-quoted scalar (JSON string syntax is valid
/// YAML), so `#` or `: ` cannot change its meaning.
pub fn render_code_server_config(password: &str, port: u16) -> String {
    let quoted = serde_json::to_string(password).expect("a string serializes");
    format!("bind-addr: 127.0.0.1:{port}\nauth: password\npassword: {quoted}\ncert: false\n")
}

/// Read the password back; understands what `render` writes and the older unquoted/single-quoted
/// forms. None when there is no usable password.
pub fn parse_code_server_password(config: &str) -> Option<String> {
    // First line of the form `password:[ \t]*(.+)` (at least one char after the spaces).
    let value = config.lines().find_map(|line| {
        let rest = line.strip_prefix("password:")?;
        let rest = rest.trim_start_matches([' ', '\t']);
        if rest.is_empty() {
            None
        } else {
            Some(rest)
        }
    })?;
    let raw = value.trim();
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw)
            .ok()
            .filter(|s| !s.is_empty());
    }
    if raw.len() >= 2 && raw.starts_with('\'') && raw.ends_with('\'') {
        let inner = raw[1..raw.len() - 1].replace("''", "'");
        return if inner.is_empty() { None } else { Some(inner) };
    }
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn generate() -> String {
        "generated-pw".to_string()
    }

    #[test]
    fn env_password_wins_and_is_written() {
        let cfg = render_code_server_config("old", 8082);
        let r = resolve_master_password(Some("from-env"), Some(&cfg), generate).unwrap();
        assert_eq!(
            r,
            MasterPassword {
                value: "from-env".to_string(),
                source: PasswordSource::Env,
                write_config: true
            }
        );
    }

    #[test]
    fn keeps_existing_config_password() {
        let cfg = render_code_server_config("kept", 8082);
        let r = resolve_master_password(None, Some(&cfg), generate).unwrap();
        assert_eq!(
            r,
            MasterPassword {
                value: "kept".to_string(),
                source: PasswordSource::Existing,
                write_config: false
            }
        );
    }

    #[test]
    fn generates_when_no_usable_config() {
        let r = resolve_master_password(None, None, generate).unwrap();
        assert_eq!(r.source, PasswordSource::Generated);
        for cfg in ["auth: password\n", "password:\n", "password:   \n", ""] {
            let r = resolve_master_password(None, Some(cfg), generate).unwrap();
            assert_eq!(
                r,
                MasterPassword {
                    value: "generated-pw".to_string(),
                    source: PasswordSource::Generated,
                    write_config: true
                }
            );
        }
    }

    #[test]
    fn empty_password_is_unset() {
        let r = resolve_master_password(Some(""), None, generate).unwrap();
        assert_eq!(r.source, PasswordSource::Generated);
    }

    #[test]
    fn refuses_line_breaks() {
        for bad in ["a\nb", "a\r\nb", "line\r"] {
            let err = resolve_master_password(Some(bad), None, generate).unwrap_err();
            assert!(err.contains("single line"));
        }
    }

    #[test]
    fn renders_the_expected_fields() {
        assert_eq!(
            render_code_server_config("pw", 8082),
            "bind-addr: 127.0.0.1:8082\nauth: password\npassword: \"pw\"\ncert: false\n"
        );
    }

    #[test]
    fn survives_yaml_special_characters() {
        for pw in [
            "a #b",
            "x: y",
            "- z",
            "'q'",
            "\"d\"",
            "a\\b",
            "100%",
            "{braces}",
            "tab\there",
        ] {
            assert_eq!(
                parse_code_server_password(&render_code_server_config(pw, 8082)).as_deref(),
                Some(pw)
            );
        }
    }

    #[test]
    fn reads_older_unquoted_and_single_quoted_forms() {
        assert_eq!(
            parse_code_server_password("bind-addr: x\npassword: abc123\ncert: false\n").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parse_code_server_password("password:   spaced   \n").as_deref(),
            Some("spaced")
        );
        assert_eq!(
            parse_code_server_password("password: 'it''s'\n").as_deref(),
            Some("it's")
        );
        assert_eq!(
            parse_code_server_password("password: abc\r\ncert: false\r\n").as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn returns_none_without_a_password() {
        assert_eq!(parse_code_server_password("auth: none\n"), None);
        assert_eq!(parse_code_server_password("password: \"\"\n"), None);
        assert_eq!(
            parse_code_server_password("password: \"unterminated\n"),
            None
        );
    }

    proptest! {
        #[test]
        fn round_trips_any_single_line_password(
            pw in "[ -~]{1,40}".prop_filter("non-blank", |s| !s.trim().is_empty()),
        ) {
            let got = parse_code_server_password(&render_code_server_config(&pw, 8082));
            prop_assert_eq!(got.as_deref(), Some(pw.as_str()));
        }
    }
}
