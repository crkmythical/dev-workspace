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
} from "../../../core/src/constants.ts";
import { ReplayWindow } from "../../../core/src/replay-window.ts";
import { decrypt, deriveKey } from "../crypto.ts";

const UPLOAD_DIR = `${SHARED_DIR}/.uploads`;
let syncKey: Buffer | null = null;
const replayWindow = new ReplayWindow();

async function getSyncKey(): Promise<Buffer | null> {
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
      // plaintext is raw file chunk bytes
      const tempPath = path.join(UPLOAD_DIR, `.uploading-${fileId}`);
      await appendFile(tempPath, plaintext);
      return c.json({ status: "chunk-received", chunk_idx: Number.parseInt(chunkIdx) });
    }

    if (op === "put-finalize") {
      // plaintext is JSON metadata
      const data = JSON.parse(plaintext.toString("utf-8"));
      const tempPath = path.join(UPLOAD_DIR, `.uploading-${data.file_id}`);
      const finalPath = path.join(SHARED_DIR, data.file_path);

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
      return c.json({ status: "upload-complete", path: data.file_path });
    }

    return c.json({ error: "unknown-op" }, 400);
  } catch (err: any) {
    return c.json({ error: "internal", detail: err.message }, 500);
  }
}

export function resetSyncKey() {
  if (syncKey) {
    syncKey.fill(0);
    syncKey = null;
  }
}
