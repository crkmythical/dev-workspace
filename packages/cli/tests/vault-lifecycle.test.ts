import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  REQUIRED_ENV_VARS,
  envValidationError,
  findMissingRequiredEnv,
} from "../src/lib/env-validate.ts";
import { planInit, planLock, planSync, planUnlock } from "../src/lib/vault-lifecycle.ts";

/**
 * Property tests for vault lifecycle (task 3.5).
 *
 *   Property 3:  Vault Lifecycle Round-Trip and Idempotence (R4.1, 4.3, 4.4, 4.6)
 *   Property 4:  Wrong-Password Rejection                   (R4.2)
 *   Property 6:  Init-Vault Idempotence                     (R3.4)
 *   Property 7:  Stale-Mount Cleanup Idempotence            (R8.1)
 *   Property 18: Env Validation Completeness                (R12.4)
 *
 * The lifecycle decision logic is extracted into pure planners so the
 * invariants are verifiable without a real gocryptfs/FUSE mount. The bats
 * harness in vault-lifecycle.bats exercises the same invariants end-to-end
 * inside the container where gocryptfs is available.
 */

const arbPassphrase = fc.string({ minLength: 0, maxLength: 64 });

describe("Property 3: Vault Lifecycle Round-Trip and Idempotence", () => {
  it("unlock on an already-mounted vault is always a no-op (idempotent)", () => {
    fc.assert(
      fc.property(fc.boolean(), arbPassphrase, (initialized, pass) => {
        const plan = planUnlock({ mounted: true, initialized, passphrase: pass });
        expect(plan.action).toBe("noop");
        if (plan.action === "noop") {
          expect(plan.exitCode).toBe(0);
          expect(plan.reason).toBe("already-mounted");
        }
      }),
    );
  });

  it("lock on a mounted vault unmounts; lock on an unmounted vault is a no-op", () => {
    fc.assert(
      fc.property(fc.boolean(), (mounted) => {
        const plan = planLock(mounted);
        if (mounted) {
          expect(plan.action).toBe("unmount");
        } else {
          expect(plan.action).toBe("noop");
          if (plan.action === "noop") expect(plan.exitCode).toBe(0);
        }
      }),
    );
  });

  it("round-trip: unlock(initialized, good pass) → mount, then lock → unmount → no-op", () => {
    fc.assert(
      fc.property(
        arbPassphrase.filter((p) => p.length > 0),
        (pass) => {
          // Start locked + initialized → unlock should mount.
          const unlock = planUnlock({ mounted: false, initialized: true, passphrase: pass });
          expect(unlock.action).toBe("mount");
          // After mount, lock returns to unmounted.
          expect(planLock(true).action).toBe("unmount");
          // After unmount, locking again is a no-op (original state restored).
          expect(planLock(false).action).toBe("noop");
        },
      ),
    );
  });

  it("repeated unlock invocations only mount once (subsequent are no-ops)", () => {
    fc.assert(
      fc.property(
        arbPassphrase.filter((p) => p.length > 0),
        fc.integer({ min: 1, max: 5 }),
        (pass, repeats) => {
          // First call against a locked vault → mount.
          expect(planUnlock({ mounted: false, initialized: true, passphrase: pass }).action).toBe(
            "mount",
          );
          // Every subsequent call observes mounted=true → no-op, regardless of count.
          for (let i = 0; i < repeats; i++) {
            expect(planUnlock({ mounted: true, initialized: true, passphrase: pass }).action).toBe(
              "noop",
            );
          }
        },
      ),
    );
  });
});

describe("Property 4: Wrong-Password Rejection", () => {
  it("empty passphrase never mounts and exits non-zero", () => {
    const plan = planUnlock({ mounted: false, initialized: true, passphrase: "" });
    expect(plan.action).toBe("error");
    if (plan.action === "error") {
      expect(plan.reason).toBe("empty-passphrase");
      expect(plan.exitCode).not.toBe(0);
    }
  });

  it("a non-empty passphrase yields a mount attempt (gocryptfs adjudicates correctness)", () => {
    // The planner cannot verify the passphrase itself; it can only ensure a
    // mount attempt happens. Correctness is enforced by gocryptfs at mount
    // time (exit non-zero on wrong key, see unlock-vault.ts step 4). This
    // property guarantees the planner never short-circuits a real attempt.
    fc.assert(
      fc.property(
        arbPassphrase.filter((p) => p.length > 0),
        (pass) => {
          const plan = planUnlock({ mounted: false, initialized: true, passphrase: pass });
          expect(plan.action).toBe("mount");
        },
      ),
    );
  });

  it("uninitialized vault never mounts regardless of passphrase", () => {
    fc.assert(
      fc.property(arbPassphrase, (pass) => {
        const plan = planUnlock({ mounted: false, initialized: false, passphrase: pass });
        expect(plan.action).toBe("error");
        if (plan.action === "error") {
          expect(plan.reason).toBe("not-initialized");
          expect(plan.exitCode).not.toBe(0);
        }
      }),
    );
  });
});

describe("Property 6: Init-Vault Idempotence", () => {
  it("init on an existing vault is always a no-op; only an uninitialized vault initializes", () => {
    fc.assert(
      fc.property(fc.boolean(), (initialized) => {
        const plan = planInit(initialized);
        if (initialized) {
          expect(plan.action).toBe("noop");
          if (plan.action === "noop") {
            expect(plan.exitCode).toBe(0);
            expect(plan.reason).toBe("already-initialized");
          }
        } else {
          expect(plan.action).toBe("init");
        }
      }),
    );
  });

  it("init is idempotent: once initialized, repeated calls never re-init", () => {
    fc.assert(
      fc.property(fc.integer({ min: 1, max: 10 }), (repeats) => {
        for (let i = 0; i < repeats; i++) {
          expect(planInit(true).action).toBe("noop");
        }
      }),
    );
  });
});

describe("Property 5/16: Sync gating (locked boundary + quiet cycle)", () => {
  it("vault-sync never commits when not mounted (locked boundary)", () => {
    fc.assert(
      fc.property(fc.boolean(), (hasStagedChanges) => {
        const plan = planSync({ mounted: false, hasStagedChanges });
        expect(plan.action).toBe("skip");
        if (plan.action === "skip") {
          expect(plan.reason).toBe("not-mounted");
          expect(plan.exitCode).toBe(0);
        }
      }),
    );
  });

  it("vault-sync is a no-op on a quiet cycle (mounted, no staged changes)", () => {
    const plan = planSync({ mounted: true, hasStagedChanges: false });
    expect(plan.action).toBe("skip");
    if (plan.action === "skip") expect(plan.reason).toBe("no-changes");
  });

  it("vault-sync commits only when mounted AND changes exist", () => {
    expect(planSync({ mounted: true, hasStagedChanges: true }).action).toBe("commit-and-push");
  });
});

describe("Property 18: Env Validation Completeness", () => {
  const allRequired = [...REQUIRED_ENV_VARS];

  it("omitting any subset of required vars names at least one missing var", () => {
    fc.assert(
      fc.property(
        // a random subset of required vars to *include* (the rest are omitted)
        fc.subarray(allRequired),
        fc.dictionary(fc.string(), fc.string()),
        (present, extras) => {
          const env: Record<string, string | undefined> = { ...extras };
          for (const name of present) env[name] = "some-value";

          const missing = findMissingRequiredEnv(env, allRequired);
          const error = envValidationError(missing);

          const omittedCount = allRequired.length - present.length;
          if (omittedCount > 0) {
            expect(missing.length).toBe(omittedCount);
            expect(error).not.toBeNull();
            // The message must explicitly name at least one missing variable.
            expect(missing.some((v) => error?.includes(v))).toBe(true);
          } else {
            expect(missing).toEqual([]);
            expect(error).toBeNull();
          }
        },
      ),
    );
  });

  it("a present-but-empty var counts as missing", () => {
    fc.assert(
      fc.property(fc.constantFrom(...allRequired), (name) => {
        const env: Record<string, string | undefined> = {};
        for (const v of allRequired) env[v] = "x";
        env[name] = ""; // present but empty
        const missing = findMissingRequiredEnv(env, allRequired);
        expect(missing).toContain(name);
      }),
    );
  });

  it("a complete required set passes validation", () => {
    const env: Record<string, string | undefined> = {};
    for (const v of allRequired) env[v] = "value";
    expect(findMissingRequiredEnv(env, allRequired)).toEqual([]);
    expect(envValidationError([])).toBeNull();
  });
});
