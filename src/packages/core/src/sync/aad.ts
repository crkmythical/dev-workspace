/**
 * AAD (Additional Authenticated Data) builder.
 * Pure string logic — no crypto dependency.
 * Shared between server (Node) and browser (WebCrypto).
 */

const encoder = new TextEncoder();

/**
 * Build AAD for an AEAD envelope.
 * Format: "op|timestamp_ms|extra1|extra2|..."
 */
export function buildAad(op: string, timestampMs: number, ...extra: string[]): Uint8Array {
  const parts = [op, String(timestampMs), ...extra];
  return encoder.encode(parts.join("|"));
}

/**
 * Build AAD specifically for put-chunk (includes file_id and chunk_idx).
 */
export function buildChunkAad(timestampMs: number, fileId: string, chunkIdx: number): Uint8Array {
  return buildAad("put-chunk", timestampMs, fileId, String(chunkIdx));
}

/**
 * Build AAD for put-finalize.
 */
export function buildFinalizeAad(
  timestampMs: number,
  fileId: string,
  totalChunks: number,
): Uint8Array {
  return buildAad("put-finalize", timestampMs, fileId, String(totalChunks));
}
