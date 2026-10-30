//! Remote self-destruct key derivation — a port of packages/infra/src/destruct-key.ts.
//!
//! Used by the remote-destruct unlock hook (derive + store a bcrypt hash) and the
//! `destruct-passphrase` command (re-derive the passphrase on demand). The destruct key is a
//! deterministic HKDF of the vault passphrase, so rotating the vault password automatically
//! rotates the destruct key.

use base64::Engine;
use hkdf::Hkdf;
use sdw_core::vault::destruct::{DESTRUCT_HKDF_INFO, DESTRUCT_HKDF_SALT, DESTRUCT_KEY_LENGTH};
use sha2::Sha256;

/// Derive a 32-byte destruct key from the vault passphrase via HKDF-SHA256.
pub fn derive_destruct_key(passphrase: &str) -> [u8; DESTRUCT_KEY_LENGTH] {
    let hk = Hkdf::<Sha256>::new(Some(DESTRUCT_HKDF_SALT.as_bytes()), passphrase.as_bytes());
    let mut okm = [0u8; DESTRUCT_KEY_LENGTH];
    hk.expand(DESTRUCT_HKDF_INFO.as_bytes(), &mut okm)
        .expect("DESTRUCT_KEY_LENGTH is a valid HKDF output length");
    okm
}

/// Encode the destruct key as a base64 passphrase for display and later bcrypt verification.
pub fn destruct_key_to_passphrase(key: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parity vectors generated from the TS (node:crypto), which is what the stored bcrypt hash and
    // the user's saved passphrase were made with:
    //   crypto.hkdfSync("sha256", pass, "destruct-key-v1", "self-destruct", 32) -> base64
    #[test]
    fn matches_the_typescript_derivation() {
        let cases = [
            (
                "correct horse battery",
                "OiKVtTF8XNiz7BImuFI/IRJFT9S6X9KfAAeD/FBpuCI=",
            ),
            ("hunter2", "08ApbP0opevP136tfE9i9JpXJ35ZasJ/pqyJUzB6yFg="),
            ("", "ONY0IW0L5OLu1ry7rqI7SqJ6OvRR+TQnkPw1R5U+yZA="),
        ];
        for (passphrase, expected) in cases {
            assert_eq!(
                destruct_key_to_passphrase(&derive_destruct_key(passphrase)),
                expected,
                "passphrase {passphrase:?}"
            );
        }
    }

    #[test]
    fn is_deterministic_and_passphrase_bound() {
        assert_eq!(
            derive_destruct_key("correct horse battery"),
            derive_destruct_key("correct horse battery")
        );
        assert_ne!(derive_destruct_key("a"), derive_destruct_key("b"));
        assert_eq!(derive_destruct_key("x").len(), DESTRUCT_KEY_LENGTH);
    }
}
