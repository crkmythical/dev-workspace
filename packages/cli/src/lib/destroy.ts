/**
 * destroyCore — Core self-destruct logic.
 *
 * Executes Phases 0-5 then RETURNS (does NOT exit, does NOT kill PID 1).
 * The caller (CLI or API route) is responsible for process termination.
 */
import { existsSync, mkdirSync } from "node:fs";
import {
  CREDENTIALS_DIR,
  DESTRUCT_COMPLETED_PATH,
  DESTRUCT_KEY_HASH_PATH,
  PENTEST_CIPHER_DIR,
  PENTEST_MOUNT,
  VAULT_CIPHER_DIR,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
import { $ } from "bun";
import { stopDesktop } from "./desktop.ts";
import { unmountVault } from "./vault.ts";

export interface DestroyOptions {
  force: boolean;
  skipShred: boolean;
  remote: boolean;
  silent: boolean;
}

function log(opts: DestroyOptions, msg: string) {
  if (!opts.silent) console.log(msg);
}

async function shredFile(path: string): Promise<void> {
  await $`shred -n3 -z ${path} && rm -f ${path}`.quiet().nothrow();
}

export async function destroyCore(opts: DestroyOptions): Promise<void> {
  // Phase 0: Stop vault-sync, optionally delete remote repo
  log(opts, "[Phase 0] Stopping vault-sync and optional remote cleanup...");
  await $`supervisorctl stop vault-sync-cron`.quiet().nothrow();

  if (opts.remote) {
    const vaultRepo = process.env.VAULT_GIT_REPO;
    if (vaultRepo) {
      const tokenPath = `${CREDENTIALS_DIR}/github-token`;
      if (existsSync(tokenPath)) {
        await $`gh repo delete --yes ${vaultRepo}`.nothrow();
      } else {
        log(opts, "  WARNING: No github-token found, cannot delete remote repo.");
      }
    }
  }

  // Phase 1: Unmount vaults
  log(opts, "[Phase 1] Unmounting vaults...");
  await stopDesktop();
  await unmountVault(WORKSPACE_MOUNT);
  await unmountVault(PENTEST_MOUNT);

  // Phase 2: Shred gocryptfs.conf (point of no return)
  log(opts, "[Phase 2] Shredding vault master keys...");
  await shredFile(`${VAULT_CIPHER_DIR}/gocryptfs.conf`);
  await shredFile(`${PENTEST_CIPHER_DIR}/gocryptfs.conf`);
  log(opts, `  *** POINT OF NO RETURN *** ${new Date().toISOString()}`);

  // Phase 3: Shred cipher data (unless --skip-shred)
  if (!opts.skipShred) {
    log(opts, "[Phase 3] Shredding cipher data (this may take a while)...");
    for (const dir of [VAULT_CIPHER_DIR, PENTEST_CIPHER_DIR]) {
      if (existsSync(dir)) {
        await $`find ${dir} -type f -exec shred -n1 -z {} +`.quiet().nothrow();
        await $`rm -rf ${dir}/*`.quiet().nothrow();
      }
    }
  } else {
    log(opts, "[Phase 3] Skipped (--skip-shred).");
  }

  // Phase 4: Shred credentials and config
  log(opts, "[Phase 4] Cleaning credentials and config...");
  await shredFile("/etc/clash/config.yaml");
  await $`rm -rf /root/.ssh /root/.gitconfig /root/.git-credentials`.quiet().nothrow();
  await $`rm -f ${DESTRUCT_KEY_HASH_PATH}`.quiet().nothrow();

  // Phase 5: Clear history, write completion marker
  log(opts, "[Phase 5] Finalizing...");
  await $`truncate -s 0 ~/.bash_history`.quiet().nothrow();
  mkdirSync("/var/run/self-destruct", { recursive: true });
  await Bun.write(DESTRUCT_COMPLETED_PATH, new Date().toISOString());

  log(opts, "destroyCore complete.");
}
