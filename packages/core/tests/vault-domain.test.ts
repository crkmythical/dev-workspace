import { describe, expect, it } from "bun:test";
import fc from "fast-check";
import {
  DESTROY_PHASES,
  type VaultCommand,
  type VaultState,
  decide,
  explainUnlockFailure,
  killable,
  lineage,
  needsPassphrase,
  parseFuserPids,
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

describe("explainUnlockFailure", () => {
  // The three strings below are what gocryptfs 2.x really printed inside the image, with
  // the container's AppArmor profile in force, with a wrong passphrase, and without /dev/fuse.
  const APPARMOR_DENIED =
    "/usr/bin/fusermount3: mount failed: Permission denied\nfs.Mount failed: fusermount exited with code 256";
  const WRONG_PASSPHRASE =
    "failed to unlock master key: cipher: message authentication failed\nPassword incorrect.";
  const NO_FUSE_DEVICE =
    "/usr/bin/fusermount3: fuse device not found, try 'modprobe fuse' first\nfs.Mount failed: fusermount exited with code 256";

  it("a wrong passphrase says so, and nothing about the container", () => {
    expect(explainUnlockFailure(WRONG_PASSPHRASE)).toEqual(["The passphrase is wrong."]);
  });

  it("a refused mount points at AppArmor and the override file, not at the passphrase", () => {
    const lines = explainUnlockFailure(APPARMOR_DENIED).join("\n");
    expect(lines).toContain("not allowed to mount");
    expect(lines).toContain("docker-compose.apparmor.yml");
    expect(lines).not.toContain("passphrase");
  });

  it("a missing /dev/fuse is its own diagnosis", () => {
    const lines = explainUnlockFailure(NO_FUSE_DEVICE).join("\n");
    expect(lines).toContain("/dev/fuse");
    expect(lines).not.toContain("AppArmor");
  });

  it("a wrong passphrase right after generating one explains the recreated-container trap", () => {
    // Boot with PASSWORD empty on an existing vault volume: the old generated password went
    // away with the old container (verified: restart keeps it, recreate does not).
    const lines = explainUnlockFailure(WRONG_PASSPHRASE, { passwordGenerated: true });
    expect(lines[0]).toBe("The passphrase is wrong.");
    const text = lines.join(" ");
    expect(text).toContain("generated a new password");
    expect(text).toContain("recreating");
    expect(text).toContain("unlock-vault");
  });

  it("says nothing about generation when the passphrase was not generated", () => {
    expect(explainUnlockFailure(WRONG_PASSPHRASE, { passwordGenerated: false })).toEqual([
      "The passphrase is wrong.",
    ]);
  });

  it("generation is irrelevant to a failure that is not about the passphrase", () => {
    expect(explainUnlockFailure(APPARMOR_DENIED, { passwordGenerated: true })).toEqual(
      explainUnlockFailure(APPARMOR_DENIED),
    );
    expect(explainUnlockFailure(NO_FUSE_DEVICE, { passwordGenerated: true })).toEqual(
      explainUnlockFailure(NO_FUSE_DEVICE),
    );
  });

  it("anything it does not recognize gets no guess", () => {
    expect(explainUnlockFailure("")).toEqual([]);
    expect(explainUnlockFailure("something unexpected")).toEqual([]);
  });

  it("ignores letter case", () => {
    expect(explainUnlockFailure("PASSWORD INCORRECT.")).toEqual(["The passphrase is wrong."]);
  });
});

describe("parseFuserPids", () => {
  it("reads what psmisc 23.7 really printed on stdout (leading spaces, no names)", () => {
    expect(parseFuserPids(" 555 561")).toEqual([555, 561]);
    expect(parseFuserPids(" 555")).toEqual([555]);
  });

  it("an empty listing is no holders", () => {
    expect(parseFuserPids("")).toEqual([]);
    expect(parseFuserPids("\n")).toEqual([]);
  });

  it("is not thrown by access letters or a header, should they ever reach stdout", () => {
    expect(parseFuserPids("/workspace:  555c 561f")).toEqual([555, 561]);
  });

  it("never yields PID 0, which `kill` would aim at the whole process group", () => {
    expect(parseFuserPids("0 12 000")).toEqual([12]);
  });
});

describe("lineage", () => {
  const parents = new Map([
    [900, 800], // the process releasing the mount
    [800, 700], // its shell
    [700, 90], // the terminal's pty host
    [90, 1],
    [1, 0],
  ]);
  const parentOf = (pid: number) => parents.get(pid);

  it("is the process and every ancestor up to PID 1", () => {
    expect(lineage(900, parentOf)).toEqual([900, 800, 700, 90, 1]);
  });

  it("stops where a parent cannot be read", () => {
    expect(lineage(900, (pid) => (pid === 800 ? undefined : parentOf(pid)))).toEqual([900, 800]);
  });

  it("an unknown process is just itself", () => {
    expect(lineage(4242, () => undefined)).toEqual([4242]);
  });

  it("terminates on a parent cycle", () => {
    expect(lineage(1, (pid) => (pid === 1 ? 2 : 1))).toEqual([1, 2]);
  });
});

describe("killable", () => {
  it("never includes the caller or anything above it", () => {
    // `cd /workspace && self-destruct` in a terminal: the caller, its shell and the terminal
    // all use the mount, and killing any of them takes the caller down before the key is erased.
    const holders = [900, 800, 700, 123, 456];
    expect(killable(holders, [900, 800, 700, 90, 1])).toEqual([123, 456]);
  });

  it("everything else that uses the mount is fair game, once", () => {
    expect(killable([5, 6, 5], [1])).toEqual([5, 6]);
  });

  it("no holders, nothing to kill", () => {
    expect(killable([], [1])).toEqual([]);
  });

  it("property: the result never intersects the protected set and is always a subset of the holders", () => {
    fc.assert(
      fc.property(
        fc.array(fc.integer({ min: 1, max: 50 })),
        fc.array(fc.integer({ min: 1, max: 50 })),
        (holders, protect) => {
          const victims = killable(holders, protect);
          for (const pid of victims) {
            expect(protect).not.toContain(pid);
            expect(holders).toContain(pid);
          }
          expect(new Set(victims).size).toBe(victims.length);
        },
      ),
    );
  });
});
