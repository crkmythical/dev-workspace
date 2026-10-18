/**
 * Destroy adapters: the shell side effects of destruction, and the wiring of
 * every port into the DestroyDeps that destroyWorkspace() needs. Shared by the
 * `self-destruct` CLI and the remote-destruct HTTP route.
 */
import { existsSync, mkdirSync } from "node:fs";
import { CLASH_CONFIG_PATH } from "@sdw/core/egress";
import { CREDENTIALS_DIR } from "@sdw/core/shared/layout";
import {
  DESTRUCT_COMPLETED_PATH,
  DESTRUCT_KEY_HASH_PATH,
  DESTRUCT_STATE_DIR,
  type DestroyDeps,
  type DestroyEffects,
  type VaultSpec,
} from "@sdw/core/vault";
import { $ } from "bun";
import { gocryptfsVolume } from "./gocryptfs.ts";
import { supervisorHolders } from "./holders.ts";
import { fileShredder } from "./shred.ts";

export interface DestroyOptions {
  /** Remote the vault ciphertext is replicated to; only needed to delete it (--remote). */
  vaultRepo?: string;
}

/** Shell side effects of destruction. */
export function createDestroyEffects(opts: DestroyOptions = {}): DestroyEffects {
  return {
    async quiesceServices() {
      await $`supervisorctl stop vault-sync-cron`.quiet().nothrow();
    },

    async deleteRemoteRepo() {
      if (!opts.vaultRepo) return "skipped-no-repo";
      if (!existsSync(`${CREDENTIALS_DIR}/github-token`)) return "skipped-no-token";
      const result = await $`gh repo delete --yes ${opts.vaultRepo}`.nothrow();
      return result.exitCode === 0 ? "deleted" : "failed";
    },

    async shredTree(dir) {
      if (!existsSync(dir)) return;
      await $`find ${dir} -type f -exec shred -n1 -z {} +`.quiet().nothrow();
      // Everything inside, hidden entries too: the shell glob `*` skips dotfiles, which used to
      // leave the replication repository (.git, with 20 zeroed files in the smoke test) and
      // .gitignore behind. The directory itself is a volume root and stays.
      await $`find ${dir} -mindepth 1 -delete`.quiet().nothrow();
    },

    async scrubCredentials() {
      await fileShredder.shred(CLASH_CONFIG_PATH, 3);
      await $`rm -rf /root/.ssh /root/.gitconfig /root/.git-credentials`.quiet().nothrow();
      await $`rm -f ${DESTRUCT_KEY_HASH_PATH}`.quiet().nothrow();
    },

    async clearShellHistory() {
      await $`truncate -s 0 ~/.bash_history`.quiet().nothrow();
    },

    async writeCompletionMarker(isoTimestamp) {
      mkdirSync(DESTRUCT_STATE_DIR, { recursive: true });
      await Bun.write(DESTRUCT_COMPLETED_PATH, isoTimestamp);
    },
  };
}

/**
 * Wire every port to its production adapter.
 *
 * `specs` is every encrypted volume of the build (core's ALL_VAULTS). It is required
 * on purpose: a caller that forgot a plugin's vault would silently leave its key behind.
 */
export function createDestroyDeps(
  specs: readonly VaultSpec[],
  log: (message: string) => void,
  opts: DestroyOptions = {},
): DestroyDeps {
  return {
    specs,
    volume: gocryptfsVolume,
    holders: supervisorHolders,
    shredder: fileShredder,
    effects: createDestroyEffects(opts),
    now: () => new Date().toISOString(),
    log,
  };
}
