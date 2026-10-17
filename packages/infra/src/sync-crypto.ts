/**
 * Node implementation of the sync AEAD (moved from server/src/crypto.ts).
 *
 * Interoperates byte for byte with the WebCrypto client in @sdw/core/sync, which
 * packages/infra/tests/sync-interop.test.ts proves against that very client and
 * against the golden wire vectors shared with the Rust PoC.
 */
import crypto from "node:crypto";
import { AES_KEY_LENGTH, type AeadPort, HKDF_INFO, HKDF_SALT, NONCE_LENGTH } from "@sdw/core/sync";

export function deriveKey(passphrase: string, salt = HKDF_SALT): Buffer {
  return Buffer.from(crypto.hkdfSync("sha256", passphrase, salt, HKDF_INFO, AES_KEY_LENGTH));
}

export function encrypt(
  key: Uint8Array,
  plaintext: Uint8Array,
  aad: Uint8Array,
): { nonce: Buffer; ciphertext: Buffer; tag: Buffer } {
  const nonce = crypto.randomBytes(NONCE_LENGTH);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, nonce);
  cipher.setAAD(aad);
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  return { nonce, ciphertext, tag: cipher.getAuthTag() };
}

/** Throws when authentication fails. */
export function decrypt(
  key: Uint8Array,
  nonce: Uint8Array,
  ciphertext: Uint8Array,
  tag: Uint8Array,
  aad: Uint8Array,
): Buffer {
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, nonce);
  decipher.setAuthTag(tag);
  decipher.setAAD(aad);
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

export function keyFingerprint(key: Uint8Array): string {
  return crypto.createHash("sha256").update(key).digest("hex").slice(0, 32);
}

/** The AeadPort over node:crypto. */
export const nodeAead: AeadPort = {
  async open(key, nonce, ciphertext, tag, aad) {
    try {
      return decrypt(key, nonce, ciphertext, tag, aad);
    } catch {
      return null;
    }
  },
};
