//! The master password: ONE secret, the vault passphrase and the gate's login.
//!
//! code-server and the desktops no longer have a password of their own: the session-cookie gate in
//! front of every route is the only perimeter, so there is no second copy of the password in a
//! config file the workload could read.
//!
//! Pure: the caller reads the saved password, supplies a generator, and writes the result.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordSource {
    /// PASSWORD in the environment.
    Env,
    /// Nothing configured: a random one was made and must be shown to the user.
    Generated,
    /// A password an earlier start generated, read back from the root-only file it was saved in.
    Saved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterPassword {
    pub value: String,
    pub source: PasswordSource,
    /// The password has to be saved for a restart to find it again (only a generated one: a
    /// configured PASSWORD comes back from the environment by itself).
    pub save: bool,
}

/// Precedence: an explicit PASSWORD always wins; otherwise the password an earlier start generated
/// and saved, so a restart opens the same vault; otherwise a new one is generated (and must be
/// saved and shown). Errs when PASSWORD contains a line break (it goes onto a prompt and into
/// files that hold one value per line).
pub fn resolve_master_password(
    env_password: Option<&str>,
    saved: Option<&str>,
    generate: impl FnOnce() -> String,
) -> Result<MasterPassword, String> {
    if let Some(ep) = env_password.filter(|s| !s.is_empty()) {
        if ep.contains(['\r', '\n']) {
            return Err("PASSWORD must be a single line".to_string());
        }
        return Ok(MasterPassword {
            value: ep.to_string(),
            source: PasswordSource::Env,
            save: false,
        });
    }
    if let Some(kept) = saved.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(MasterPassword {
            value: kept.to_string(),
            source: PasswordSource::Saved,
            save: false,
        });
    }
    Ok(MasterPassword {
        value: generate(),
        source: PasswordSource::Generated,
        save: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate() -> String {
        "generated-pw".to_string()
    }

    #[test]
    fn env_password_wins_over_a_saved_one_and_is_not_saved() {
        let r = resolve_master_password(Some("from-env"), Some("old"), generate).unwrap();
        assert_eq!(
            r,
            MasterPassword {
                value: "from-env".to_string(),
                source: PasswordSource::Env,
                save: false
            }
        );
    }

    #[test]
    fn a_saved_password_is_kept_across_restarts() {
        let r = resolve_master_password(None, Some("kept\n"), generate).unwrap();
        assert_eq!(
            r,
            MasterPassword {
                value: "kept".to_string(),
                source: PasswordSource::Saved,
                save: false
            }
        );
    }

    #[test]
    fn generates_and_asks_to_save_when_nothing_is_configured_or_saved() {
        for saved in [None, Some(""), Some("  \n")] {
            let r = resolve_master_password(None, saved, generate).unwrap();
            assert_eq!(
                r,
                MasterPassword {
                    value: "generated-pw".to_string(),
                    source: PasswordSource::Generated,
                    save: true
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
}
