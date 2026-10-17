/**
 * Notes for the user ("backup is failing", "egress is degraded") as plain files.
 *
 * They live inside the vault (see NOTIFICATIONS_DIR), so they are readable while the
 * vault is unlocked and absent otherwise. That is also why a write is REFUSED while
 * nothing is mounted at the workspace root: the directory would then be an ordinary
 * one on the container's own, unencrypted filesystem, and the note would end up on
 * disk in the clear (and hidden under the mount once the vault comes up).
 */
import { mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";

/** A directory on a different device than its parent is a mount point. */
export function isMountPoint(dir: string): boolean {
  try {
    return statSync(dir).dev !== statSync(path.dirname(dir)).dev;
  } catch {
    return false;
  }
}

/**
 * Which directory must be a live mount for notes below it to be written. Injectable so
 * the refusal can be tested on a machine where writing to the real root would fail anyway.
 */
export interface MountGuard {
  root: string;
  isMounted: (dir: string) => boolean;
}

const WORKSPACE_GUARD: MountGuard = { root: WORKSPACE_MOUNT, isMounted: isMountPoint };

function blocked(file: string, guard: MountGuard): boolean {
  return file.startsWith(`${guard.root}/`) && !guard.isMounted(guard.root);
}

/** Write a note. False when there was nowhere safe to write it (not fatal). */
export function writeNote(
  file: string,
  body: string,
  guard: MountGuard = WORKSPACE_GUARD,
): boolean {
  if (blocked(file, guard)) return false;
  try {
    mkdirSync(path.dirname(file), { recursive: true });
    writeFileSync(file, body);
    return true;
  } catch {
    return false;
  }
}

/** Remove a note, if there is one. */
export function removeNote(file: string, guard: MountGuard = WORKSPACE_GUARD): void {
  if (blocked(file, guard)) return;
  try {
    rmSync(file, { force: true });
  } catch {}
}
