#!/usr/bin/env bun
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import {
  SYNC_FAILURE_THRESHOLD,
  VAULT_CIPHER_DIR,
  VAULT_SYNC_STATE_DIR,
  planSync,
} from "@sdw/core/vault";
import {
  commitStaged,
  createFileFailureTracker,
  hasUnpushedCommits,
  originOf,
  pushToOrigin,
  stageChanges,
} from "@sdw/infra";
import { $ } from "bun";
/**
 * vault-sync — Periodic vault ciphertext sync to a git remote
 *
 * Run by cron every 30 minutes. Cron does not pass the container's environment on, so this
 * job does not read VAULT_GIT_REPO: the start-up makes the cipher directory a repository
 * with that remote (prepare-replication), and "has an origin" is what turns replication on.
 */

// Same tracker the backup watcher uses: counter, last-success marker, notification file.
const tracker = createFileFailureTracker({
  stateDir: VAULT_SYNC_STATE_DIR,
  threshold: SYNC_FAILURE_THRESHOLD,
  notificationFile: "sync-failure.md",
  notificationBody: (n) =>
    `# ⚠ Vault Sync Failure\n\nFailed ${n} times. Run \`doctor\` to diagnose.\n`,
});

const hasRemote = (await originOf(VAULT_CIPHER_DIR)) !== undefined;
const mounted = (await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow()).exitCode === 0;

// Nowhere to push to (replication is opt-in), or the vault is locked: nothing to do.
const gate = planSync({ hasRemote, mounted, hasStagedChanges: false, hasUnpushedCommits: false });
if (gate.action === "skip" && gate.reason !== "no-changes") process.exit(0);

const identity = { fallbackName: "Vault", fallbackEmail: "vault@local" };
try {
  const plan = planSync({
    hasRemote,
    mounted,
    hasStagedChanges: await stageChanges(VAULT_CIPHER_DIR),
    hasUnpushedCommits: await hasUnpushedCommits(VAULT_CIPHER_DIR),
  });
  if (plan.action === "skip") process.exit(0); // quiet cycle

  if (plan.action === "commit-and-push") {
    const committed = await commitStaged({
      dir: VAULT_CIPHER_DIR,
      message: `auto-sync ${new Date().toISOString()}`,
      ...identity,
    });
    if (!committed) throw new Error("commit failed");
  }
  // Also for "push-only": a commit whose push failed earlier is still waiting.
  if (!(await pushToOrigin({ dir: VAULT_CIPHER_DIR, ...identity }))) throw new Error("push failed");
  tracker.recordSuccess();
} catch (err) {
  tracker.recordFailure();
  console.error(`vault-sync failed: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
}
