/**
 * Client side of the sync protocol, on WebCrypto.
 *
 * WebCrypto is a web standard available in browsers and in Bun/Node alike, so
 * this module stays runtime-neutral. The SPA uses it for real; the tests use the
 * very same code to talk to the server's Node implementation and to check it
 * against the golden wire vectors, which is what keeps the two ends honest.
 */
import { buildAad, buildChunkAad, buildFinalizeAad } from "./aad.ts";
import { encodeEnvelope, toBase64, wrapPng } from "./envelope.ts";
import {
  AES_KEY_LENGTH,
  NONCE_LENGTH,
  SYNC_HEADERS,
  SYNC_KDF_ITERATIONS,
  SYNC_KDF_SALT,
  TAG_LENGTH,
} from "./protocol.ts";

/**
 * WebCrypto's parameter types are DOM-library types, and core is compiled without the
 * DOM library on purpose (a stray DOM API elsewhere in core must still fail the type
 * check). Bytes therefore cross the crypto.subtle boundary through this one alias.
 */
type Bytes = Uint8Array<ArrayBuffer>;
const bytesOf = (b: Uint8Array): Bytes => b as Bytes;

const text = new TextEncoder();

function kdfParams() {
  return {
    name: "PBKDF2" as const,
    hash: "SHA-256" as const,
    salt: bytesOf(text.encode(SYNC_KDF_SALT)),
    iterations: SYNC_KDF_ITERATIONS,
  };
}

async function kdfMaterial(passphrase: string): Promise<CryptoKey> {
  return crypto.subtle.importKey("raw", bytesOf(text.encode(passphrase)), "PBKDF2", false, [
    "deriveKey",
    "deriveBits",
  ]);
}

/**
 * Derive the AES-GCM sync key from the vault passphrase (non-extractable). PBKDF2-HMAC-SHA256
 * password stretching: the passphrase is the vault master password and these envelopes cross
 * Cloudflare, so a captured one must not be brute-forceable at raw-hash speed. Matches
 * core::sync::crypto::derive_key in Rust (pinned by sync-service/tests/parity.rs).
 */
export async function deriveSyncKey(passphrase: string): Promise<CryptoKey> {
  return crypto.subtle.deriveKey(
    kdfParams(),
    await kdfMaterial(passphrase),
    { name: "AES-GCM", length: AES_KEY_LENGTH * 8 },
    false,
    ["encrypt", "decrypt"],
  );
}

/** The same key as raw bytes. For tests and parity-vector checks only. */
export async function deriveSyncKeyBytes(passphrase: string): Promise<Uint8Array<ArrayBuffer>> {
  const bits = await crypto.subtle.deriveBits(
    kdfParams(),
    await kdfMaterial(passphrase),
    AES_KEY_LENGTH * 8,
  );
  return new Uint8Array(bits);
}

/**
 * Encrypt and frame: returns nonce || ciphertext || tag. `nonce` is random unless
 * a test pins it to reproduce a golden vector.
 */
export async function sealEnvelope(
  key: CryptoKey,
  plaintext: Uint8Array,
  aad: Uint8Array,
  nonce: Uint8Array = crypto.getRandomValues(new Uint8Array(NONCE_LENGTH)),
): Promise<Uint8Array<ArrayBuffer>> {
  const sealed = await crypto.subtle.encrypt(
    {
      name: "AES-GCM",
      iv: bytesOf(nonce),
      additionalData: bytesOf(aad),
      tagLength: TAG_LENGTH * 8,
    },
    key,
    bytesOf(plaintext),
  );
  return encodeEnvelope(nonce, new Uint8Array(sealed));
}

export interface SyncRequestParts {
  headers: Record<string, string>;
  body: Uint8Array<ArrayBuffer>;
}

function uploadHeaders(op: string, ts: number, fileId: string, chunkIdx: number) {
  return {
    "Content-Type": "image/png",
    [SYNC_HEADERS.op]: op,
    [SYNC_HEADERS.ts]: String(ts),
    [SYNC_HEADERS.fileId]: fileId,
    [SYNC_HEADERS.chunkIdx]: String(chunkIdx),
  };
}

/**
 * One chunk of a file. `ts` must be fresh for EVERY request: the server accepts
 * a timestamp only within a short tolerance window, so a slow multi-chunk upload
 * that reused its first timestamp would be rejected part way through.
 */
export async function buildChunkRequest(
  key: CryptoKey,
  ts: number,
  fileId: string,
  chunkIdx: number,
  chunk: Uint8Array,
): Promise<SyncRequestParts> {
  const envelope = await sealEnvelope(key, chunk, buildChunkAad(ts, fileId, chunkIdx));
  return { headers: uploadHeaders("put-chunk", ts, fileId, chunkIdx), body: wrapPng(envelope) };
}

export interface FinalizeMeta {
  fileId: string;
  filePath: string;
  contentSha256: string;
  totalChunks: number;
}

/** The request that turns the uploaded chunks into a file at `filePath`. */
export async function buildFinalizeRequest(
  key: CryptoKey,
  ts: number,
  meta: FinalizeMeta,
): Promise<SyncRequestParts> {
  const payload = text.encode(
    JSON.stringify({
      file_id: meta.fileId,
      file_path: meta.filePath,
      content_sha256: meta.contentSha256,
      total_chunks: meta.totalChunks,
    }),
  );
  const envelope = await sealEnvelope(
    key,
    payload,
    buildFinalizeAad(ts, meta.fileId, meta.totalChunks),
  );
  return {
    headers: uploadHeaders("put-finalize", ts, meta.fileId, meta.totalChunks),
    body: wrapPng(envelope),
  };
}

/**
 * Authenticator for listing the shared directory: an AEAD envelope over an empty
 * message, bound to the op and a fresh timestamp. It proves possession of the
 * sync key without sending anything the server could not already see.
 */
export async function buildListAuth(key: CryptoKey, ts: number): Promise<Record<string, string>> {
  const envelope = await sealEnvelope(key, new Uint8Array(0), buildAad("list", ts));
  return {
    [SYNC_HEADERS.ts]: String(ts),
    [SYNC_HEADERS.auth]: toBase64(envelope),
  };
}
