//! Signed session tokens for the unified auth gateway (pure; no I/O).
//!
//! Cookie value: `v1.<iat>.<exp>.<hex HMAC-SHA256(key, "v1.<iat>.<exp>")>`. Verification recomputes
//! the MAC in constant time (hmac's `verify_slice`) and checks expiry. No PII — a single shared
//! master password is the whole identity model, so the token only carries issue/expiry seconds.
//!
//! The signing key is chosen by the sync-service: a fresh random 32 bytes per boot by default
//! (sessions end on restart), or, when AUTH_SESSION_PERSIST is set, a random key kept in
//! SESSION_KEY_PATH (sessions survive a restart). It is never derived from the master password: a
//! cookie is a MAC of known text under the key, so a key derived from the password would let anyone
//! who has seen one cookie test password guesses offline, and the password is the vault passphrase.
//! Cookies ride the WebSocket upgrade handshake, which is the whole point: one gate covers
//! code-server, KasmVNC and Selkies WebSockets uniformly.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Cookie name the gate sets and reads.
pub const SESSION_COOKIE: &str = "sdw_session";
/// Default session lifetime in seconds (12h).
pub const DEFAULT_SESSION_TTL_SECS: u64 = 12 * 60 * 60;
/// Where the persistent session key lives (AUTH_SESSION_PERSIST): 64 hex characters, mode 0600, in
/// the container's own file system (not the vault, not a volume), so it outlives a restart and
/// goes away with the container.
pub const SESSION_KEY_PATH: &str = "/var/lib/sdw/session.key";

/// Login throttle (every attempt, right or wrong, counts). The master password is also the vault
/// passphrase, so the login must not be an unlimited online guessing oracle: per claimed client
/// address, and across all addresses (the address comes from a header a direct caller can forge).
/// A flood of forged addresses can therefore lock new logins out for the rest of a window; existing
/// sessions and Cloudflare Access sign-ins are unaffected.
pub const LOGIN_RATE_LIMIT_WINDOW_MS: i64 = 60_000;
pub const LOGIN_RATE_LIMIT_PER_KEY: u64 = 10;
pub const LOGIN_RATE_LIMIT_GLOBAL: u64 = 60;

/// Auth routes — served by the sync-service, allowlisted (un-gated) in Caddy.
pub const AUTH_LOGIN_PATH: &str = "/auth/login";
pub const AUTH_VERIFY_PATH: &str = "/auth/verify";
pub const AUTH_LOGOUT_PATH: &str = "/auth/logout";

const TOKEN_VERSION: &str = "v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionVerdict {
    Valid,
    Expired,
    Invalid,
}

fn mac_hex(key: &[u8], body: &str) -> String {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(body.as_bytes());
    hex::encode(m.finalize().into_bytes())
}

/// A token signed for the window [iat, exp) (seconds since the epoch).
pub fn sign_session(key: &[u8], iat: u64, exp: u64) -> String {
    let body = format!("{TOKEN_VERSION}.{iat}.{exp}");
    let tag = mac_hex(key, &body);
    format!("{body}.{tag}")
}

/// Verify a token's signature (constant time) and expiry at `now` (seconds). Any structural flaw,
/// a wrong key, or a tampered field is `Invalid`; a well-signed but elapsed token is `Expired`.
pub fn verify_session(key: &[u8], token: &str, now: u64) -> SessionVerdict {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 4 || parts[0] != TOKEN_VERSION {
        return SessionVerdict::Invalid;
    }
    // exp must parse; iat is carried but not otherwise trusted.
    let exp = match (parts[1].parse::<u64>(), parts[2].parse::<u64>()) {
        (Ok(_iat), Ok(exp)) => exp,
        _ => return SessionVerdict::Invalid,
    };
    // Sign over the ORIGINAL field bytes, so re-serialization can never disagree.
    let body = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
    let got = match hex::decode(parts[3]) {
        Ok(b) => b,
        Err(_) => return SessionVerdict::Invalid,
    };
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(body.as_bytes());
    if m.verify_slice(&got).is_err() {
        return SessionVerdict::Invalid;
    }
    if now >= exp {
        SessionVerdict::Expired
    } else {
        SessionVerdict::Valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    #[test]
    fn signs_and_verifies_within_window() {
        let token = sign_session(KEY, 1000, 2000);
        assert_eq!(verify_session(KEY, &token, 1500), SessionVerdict::Valid);
        assert_eq!(verify_session(KEY, &token, 1000), SessionVerdict::Valid);
    }

    #[test]
    fn expired_after_exp() {
        let token = sign_session(KEY, 1000, 2000);
        assert_eq!(verify_session(KEY, &token, 2000), SessionVerdict::Expired);
        assert_eq!(verify_session(KEY, &token, 9999), SessionVerdict::Expired);
    }

    #[test]
    fn wrong_key_is_invalid() {
        let token = sign_session(KEY, 1000, 2000);
        assert_eq!(
            verify_session(b"the-other-32-byte-key-aaaaaaaaaa!", &token, 1500),
            SessionVerdict::Invalid
        );
    }

    #[test]
    fn tampering_is_invalid() {
        let token = sign_session(KEY, 1000, 2000);
        // Extend the expiry without re-signing.
        let mut p: Vec<&str> = token.split('.').collect();
        let forged = format!("{}.{}.{}.{}", p[0], p[1], "99999", p[3]);
        assert_eq!(verify_session(KEY, &forged, 1500), SessionVerdict::Invalid);
        // Flip the tag.
        p[3] = "deadbeef";
        let bad_tag = p.join(".");
        assert_eq!(verify_session(KEY, &bad_tag, 1500), SessionVerdict::Invalid);
    }

    #[test]
    fn malformed_is_invalid() {
        for t in [
            "",
            "v1",
            "v1.1.2",
            "v2.1.2.abcd",
            "v1.notanumber.2.abcd",
            "v1.1.2.nothex",
            "v1.1.2.3.4.5",
        ] {
            assert_eq!(verify_session(KEY, t, 1), SessionVerdict::Invalid, "{t:?}");
        }
    }
}
