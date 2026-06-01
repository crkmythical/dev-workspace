import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  BACKUP_BASE_BACKOFF_MS,
  BACKUP_DEBOUNCE_MS,
  BACKUP_EXCLUSION_SET,
  BACKUP_FAILURE_THRESHOLD,
  BACKUP_FALLBACK_INTERVAL_MS,
  BACKUP_PRUNE_CADENCE_MS,
  BACKUP_STALENESS_MS,
  type DebounceState,
  type DoctorBackupState,
  type GateResult,
  type PruneState,
  backoffDelayMs,
  evaluateBackupGate,
  isExcludedPath,
  planDebounce,
  planDoctorBackupStatus,
  planPrune,
  shouldNotify,
} from "../src/backup.ts";

// ─── Arbitraries ─────────────────────────────────────────────────────────────

const arbTimestamp = fc.integer({ min: 1_000_000_000_000, max: 2_000_000_000_000 });
const arbNullableTimestamp = fc.oneof(fc.constant(null), arbTimestamp);
const arbFailures = fc.integer({ min: 0, max: 20 });

const arbDebounceState: fc.Arbitrary<DebounceState> = fc.record({
  lastEventMs: arbNullableTimestamp,
  nowMs: arbTimestamp,
  backupInFlight: fc.boolean(),
  lastTriggerMs: arbNullableTimestamp,
  consecutiveFailures: arbFailures,
  lastFailureMs: arbNullableTimestamp,
});

const arbPruneState: fc.Arbitrary<PruneState> = fc.record({
  lastPruneMs: arbNullableTimestamp,
  nowMs: arbTimestamp,
});

// ─── Property Tests ──────────────────────────────────────────────────────────

describe("Feature: realtime-backup-s3, Property 1: Debounce trigger correctness", () => {
  it("triggers iff conditions met (not in-flight, not in backoff)", () => {
    fc.assert(
      fc.property(arbDebounceState.filter((s) => !s.backupInFlight), (state) => {
        const decision = planDebounce(state);
        const inBackoff =
          state.consecutiveFailures > 0 &&
          state.lastFailureMs !== null &&
          state.nowMs - state.lastFailureMs < backoffDelayMs(state.consecutiveFailures);

        if (state.lastTriggerMs === null) {
          expect(decision).toBe("trigger");
        } else if (inBackoff) {
          expect(decision).toBe("wait");
        } else if (state.nowMs - state.lastTriggerMs >= BACKUP_FALLBACK_INTERVAL_MS) {
          expect(decision).toBe("trigger");
        } else if (state.lastEventMs === null) {
          expect(decision).toBe("wait");
        } else if (state.nowMs - state.lastEventMs >= BACKUP_DEBOUNCE_MS) {
          expect(decision).toBe("trigger");
        } else {
          expect(decision).toBe("wait");
        }
      }),
      { numRuns: 200 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 2: In-flight coalesce invariant", () => {
  it("always returns coalesce when backup is in flight", () => {
    fc.assert(
      fc.property(arbDebounceState.filter((s) => s.backupInFlight), (state) => {
        expect(planDebounce(state)).toBe("coalesce");
      }),
      { numRuns: 200 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 3: Exclusion path filtering", () => {
  it("excludes paths with any segment matching the exclusion set", () => {
    const arbExcludedSegment = fc.constantFrom(...BACKUP_EXCLUSION_SET);
    const arbSafeSegment = fc.string({ minLength: 1, maxLength: 20 }).filter(
      (s) => !BACKUP_EXCLUSION_SET.includes(s) && !s.includes("/"),
    );

    fc.assert(
      fc.property(
        fc.array(arbSafeSegment, { minLength: 0, maxLength: 3 }),
        arbExcludedSegment,
        fc.array(arbSafeSegment, { minLength: 0, maxLength: 3 }),
        (prefix, excluded, suffix) => {
          const path = [...prefix, excluded, ...suffix].join("/");
          expect(isExcludedPath(path)).toBe(true);
        },
      ),
      { numRuns: 200 },
    );
  });

  it("does not exclude paths with no matching segment", () => {
    const arbSafeSegment = fc.string({ minLength: 1, maxLength: 20 }).filter(
      (s) => !BACKUP_EXCLUSION_SET.includes(s) && !s.includes("/"),
    );

    fc.assert(
      fc.property(fc.array(arbSafeSegment, { minLength: 1, maxLength: 5 }), (segments) => {
        const path = segments.join("/");
        expect(isExcludedPath(path)).toBe(false);
      }),
      { numRuns: 200 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 4: Retention plan schedule", () => {
  it("prune-due iff never pruned or cadence elapsed", () => {
    fc.assert(
      fc.property(arbPruneState, (state) => {
        const decision = planPrune(state);
        if (state.lastPruneMs === null) {
          expect(decision).toBe("prune-due");
        } else if (state.nowMs - state.lastPruneMs >= BACKUP_PRUNE_CADENCE_MS) {
          expect(decision).toBe("prune-due");
        } else {
          expect(decision).toBe("skip");
        }
      }),
      { numRuns: 200 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 5: Exponential backoff computation", () => {
  it("returns BASE * 2^min(n, 4)", () => {
    fc.assert(
      fc.property(fc.integer({ min: 0, max: 20 }), (n) => {
        const expected = BACKUP_BASE_BACKOFF_MS * Math.pow(2, Math.min(n, 4));
        expect(backoffDelayMs(n)).toBe(expected);
      }),
      { numRuns: 100 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 6: Failure notification threshold", () => {
  it("shouldNotify iff failures >= threshold", () => {
    fc.assert(
      fc.property(fc.integer({ min: 0, max: 20 }), (n) => {
        expect(shouldNotify(n)).toBe(n >= BACKUP_FAILURE_THRESHOLD);
      }),
      { numRuns: 100 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 7: Opt-in gate evaluation", () => {
  it("dormant when RESTIC_REPOSITORY absent/empty", () => {
    fc.assert(
      fc.property(
        fc.oneof(fc.constant(undefined), fc.constant("")),
        fc.string(),
        fc.string(),
        fc.string(),
        (repo, pw, key, secret) => {
          const result = evaluateBackupGate({
            RESTIC_REPOSITORY: repo,
            RESTIC_PASSWORD: pw,
            AWS_ACCESS_KEY_ID: key,
            AWS_SECRET_ACCESS_KEY: secret,
          });
          expect(result.status).toBe("dormant");
        },
      ),
      { numRuns: 50 },
    );
  });

  it("enabled when all four present and non-empty", () => {
    fc.assert(
      fc.property(
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        fc.string({ minLength: 1 }),
        (repo, pw, key, secret) => {
          const result = evaluateBackupGate({
            RESTIC_REPOSITORY: repo,
            RESTIC_PASSWORD: pw,
            AWS_ACCESS_KEY_ID: key,
            AWS_SECRET_ACCESS_KEY: secret,
          });
          expect(result.status).toBe("enabled");
        },
      ),
      { numRuns: 50 },
    );
  });

  it("misconfigured when repo present but credentials missing", () => {
    const result = evaluateBackupGate({ RESTIC_REPOSITORY: "s3:test" });
    expect(result.status).toBe("misconfigured");
    if (result.status === "misconfigured") {
      expect(result.missing).toContain("RESTIC_PASSWORD");
      expect(result.missing).toContain("AWS_ACCESS_KEY_ID");
      expect(result.missing).toContain("AWS_SECRET_ACCESS_KEY");
    }
  });
});

describe("Feature: realtime-backup-s3, Property 8: Doctor backup status", () => {
  it("returns correct status for all state combinations", () => {
    const arbGateResult: fc.Arbitrary<GateResult> = fc.oneof(
      fc.constant({ status: "dormant" as const }),
      fc.constant({ status: "enabled" as const }),
      fc.constant({ status: "misconfigured" as const, missing: ["RESTIC_PASSWORD"] }),
    );

    const arbDoctorState: fc.Arbitrary<DoctorBackupState> = fc.record({
      gateResult: arbGateResult,
      lastSuccessMs: arbNullableTimestamp,
      consecutiveFailures: arbFailures,
      nowMs: arbTimestamp,
    });

    fc.assert(
      fc.property(arbDoctorState, (state) => {
        const status = planDoctorBackupStatus(state);
        if (state.gateResult.status === "dormant") {
          expect(status).toBe("not-configured");
        } else if (state.gateResult.status === "misconfigured") {
          expect(status).toBe("fail");
        } else if (state.consecutiveFailures >= BACKUP_FAILURE_THRESHOLD) {
          expect(status).toBe("fail");
        } else if (state.lastSuccessMs === null) {
          expect(status).toBe("warn");
        } else if (state.nowMs - state.lastSuccessMs >= BACKUP_STALENESS_MS) {
          expect(status).toBe("warn");
        } else {
          expect(status).toBe("ok");
        }
      }),
      { numRuns: 200 },
    );
  });
});

describe("Feature: realtime-backup-s3, Property 9: Backoff is monotonically non-decreasing", () => {
  it("backoff(a) <= backoff(b) for a < b (both <= 4)", () => {
    fc.assert(
      fc.property(
        fc.integer({ min: 0, max: 3 }),
        fc.integer({ min: 1, max: 4 }),
        (a, bOffset) => {
          const b = a + bOffset;
          if (b <= 4) {
            expect(backoffDelayMs(a)).toBeLessThanOrEqual(backoffDelayMs(b));
          }
        },
      ),
      { numRuns: 100 },
    );
  });
});
