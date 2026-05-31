import crypto from "node:crypto";
import { existsSync } from "node:fs";
import { appendFile, mkdir, readFile, rename, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import type { Context } from "hono";
import { buildAad } from "../../../core/src/aad.ts";
import {
  NONCE_LENGTH,
  PNG_HEADER_SIZE,
  SHARED_DIR,
  SYNC_PASSPHRASE_PATH,
  TAG_LENGTH,
  UPLOAD_TRACKING_DIR,
} from "../../../core/src/constants.ts";
import { checkRelativePath } from "../../../core/src/path-safety.ts";
import { ReplayWindow } from "../../../core/src/replay-window.ts";
import { decrypt, deriveKey } from "../crypto.ts";

const UPLOAD_DIR = UPLOAD_TRACKING_DIR;
let syncKey: Buffer | null = null;
const replayWindow = new ReplayWindow();

/** Zero and drop the cached AEAD key (e.g. when the vault is locked). */
export function resetSyncKey() {
  if (syncKey) {
    syncKey.fill(0);
    syncKey = null;
  }
}

/**
 * Resolve the AEAD sync key, honouring vault lock state.
 *
 * The passphrase file lives inside the vault (`/workspace/.credentials/...`),
 * so it is only readable while the vault is FUSE-mounted. We treat its
 * disappearance as the authoritative "vault locked" signal and drop any cached
 * key from memory — this fulfils the design requirement that the sync-service
 * drops its in-memory key on lock (design.md §"lock-vault"), deterministically
 * and without a separate socket listener. On re-unlock the file reappears and
 * the key is re-derived. The existsSync stat is cheap relative to a request.
 */
async function getSyncKey(): Promise<Buffer | null> {
  if (!existsSync(SYNC_PASSPHRASE_PATH)) {
    resetSyncKey();
    return null;
  }
  if (syncKey) return syncKey;
  try {
    const passphrase = await readFile(SYNC_PASSPHRASE_PATH, "utf-8");
    syncKey = deriveKey(passphrase.trim());
    return syncKey;
  } catch {
    return null;
  }
}

export async function uploadRoute(c: Context) {
  try {
    const key = await getSyncKey();
    if (!key) return c.json({ error: "vault-locked" }, 503);

    const body = Buffer.from(await c.req.arrayBuffer());
    if (body.length < PNG_HEADER_SIZE + NONCE_LENGTH + TAG_LENGTH) {
      return c.json({ error: "too-short" }, 400);
    }

    // Strip PNG header
    const raw = body.subarray(PNG_HEADER_SIZE);

    // Parse: nonce(12) + ciphertext_with_tag
    const nonce = raw.subarray(0, NONCE_LENGTH);
    // WebCrypto appends tag to ciphertext, so: ciphertext(N-16) + tag(16)
    const ciphertextWithTag = raw.subarray(NONCE_LENGTH);
    const ciphertext = ciphertextWithTag.subarray(0, ciphertextWithTag.length - TAG_LENGTH);
    const tag = ciphertextWithTag.subarray(ciphertextWithTag.length - TAG_LENGTH);

    // Reconstruct AAD from headers (SPA sends these as plaintext metadata)
    const op = c.req.header("X-Sync-Op") || "put-chunk";
    const ts = c.req.header("X-Sync-Ts") || String(Date.now());
    const fileId = c.req.header("X-Sync-FileId") || "";
    const chunkIdx = c.req.header("X-Sync-ChunkIdx") || "0";

    // Replay protection
    const nonceHex = Buffer.from(nonce).toString("hex");
    if (!replayWindow.check(nonceHex, Number.parseInt(ts))) {
      return c.json({ error: "replay-detected" }, 400);
    }

    const aad = buildAad(op, Number.parseInt(ts), fileId, chunkIdx);

    // Decrypt
    let plaintext: Buffer;
    try {
      plaintext = decrypt(key, Buffer.from(nonce), Buffer.from(ciphertext), Buffer.from(tag), aad);
    } catch {
      return c.json({ error: "aead-failure" }, 400);
    }

    await mkdir(UPLOAD_DIR, { recursive: true });
    await mkdir(SHARED_DIR, { recursive: true });

    if (op === "put-chunk") {
      // plaintext is raw file chunk bytes. fileId is attacker-influenced and is
      // used to build a temp path — reject anything that isn't a flat, safe
      // path segment (no traversal, no separators).
      const fileIdCheck = checkRelativePath(fileId);
      if (!fileIdCheck.ok || fileIdCheck.normalized.includes("/")) {
        return c.json({ error: "bad-file-id" }, 400);
      }
      const tempPath = path.join(UPLOAD_DIR, `.uploading-${fileIdCheck.normalized}`);
      await appendFile(tempPath, plaintext);
      return c.json({ status: "chunk-received", chunk_idx: Number.parseInt(chunkIdx) });
    }

    if (op === "put-finalize") {
      // plaintext is JSON metadata. Both file_id (temp path) and file_path
      // (final path under SHARED_DIR) come from decrypted, attacker-influenced
      // data and must be validated before any filesystem use (defense-in-depth
      // against path traversal even though the payload is AEAD-authenticated).
      const data = JSON.parse(plaintext.toString("utf-8"));

      const idCheck = checkRelativePath(String(data.file_id ?? ""));
      if (!idCheck.ok || idCheck.normalized.includes("/")) {
        return c.json({ error: "bad-file-id" }, 400);
      }
      const pathCheck = checkRelativePath(String(data.file_path ?? ""));
      if (!pathCheck.ok) {
        return c.json({ error: "bad-file-path" }, 400);
      }

      const tempPath = path.join(UPLOAD_DIR, `.uploading-${idCheck.normalized}`);
      const finalPath = path.join(SHARED_DIR, pathCheck.normalized);

      if (!existsSync(tempPath)) {
        return c.json({ error: "no-chunks" }, 400);
      }

      // Verify checksum
      if (data.content_sha256) {
        const fileData = await readFile(tempPath);
        const hash = crypto.createHash("sha256").update(fileData).digest("hex");
        if (hash !== data.content_sha256) {
          await unlink(tempPath).catch(() => {});
          return c.json({ error: "checksum-mismatch" }, 400);
        }
      }

      await mkdir(path.dirname(finalPath), { recursive: true });
      await rename(tempPath, finalPath);
      return c.json({ status: "upload-complete", path: pathCheck.normalized });
    }

    return c.json({ error: "unknown-op" }, 400);
  } catch {
    // Do not leak internal error details to the client (info disclosure).
    return c.json({ error: "internal" }, 500);
  }
}
