/**
 * Pure decision logic for vault lifecycle commands (init / unlock / lock / sync).
 *
 * These functions have NO side effects: they take an observed state and return
 * a plan describing what the calling script should do. Keeping the branching
 * logic here makes the idempotence / gating invariants property-testable
 * without needing a real gocryptfs/FUSE mount.
 *
 * The user-facing scripts (init-vault.ts, unlock-vault.ts, lock-vault.ts,
 * vault-sync.ts) gather the runtime state and execute the returned plan.
 */

// --- unlock-vault ---

export interface UnlockState {
  /** Is the mount point already a FUSE mountpoint? */
  mounted: boolean;
  /** Does <cipherDir>/gocryptfs.conf exist? */
  initialized: boolean;
  /** The passphrase supplied by the user (already trimmed). */
  passphrase: string;
}

export type UnlockPlan =
  | { action: "noop"; reason: "already-mounted"; exitCode: 0 }
  | { action: "error"; reason: "not-initialized"; exitCode: 1 }
  | { action: "error"; reason: "empty-passphrase"; exitCode: 1 }
  | { action: "mount"; warnShort: boolean };

/**
 * Decide what unlock-vault should do given the observed state.
 * Idempotence (Property 3): an already-mounted vault yields a no-op.
 */
export function planUnlock(state: UnlockState): UnlockPlan {
  if (state.mounted) {
    return { action: "noop", reason: "already-mounted", exitCode: 0 };
  }
  if (!state.initialized) {
    return { action: "error", reason: "not-initialized", exitCode: 1 };
  }
  if (state.passphrase.length === 0) {
    return { action: "error", reason: "empty-passphrase", exitCode: 1 };
  }
  return { action: "mount", warnShort: state.passphrase.length < 8 };
}

// --- lock-vault ---

export type LockPlan =
  | { action: "noop"; reason: "not-mounted"; exitCode: 0 }
  | { action: "unmount" };

/**
 * Decide what lock-vault should do.
 * Round-trip (Property 3): a mounted vault unmounts; an unmounted vault is a no-op.
 */
export function planLock(mounted: boolean): LockPlan {
  if (!mounted) {
    return { action: "noop", reason: "not-mounted", exitCode: 0 };
  }
  return { action: "unmount" };
}

// --- init-vault ---

export type InitPlan =
  | { action: "noop"; reason: "already-initialized"; exitCode: 0 }
  | { action: "init" };

/**
 * Decide what init-vault should do.
 * Idempotence (Property 6): a vault with an existing gocryptfs.conf is a no-op,
 * never re-initialized, never re-prompted.
 */
export function planInit(initialized: boolean): InitPlan {
  if (initialized) {
    return { action: "noop", reason: "already-initialized", exitCode: 0 };
  }
  return { action: "init" };
}

// --- vault-sync ---

export interface SyncState {
  /** Is the workspace mount present? */
  mounted: boolean;
  /** Did `git add -A` followed by `git diff --cached --quiet` report changes? */
  hasStagedChanges: boolean;
}

export type SyncPlan =
  | { action: "skip"; reason: "not-mounted"; exitCode: 0 }
  | { action: "skip"; reason: "no-changes"; exitCode: 0 }
  | { action: "commit-and-push" };

/**
 * Decide what vault-sync should do.
 * Locked-boundary invariant (Property 5): when not mounted, never commit.
 * Quiet-cycle no-op (Property 16): when mounted but nothing changed, never commit.
 */
export function planSync(state: SyncState): SyncPlan {
  if (!state.mounted) {
    return { action: "skip", reason: "not-mounted", exitCode: 0 };
  }
  if (!state.hasStagedChanges) {
    return { action: "skip", reason: "no-changes", exitCode: 0 };
  }
  return { action: "commit-and-push" };
}
