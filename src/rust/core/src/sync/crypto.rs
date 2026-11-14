//! Sync key derivation + AEAD shared by the Rust server, native client and browser WASM.
//!
//! PBKDF2-HMAC-SHA256 (password stretching) for the key; AES-256-GCM with a 12-byte nonce and the
//! WebCrypto wire layout `ciphertext || tag`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;

use super::protocol::{AES_KEY_LENGTH, SYNC_KDF_ITERATIONS, SYNC_KDF_SALT};

/// Derive the 32-byte AEAD sync key from the vault passphrase with PBKDF2-HMAC-SHA256.
///
/// The passphrase is the master password and these envelopes cross Cloudflare (a MITM boundary in
/// the threat model), so it is PASSWORD-STRETCHED (600k iterations), not merely HKDF-expanded: a
/// captured envelope cannot be brute-forced at raw-hash speed. The frozen vectors in
/// sync-service/tests/parity.rs pin the bytes produced by earlier releases.
pub fn derive_key(passphrase: &str) -> [u8; AES_KEY_LENGTH] {
    let mut okm = [0u8; AES_KEY_LENGTH];
    pbkdf2_hmac::<Sha256>(
        passphrase.as_bytes(),
        SYNC_KDF_SALT.as_bytes(),
        SYNC_KDF_ITERATIONS,
        &mut okm,
    );
    okm
}

/// Encrypt, returning `ciphertext || tag` (tag appended, 16 bytes).
pub fn encrypt(key: &[u8], nonce: &[u8], plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| e.to_string())
}

/// Decrypt `ciphertext || tag`. Fails on tag/AAD mismatch.
pub fn decrypt(
    key: &[u8],
    nonce: &[u8],
    ct_with_tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ct_with_tag,
                aad,
            },
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_deterministic_and_passphrase_bound() {
        // Exact bytes are pinned cross-language in parity.rs; here we pin the shape: same
        // passphrase -> same key, different passphrase -> different key, always 32 bytes.
        assert_eq!(derive_key("hunter2"), derive_key("hunter2"));
        assert_ne!(derive_key("a"), derive_key("b"));
        assert_eq!(derive_key("x").len(), AES_KEY_LENGTH);
    }
}
