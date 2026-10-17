#!/usr/bin/env bun
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import {
  SYNC_FAILURE_THRESHOLD,
  VAULT_CIPHER_DIR,
  VAULT_SYNC_STATE_DIR,
  planSync,
} from "@sdw/core/vault";
import { createFileFailureTracker } from "@sdw/infra";
/**
 * vault-sync — Periodic vault ciphertext sync to GitHub
 */
import { $ } from "bun";

// Same tracker the backup watcher uses: counter, last-success marker, notification file.
const tracker = createFileFailureTracker({
  stateDir: VAULT_SYNC_STATE_DIR,
  threshold: SYNC_FAILURE_THRESHOLD,
  notificationFile: "sync-failure.md",
  notificationBody: (n) =>
    `# ⚠ Vault Sync Failure\n\nFailed ${n} times. Run \`doctor\` to diagnose.\n`,
});

// Skip if vault not mounted
const mounted = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (planSync({ mounted: mounted.exitCode === 0, hasStagedChanges: true }).action === "skip") {
  process.exit(0);
}

// Check for changes
const addResult = await $`cd ${VAULT_CIPHER_DIR} && git add -A`.quiet().nothrow();
const diffResult = await $`cd ${VAULT_CIPHER_DIR} && git diff --cached --quiet`.quiet().nothrow();
// diffResult.exitCode === 0 means nothing staged (no changes)
if (planSync({ mounted: true, hasStagedChanges: diffResult.exitCode !== 0 }).action === "skip") {
  process.exit(0); // No changes
}

// Commit
await $`cd ${VAULT_CIPHER_DIR} && git commit -q -m "auto-sync ${new Date().toISOString()}"`.quiet();

// Push (with rebase retry)
let pushOk = false;
const push1 = await $`cd ${VAULT_CIPHER_DIR} && git push -q`.quiet().nothrow();
if (push1.exitCode === 0) {
  pushOk = true;
} else {
  const rebase = await $`cd ${VAULT_CIPHER_DIR} && git pull --rebase -q`.quiet().nothrow();
  if (rebase.exitCode === 0) {
    const push2 = await $`cd ${VAULT_CIPHER_DIR} && git push -q`.quiet().nothrow();
    pushOk = push2.exitCode === 0;
  }
}

if (pushOk) {
  tracker.recordSuccess();
} else {
  tracker.recordFailure();
  process.exit(1);
}
