#!/usr/bin/env bun
/**
 * lock-vault — Unmount the gocryptfs vault.
 *
 * Thin driving adapter over lockVault(): desktops and the backup watcher are
 * stopped first (they hold fds on the mount), then the vault is unmounted.
 */
import { WORKSPACE_VAULT, lockVault } from "@sdw/core/vault";
import { broadcastState, gocryptfsVolume, supervisorHolders } from "@sdw/infra";

const deps = { volume: gocryptfsVolume, holders: supervisorHolders };

const result = await lockVault(WORKSPACE_VAULT, deps);

if (result.status === "noop") {
  console.log("Vault is not currently mounted.");
  process.exit(0);
}

if (result.status === "rejected") {
  console.error("ERROR: This environment has been destroyed.");
  process.exit(1);
}

if (result.status === "busy") {
  console.error("Could not unmount cleanly (open file handles?).");
  const answer = prompt("Force unmount? [y/N]: ");
  if (answer?.toLowerCase() !== "y") {
    console.log("Aborted.");
    process.exit(1);
  }
  const forced = await lockVault(WORKSPACE_VAULT, deps, { mode: "lazy" });
  if (forced.status !== "locked") {
    console.error("ERROR: Force unmount failed.");
    process.exit(1);
  }
  console.log("Vault force-locked.");
} else {
  console.log("Vault locked.");
}

await broadcastState("locked");
console.log("Plaintext data is no longer accessible.");
