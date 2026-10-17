import { describe, expect, it } from "bun:test";
import {
  BOOT_PLAN,
  type BootState,
  type BootStepId,
  type BootStepImpls,
  type BootStepSpec,
  runBoot,
} from "../src/workspace/index.ts";

/**
 * The start-up plan is data, so its ordering rules and failure policy are tested here
 * without a container: a recorder stands in for every step's effects.
 */
const ids = BOOT_PLAN.map((s) => s.id) as BootStepId[];
const indexOf = (id: BootStepId) => ids.indexOf(id);
const freshState = (over: Partial<BootState> = {}): BootState => ({
  vaultMounted: false,
  degraded: [],
  ...over,
});

/** An implementation per step that records its id, and can be told to fail. */
function recorder(opts: { failing?: Partial<Record<BootStepId, Error>>; mount?: boolean } = {}) {
  const ran: BootStepId[] = [];
  const impls = Object.fromEntries(
    ids.map((id) => [
      id,
      async (state: BootState) => {
        ran.push(id);
        const failure = opts.failing?.[id];
        if (failure) throw failure;
        if (id === "unlock-vault" && opts.mount) state.vaultMounted = true;
      },
    ]),
  ) as BootStepImpls;
  return { ran, impls };
}

describe("BOOT_PLAN: shape", () => {
  it("lists every step once", () => {
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("starts with the pre-flight check and ends by handing over to supervisord", () => {
    expect(ids[0]).toBe("preflight");
    expect(ids.at(-1)).toBe("supervisord");
  });

  it("only the steps without which running makes no sense can abort the boot", () => {
    const abort = BOOT_PLAN.filter((s) => s.onFail === "abort").map((s) => s.id);
    expect(abort).toEqual(["preflight", "resolve-auth", "supervisord"]);
  });

  it("egress is never a reason to stop: bringing it up is a degrade-policy step", () => {
    const step = BOOT_PLAN.find((s) => s.id === "bring-up-egress");
    expect(step?.onFail).toBe("degrade");
  });
});

describe("BOOT_PLAN: ordering rules", () => {
  // [first, then, why]: each pair is a constraint the old script kept only by line order.
  const MUST_PRECEDE: [BootStepId, BootStepId, string][] = [
    ["cleanup-stale-mounts", "unlock-vault", "a stale mount would fake an unlocked vault"],
    ["render-clash-config", "bring-up-egress", "Clash needs its config before it starts"],
    ["resolve-auth", "init-vault", "the master password is the vault passphrase"],
    ["resolve-auth", "unlock-vault", "the master password is the vault passphrase"],
    ["init-vault", "unlock-vault", "there is nothing to unlock before init"],
    ["init-vault", "prepare-replication", "the repository is made in the cipher directory"],
    ["prepare-replication", "banner", "the banner reports the final state"],
    ["bring-up-egress", "notify-egress", "the note describes what bring-up observed"],
    ["unlock-vault", "notify-egress", "the note is written inside the vault"],
    ["unlock-vault", "prepare-desktops", "desktop HOME directories live inside the vault"],
    ["unlock-vault", "prepare-backup", "the backup watches the plaintext mount"],
    ["prepare-desktops", "banner", "the banner reports the final state"],
    ["prepare-backup", "banner", "the banner reports the final state"],
    ["notify-egress", "banner", "the banner reports the final state"],
    ["banner", "supervisord", "supervisord runs until the container stops"],
  ];

  for (const [first, then, why] of MUST_PRECEDE) {
    it(`${first} runs before ${then} (${why})`, () => {
      expect(indexOf(first)).toBeGreaterThanOrEqual(0);
      expect(indexOf(first)).toBeLessThan(indexOf(then));
    });
  }
});

describe("BOOT_PLAN: nothing is written under the mount point while the vault is locked", () => {
  const NEEDS_VAULT: BootStepId[] = ["notify-egress", "prepare-desktops", "prepare-backup"];

  for (const id of NEEDS_VAULT) {
    it(`${id} is skipped until the vault is mounted`, () => {
      const step = BOOT_PLAN.find((s) => s.id === id) as BootStepSpec | undefined;
      expect(step?.when).toBeDefined();
      expect(step?.when?.(freshState({ vaultMounted: false }))).toBe(false);
      expect(step?.when?.(freshState({ vaultMounted: true }))).toBe(true);
    });
  }

  it("a locked vault runs none of them, and the boot still completes", async () => {
    const { ran, impls } = recorder({ mount: false });
    const report = await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    expect(report.aborted).toBe(false);
    for (const id of NEEDS_VAULT) expect(ran).not.toContain(id);
    expect(ran.at(-1)).toBe("supervisord");
  });

  it("a mounted vault runs all of them", async () => {
    const { ran, impls } = recorder({ mount: true });
    await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    for (const id of NEEDS_VAULT) expect(ran).toContain(id);
  });
});

describe("runBoot", () => {
  it("runs the steps in plan order", async () => {
    const { ran, impls } = recorder({ mount: true });
    const report = await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    expect(ran).toEqual(ids);
    expect(report.steps.map((s) => s.outcome)).toEqual(ids.map(() => "ok"));
    expect(report.aborted).toBe(false);
  });

  it("a failing degrade step is recorded and the boot goes on", async () => {
    const { ran, impls } = recorder({
      mount: true,
      failing: { "bring-up-egress": new Error("clash exploded"), "prepare-backup": new Error("x") },
    });
    const state = freshState();
    const logs: string[] = [];
    const report = await runBoot(BOOT_PLAN, impls, state, (m) => logs.push(m));

    expect(report.aborted).toBe(false);
    expect(ran.at(-1)).toBe("supervisord");
    expect(state.degraded).toEqual(["bring-up-egress", "prepare-backup"]);
    const egress = report.steps.find((s) => s.id === "bring-up-egress");
    expect(egress).toEqual({
      id: "bring-up-egress",
      outcome: "degraded",
      detail: "clash exploded",
    });
    expect(logs.some((l) => l.includes('"bring-up-egress"') && l.includes("clash exploded"))).toBe(
      true,
    );
  });

  it("a failing abort step stops the boot: nothing after it runs", async () => {
    const { ran, impls } = recorder({ failing: { "resolve-auth": new Error("no password") } });
    const logs: string[] = [];
    const report = await runBoot(BOOT_PLAN, impls, freshState(), (m) => logs.push(m));

    expect(report.aborted).toBe(true);
    expect(ran.at(-1)).toBe("resolve-auth");
    expect(ran).not.toContain("init-vault");
    expect(ran).not.toContain("supervisord");
    expect(report.steps.at(-1)).toEqual({
      id: "resolve-auth",
      outcome: "aborted",
      detail: "no password",
    });
    expect(logs.at(-1)).toContain('boot aborted at "resolve-auth": no password');
  });

  it("a thrown non-Error is reported too", async () => {
    const { impls } = recorder();
    impls["git-globals"] = async () => {
      throw "plain string";
    };
    const report = await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    expect(report.steps.find((s) => s.id === "git-globals")?.detail).toBe("plain string");
  });

  it("records skipped steps as skipped, not as ok", async () => {
    const { impls } = recorder({ mount: false });
    const report = await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    const skipped = report.steps.filter((s) => s.outcome === "skipped").map((s) => s.id);
    expect(skipped).toEqual(["notify-egress", "prepare-desktops", "prepare-backup"]);
  });

  it("a later step sees what an earlier one put in the state", async () => {
    const { impls } = recorder();
    let seen: BootState["egress"];
    impls["bring-up-egress"] = async (s) => {
      s.egress = { status: "down", checkedAt: "t", providerNodes: null };
    };
    impls.banner = async (s) => {
      seen = s.egress;
    };
    await runBoot(BOOT_PLAN, impls, freshState(), () => {});
    expect(seen?.status).toBe("down");
  });
});
