/**
 * Envelope framing: PNG header, then nonce, ciphertext and tag.
 *
 *   upload body = PNG_HEADER(33) || nonce(12) || ciphertext || tag(16)
 *
 * Pure byte manipulation shared by the browser SPA and the server, so the two
 * ends cannot drift apart on layout. The Rust PoC mirrors it in wire.rs.
 */
import { NONCE_LENGTH, PNG_HEADER, PNG_HEADER_SIZE, TAG_LENGTH } from "./protocol.ts";

export interface ParsedEnvelope {
  nonce: Uint8Array;
  ciphertext: Uint8Array;
  tag: Uint8Array;
}

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

/** Inverse of wrapPng + encodeEnvelope. Null when too short to hold header, nonce and tag. */
export function parseUploadBody(body: Uint8Array): ParsedEnvelope | null {
  if (body.length < PNG_HEADER_SIZE + NONCE_LENGTH + TAG_LENGTH) return null;
  return splitEnvelope(body.subarray(PNG_HEADER_SIZE));
}

/** Split `nonce || ciphertext || tag` (no PNG header). Null when shorter than nonce + tag. */
export function splitEnvelope(raw: Uint8Array): ParsedEnvelope | null {
  if (raw.length < NONCE_LENGTH + TAG_LENGTH) return null;
  const rest = raw.subarray(NONCE_LENGTH);
  return {
    nonce: raw.subarray(0, NONCE_LENGTH),
    ciphertext: rest.subarray(0, rest.length - TAG_LENGTH),
    tag: rest.subarray(rest.length - TAG_LENGTH),
  };
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

/** Null on malformed input rather than throwing: header values are untrusted. */
export function fromBase64(text: string): Uint8Array<ArrayBuffer> | null {
  try {
    const binary = atob(text);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}
