/**
 * KeyProvider backed by the passphrase file inside the vault.
 *
 * The file is only readable while the vault is FUSE-mounted, so its
 * disappearance is the authoritative "vault locked" signal: the cached key is
 * zeroed and dropped, and the next request after an unlock re-derives it. This
 * is deterministic and needs no separate socket listener; the existsSync stat is
 * cheap next to a request.
 */
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import type { KeyProvider } from "@sdw/core/sync";
import { SYNC_PASSPHRASE_PATH } from "@sdw/core/sync/layout";
import { deriveKey } from "./sync-crypto.ts";

export function createFileKeyProvider(passphrasePath: string = SYNC_PASSPHRASE_PATH): KeyProvider {
  let cached: Buffer | null = null;

  const drop = () => {
    if (cached) {
      cached.fill(0);
      cached = null;
    }
  };

  return {
    async current() {
      if (!existsSync(passphrasePath)) {
        drop();
        return null;
      }
      if (cached) return cached;
      try {
        cached = deriveKey((await readFile(passphrasePath, "utf-8")).trim());
        return cached;
      } catch {
        return null;
      }
    },
  };
}
