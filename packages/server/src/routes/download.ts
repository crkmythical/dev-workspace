import { SYNC_HEADERS, type SyncDeps, listShared } from "@sdw/core/sync";
import type { FeatureFlags } from "@sdw/core/workspace";
import type { Context } from "hono";
import { statusFor } from "./upload.ts";

/**
 * GET /sync/api/download — lists the shared directory, and only that.
 *
 * File content is deliberately NOT served: the SPA never downloads it, and the
 * old `?path=` branch returned plaintext (PNG header, no encryption, no auth).
 * Listing now requires the same proof of the sync key as an upload.
 */
export async function downloadRoute(c: Context, features: FeatureFlags, deps: SyncDeps) {
  // Off unless SYNC_ENABLED is set: a disabled feature looks like a missing one.
  if (!features.sync) return c.body(null, 404);

  // File content is gone; make that explicit rather than quietly ignoring the parameter.
  if (c.req.query("path") !== undefined) return c.body(null, 404);

  const nowMs = Date.now();
  const outcome = await listShared(
    {
      ts: Number.parseInt(c.req.header(SYNC_HEADERS.ts) ?? "", 10),
      auth: c.req.header(SYNC_HEADERS.auth) ?? null,
      nowMs,
    },
    deps,
  );
  return outcome.ok
    ? c.json({ files: outcome.files })
    : c.json({ error: outcome.code }, statusFor(outcome.code));
}
