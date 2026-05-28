#!/usr/bin/env bun
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  SYNC_FAILURE_THRESHOLD,
  VAULT_CIPHER_DIR,
  VAULT_SYNC_STATE_DIR,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
/**
 * vault-sync — Periodic vault ciphertext sync to GitHub
 */
import { $ } from "bun";

const LAST_SUCCESS = `${VAULT_SYNC_STATE_DIR}/last-success`;
const FAILURE_COUNT = `${VAULT_SYNC_STATE_DIR}/consecutive-failures`;

mkdirSync(VAULT_SYNC_STATE_DIR, { recursive: true });

// Skip if vault not mounted
const mounted = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mounted.exitCode !== 0) process.exit(0);

// Check for changes
const addResult = await $`cd ${VAULT_CIPHER_DIR} && git add -A`.quiet().nothrow();
const diffResult = await $`cd ${VAULT_CIPHER_DIR} && git diff --cached --quiet`.quiet().nothrow();
if (diffResult.exitCode === 0) process.exit(0); // No changes

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
  writeFileSync(LAST_SUCCESS, new Date().toISOString());
  writeFileSync(FAILURE_COUNT, "0");
} else {
  let failures = 0;
  try {
    failures = Number.parseInt(readFileSync(FAILURE_COUNT, "utf-8"));
  } catch {}
  failures++;
  writeFileSync(FAILURE_COUNT, String(failures));

  if (failures >= SYNC_FAILURE_THRESHOLD) {
    mkdirSync(`${WORKSPACE_MOUNT}/.notifications`, { recursive: true });
    writeFileSync(
      `${WORKSPACE_MOUNT}/.notifications/sync-failure.md`,
      `# ⚠️ Vault Sync Failure\n\nFailed ${failures} times. Run \`doctor\` to diagnose.\n`,
    );
  }
  process.exit(1);
}
