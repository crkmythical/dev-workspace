/**
 * Shared cryptographic utilities — destruct key derivation.
 *
 * Used by:
 *   - packages/cli/src/unlock-vault.ts (derive + store hash on unlock)
 *   - packages/server/src/routes/destruct.ts (verify passphrase)
 */
import crypto from "node:crypto";
import { AES_KEY_LENGTH, DESTRUCT_HKDF_INFO, DESTRUCT_HKDF_SALT } from "./constants.ts";

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
