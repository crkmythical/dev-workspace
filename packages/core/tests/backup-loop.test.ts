import { describe, expect, it } from "bun:test";
import {
  BACKUP_DEBOUNCE_MS,
  BACKUP_FALLBACK_INTERVAL_MS,
  type BackupLoop,
  backoffDelayMs,
  decideNext,
  newLoop,
  onBackupFinished,
  onBackupStarted,
  onFsEvent,
} from "../src/backup/index.ts";

/**
 * The watcher's loop replayed on a fake clock: a 1-second tick, a backup that takes
 * `backupMs` and either succeeds or fails, and a script of filesystem events. What is
 * asserted is the list of moments backups STARTED, because the bug this guards against
 * (a backup storm after a single edit) is only visible over time, not in one decision.
 */
interface Run {
  /** Seconds at which a backup started. */
  starts: number[];
  loop: BackupLoop;
}

function simulate(opts: {
  seconds: number;
  /** Seconds at which the filesystem reports an event. */
  events?: number[];
  backupMs?: number;
  /** Backup n (0-based) fails when this returns true. */
  fails?: (n: number) => boolean;
  startFailures?: number;
}): Run {
  const { seconds, events = [], backupMs = 5_000, fails = () => false } = opts;
  const eventAt = new Set(events);
  let loop = newLoop(opts.startFailures ?? 0);
  let finishAt: number | null = null;
  let finishOk = true;
  let failureCount = opts.startFailures ?? 0;
  let n = 0;
  const starts: number[] = [];

  for (let t = 0; t <= seconds; t++) {
    const nowMs = t * 1000;
    if (eventAt.has(t)) loop = onFsEvent(loop, nowMs);
    if (finishAt !== null && nowMs >= finishAt) {
      if (!finishOk) failureCount += 1;
      else failureCount = 0;
      loop = onBackupFinished(
        loop,
        finishOk ? { ok: true } : { ok: false, failures: failureCount },
        nowMs,
      );
      finishAt = null;
    }
    if (decideNext(loop, nowMs) === "trigger") {
      starts.push(t);
      loop = onBackupStarted(loop, nowMs);
      finishOk = !fails(n++);
      finishAt = nowMs + backupMs;
    }
  }
  return { starts, loop };
}

const FALLBACK_S = BACKUP_FALLBACK_INTERVAL_MS / 1000;
const DEBOUNCE_S = BACKUP_DEBOUNCE_MS / 1000;

describe("backup loop over time", () => {
  it("one burst of edits yields ONE extra backup, not a storm (regression)", () => {
    // Edits at t=1..5, then silence for the rest of the hour. The first run is the
    // baseline at t=0; the burst is saved once, 10s after it went quiet; after that
    // only the 5-minute fallback fires.
    const { starts } = simulate({ seconds: 3600, events: [1, 2, 3, 4, 5] });
    expect(starts[0]).toBe(0);
    expect(starts[1]).toBe(5 + DEBOUNCE_S);
    const rest = starts.slice(2);
    // the fallback, every 5 minutes after the last backup started
    for (let i = 0; i < rest.length; i++) {
      expect(rest[i]).toBe(starts[1] + (i + 1) * FALLBACK_S);
    }
    expect(starts.length).toBeLessThanOrEqual(2 + Math.ceil(3600 / FALLBACK_S));
  });

  it("with no activity at all, only the baseline and the fallback run", () => {
    const { starts } = simulate({ seconds: 1000 });
    expect(starts).toEqual([0, FALLBACK_S, 2 * FALLBACK_S, 3 * FALLBACK_S]);
  });

  it("continuous editing waits for quiet: nothing is saved until the edits stop", () => {
    const edits = Array.from({ length: 30 }, (_, i) => 1 + i * 2); // every 2s until t=59
    const { starts } = simulate({ seconds: 200, events: edits });
    // baseline, then ONE backup after the last edit (t=59) + 10s of quiet
    expect(starts).toEqual([0, 59 + DEBOUNCE_S]);
  });

  it("an edit during a backup is saved afterwards, once the quiet window has passed", () => {
    // The baseline backup runs t=0..5; the edit at t=2 lands mid-flight.
    const { starts } = simulate({ seconds: 100, events: [2] });
    expect(starts).toEqual([0, 5 + DEBOUNCE_S]);
  });

  it("a failed backup is retried after its backoff, then the loop settles", () => {
    // Backup 0 (the baseline) fails, backup 1 succeeds.
    const { starts, loop } = simulate({ seconds: 120, fails: (n) => n === 0 });
    expect(starts[0]).toBe(0);
    // finished (failed) at t=5, backoff for 1 failure is 10s -> retry at t=15
    expect(starts[1]).toBe(5 + backoffDelayMs(1) / 1000);
    expect(starts).toHaveLength(2);
    expect(loop.consecutiveFailures).toBe(0);
  });

  it("repeated failures back off further and keep retrying (no storm, no giving up)", () => {
    const { starts } = simulate({ seconds: 600, fails: () => true });
    expect(starts.length).toBeGreaterThan(4);
    const gaps = starts.slice(1).map((t, i) => t - starts[i] - 5); // idle time between attempts
    // backoff doubles (10s, 20s, 40s, 80s) and then stays capped at 80s
    expect(gaps.slice(0, 4)).toEqual([10, 20, 40, 80]);
    for (const gap of gaps.slice(4)) expect(gap).toBe(80);
  });

  it("a counter persisted from before a restart does not stop the baseline run", () => {
    const { starts } = simulate({ seconds: 60, startFailures: 2 });
    expect(starts[0]).toBe(0);
  });

  it("never runs two backups at once", () => {
    const events = Array.from({ length: 200 }, (_, i) => i);
    const { starts } = simulate({ seconds: 300, events, backupMs: 8_000 });
    for (let i = 1; i < starts.length; i++)
      expect(starts[i] - starts[i - 1]).toBeGreaterThanOrEqual(8);
  });
});

describe("backup loop transitions", () => {
  it("an event during a backup sets the pending flag, one before it does not", () => {
    let loop = onFsEvent(newLoop(), 100);
    expect(loop.pendingDuringFlight).toBe(false);
    loop = onBackupStarted(loop, 200);
    expect(loop.pendingDuringFlight).toBe(false);
    loop = onFsEvent(loop, 250);
    expect(loop.pendingDuringFlight).toBe(true);
  });

  it("finishing restarts the quiet window only if something happened mid-flight", () => {
    const quiet = onBackupFinished(
      onBackupStarted(onFsEvent(newLoop(), 100), 200),
      { ok: true },
      900,
    );
    expect(quiet.lastEventMs).toBe(100);

    const busy = onBackupFinished(
      onFsEvent(onBackupStarted(newLoop(), 200), 250),
      { ok: true },
      900,
    );
    expect(busy.lastEventMs).toBe(900);
    expect(busy.pendingDuringFlight).toBe(false);
  });

  it("success clears the failure state, failure records it", () => {
    const failed = onBackupFinished(onBackupStarted(newLoop(), 0), { ok: false, failures: 3 }, 500);
    expect(failed.consecutiveFailures).toBe(3);
    expect(failed.lastFailureMs).toBe(500);
    expect(failed.backupInFlight).toBe(false);

    const ok = onBackupFinished(onBackupStarted(failed, 600), { ok: true }, 900);
    expect(ok.consecutiveFailures).toBe(0);
    expect(ok.lastFailureMs).toBeNull();
  });

  it("transitions do not mutate their input", () => {
    const before = newLoop(1);
    const frozen = JSON.stringify(before);
    onFsEvent(before, 1);
    onBackupStarted(before, 2);
    onBackupFinished(before, { ok: false, failures: 2 }, 3);
    expect(JSON.stringify(before)).toBe(frozen);
  });
});
