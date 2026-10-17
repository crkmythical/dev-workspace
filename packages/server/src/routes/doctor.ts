import { existsSync } from "node:fs";
import { SYNC_PASSPHRASE_PATH } from "@sdw/core/constants";
import { parseFeatureFlags } from "@sdw/core/features";
import type { Context } from "hono";

export async function doctorRoute(c: Context) {
  const enabled = parseFeatureFlags(process.env).sync;
  const hasKey = existsSync(SYNC_PASSPHRASE_PATH);
  return c.json({
    // "disabled" is reported as such, not as a locked vault.
    state: !enabled ? "disabled" : hasKey ? "ready" : "vault-locked",
    sync_enabled: enabled,
    sync_key_available: hasKey,
  });
}
