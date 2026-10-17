import { SYNC_HEADERS, type SyncDeps, type SyncErrorCode, receiveUpload } from "@sdw/core/sync";
import type { FeatureFlags } from "@sdw/core/workspace";
import type { Context } from "hono";

/** Authentication and framing failures are the client's fault; a locked vault is "unavailable". */
export function statusFor(code: SyncErrorCode): 400 | 401 | 500 | 503 {
  if (code === "vault-locked") return 503;
  if (code === "unauthorized") return 401;
  if (code === "internal") return 500;
  return 400;
}

/**
 * POST /sync/api/upload — a thin adapter over receiveUpload(): read the request,
 * call the use case, translate the outcome. All decisions live in core.
 */
export async function uploadRoute(c: Context, features: FeatureFlags, deps: SyncDeps) {
  // Off unless SYNC_ENABLED is set: a disabled feature looks like a missing one.
  if (!features.sync) return c.body(null, 404);

  const nowMs = Date.now();
  const outcome = await receiveUpload(
    {
      body: new Uint8Array(await c.req.arrayBuffer()),
      op: c.req.header(SYNC_HEADERS.op) || "put-chunk",
      // A missing timestamp falls back to "now"; the AAD then will not match and the
      // envelope fails authentication, exactly as it always has.
      ts: Number.parseInt(c.req.header(SYNC_HEADERS.ts) ?? String(nowMs), 10),
      fileId: c.req.header(SYNC_HEADERS.fileId) || "",
      chunkIdx: c.req.header(SYNC_HEADERS.chunkIdx) || "0",
      nowMs,
    },
    deps,
  );
  return outcome.ok
    ? c.json(outcome.result)
    : c.json({ error: outcome.code }, statusFor(outcome.code));
}
