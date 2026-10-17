/**
 * The backup watcher's state machine, as pure transitions.
 *
 * The watcher (cli/backup-watcher.ts) owns the effects: inotify, the 1-second tick,
 * spawning restic. Every change to its state goes through these functions instead of
 * being scattered across callbacks, so a whole day of events, ticks and backups can be
 * replayed in a test (see tests/backup-loop.test.ts) instead of being trusted.
 */
import { type DebounceDecision, planDebounce } from "./domain.ts";

export interface BackupLoop {
  /** Most recent filesystem event; never cleared (see planDebounce rule 6). */
  lastEventMs: number | null;
  /** When the last backup STARTED, success or failure. */
  lastTriggerMs: number | null;
  backupInFlight: boolean;
  /** An event arrived while a backup was running. */
  pendingDuringFlight: boolean;
  consecutiveFailures: number;
  lastFailureMs: number | null;
}

/** A fresh loop; `persistedFailures` is the counter that survived a restart. */
export function newLoop(persistedFailures = 0): BackupLoop {
  return {
    lastEventMs: null,
    lastTriggerMs: null,
    backupInFlight: false,
    pendingDuringFlight: false,
    consecutiveFailures: persistedFailures,
    lastFailureMs: null,
  };
}

export function onFsEvent(loop: BackupLoop, nowMs: number): BackupLoop {
  return {
    ...loop,
    lastEventMs: nowMs,
    pendingDuringFlight: loop.pendingDuringFlight || loop.backupInFlight,
  };
}

/** What the tick should do now. */
export function decideNext(loop: BackupLoop, nowMs: number): DebounceDecision {
  return planDebounce({
    lastEventMs: loop.lastEventMs,
    nowMs,
    backupInFlight: loop.backupInFlight,
    lastTriggerMs: loop.lastTriggerMs,
    consecutiveFailures: loop.consecutiveFailures,
    lastFailureMs: loop.lastFailureMs,
  });
}

export function onBackupStarted(loop: BackupLoop, nowMs: number): BackupLoop {
  return {
    ...loop,
    backupInFlight: true,
    lastTriggerMs: nowMs,
    // Only events that arrive DURING this backup matter for the flag, not earlier ones.
    pendingDuringFlight: false,
  };
}

export type BackupOutcome =
  | { ok: true }
  /** `failures` is the consecutive count after this failure (the tracker's answer). */
  | { ok: false; failures: number };

export function onBackupFinished(
  loop: BackupLoop,
  outcome: BackupOutcome,
  nowMs: number,
): BackupLoop {
  const next: BackupLoop = outcome.ok
    ? { ...loop, consecutiveFailures: 0, lastFailureMs: null }
    : { ...loop, consecutiveFailures: outcome.failures, lastFailureMs: nowMs };
  // Edits that landed mid-backup restart the quiet window, so the next backup does not
  // fire the instant this one ends while the user is still typing.
  return {
    ...next,
    backupInFlight: false,
    lastEventMs: loop.pendingDuringFlight ? nowMs : next.lastEventMs,
    pendingDuringFlight: false,
  };
}
