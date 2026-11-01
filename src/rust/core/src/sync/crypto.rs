//! AEAD primitives — must match packages/server/src/crypto.ts byte-for-byte.
//!
//! HKDF-SHA256 (RFC 5869) for key derivation; AES-256-GCM with a 12-byte nonce
//! and the WebCrypto wire layout `ciphertext || tag`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

use super::protocol::{AES_KEY_LENGTH, HKDF_INFO, HKDF_SALT};

/// Derive the 32-byte AEAD key from a passphrase (HKDF-SHA256).
pub fn derive_key(passphrase: &str) -> [u8; AES_KEY_LENGTH] {
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT.as_bytes()), passphrase.as_bytes());
    let mut okm = [0u8; AES_KEY_LENGTH];
    hk.expand(HKDF_INFO.as_bytes(), &mut okm)
        .expect("AES_KEY_LENGTH is a valid HKDF output length");
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
