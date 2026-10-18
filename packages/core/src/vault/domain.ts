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
  /**
   * Let users other than the one who mounted it use the volume. FUSE refuses every other uid by
   * default, and a volume that holds a root file system needs it: its programs drop to their own
   * system user (tor runs as debian-tor, apt downloads as _apt) and would get "Permission denied"
   * on every file, their own executables included.
   */
  allowOther?: boolean;
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

/**
 * Plain-language lines for a failed unlock, from what gocryptfs printed. A wrong passphrase
 * and a mount the container is not allowed to make both leave the vault locked, but they are
 * fixed in opposite places, and the second one used to be reported as "password mismatch?".
 *
 * `passwordGenerated`: the passphrase tried is one this start-up just made up because PASSWORD
 * is empty. It lives in the container, so recreating the container (not restarting it) throws
 * it away, and the vault made under the old one then looks like a plain wrong passphrase.
 */
export function explainUnlockFailure(
  detail: string,
  opts: { passwordGenerated?: boolean } = {},
): string[] {
  const text = detail.toLowerCase();
  if (text.includes("password incorrect")) {
    if (!opts.passwordGenerated) return ["The passphrase is wrong."];
    return [
      "The passphrase is wrong.",
      "PASSWORD is not set, so this start generated a new password. A generated password lives",
      "only in the container and does not survive recreating it (docker compose up -d after a",
      "rebuild, docker rm). If the vault was made under an earlier one, open it with the old",
      "password: docker exec -it dev-workspace unlock-vault. Then set PASSWORD in .env to it.",
    ];
  }
  if (text.includes("mount failed") && text.includes("permission denied")) {
    return [
      "The container is not allowed to mount a FUSE filesystem.",
      "On a Linux host with AppArmor (Ubuntu, Debian) add docker-compose.apparmor.yml: set",
      "COMPOSE_FILE in .env (see .env.example) or start with -f docker-compose.yml -f docker-compose.apparmor.yml",
    ];
  }
  if (text.includes("fuse device not found") || text.includes("/dev/fuse")) {
    return [
      "/dev/fuse is not available in the container (docker-compose.yml passes it as a device).",
    ];
  }
  return [];
}

// --- Releasing a busy mount ---

/** The PIDs in `fuser`'s output: it prints them, space separated, on stdout. */
export function parseFuserPids(stdout: string): number[] {
  return [...stdout.matchAll(/\d+/g)].map((m) => Number(m[0])).filter((pid) => pid > 0);
}

/**
 * `pid` followed by every process above it, up to PID 1. Stops at a process whose parent
 * cannot be read (it has exited) and never loops.
 */
export function lineage(pid: number, parentOf: (pid: number) => number | undefined): number[] {
  const chain: number[] = [];
  let current: number | undefined = pid;
  while (current !== undefined && current > 0 && !chain.includes(current)) {
    chain.push(current);
    current = parentOf(current);
  }
  return chain;
}

/**
 * Which of the processes using a mount may be killed to free it: all of them except
 * `protect`. The caller passes its own lineage. The process that releases the mount is the
 * one that goes on to destroy the vault, and killing its shell hangs up the terminal and
 * takes it down too; a `fuser -k` that did that left the keys in place (found by running
 * self-destruct from a terminal inside the container).
 */
export function killable(holders: readonly number[], protect: readonly number[]): number[] {
  const keep = new Set(protect);
  return [...new Set(holders)].filter((pid) => !keep.has(pid));
}

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
  /**
   * Does the ciphertext repository exist and have an `origin` to push to? This is observed
   * from the repository, not from VAULT_GIT_REPO: the job runs from cron, which does not
   * pass the container's environment on. Replication is opt-in, so "no" is not an error.
   */
  hasRemote: boolean;
  /** Is the workspace mount present? */
  mounted: boolean;
  /** Did `git add -A` followed by `git diff --cached --quiet` report changes? */
  hasStagedChanges: boolean;
  /** Commits that exist here but not on origin yet, because an earlier push failed. */
  hasUnpushedCommits: boolean;
}

export type SyncPlan =
  | { action: "skip"; reason: "no-remote"; exitCode: 0 }
  | { action: "skip"; reason: "not-mounted"; exitCode: 0 }
  | { action: "skip"; reason: "no-changes"; exitCode: 0 }
  | { action: "commit-and-push" }
  | { action: "push-only" };

/**
 * Opt-in invariant: with nowhere to push to, do nothing (and do not fail: the cron job
 * used to crash every cycle on a vault that was never made a repository).
 * Locked-boundary invariant: when not mounted, never commit.
 * Quiet-cycle invariant: when mounted, nothing changed and nothing is waiting to be
 * pushed, never commit.
 * Retry invariant: a commit whose push failed is pushed on a later cycle even if nothing
 * else changed. Otherwise the next cycle looked like a quiet one, the failure was never
 * repeated, and the last changes stayed off the remote until the next edit (found by
 * breaking the remote and running the job three times).
 */
export function planSync(state: SyncState): SyncPlan {
  if (!state.hasRemote) {
    return { action: "skip", reason: "no-remote", exitCode: 0 };
  }
  if (!state.mounted) {
    return { action: "skip", reason: "not-mounted", exitCode: 0 };
  }
  if (state.hasStagedChanges) return { action: "commit-and-push" };
  if (state.hasUnpushedCommits) return { action: "push-only" };
  return { action: "skip", reason: "no-changes", exitCode: 0 };
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
