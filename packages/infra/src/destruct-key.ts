/**
 * Remote self-destruct key derivation (moved from core/crypto-utils.ts: it needs
 * node:crypto, and core must stay runtime-neutral so the browser SPA can import it).
 *
 * Used by:
 *   - the remote-destruct unlock hook (derive + store a bcrypt hash)
 *   - the `destruct-passphrase` command (re-derive the passphrase on demand)
 */
import crypto from "node:crypto";
import { AES_KEY_LENGTH, DESTRUCT_HKDF_INFO, DESTRUCT_HKDF_SALT } from "@sdw/core/constants";

/**
 * Derive a 32-byte destruct key from the vault passphrase via HKDF-SHA256.
 * The destruct key is a deterministic function of the passphrase, so rotating
 * the vault password automatically rotates the destruct key.
 */
export function deriveDestructKey(passphrase: string): Buffer {
  return Buffer.from(
    crypto.hkdfSync("sha256", passphrase, DESTRUCT_HKDF_SALT, DESTRUCT_HKDF_INFO, AES_KEY_LENGTH),
  );
}

/**
 * Encode the destruct key as a base64 passphrase string suitable for
 * display to the user and later verification via bcrypt.
 */
export function destructKeyToPassphrase(key: Buffer): string {
  return key.toString("base64");
}
