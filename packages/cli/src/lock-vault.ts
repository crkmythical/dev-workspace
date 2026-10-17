#!/usr/bin/env bun
/**
 * lock-vault — Unmount the gocryptfs vault.
 *
 * Thin driving adapter over lockVault(): desktops and the backup watcher are
 * stopped first (they hold fds on the mount), then the vault is unmounted.
 */
import { WORKSPACE_VAULT, lockVault } from "@sdw/core/vault";
import { broadcastState, gocryptfsVolume, supervisorHolders } from "@sdw/infra";

const deps = { volume: gocryptfsVolume, holders: supervisorHolders, log: console.log };

const result = await lockVault(WORKSPACE_VAULT, deps);

if (result.status === "noop") {
  console.log("Vault is not currently mounted.");
  process.exit(0);
}

if (result.status === "rejected") {
  console.error("ERROR: This environment has been destroyed.");
  process.exit(1);
}

let forced = false;
if (result.status === "busy") {
  console.error("Could not unmount cleanly (open file handles?).");
  const answer = prompt("Force unmount? [y/N]: ");
  if (answer?.toLowerCase() !== "y") {
    console.log("Aborted.");
    process.exit(1);
  }
  const lazy = await lockVault(WORKSPACE_VAULT, deps, { mode: "lazy" });
  if (lazy.status !== "locked") {
    console.error("ERROR: Force unmount failed.");
    process.exit(1);
  }
  forced = true;
  console.log("Vault force-locked.");
} else {
  console.log("Vault locked.");
}

await broadcastState("locked");
if (forced) {
  // A lazy unmount detaches the mount at once, but whoever already had a file open keeps it.
  console.log("Processes that still had files open can keep using them until they exit.");
  console.log(
    "To cut them off now, restart the container (or: supervisorctl restart code-server).",
  );
} else {
  console.log("Plaintext data is no longer accessible.");
}
