/**
 * Server-side cryptography using Node crypto module (via Bun compatibility).
 */
import crypto from "node:crypto";
import { HKDF_SALT, HKDF_INFO, AES_KEY_LENGTH, NONCE_LENGTH } from "../../core/src/constants.ts";

export function deriveKey(passphrase: string, salt = HKDF_SALT): Buffer {
  return Buffer.from(crypto.hkdfSync("sha256", passphrase, salt, HKDF_INFO, AES_KEY_LENGTH));
}

export function encrypt(
  key: Buffer,
  plaintext: Buffer,
  aad: Uint8Array,
): { nonce: Buffer; ciphertext: Buffer; tag: Buffer } {
  const nonce = crypto.randomBytes(NONCE_LENGTH);
  const cipher = crypto.createCipheriv("aes-256-gcm", key, nonce);
  cipher.setAAD(Buffer.from(aad));
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final()]);
  const tag = cipher.getAuthTag();
  return { nonce, ciphertext, tag };
}

export function decrypt(
  key: Buffer,
  nonce: Buffer,
  ciphertext: Buffer,
  tag: Buffer,
  aad: Uint8Array,
): Buffer {
  const decipher = crypto.createDecipheriv("aes-256-gcm", key, nonce);
  decipher.setAuthTag(tag);
  decipher.setAAD(Buffer.from(aad));
  return Buffer.concat([decipher.update(ciphertext), decipher.final()]);
}

export function keyFingerprint(key: Buffer): string {
  return crypto.createHash("sha256").update(key).digest("hex").slice(0, 32);
}
