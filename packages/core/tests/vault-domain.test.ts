import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  DESTROY_PHASES,
  type VaultCommand,
  type VaultState,
  decide,
  needsPassphrase,
  planSync,
} from "../src/vault/domain.ts";

/**
 * Property tests for the vault domain. decide() consolidates the former
 * planInit / planUnlock / planLock planners over an explicit VaultState.
 *
 *   Property 3:  Vault Lifecycle Round-Trip and Idempotence (R4.1, 4.3, 4.4, 4.6)
 *   Property 4:  Wrong-Password Rejection                   (R4.2)
 *   Property 5:  Vault State Invariants on Locked Boundary  (R5.8, 8.6, 9.6, 13.5)
 *   Property 6:  Init-Vault Idempotence                     (R3.4)
 *   Property 16: Vault-Sync No-Op on Quiet Cycle            (R9.3, 9.4)
 *   Extra:       "destroyed" is terminal (self-destruct inert mode)
 *
 * The bats harness (tests/property/vault-lifecycle.bats) exercises the same
 * invariants end-to-end against a real gocryptfs mount inside the container.
 */

const arbPassphrase = fc.string({ minLength: 0, maxLength: 64 });
const arbNonEmpty = arbPassphrase.filter((p) => p.length > 0);
const arbState = fc.constantFrom<VaultState>("uninitialized", "locked", "unlocked", "destroyed");
const arbCommand: fc.Arbitrary<VaultCommand> = fc.oneof(
  fc.constant<VaultCommand>({ type: "init" }),
  arbPassphrase.map((passphrase): VaultCommand => ({ type: "unlock", passphrase })),
  fc.constant<VaultCommand>({ type: "lock" }),
  fc.constant<VaultCommand>({ type: "destroy" }),
);

describe("decide: totality", () => {
  it("returns exactly one well-formed decision for every state and command", () => {
    fc.assert(
      fc.property(arbState, arbCommand, (state, cmd) => {
        const d = decide(state, cmd);
        expect(["perform", "noop", "reject"]).toContain(d.do);
      }),
    );
  });
});

describe("Property 3: Vault Lifecycle Round-Trip and Idempotence", () => {
  it("unlock on an already-unlocked vault is always a no-op", () => {
    fc.assert(
      fc.property(arbPassphrase, (passphrase) => {
        expect(decide("unlocked", { type: "unlock", passphrase })).toEqual({
          do: "noop",
          reason: "already-unlocked",
        });
      }),
    );
  });

  it("lock performs only on an unlocked vault; otherwise it is a no-op", () => {
    expect(decide("unlocked", { type: "lock" }).do).toBe("perform");
    for (const state of ["locked", "uninitialized"] as const) {
      expect(decide(state, { type: "lock" })).toEqual({ do: "noop", reason: "already-locked" });
    }
  });

  it("round-trip: unlock mounts, lock unmounts, locking again is a no-op", () => {
    fc.assert(
      fc.property(arbNonEmpty, (passphrase) => {
        expect(decide("locked", { type: "unlock", passphrase }).do).toBe("perform");
        expect(decide("unlocked", { type: "lock" }).do).toBe("perform");
        expect(decide("locked", { type: "lock" }).do).toBe("noop");
      }),
    );
  });

  it("repeated unlocks only mount once (every later call observes unlocked)", () => {
    fc.assert(
      fc.property(arbNonEmpty, fc.integer({ min: 1, max: 5 }), (passphrase, repeats) => {
        expect(decide("locked", { type: "unlock", passphrase }).do).toBe("perform");
        for (let i = 0; i < repeats; i++) {
          expect(decide("unlocked", { type: "unlock", passphrase }).do).toBe("noop");
        }
      }),
    );
  });
});

describe("Property 4: Wrong-Password Rejection", () => {
  it("an empty passphrase never mounts", () => {
    expect(decide("locked", { type: "unlock", passphrase: "" })).toEqual({
      do: "reject",
      reason: "empty-passphrase",
    });
  });

  it("a non-empty passphrase always yields a mount attempt (gocryptfs adjudicates correctness)", () => {
    fc.assert(
      fc.property(arbNonEmpty, (passphrase) => {
        expect(decide("locked", { type: "unlock", passphrase }).do).toBe("perform");
      }),
    );
  });

  it("an uninitialized vault never mounts, whatever the passphrase", () => {
    fc.assert(
      fc.property(arbPassphrase, (passphrase) => {
        expect(decide("uninitialized", { type: "unlock", passphrase })).toEqual({
          do: "reject",
          reason: "not-initialized",
        });
      }),
    );
  });

  it("flags a short passphrase (< 8 chars) without blocking it", () => {
    fc.assert(
      fc.property(arbNonEmpty, (passphrase) => {
        const d = decide("locked", { type: "unlock", passphrase });
        expect(d.do).toBe("perform");
        if (d.do === "perform") expect(d.warnShortPassphrase).toBe(passphrase.length < 8);
      }),
    );
  });
});

describe("Property 6: Init-Vault Idempotence", () => {
  it("init performs only on an uninitialized vault", () => {
    expect(decide("uninitialized", { type: "init" }).do).toBe("perform");
    for (const state of ["locked", "unlocked"] as const) {
      expect(decide(state, { type: "init" })).toEqual({
        do: "noop",
        reason: "already-initialized",
      });
    }
  });

  it("once initialized, repeated init calls never re-init", () => {
    fc.assert(
      fc.property(fc.integer({ min: 1, max: 10 }), (repeats) => {
        for (let i = 0; i < repeats; i++) {
          expect(decide("locked", { type: "init" }).do).toBe("noop");
        }
      }),
    );
  });
});

describe("destroyed is terminal", () => {
  it("rejects every command except a (no-op) repeat of destroy", () => {
    fc.assert(
      fc.property(arbCommand, (cmd) => {
        const d = decide("destroyed", cmd);
        if (cmd.type === "destroy") {
          expect(d).toEqual({ do: "noop", reason: "already-destroyed" });
        } else {
          expect(d).toEqual({ do: "reject", reason: "destroyed" });
        }
      }),
    );
  });

  it("destroy performs from every live state", () => {
    for (const state of ["uninitialized", "locked", "unlocked"] as const) {
      expect(decide(state, { type: "destroy" }).do).toBe("perform");
    }
  });
});

describe("needsPassphrase", () => {
  it("is true only for a locked vault", () => {
    expect(needsPassphrase("locked")).toBe(true);
    for (const state of ["uninitialized", "unlocked", "destroyed"] as const) {
      expect(needsPassphrase(state)).toBe(false);
    }
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

describe("DESTROY_PHASES", () => {
  it("lists the phases in the only order they may run", () => {
    expect(DESTROY_PHASES.map((p) => p.id)).toEqual([
      "quiesce",
      "unmount",
      "shred-keys",
      "shred-data",
      "scrub-credentials",
      "mark-complete",
    ]);
  });

  it("makes shred-keys the Point of No Return: nothing after it is reversible", () => {
    const firstIrreversible = DESTROY_PHASES.findIndex((p) => p.irreversible);
    expect(DESTROY_PHASES[firstIrreversible].id).toBe("shred-keys");
    for (const phase of DESTROY_PHASES.slice(firstIrreversible)) {
      expect(phase.irreversible).toBe(true);
    }
  });

  it("finishes with the completion marker", () => {
    expect(DESTROY_PHASES.at(-1)?.id).toBe("mark-complete");
  });
});
