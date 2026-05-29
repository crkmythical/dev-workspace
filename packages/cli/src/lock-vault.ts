#!/usr/bin/env bun
import { STATE_SOCKET_PATH, WORKSPACE_MOUNT } from "@sdw/core/constants";
/**
 * lock-vault — Unmount the gocryptfs vault
 */
import { $ } from "bun";
import { planLock } from "./lib/vault-lifecycle.ts";

// 1. Check if mounted
const check = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
const lockPlan = planLock(check.exitCode === 0);
if (lockPlan.action === "noop") {
  console.log("Vault is not currently mounted.");
  process.exit(lockPlan.exitCode);
}

// 2. Stop desktop if running (before unmount to avoid FUSE EBUSY)
const desktopStatus = await $`supervisorctl status desktop:desktop-xvnc`.quiet().nothrow();
if (desktopStatus.exitCode === 0 && desktopStatus.text().includes("RUNNING")) {
  console.log("Stopping desktop session...");
  await $`supervisorctl stop desktop:desktop-xvnc desktop:desktop-openbox desktop:desktop-tint2`.quiet().nothrow();
}

// 3. Unmount
console.log("Locking vault...");
const umount = await $`fusermount -u ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (umount.exitCode !== 0) {
  console.error("Could not unmount cleanly (open file handles?).");
  const answer = prompt("Force unmount? [y/N]: ");
  if (answer?.toLowerCase() === "y") {
    await $`fusermount -uz ${WORKSPACE_MOUNT}`.quiet();
    console.log("Vault force-locked.");
  } else {
    console.log("Aborted.");
    process.exit(1);
  }
} else {
  console.log("Vault locked.");
}

// 4. Broadcast
await $`echo '{"state":"locked"}' | socat - UNIX-CONNECT:${STATE_SOCKET_PATH}`.quiet().nothrow();
console.log("Plaintext data is no longer accessible.");
