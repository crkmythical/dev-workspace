#!/usr/bin/env bun
/**
 * unlock-vault — Mount the gocryptfs vault at /workspace and run the post-unlock hooks.
 *
 * Thin driving adapter. The decision, the mount and the hooks live in
 * @sdw/core/vault and @sdw/infra, and are shared with the container's
 * boot-time auto-unlock, so both paths leave the system in the same state.
 */
import { WORKSPACE_VAULT, explainUnlockFailure, unlockVault } from "@sdw/core/vault";
import { loadConfig } from "@sdw/core/workspace";
import { gocryptfsVolume, interactiveUnlockExtras, workspaceUnlockHooks } from "@sdw/infra";
import { readPassphrase } from "./lib/passphrase.ts";

const config = loadConfig(process.env);

const result = await unlockVault(
  WORKSPACE_VAULT,
  () => readPassphrase(undefined, "Vault passphrase: "),
  {
    volume: gocryptfsVolume,
    hooks: [
      ...workspaceUnlockHooks({ flags: config.features, announce: console.log }),
      ...interactiveUnlockExtras({ backupEnabled: Boolean(config.backup.RESTIC_REPOSITORY) }),
    ],
  },
);

if (result.status === "noop") {
  console.log("Vault already unlocked.");
  process.exit(0);
}

if (result.status === "rejected") {
  const message = {
    "not-initialized": "ERROR: Vault not initialized. Run 'init-vault' first.",
    "empty-passphrase": "ERROR: Passphrase cannot be empty.",
    destroyed: "ERROR: This environment has been destroyed.",
  }[result.reason];
  console.error(message);
  process.exit(1);
}

if (result.status === "failed") {
  console.error("ERROR: Failed to unlock vault.");
  if (result.detail) console.error(`  ${result.detail.trim().split("\n").join("\n  ")}`);
  const why = explainUnlockFailure(result.detail);
  for (const line of why.length > 0 ? why : ["Wrong passphrase, or the mount failed."]) {
    console.error(`  ${line}`);
  }
  process.exit(1);
}

if (result.warnShortPassphrase) {
  console.warn("⚠ WARNING: Passphrase is very short (<8 chars). Use 16+ for production.");
}
for (const failure of result.hookFailures) {
  console.warn(`WARNING: post-unlock step failed: ${failure}`);
}
console.log("\n✓ Vault unlocked. Workspace available at /workspace\n");
