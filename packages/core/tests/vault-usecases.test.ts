import { describe, expect, it } from "bun:test";
import {
  DESTROY_PHASES,
  type DestroyDeps,
  type UnlockHook,
  cancelLock,
  destroyWorkspace,
  lockVault,
  unlockVault,
} from "../src/vault/index.ts";
import {
  SECOND,
  SPECS,
  WORKSPACE,
  fakeEffects,
  fakeHolders,
  fakeShredder,
  fakeVolume,
} from "./support/fakes.ts";

/**
 * Use-case tests against fake ports: orchestration order and failure handling
 * are verified here, without a container, gocryptfs or root.
 *
 * Covers invariants that previously had no automated test:
 *   - holders are stopped BEFORE any unmount (FUSE EBUSY safety)
 *   - destroy phases run in order, survive a failing phase, and report honestly
 *   - one unlock path runs its hooks, in order, isolated from each other
 */

function hook(name: string, calls: string[], opts: { fail?: boolean } = {}): UnlockHook {
  return {
    name,
    async run() {
      calls.push(`hook:${name}`);
      if (opts.fail) throw new Error(`${name} exploded`);
    },
  };
}

describe("lockVault", () => {
  it("stops every holder BEFORE unmounting", async () => {
    const calls: string[] = [];
    const result = await lockVault(WORKSPACE, {
      volume: fakeVolume("unlocked", calls),
      holders: fakeHolders(calls),
    });
    expect(result).toEqual({ status: "locked" });
    expect(calls).toEqual(["holders.stopAll", "unmount:workspace:graceful"]);
  });

  it("is a no-op on a locked vault: nothing is stopped or unmounted", async () => {
    const calls: string[] = [];
    const result = await lockVault(WORKSPACE, {
      volume: fakeVolume("locked", calls),
      holders: fakeHolders(calls),
    });
    expect(result).toEqual({ status: "noop", reason: "already-locked" });
    expect(calls).toEqual([]);
  });

  it("is a no-op on an uninitialized vault", async () => {
    const calls: string[] = [];
    const result = await lockVault(WORKSPACE, {
      volume: fakeVolume("uninitialized", calls),
      holders: fakeHolders(calls),
    });
    expect(result.status).toBe("noop");
    expect(calls).toEqual([]);
  });

  it("is rejected on a destroyed vault and touches nothing", async () => {
    const calls: string[] = [];
    const result = await lockVault(WORKSPACE, {
      volume: fakeVolume("destroyed", calls),
      holders: fakeHolders(calls),
    });
    expect(result).toEqual({ status: "rejected", reason: "destroyed" });
    expect(calls).toEqual([]);
  });

  it("reports busy when a graceful unmount is refused", async () => {
    const calls: string[] = [];
    const result = await lockVault(WORKSPACE, {
      volume: fakeVolume("unlocked", calls, { unmount: { graceful: false } }),
      holders: fakeHolders(calls),
    });
    expect(result).toEqual({ status: "busy" });
  });

  it("a forced retry stops holders again and detaches lazily", async () => {
    const calls: string[] = [];
    const deps = {
      volume: fakeVolume("unlocked", calls, { unmount: { graceful: false } }),
      holders: fakeHolders(calls),
    };
    await lockVault(WORKSPACE, deps);
    calls.length = 0;
    const forced = await lockVault(WORKSPACE, deps, { mode: "lazy" });
    expect(forced).toEqual({ status: "locked" });
    expect(calls).toEqual(["holders.stopAll", "unmount:workspace:lazy"]);
  });

  it("a lock the user declines to force puts back what it stopped", async () => {
    const calls: string[] = [];
    const deps = {
      volume: fakeVolume("unlocked", calls, { unmount: { graceful: false } }),
      holders: fakeHolders(calls),
    };
    expect(await lockVault(WORKSPACE, deps)).toEqual({ status: "busy" });
    expect(await cancelLock(deps)).toEqual(["backup watcher"]);
    // Stopped first, unmount refused, then started again: in that order, and nothing else.
    expect(calls).toEqual(["holders.stopAll", "unmount:workspace:graceful", "holders.restore"]);
  });

  it("has nothing to put back for holders that cannot be restored", async () => {
    expect(await cancelLock({ holders: { stopAll: async () => {} } })).toEqual([]);
  });

  it("lets the volume tell the user what it had to do to release the mount", async () => {
    const logs: string[] = [];
    await lockVault(
      SECOND,
      {
        volume: fakeVolume("unlocked", [], { unmountReport: "killed 2 process(es)" }),
        holders: { stopAll: async () => {} },
        log: (m) => logs.push(m),
      },
      { mode: "escalate" },
    );
    expect(logs).toEqual(["killed 2 process(es)"]);
  });

  it("works for any further vault too (a second VaultSpec, same use case)", async () => {
    const calls: string[] = [];
    const noHolders = { stopAll: async () => {} };
    const result = await lockVault(
      SECOND,
      { volume: fakeVolume("unlocked", calls), holders: noHolders },
      { mode: "escalate" },
    );
    expect(result).toEqual({ status: "locked" });
    expect(calls).toEqual(["unmount:second:escalate"]);
  });
});

describe("unlockVault", () => {
  const pass = async () => "correct horse battery";

  it("mounts, then runs the hooks in order", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("locked", calls),
      hooks: [hook("a", calls), hook("b", calls)],
    });
    expect(result).toEqual({ status: "unlocked", warnShortPassphrase: false, hookFailures: [] });
    expect(calls).toEqual(["mount:workspace", "hook:a", "hook:b"]);
  });

  it("hands the spec and passphrase to every hook", async () => {
    const seen: Array<{ id: string; passphrase: string }> = [];
    await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("locked", []),
      hooks: [
        {
          name: "spy",
          async run(ctx) {
            seen.push({ id: ctx.spec.id, passphrase: ctx.passphrase });
          },
        },
      ],
    });
    expect(seen).toEqual([{ id: "workspace", passphrase: "correct horse battery" }]);
  });

  it("an already-unlocked vault is a no-op: no mount, no hooks", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("unlocked", calls),
      hooks: [hook("a", calls)],
    });
    expect(result).toEqual({ status: "noop", reason: "already-unlocked" });
    expect(calls).toEqual([]);
  });

  it("asks for the passphrase only when a mount will actually be attempted", async () => {
    let reads = 0;
    const read = async () => {
      reads++;
      return "correct horse battery";
    };
    await unlockVault(WORKSPACE, read, { volume: fakeVolume("unlocked", []), hooks: [] });
    await unlockVault(WORKSPACE, read, { volume: fakeVolume("uninitialized", []), hooks: [] });
    await unlockVault(WORKSPACE, read, { volume: fakeVolume("destroyed", []), hooks: [] });
    expect(reads).toBe(0);
    await unlockVault(WORKSPACE, read, { volume: fakeVolume("locked", []), hooks: [] });
    expect(reads).toBe(1);
  });

  it("rejects an uninitialized vault", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("uninitialized", calls),
      hooks: [hook("a", calls)],
    });
    expect(result).toEqual({ status: "rejected", reason: "not-initialized" });
    expect(calls).toEqual([]);
  });

  it("rejects an empty passphrase without mounting", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, async () => "", {
      volume: fakeVolume("locked", calls),
      hooks: [hook("a", calls)],
    });
    expect(result).toEqual({ status: "rejected", reason: "empty-passphrase" });
    expect(calls).toEqual([]);
  });

  it("rejects a destroyed vault", async () => {
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("destroyed", []),
      hooks: [],
    });
    expect(result).toEqual({ status: "rejected", reason: "destroyed" });
  });

  it("reports a failed mount and runs no hooks", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("locked", calls, { mountOk: false, mountDetail: "wrong password" }),
      hooks: [hook("a", calls)],
    });
    expect(result).toEqual({ status: "failed", detail: "wrong password" });
    expect(calls).toEqual(["mount:workspace"]);
  });

  it("isolates hook failures: later hooks still run and the unlock stands", async () => {
    const calls: string[] = [];
    const result = await unlockVault(WORKSPACE, pass, {
      volume: fakeVolume("locked", calls),
      hooks: [hook("a", calls, { fail: true }), hook("b", calls)],
    });
    expect(calls).toEqual(["mount:workspace", "hook:a", "hook:b"]);
    expect(result).toEqual({
      status: "unlocked",
      warnShortPassphrase: false,
      hookFailures: ["a: a exploded"],
    });
  });

  it("flags a short passphrase", async () => {
    const result = await unlockVault(WORKSPACE, async () => "short", {
      volume: fakeVolume("locked", []),
      hooks: [],
    });
    expect(result).toMatchObject({ status: "unlocked", warnShortPassphrase: true });
  });
});

describe("destroyWorkspace", () => {
  const NOW = "2026-01-01T00:00:00.000Z";

  function makeDeps(calls: string[], logs: string[], over: Partial<DestroyDeps> = {}): DestroyDeps {
    return {
      specs: SPECS,
      volume: fakeVolume("unlocked", calls),
      holders: fakeHolders(calls),
      shredder: fakeShredder(calls),
      effects: fakeEffects(calls),
      now: () => NOW,
      log: (m) => logs.push(m),
      ...over,
    };
  }
  const plain = { skipShred: false, remote: false };

  it("runs every side effect in the one valid order", async () => {
    const calls: string[] = [];
    const report = await destroyWorkspace(makeDeps(calls, []), plain);
    expect(calls).toEqual([
      "effects.quiesce",
      "holders.stopAll",
      "unmount:workspace:escalate",
      "unmount:second:escalate",
      "shred:/c/ws/gocryptfs.conf:3",
      "shred:/c/2nd/gocryptfs.conf:3",
      "effects.shredTree:/c/ws",
      "effects.shredTree:/c/2nd",
      "effects.scrubCredentials",
      "effects.clearHistory",
      `effects.marker:${NOW}`,
    ]);
    expect(report.complete).toBe(true);
    expect(report.keysDestroyed).toBe(true);
  });

  it("reports the phases in DESTROY_PHASES order", async () => {
    const report = await destroyWorkspace(makeDeps([], []), plain);
    expect(report.phases.map((p) => p.phase)).toEqual(DESTROY_PHASES.map((p) => p.id));
  });

  it("stops holders and unmounts BEFORE shredding keys; the marker is last", async () => {
    const calls: string[] = [];
    await destroyWorkspace(makeDeps(calls, []), plain);
    const at = (needle: string) => calls.findIndex((c) => c.startsWith(needle));
    expect(at("holders.stopAll")).toBeLessThan(at("unmount:workspace"));
    expect(at("unmount:second")).toBeLessThan(at("shred:"));
    expect(at("shred:")).toBeLessThan(at("effects.shredTree"));
    expect(at("effects.marker")).toBe(calls.length - 1);
  });

  it("puts what the unmount had to kill into the destroy log", async () => {
    const logs: string[] = [];
    await destroyWorkspace(
      makeDeps([], logs, {
        volume: fakeVolume("unlocked", [], { unmountReport: "killed 1 process(es)" }),
      }),
      plain,
    );
    // Once per vault: the workspace and the second one.
    expect(logs.filter((l) => l === "killed 1 process(es)")).toHaveLength(2);
  });

  it("logs the Point of No Return only after the keys are really gone", async () => {
    const logs: string[] = [];
    await destroyWorkspace(makeDeps([], logs), plain);
    expect(logs.some((l) => l.includes("POINT OF NO RETURN"))).toBe(true);
  });

  it("does NOT claim success when key erasure fails, and keeps going", async () => {
    const calls: string[] = [];
    const logs: string[] = [];
    const failOn = new Set(["/c/ws/gocryptfs.conf"]);
    const report = await destroyWorkspace(
      makeDeps(calls, logs, { shredder: fakeShredder(calls, failOn) }),
      plain,
    );
    expect(report.keysDestroyed).toBe(false);
    expect(report.complete).toBe(false);
    expect(report.phases.find((p) => p.phase === "shred-keys")?.ok).toBe(false);
    expect(logs.some((l) => l.includes("POINT OF NO RETURN"))).toBe(false);
    expect(logs.some((l) => l.includes("KEY ERASURE FAILED"))).toBe(true);
    // best effort: later phases still ran, down to the completion marker
    expect(calls).toContain("effects.shredTree:/c/ws");
    expect(calls.at(-1)).toBe(`effects.marker:${NOW}`);
  });

  it("keeps going when a phase throws, and reports it", async () => {
    const calls: string[] = [];
    const report = await destroyWorkspace(
      makeDeps(calls, [], { volume: fakeVolume("unlocked", calls, { throwOnUnmount: true }) }),
      plain,
    );
    expect(report.phases.find((p) => p.phase === "unmount")?.ok).toBe(false);
    expect(report.complete).toBe(false);
    expect(calls).toContain("shred:/c/ws/gocryptfs.conf:3");
    expect(calls.at(-1)).toBe(`effects.marker:${NOW}`);
  });

  it("reports an unmount that could not release the mount", async () => {
    const calls: string[] = [];
    const report = await destroyWorkspace(
      makeDeps(calls, [], {
        volume: fakeVolume("unlocked", calls, { unmount: { escalate: false } }),
      }),
      plain,
    );
    const unmount = report.phases.find((p) => p.phase === "unmount");
    expect(unmount?.ok).toBe(false);
    expect(unmount?.detail).toContain("workspace");
  });

  it("a throwing scrub does not stop the completion marker", async () => {
    const calls: string[] = [];
    const effects = {
      ...fakeEffects(calls),
      async scrubCredentials() {
        calls.push("effects.scrubCredentials");
        throw new Error("rm failed");
      },
    };
    const report = await destroyWorkspace(makeDeps(calls, [], { effects }), plain);
    expect(report.phases.find((p) => p.phase === "scrub-credentials")?.ok).toBe(false);
    expect(report.keysDestroyed).toBe(true);
    expect(calls.at(-1)).toBe(`effects.marker:${NOW}`);
  });

  it("--skip-shred leaves the data phase out but still destroys the keys", async () => {
    const calls: string[] = [];
    const report = await destroyWorkspace(makeDeps(calls, []), { skipShred: true, remote: false });
    expect(calls.some((c) => c.startsWith("effects.shredTree"))).toBe(false);
    expect(calls).toContain("shred:/c/ws/gocryptfs.conf:3");
    expect(report.phases.find((p) => p.phase === "shred-data")).toEqual({
      phase: "shred-data",
      ok: true,
      detail: "skipped",
    });
    expect(report.complete).toBe(true);
  });

  it("deletes the remote repo only when asked", async () => {
    const without: string[] = [];
    await destroyWorkspace(makeDeps(without, []), plain);
    expect(without).not.toContain("effects.deleteRemote");

    const withRemote: string[] = [];
    await destroyWorkspace(makeDeps(withRemote, []), { skipShred: false, remote: true });
    expect(withRemote).toContain("effects.deleteRemote");
  });

  it("warns, but does not fail, when the remote repo cannot be deleted for lack of a token", async () => {
    const calls: string[] = [];
    const logs: string[] = [];
    const report = await destroyWorkspace(
      makeDeps(calls, logs, { effects: fakeEffects(calls, { remote: "skipped-no-token" }) }),
      { skipShred: false, remote: true },
    );
    expect(logs.some((l) => l.includes("No github-token"))).toBe(true);
    expect(report.phases.find((p) => p.phase === "quiesce")?.ok).toBe(true);
  });

  it("fails the quiesce phase when remote deletion errors", async () => {
    const calls: string[] = [];
    const report = await destroyWorkspace(
      makeDeps(calls, [], { effects: fakeEffects(calls, { remote: "failed" }) }),
      { skipShred: false, remote: true },
    );
    expect(report.phases.find((p) => p.phase === "quiesce")?.ok).toBe(false);
  });

  it("is re-runnable: a second run completes the same way", async () => {
    const first = await destroyWorkspace(makeDeps([], []), plain);
    const second = await destroyWorkspace(makeDeps([], []), plain);
    expect(second).toEqual(first);
  });
});
