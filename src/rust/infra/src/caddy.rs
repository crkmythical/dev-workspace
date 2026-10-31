//! Caddy helpers — a port of packages/infra/src/caddy.ts.

use crate::cmd;

/// bcrypt hash of a password in the form Caddy's `basic_auth` expects (it guards the desktops).
/// Empty when `caddy` cannot produce one; the caller decides what that means.
pub fn caddy_password_hash(password: &str) -> String {
    cmd::capture("caddy", &["hash-password", "--plaintext", password])
        .1
        .trim()
        .to_string()
}
