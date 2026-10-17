/**
 * Destroy adapters: the shell side effects of destruction, and the wiring of
 * every port into the DestroyDeps that destroyWorkspace() needs. Shared by the
 * `self-destruct` CLI and the remote-destruct HTTP route.
 */
import { existsSync, mkdirSync } from "node:fs";
import {
  CREDENTIALS_DIR,
  DESTRUCT_COMPLETED_PATH,
  DESTRUCT_KEY_HASH_PATH,
  DESTRUCT_STATE_DIR,
} from "@sdw/core/constants";
import {
  type DestroyDeps,
  type DestroyEffects,
  PENTEST_VAULT,
  WORKSPACE_VAULT,
} from "@sdw/core/vault";
import { $ } from "bun";
import { gocryptfsVolume } from "./gocryptfs.ts";
import { supervisorHolders } from "./holders.ts";
import { fileShredder } from "./shred.ts";

export const shellDestroyEffects: DestroyEffects = {
  async quiesceServices() {
    await $`supervisorctl stop vault-sync-cron`.quiet().nothrow();
  },

  async deleteRemoteRepo() {
    const vaultRepo = process.env.VAULT_GIT_REPO;
    if (!vaultRepo) return "skipped-no-repo";
    if (!existsSync(`${CREDENTIALS_DIR}/github-token`)) return "skipped-no-token";
    const result = await $`gh repo delete --yes ${vaultRepo}`.nothrow();
    return result.exitCode === 0 ? "deleted" : "failed";
  },

  async shredTree(dir) {
    if (!existsSync(dir)) return;
    await $`find ${dir} -type f -exec shred -n1 -z {} +`.quiet().nothrow();
    await $`rm -rf ${dir}/*`.quiet().nothrow();
  },

  async scrubCredentials() {
    await fileShredder.shred("/etc/clash/config.yaml", 3);
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

/** Wire every port to its production adapter. */
export function createDestroyDeps(log: (message: string) => void): DestroyDeps {
  return {
    specs: [WORKSPACE_VAULT, PENTEST_VAULT],
    volume: gocryptfsVolume,
    holders: supervisorHolders,
    shredder: fileShredder,
    effects: shellDestroyEffects,
    now: () => new Date().toISOString(),
    log,
  };
}
