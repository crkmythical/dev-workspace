import { describe, expect, it } from "bun:test";
import {
  BACKUP_BASE_BACKOFF_MS,
  BACKUP_DEBOUNCE_MS,
  BACKUP_EXCLUSION_SET,
  BACKUP_FAILURE_THRESHOLD,
  BACKUP_FALLBACK_INTERVAL_MS,
  BACKUP_PRUNE_CADENCE_MS,
  BACKUP_STALENESS_MS,
  BACKUP_STATE_DIR,
  backoffDelayMs,
  evaluateBackupGate,
  isExcludedPath,
  planDebounce,
  planDoctorBackupStatus,
  planPrune,
  shouldNotify,
} from "../src/backup.ts";

const NOW = 1_700_000_000_000;

describe("planDebounce", () => {
  const base = {
    nowMs: NOW,
    backupInFlight: false,
    lastTriggerMs: NOW - 60_000,
    consecutiveFailures: 0,
    lastFailureMs: null,
  };

  it("triggers after 10s quiet", () => {
    expect(planDebounce({ ...base, lastEventMs: NOW - BACKUP_DEBOUNCE_MS })).toBe("trigger");
  });

  it("waits during quiet (< 10s)", () => {
    expect(planDebounce({ ...base, lastEventMs: NOW - 5_000 })).toBe("wait");
  });

  it("coalesces when backup in flight", () => {
    expect(planDebounce({ ...base, backupInFlight: true, lastEventMs: NOW - 30_000 })).toBe("coalesce");
  });

  it("triggers on fallback after 5 minutes", () => {
    expect(planDebounce({ ...base, lastEventMs: null, lastTriggerMs: NOW - BACKUP_FALLBACK_INTERVAL_MS })).toBe("trigger");
  });

  it("triggers immediately on first run (lastTriggerMs === null)", () => {
    expect(planDebounce({ ...base, lastTriggerMs: null, lastEventMs: null })).toBe("trigger");
  });

  it("waits when within backoff window after failure", () => {
    const state = {
      ...base,
      lastEventMs: NOW - 30_000, // quiet enough to normally trigger
      consecutiveFailures: 2,
      lastFailureMs: NOW - 3_000, // 3s ago, backoff for 2 failures = 20s
    };
    expect(planDebounce(state)).toBe("wait");
  });

  it("triggers after backoff window elapses", () => {
    const state = {
      ...base,
      lastEventMs: NOW - 30_000,
      consecutiveFailures: 1,
      lastFailureMs: NOW - 15_000, // 15s ago, backoff for 1 failure = 10s → elapsed
    };
    expect(planDebounce(state)).toBe("trigger");
  });
});

describe("planPrune", () => {
  it("prune-due when never pruned", () => {
    expect(planPrune({ lastPruneMs: null, nowMs: NOW })).toBe("prune-due");
  });

  it("prune-due after cadence elapsed", () => {
    expect(planPrune({ lastPruneMs: NOW - BACKUP_PRUNE_CADENCE_MS, nowMs: NOW })).toBe("prune-due");
  });

  it("skip before cadence", () => {
    expect(planPrune({ lastPruneMs: NOW - 1000, nowMs: NOW })).toBe("skip");
  });
});

describe("backoffDelayMs", () => {
  it("returns correct exponential values", () => {
    expect(backoffDelayMs(0)).toBe(5_000);
    expect(backoffDelayMs(1)).toBe(10_000);
    expect(backoffDelayMs(2)).toBe(20_000);
    expect(backoffDelayMs(3)).toBe(40_000);
    expect(backoffDelayMs(4)).toBe(80_000);
  });

  it("caps at failure count 4+", () => {
    expect(backoffDelayMs(5)).toBe(80_000);
    expect(backoffDelayMs(10)).toBe(80_000);
  });
});

describe("evaluateBackupGate", () => {
  it("dormant when RESTIC_REPOSITORY absent", () => {
    expect(evaluateBackupGate({})).toEqual({ status: "dormant" });
  });

  it("dormant when RESTIC_REPOSITORY empty", () => {
    expect(evaluateBackupGate({ RESTIC_REPOSITORY: "" })).toEqual({ status: "dormant" });
  });

  it("enabled when all present", () => {
    expect(evaluateBackupGate({
      RESTIC_REPOSITORY: "s3:test",
      RESTIC_PASSWORD: "pw",
      AWS_ACCESS_KEY_ID: "key",
      AWS_SECRET_ACCESS_KEY: "secret",
    })).toEqual({ status: "enabled" });
  });

  it("misconfigured when repo present but password missing", () => {
    const result = evaluateBackupGate({
      RESTIC_REPOSITORY: "s3:test",
      AWS_ACCESS_KEY_ID: "key",
      AWS_SECRET_ACCESS_KEY: "secret",
    });
    expect(result.status).toBe("misconfigured");
    if (result.status === "misconfigured") {
      expect(result.missing).toEqual(["RESTIC_PASSWORD"]);
    }
  });
});

describe("planDoctorBackupStatus", () => {
  it("not-configured when dormant", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "dormant" },
      lastSuccessMs: null,
      consecutiveFailures: 0,
      nowMs: NOW,
    })).toBe("not-configured");
  });

  it("fail when misconfigured", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "misconfigured", missing: ["RESTIC_PASSWORD"] },
      lastSuccessMs: null,
      consecutiveFailures: 0,
      nowMs: NOW,
    })).toBe("fail");
  });

  it("fail when failures >= threshold", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "enabled" },
      lastSuccessMs: NOW - 1000,
      consecutiveFailures: BACKUP_FAILURE_THRESHOLD,
      nowMs: NOW,
    })).toBe("fail");
  });

  it("warn when no success recorded", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "enabled" },
      lastSuccessMs: null,
      consecutiveFailures: 0,
      nowMs: NOW,
    })).toBe("warn");
  });

  it("warn when success is stale", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "enabled" },
      lastSuccessMs: NOW - BACKUP_STALENESS_MS - 1,
      consecutiveFailures: 0,
      nowMs: NOW,
    })).toBe("warn");
  });

  it("ok when recent success and no failures", () => {
    expect(planDoctorBackupStatus({
      gateResult: { status: "enabled" },
      lastSuccessMs: NOW - 5000,
      consecutiveFailures: 0,
      nowMs: NOW,
    })).toBe("ok");
  });
});

describe("isExcludedPath", () => {
  it("matches first segment", () => {
    expect(isExcludedPath("node_modules/pkg/index.js")).toBe(true);
    expect(isExcludedPath(".git/objects/abc")).toBe(true);
  });

  it("matches nested segment", () => {
    expect(isExcludedPath("project/node_modules/pkg/index.js")).toBe(true);
    expect(isExcludedPath("deep/path/.cache/file")).toBe(true);
  });

  it("rejects non-matching paths", () => {
    expect(isExcludedPath("src/main.ts")).toBe(false);
    expect(isExcludedPath("docs/readme.md")).toBe(false);
  });

  it("handles single segment", () => {
    expect(isExcludedPath("node_modules")).toBe(true);
    expect(isExcludedPath("src")).toBe(false);
  });

  it("handles empty string", () => {
    expect(isExcludedPath("")).toBe(false);
  });
});

describe("shouldNotify", () => {
  it("false below threshold", () => {
    expect(shouldNotify(0)).toBe(false);
    expect(shouldNotify(2)).toBe(false);
  });

  it("true at threshold", () => {
    expect(shouldNotify(3)).toBe(true);
  });

  it("true above threshold", () => {
    expect(shouldNotify(10)).toBe(true);
  });
});

describe("backup constants", () => {
  it("BACKUP_STATE_DIR is outside /workspace", () => {
    expect(BACKUP_STATE_DIR.startsWith("/workspace")).toBe(false);
  });

  it("BACKUP_EXCLUSION_SET includes .notifications", () => {
    expect(BACKUP_EXCLUSION_SET).toContain(".notifications");
  });

  it("debounce < fallback", () => {
    expect(BACKUP_DEBOUNCE_MS).toBeLessThan(BACKUP_FALLBACK_INTERVAL_MS);
  });
});
