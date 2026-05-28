#!/usr/bin/env bun
/**
 * lock-vault — Unmount the gocryptfs vault
 */
import { $ } from "bun";
import { WORKSPACE_MOUNT, STATE_SOCKET_PATH } from "@sdw/core/constants";

// 1. Check if mounted
const check = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (check.exitCode !== 0) {
  console.log("Vault is not currently mounted.");
  process.exit(0);
}

// 2. Unmount
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

// 3. Broadcast
await $`echo '{"state":"locked"}' | socat - UNIX-CONNECT:${STATE_SOCKET_PATH}`.quiet().nothrow();
console.log("Plaintext data is no longer accessible.");
