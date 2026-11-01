/**
 * Envelope framing: PNG header, then nonce, ciphertext and tag.
 *
 *   upload body = PNG_HEADER(33) || nonce(12) || ciphertext || tag(16)
 *
 * Pure byte manipulation the browser SPA uses to BUILD an upload body; the server parses the same
 * layout in rust/core/src/sync/wire.rs. The TS parse/split side was removed with the old TS server
 * receive path — only the build side runs in the browser now.
 */
import { PNG_HEADER, PNG_HEADER_SIZE } from "./protocol.ts";

/** Prefix a payload with the PNG header (DLP camouflage). */
export function wrapPng(payload: Uint8Array): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(PNG_HEADER_SIZE + payload.length);
  out.set(PNG_HEADER, 0);
  out.set(payload, PNG_HEADER_SIZE);
  return out;
}

/** nonce || ciphertext-with-tag: the layout WebCrypto produces (it appends the tag). */
export function encodeEnvelope(
  nonce: Uint8Array,
  ciphertextWithTag: Uint8Array,
): Uint8Array<ArrayBuffer> {
  const out = new Uint8Array(nonce.length + ciphertextWithTag.length);
  out.set(nonce, 0);
  out.set(ciphertextWithTag, nonce.length);
  return out;
}

export function toHex(bytes: Uint8Array): string {
  let out = "";
  for (const b of bytes) out += b.toString(16).padStart(2, "0");
  return out;
}

export function toBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary);
}
