import type { Context } from "hono";
import { existsSync } from "node:fs";
import { SYNC_PASSPHRASE_PATH } from "../../../core/src/constants.ts";

export async function doctorRoute(c: Context) {
  const hasKey = existsSync(SYNC_PASSPHRASE_PATH);
  return c.json({
    state: hasKey ? "ready" : "vault-locked",
    sync_key_available: hasKey,
  });
}
