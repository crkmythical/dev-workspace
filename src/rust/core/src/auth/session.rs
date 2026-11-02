//! Signed session tokens for the unified auth gateway (pure; no I/O).
//!
//! Cookie value: `v1.<iat>.<exp>.<hex HMAC-SHA256(key, "v1.<iat>.<exp>")>`. Verification recomputes
//! the MAC in constant time (hmac's `verify_slice`) and checks expiry. No PII — a single shared
//! master password is the whole identity model, so the token only carries issue/expiry seconds.
//!
//! The signing key is chosen by the sync-service: a fresh random 32 bytes per boot by default
//! (sessions end on restart), or `derive_session_key_from_password` when AUTH_SESSION_PERSIST is
//! set (sessions survive a restart). Cookies ride the WebSocket upgrade handshake, which is the
//! whole point: one gate covers code-server, KasmVNC and Selkies WebSockets uniformly.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Cookie name the gate sets and reads.
pub const SESSION_COOKIE: &str = "sdw_session";
/// Default session lifetime in seconds (12h).
pub const DEFAULT_SESSION_TTL_SECS: u64 = 12 * 60 * 60;

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

/// Derive a stable 32-byte key from the master password (AUTH_SESSION_PERSIST mode) so sessions
/// survive a restart. HMAC-SHA256 keyed by the password over a fixed, auth-specific label — a
/// PRF whose output is domain-separated from the vault and sync keys by that label. The default
/// (random per boot) does not call this.
pub fn derive_session_key_from_password(password: &str) -> [u8; 32] {
    let mut m =
        HmacSha256::new_from_slice(password.as_bytes()).expect("HMAC accepts any key length");
    m.update(b"sdw-session-key-v1");
    let out = m.finalize().into_bytes();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out[..32]); // SHA-256 is exactly 32 bytes
    key
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

    #[test]
    fn derive_is_deterministic_and_password_specific() {
        let a = derive_session_key_from_password("hunter2");
        let b = derive_session_key_from_password("hunter2");
        let c = derive_session_key_from_password("hunter3");
        assert_eq!(a, b);
        assert_ne!(a, c);
        // A token signed with a derived key verifies with the same derivation (restart survival).
        let t = sign_session(&a, 10, 20);
        assert_eq!(verify_session(&b, &t, 15), SessionVerdict::Valid);
    }
}
