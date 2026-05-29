import { existsSync, readFileSync } from "node:fs";
import type { Context } from "hono";
import { destroyCore } from "../../../cli/src/lib/destroy.ts";
import {
  DESTRUCT_KEY_HASH_PATH,
  DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS,
  DESTRUCT_RATE_LIMIT_WINDOW_MS,
  DESTRUCT_RESPONSE_DELAY_MS,
} from "../../../core/src/constants.ts";

// Module-level rate limiter (per client IP).
const attempts = new Map<string, { count: number; resetAt: number }>();

function getClientIP(c: Context): string {
  const xff = c.req.header("x-forwarded-for");
  if (xff) return xff.split(",")[0].trim();
  return "unknown";
}

function isRateLimited(ip: string): boolean {
  const now = Date.now();
  const entry = attempts.get(ip);
  if (!entry || now > entry.resetAt) {
    attempts.set(ip, { count: 1, resetAt: now + DESTRUCT_RATE_LIMIT_WINDOW_MS });
    return false;
  }
  entry.count++;
  return entry.count > DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS;
}

export async function destructRoute(c: Context): Promise<Response> {
  const ip = getClientIP(c);

  // Rate limit check
  if (isRateLimited(ip)) {
    return c.body(null, 429);
  }

  // Key hash must exist (vault was unlocked at least once)
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
  setTimeout(async () => {
    await destroyCore({ force: true, skipShred: false, remote: false, silent: true });
    process.kill(1, "SIGTERM");
  }, DESTRUCT_RESPONSE_DELAY_MS);

  return c.json({ status: "destroyed" }, 200);
}
