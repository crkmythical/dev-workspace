/**
 * Realtime Backup — Pure decision logic (no IO).
 *
 * This module contains all constants, types, and pure functions for the
 * event-driven restic backup feature. The IO execution layer lives in
 * packages/cli/src/backup-watcher.ts.
 */

// ─── Constants ───────────────────────────────────────────────────────────────

/** Debounce quiet window: 10 seconds of no filesystem events before triggering backup. */
export const BACKUP_DEBOUNCE_MS = 10_000;

/** Fallback interval: force a backup every 5 minutes even without inotify events
 *  (safety net for inotify unreliability on gocryptfs FUSE). */
export const BACKUP_FALLBACK_INTERVAL_MS = 300_000;

/** Runtime state directory (non-FUSE, container-local). Loss on restart is non-fatal. */
export const BACKUP_STATE_DIR = "/var/run/backup-state";

/** Consecutive failures before writing a notification to /workspace/.notifications/. */
export const BACKUP_FAILURE_THRESHOLD = 3;

/** Doctor warns if last successful backup is older than this (10 minutes). */
export const BACKUP_STALENESS_MS = 600_000;

/** Maximum retry attempts before giving up on a single backup cycle. */
export const BACKUP_MAX_RETRIES = 5;

/** Base delay for exponential backoff (5 seconds). */
export const BACKUP_BASE_BACKOFF_MS = 5_000;

/** Retention policy: keep-hourly snapshots. */
export const BACKUP_KEEP_HOURLY = 24;

/** Retention policy: keep-daily snapshots. */
export const BACKUP_KEEP_DAILY = 7;

/** Retention policy: keep-weekly snapshots. */
export const BACKUP_KEEP_WEEKLY = 4;

/** Prune cadence: run `restic forget --prune` at most every 6 hours. */
export const BACKUP_PRUNE_CADENCE_MS = 6 * 60 * 60 * 1000;

/**
 * Exclusion set — single source of truth for both watcher-level ignore and
 * restic --exclude arguments. Checked against ALL path segments (not just the
 * first), so nested occurrences (e.g. project/node_modules) are also excluded.
 */
export const BACKUP_EXCLUSION_SET: readonly string[] = [
  "node_modules",
  ".git",
  ".cache",
  ".desktop",
  ".desktop-vnc",
  ".notifications",
  ".code-server",
  ".uploads",
  "dist",
  "build",
  ".vscode",
  ".idea",
  "__pycache__",
  ".mypy_cache",
  ".pytest_cache",
  "target",
  ".next",
  ".nuxt",
] as const;

// ─── Types ───────────────────────────────────────────────────────────────────

export type DebounceDecision = "trigger" | "wait" | "coalesce";

export interface DebounceState {
  /** Timestamp of most recent filesystem event (null = none received yet). */
  lastEventMs: number | null;
  /** Current time. */
  nowMs: number;
  /** Whether a restic backup subprocess is currently running. */
  backupInFlight: boolean;
  /** Timestamp of last backup START — success or failure (null = never triggered). */
  lastTriggerMs: number | null;
  /** Current consecutive failure count (0 = last backup succeeded or never ran). */
  consecutiveFailures: number;
  /** Timestamp of last backup failure (null = never failed). */
  lastFailureMs: number | null;
}

export type PruneDecision = "prune-due" | "skip";

export interface PruneState {
  /** Timestamp of last successful prune (null = never pruned). */
  lastPruneMs: number | null;
  /** Current time. */
  nowMs: number;
}

export interface BackupGateEnv {
  RESTIC_REPOSITORY?: string;
  RESTIC_PASSWORD?: string;
  AWS_ACCESS_KEY_ID?: string;
  AWS_SECRET_ACCESS_KEY?: string;
}

export type GateResult =
  | { status: "dormant" }
  | { status: "enabled" }
  | { status: "misconfigured"; missing: string[] };

export type DoctorBackupStatus = "ok" | "warn" | "fail" | "not-configured";

export interface DoctorBackupState {
  gateResult: GateResult;
  lastSuccessMs: number | null;
  consecutiveFailures: number;
  nowMs: number;
}

// ─── Pure Functions ──────────────────────────────────────────────────────────

/**
 * Decide whether to trigger a backup, wait, or coalesce with an in-flight run.
 *
 * Decision priority:
 * 1. In-flight → coalesce (never start a second concurrent backup)
 * 2. First run (lastTriggerMs === null) → trigger (immediate baseline)
 * 3. Within backoff window after failure → wait
 * 4. Fallback elapsed (≥ 5 min since last trigger) → trigger
 * 5. No events yet → wait
 * 6. Quiet ≥ 10s since last event → trigger
 * 7. Otherwise → wait
 */
export function planDebounce(state: DebounceState): DebounceDecision {
  if (state.backupInFlight) return "coalesce";
  if (state.lastTriggerMs === null) return "trigger";
  // Backoff: after failure, wait until backoff period elapses
  if (state.consecutiveFailures > 0 && state.lastFailureMs !== null) {
    const backoff = backoffDelayMs(state.consecutiveFailures);
    if (state.nowMs - state.lastFailureMs < backoff) return "wait";
  }
  if (state.nowMs - state.lastTriggerMs >= BACKUP_FALLBACK_INTERVAL_MS) {
    return "trigger";
  }
  if (state.lastEventMs === null) return "wait";
  const quiet = state.nowMs - state.lastEventMs;
  return quiet >= BACKUP_DEBOUNCE_MS ? "trigger" : "wait";
}

/** Decide whether a prune cycle is due. */
export function planPrune(state: PruneState): PruneDecision {
  if (state.lastPruneMs === null) return "prune-due";
  return state.nowMs - state.lastPruneMs >= BACKUP_PRUNE_CADENCE_MS
    ? "prune-due"
    : "skip";
}

/** Compute exponential backoff delay: 5s × 2^min(n, 4). Capped at ~80s. */
export function backoffDelayMs(consecutiveFailures: number): number {
  return BACKUP_BASE_BACKOFF_MS * Math.pow(2, Math.min(consecutiveFailures, 4));
}

/** Evaluate the opt-in gate from environment variables. */
export function evaluateBackupGate(env: BackupGateEnv): GateResult {
  if (!env.RESTIC_REPOSITORY) return { status: "dormant" };
  const missing: string[] = [];
  if (!env.RESTIC_PASSWORD) missing.push("RESTIC_PASSWORD");
  if (!env.AWS_ACCESS_KEY_ID) missing.push("AWS_ACCESS_KEY_ID");
  if (!env.AWS_SECRET_ACCESS_KEY) missing.push("AWS_SECRET_ACCESS_KEY");
  return missing.length > 0
    ? { status: "misconfigured", missing }
    : { status: "enabled" };
}

/** Whether the failure count has reached the notification threshold. */
export function shouldNotify(consecutiveFailures: number): boolean {
  return consecutiveFailures >= BACKUP_FAILURE_THRESHOLD;
}

/** Compute the doctor status for the backup component. */
export function planDoctorBackupStatus(state: DoctorBackupState): DoctorBackupStatus {
  if (state.gateResult.status === "dormant") return "not-configured";
  if (state.gateResult.status === "misconfigured") return "fail";
  if (state.consecutiveFailures >= BACKUP_FAILURE_THRESHOLD) return "fail";
  if (state.lastSuccessMs === null) return "warn";
  if (state.nowMs - state.lastSuccessMs >= BACKUP_STALENESS_MS) return "warn";
  return "ok";
}

/**
 * Check if a relative path (from /workspace) should be excluded.
 * Checks ALL path segments — so nested occurrences like
 * `project/node_modules/pkg/index.js` are correctly excluded.
 */
export function isExcludedPath(relativePath: string): boolean {
  const segments = relativePath.split("/");
  return segments.some((seg) => BACKUP_EXCLUSION_SET.includes(seg));
}
