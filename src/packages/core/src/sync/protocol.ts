/**
 * Wire-protocol constants of the browser <-> server file sync.
 *
 * Pure data with no runtime imports, so the browser SPA, the server and the Rust
 * PoC all derive from one definition (rust/sync-service pins it through the
 * golden wire vectors, see packages/server/scripts/gen-wire-vectors.ts).
 */

// Crypto
export const HKDF_SALT = "sync-key-v1";
export const HKDF_INFO = "aead-key";
export const NONCE_LENGTH = 12;
export const TAG_LENGTH = 16;
export const AES_KEY_LENGTH = 32;

// PNG camouflage (33 bytes)
export const PNG_HEADER = new Uint8Array([
  0x89,
  0x50,
  0x4e,
  0x47,
  0x0d,
  0x0a,
  0x1a,
  0x0a, // PNG signature (8)
  0x00,
  0x00,
  0x00,
  0x0d, // IHDR length (4)
  0x49,
  0x48,
  0x44,
  0x52, // "IHDR" (4)
  0x00,
  0x00,
  0x00,
  0x01, // width=1 (4)
  0x00,
  0x00,
  0x00,
  0x01, // height=1 (4)
  0x08,
  0x02,
  0x00,
  0x00,
  0x00, // bitDepth=8, colorType=2, comp=0, filter=0, interlace=0 (5)
  0x90,
  0x77,
  0x53,
  0xde, // CRC (4)
]);
export const PNG_HEADER_SIZE = 33;

// Transfer limits
export const CHUNK_SIZE = 4 * 1024 * 1024; // 4 MiB
export const SIZE_LIMIT = 500 * 1024 * 1024; // 500 MB

// Replay protection
export const REPLAY_WINDOW_SIZE = 10000;
export const REPLAY_TOLERANCE_MS = 300_000; // 5 min

// SPA polling cadence
export const POLL_INTERVAL_FOCUSED_MS = 2000;
export const POLL_INTERVAL_BACKGROUND_MS = 10000;

/** HTTP headers carrying the plaintext inputs of the AAD, and the list authenticator. */
export const SYNC_HEADERS = {
  op: "X-Sync-Op",
  ts: "X-Sync-Ts",
  fileId: "X-Sync-FileId",
  chunkIdx: "X-Sync-ChunkIdx",
  auth: "X-Sync-Auth",
} as const;

/** The API surface, defined once for the SPA and the server. */
export const SYNC_API = {
  upload: "/sync/api/upload",
  list: "/sync/api/download",
  doctor: "/sync/api/doctor",
  destruct: "/sync/api/destruct",
} as const;
