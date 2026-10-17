/**
 * Container filesystem map shared by every context. Each context's own
 * `layout.ts` derives its paths from these roots, so no absolute prefix is
 * repeated, and nothing outside a `layout.ts` spells an absolute path out.
 */

/** Plaintext FUSE mount of the workspace vault. */
export const WORKSPACE_MOUNT = "/workspace";

/** Credentials stored inside the vault (ssh keys, tokens). */
export const CREDENTIALS_DIR = `${WORKSPACE_MOUNT}/.credentials`;

/** Container-local runtime state (not persisted, not inside the vault). */
export const RUN_DIR = "/var/run";
