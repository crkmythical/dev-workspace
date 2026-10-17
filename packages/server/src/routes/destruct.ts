import { existsSync, readFileSync } from "node:fs";
import {
  DESTRUCT_KEY_HASH_PATH,
  DESTRUCT_RATE_LIMIT_GLOBAL_MAX,
  DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
  DESTRUCT_RATE_LIMIT_WINDOW_MS,
  DESTRUCT_RESPONSE_DELAY_MS,
} from "@sdw/core/constants";
import { parseFeatureFlags } from "@sdw/core/features";
import { RateLimiter } from "@sdw/core/rate-limit";
import { destroyWorkspace } from "@sdw/core/vault";
import { createDestroyDeps } from "@sdw/infra";
import type { Context } from "hono";

// Per-IP budget with a global backstop. The per-IP key comes from a
// client-influenced header, so the global cap is what actually bounds a
// spoofed-IP brute force (see rate-limit.ts).
const limiter = new RateLimiter({
  windowMs: DESTRUCT_RATE_LIMIT_WINDOW_MS,
  perKeyMax: DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
  globalMax: DESTRUCT_RATE_LIMIT_GLOBAL_MAX,
});

function getClientIP(c: Context): string {
  // Prefer Cloudflare's edge-set CF-Connecting-IP: when the only ingress is the
  // CF tunnel, the external client cannot overwrite it. Fall back to the first
  // X-Forwarded-For hop, then a constant (which still shares the global cap).
  const cf = c.req.header("cf-connecting-ip");
  if (cf) return cf.trim();
  const xff = c.req.header("x-forwarded-for");
  if (xff) return xff.split(",")[0].trim();
  return "unknown";
}

export async function destructRoute(c: Context): Promise<Response> {
  // Off unless REMOTE_DESTRUCT_ENABLED is set. Checked before anything else so a
  // disabled endpoint is indistinguishable from a missing one, even if a stale
  // key hash is still on the persistent volume.
  if (!parseFeatureFlags(process.env).remoteDestruct) {
    return c.body(null, 404);
  }

  const ip = getClientIP(c);

  // Rate limit check (per-IP + global backstop)
  if (limiter.check(ip)) {
    return c.body(null, 429);
  }

  // Key hash must exist (vault was unlocked with the feature enabled)
  if (!existsSync(DESTRUCT_KEY_HASH_PATH)) {
    return c.body(null, 404);
  }

  // Parse body
  let passphrase: string;
  try {
    const body = await c.req.json();
    passphrase = body?.passphrase;
    if (typeof passphrase !== "string" || !passphrase) {
      return c.body(null, 404);
    }
  } catch {
    return c.body(null, 404);
  }

  // Verify passphrase against stored bcrypt hash
  const hash = readFileSync(DESTRUCT_KEY_HASH_PATH, "utf-8").trim();
  const valid = await Bun.password.verify(passphrase, hash);
  if (!valid) {
    return c.body(null, 404);
  }

  // Schedule destruction after response is sent.
  // We kill PID 1 directly (not supervisorctl stop all) because sync-service
  // itself runs under supervisord — stopping all would kill us mid-destroy.
  // try/finally: destroyWorkspace crosses an irreversible "point of no return"
  // (shredding gocryptfs.conf). If it throws partway, we must STILL terminate
  // PID 1 so the container never lingers in a half-destroyed state with the
  // vault key gone but processes alive.
  setTimeout(async () => {
    try {
      const report = await destroyWorkspace(
        createDestroyDeps(() => {}),
        {
          skipShred: false,
          remote: false,
        },
      );
      if (!report.complete) {
        const failed = report.phases.filter((p) => !p.ok);
        console.error("destroyWorkspace finished with failures:", JSON.stringify(failed));
      }
    } catch (err) {
      console.error("destroyWorkspace failed mid-destruct; terminating anyway:", err);
    } finally {
      process.kill(1, "SIGTERM");
    }
  }, DESTRUCT_RESPONSE_DELAY_MS);

  return c.json({ status: "destroyed" }, 200);
}
