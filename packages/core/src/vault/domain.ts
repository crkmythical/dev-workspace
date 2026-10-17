/**
 * Vault domain: lifecycle rules and the destroy-phase table.
 *
 * Pure: no IO and no runtime imports, so every rule is property-testable
 * without gocryptfs/FUSE. This supersedes cli/lib/vault-lifecycle.ts: the former
 * planInit / planUnlock / planLock are consolidated into decide() over an
 * explicit VaultState. planSync (ciphertext replication gate) lives here too.
 */

/** "workspace" for the main vault; a plugin that owns a volume picks its own id. */
export type VaultId = string;

/** One encrypted volume. The workspace and each plugin's volume are instances of one model. */
export interface VaultSpec {
  id: VaultId;
  cipherDir: string;
  mountPoint: string;
}

/** "destroyed" is terminal: the self-destruct completion marker exists. */
export type VaultState = "uninitialized" | "locked" | "unlocked" | "destroyed";

export type VaultCommand =
  | { type: "init" }
  | { type: "unlock"; passphrase: string }
  | { type: "lock" }
  | { type: "destroy" };

export type Decision =
  | { do: "perform"; warnShortPassphrase?: boolean }
  | {
      do: "noop";
      reason: "already-initialized" | "already-unlocked" | "already-locked" | "already-destroyed";
    }
  | { do: "reject"; reason: "not-initialized" | "empty-passphrase" | "destroyed" };

/** Only a locked vault needs a passphrase, so callers prompt lazily after this check. */
export function needsPassphrase(state: VaultState): boolean {
  return state === "locked";
}

/**
 * Decide what a command should do in a given state.
 *   - init / unlock / lock are idempotent (a repeat is a no-op, never an error);
 *   - unlock refuses an uninitialized vault and an empty passphrase;
 *   - a destroyed vault rejects everything except a (no-op) repeat of destroy.
 * The passphrase itself is adjudicated by gocryptfs at mount time, not here.
 */
export function decide(state: VaultState, cmd: VaultCommand): Decision {
  if (state === "destroyed") {
    return cmd.type === "destroy"
      ? { do: "noop", reason: "already-destroyed" }
      : { do: "reject", reason: "destroyed" };
  }
  switch (cmd.type) {
    case "init":
      return state === "uninitialized"
        ? { do: "perform" }
        : { do: "noop", reason: "already-initialized" };
    case "unlock":
      if (state === "unlocked") return { do: "noop", reason: "already-unlocked" };
      if (state === "uninitialized") return { do: "reject", reason: "not-initialized" };
      if (cmd.passphrase.length === 0) return { do: "reject", reason: "empty-passphrase" };
      return { do: "perform", warnShortPassphrase: cmd.passphrase.length < 8 };
    case "lock":
      return state === "unlocked" ? { do: "perform" } : { do: "noop", reason: "already-locked" };
    case "destroy":
      return { do: "perform" };
  }
}

// --- vault-sync (ciphertext replication) ---

/** Consecutive failed pushes before a notification is written. */
export const SYNC_FAILURE_THRESHOLD = 3;

/**
 * The .gitignore written into the ciphertext repository when a vault is created. One
 * definition for every command that creates it; the vault-sync exclusion tests run
 * against THIS text, not a copy of it.
 */
export const VAULT_GITIGNORE = `node_modules/
target/
build/
dist/
.cache/
**/*.log
.idea/
.vscode/
__pycache__/
.DS_Store
*.tmp
*.swp
shared/
`;

/** Large binaries go through git-lfs in the ciphertext repository. */
export const VAULT_GITATTRIBUTES =
  "*.bin filter=lfs diff=lfs merge=lfs -text\n*.jar filter=lfs diff=lfs merge=lfs -text\n";

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
 * Locked-boundary invariant: when not mounted, never commit.
 * Quiet-cycle invariant: when mounted but nothing changed, never commit.
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

// --- destruction ---

/**
 * Destroy phases, in the only order they may run. Phases flagged irreversible
 * cannot be undone; "shred-keys" is the Point of No Return.
 */
export const DESTROY_PHASES = [
  {
    id: "quiesce",
    irreversible: false,
    label: "[Phase 0] Stopping vault-sync and optional remote cleanup...",
  },
  { id: "unmount", irreversible: false, label: "[Phase 1] Unmounting vaults..." },
  { id: "shred-keys", irreversible: true, label: "[Phase 2] Shredding vault master keys..." },
  {
    id: "shred-data",
    irreversible: true,
    label: "[Phase 3] Shredding cipher data (this may take a while)...",
  },
  {
    id: "scrub-credentials",
    irreversible: true,
    label: "[Phase 4] Cleaning credentials and config...",
  },
  { id: "mark-complete", irreversible: true, label: "[Phase 5] Finalizing..." },
] as const;

export type DestroyPhaseId = (typeof DESTROY_PHASES)[number]["id"];
