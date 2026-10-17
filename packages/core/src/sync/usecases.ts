/**
 * Sync use cases: receive an upload, list the shared directory.
 *
 * The orchestration order is the security argument, so it is written down once
 * here and exercised with a real client and fake storage (no HTTP, no disk):
 *
 *   key available -> framing -> timestamp -> authenticate -> claim nonce -> act
 *
 * Nothing touches storage before the envelope has authenticated, and every
 * failure is a named code the HTTP layer maps to a status.
 *
 * The nonce is claimed only AFTER authentication. The replay window is a bounded
 * ring buffer: if unauthenticated requests could claim slots, a flood of bogus
 * envelopes would push a genuine nonce out and let a captured request be replayed.
 */
import { buildAad } from "./aad.ts";
import { fromBase64, parseUploadBody, splitEnvelope, toHex } from "./envelope.ts";
import { checkRelativePath } from "./path-safety.ts";
import type { AeadPort, KeyProvider, SharedFile, UploadStore } from "./ports.ts";
import type { ReplayWindow } from "./replay-window.ts";

export type SyncErrorCode =
  | "vault-locked"
  | "unauthorized"
  | "too-short"
  | "bad-timestamp"
  | "replay-detected"
  | "aead-failure"
  | "bad-file-id"
  | "bad-file-path"
  | "bad-finalize-json"
  | "no-chunks"
  | "checksum-mismatch"
  | "unknown-op"
  | "internal";

export interface SyncDeps {
  keys: KeyProvider;
  aead: AeadPort;
  store: UploadStore;
  replay: ReplayWindow;
}

export interface UploadRequest {
  body: Uint8Array;
  op: string;
  ts: number;
  fileId: string;
  chunkIdx: string;
  nowMs: number;
}

export type UploadOutcome =
  | { ok: true; result: { status: "chunk-received"; chunk_idx: number } }
  | { ok: true; result: { status: "upload-complete"; path: string } }
  | { ok: false; code: SyncErrorCode };

const fail = (code: SyncErrorCode): { ok: false; code: SyncErrorCode } => ({ ok: false, code });

/** A file id becomes part of a temp file name, so it must be one safe path segment. */
function flatSegment(input: string): string | null {
  const check = checkRelativePath(input);
  return check.ok && !check.normalized.includes("/") ? check.normalized : null;
}

export async function receiveUpload(req: UploadRequest, deps: SyncDeps): Promise<UploadOutcome> {
  try {
    const key = await deps.keys.current();
    if (!key) return fail("vault-locked");

    const envelope = parseUploadBody(req.body);
    if (!envelope) return fail("too-short");

    // A timestamp that is not a number would slip past a tolerance comparison.
    if (!Number.isFinite(req.ts)) return fail("bad-timestamp");

    const aad = buildAad(req.op, req.ts, req.fileId, req.chunkIdx);
    const plaintext = await deps.aead.open(
      key,
      envelope.nonce,
      envelope.ciphertext,
      envelope.tag,
      aad,
    );
    if (!plaintext) return fail("aead-failure");
    // Only an authentic envelope may claim a replay-window slot (see the header).
    if (!deps.replay.check(toHex(envelope.nonce), req.ts, req.nowMs)) {
      return fail("replay-detected");
    }

    if (req.op === "put-chunk") {
      // fileId is attacker-influenced and names a temp file.
      const id = flatSegment(req.fileId);
      if (!id) return fail("bad-file-id");
      await deps.store.appendChunk(id, plaintext);
      return {
        ok: true,
        result: { status: "chunk-received", chunk_idx: Number.parseInt(req.chunkIdx, 10) },
      };
    }

    if (req.op === "put-finalize") {
      // Both ids in the decrypted metadata are checked again before any filesystem
      // use: defense in depth even though the payload is authenticated.
      let data: unknown;
      try {
        data = JSON.parse(new TextDecoder().decode(plaintext));
      } catch {
        return fail("bad-finalize-json");
      }
      if (typeof data !== "object" || data === null) return fail("bad-finalize-json");
      const meta = data as Record<string, unknown>;

      const id = flatSegment(String(meta.file_id ?? ""));
      if (!id) return fail("bad-file-id");
      const target = checkRelativePath(String(meta.file_path ?? ""));
      if (!target.ok) return fail("bad-file-path");
      if (!(await deps.store.hasChunks(id))) return fail("no-chunks");

      if (meta.content_sha256) {
        if ((await deps.store.sha256Hex(id)) !== meta.content_sha256) {
          await deps.store.discard(id);
          return fail("checksum-mismatch");
        }
      }
      await deps.store.commit(id, target.normalized);
      return { ok: true, result: { status: "upload-complete", path: target.normalized } };
    }

    return fail("unknown-op");
  } catch {
    // Never leak internal detail to the client.
    return fail("internal");
  }
}

// --- listing ---

export interface ListRequest {
  ts: number;
  /** base64(nonce || tag) of an AEAD over an empty message, from buildListAuth. */
  auth: string | null;
  nowMs: number;
}

export type ListOutcome = { ok: true; files: SharedFile[] } | { ok: false; code: SyncErrorCode };

/**
 * Listing reveals file names and sizes, so it demands the same proof of the sync
 * key as an upload. Every authentication failure is the same "unauthorized".
 */
export async function listShared(req: ListRequest, deps: SyncDeps): Promise<ListOutcome> {
  try {
    const key = await deps.keys.current();
    if (!key) return fail("vault-locked");

    const bytes = req.auth ? fromBase64(req.auth) : null;
    const envelope = bytes ? splitEnvelope(bytes) : null;
    // The message is empty, so there must be no ciphertext between nonce and tag.
    if (!envelope || envelope.ciphertext.length !== 0) return fail("unauthorized");
    if (!Number.isFinite(req.ts)) return fail("unauthorized");

    const opened = await deps.aead.open(
      key,
      envelope.nonce,
      envelope.ciphertext,
      envelope.tag,
      buildAad("list", req.ts),
    );
    if (!opened) return fail("unauthorized");
    if (!deps.replay.check(toHex(envelope.nonce), req.ts, req.nowMs)) return fail("unauthorized");

    return { ok: true, files: await deps.store.list() };
  } catch {
    return fail("internal");
  }
}
