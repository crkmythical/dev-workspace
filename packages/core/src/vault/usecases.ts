/**
 * Vault use cases: lock, unlock and destroy.
 *
 * Each ordering invariant is written exactly once here and exercised with fake
 * ports (no container needed):
 *   - holders are stopped BEFORE any unmount (FUSE EBUSY safety);
 *   - a destroy runs its phases in the DESTROY_PHASES order, keeps going when a
 *     phase fails, and reports honestly instead of claiming success.
 */
import {
  DESTROY_PHASES,
  type DestroyPhaseId,
  type VaultSpec,
  decide,
  needsPassphrase,
} from "./domain.ts";
import type {
  DestroyEffects,
  HolderControl,
  Shredder,
  UnlockHook,
  UnmountMode,
  VolumeDriver,
} from "./ports.ts";

// --- lock ---

export interface LockDeps {
  volume: VolumeDriver;
  holders: HolderControl;
  /** Where the volume reports what it had to do to release the mount (e.g. kill processes). */
  log?: (message: string) => void;
}

export type LockResult =
  | { status: "noop"; reason: "already-locked" }
  | { status: "rejected"; reason: "destroyed" }
  | { status: "locked" }
  /** The unmount was refused (open handles). The caller may retry with mode "lazy". */
  | { status: "busy" };

export async function lockVault(
  spec: VaultSpec,
  deps: LockDeps,
  opts: { mode?: UnmountMode } = {},
): Promise<LockResult> {
  const decision = decide(await deps.volume.state(spec), { type: "lock" });
  if (decision.do === "noop") return { status: "noop", reason: "already-locked" };
  if (decision.do === "reject") return { status: "rejected", reason: "destroyed" };

  // The ordering invariant: holders first, then unmount.
  await deps.holders.stopAll();
  const released = await deps.volume.unmount(spec, opts.mode ?? "graceful", deps.log);
  return released ? { status: "locked" } : { status: "busy" };
}

/**
 * The lock came back "busy" and the caller chose not to force it. The vault stays mounted, so
 * what the attempt stopped (desktops, the backup watcher) must run again: left stopped, the
 * workspace would go on without its backups and nobody would be told.
 */
export async function cancelLock(deps: Pick<LockDeps, "holders">): Promise<string[]> {
  return (await deps.holders.restore?.()) ?? [];
}

// --- unlock ---

export interface UnlockDeps {
  volume: VolumeDriver;
  /** Run in order after a successful mount. */
  hooks: readonly UnlockHook[];
}

export type UnlockResult =
  | { status: "noop"; reason: "already-unlocked" }
  | { status: "rejected"; reason: "not-initialized" | "empty-passphrase" | "destroyed" }
  /** The mount itself failed (wrong passphrase or mount error). */
  | { status: "failed"; detail: string }
  | { status: "unlocked"; warnShortPassphrase: boolean; hookFailures: string[] };

/**
 * One unlock path for both the interactive `unlock-vault` command and the
 * container's boot-time auto-unlock. The passphrase is read lazily, only when a
 * mount is actually going to be attempted.
 */
export async function unlockVault(
  spec: VaultSpec,
  readPassphrase: () => Promise<string>,
  deps: UnlockDeps,
): Promise<UnlockResult> {
  const state = await deps.volume.state(spec);
  const passphrase = needsPassphrase(state) ? await readPassphrase() : "";
  const decision = decide(state, { type: "unlock", passphrase });
  if (decision.do === "noop") return { status: "noop", reason: "already-unlocked" };
  if (decision.do === "reject") return { status: "rejected", reason: decision.reason };

  const mounted = await deps.volume.mount(spec, passphrase);
  if (!mounted.ok) return { status: "failed", detail: mounted.detail ?? "" };

  const hookFailures: string[] = [];
  for (const hook of deps.hooks) {
    try {
      await hook.run({ spec, passphrase });
    } catch (err) {
      hookFailures.push(`${hook.name}: ${err instanceof Error ? err.message : String(err)}`);
    }
  }
  return {
    status: "unlocked",
    warnShortPassphrase: decision.warnShortPassphrase === true,
    hookFailures,
  };
}

// --- destroy ---

export interface DestroyOptions {
  skipShred: boolean;
  remote: boolean;
}

export interface PhaseReport {
  phase: DestroyPhaseId;
  ok: boolean;
  detail?: string;
}

export interface DestroyReport {
  phases: PhaseReport[];
  /** True only if every vault's gocryptfs.conf is gone. */
  keysDestroyed: boolean;
  /** True only if every phase succeeded. */
  complete: boolean;
}

export interface DestroyDeps {
  specs: readonly VaultSpec[];
  volume: VolumeDriver;
  holders: HolderControl;
  shredder: Shredder;
  effects: DestroyEffects;
  now: () => string;
  log: (message: string) => void;
}

const KEY_FILE = "gocryptfs.conf";
const KEY_SHRED_PASSES = 3;

function labelOf(id: DestroyPhaseId): string {
  return DESTROY_PHASES.find((p) => p.id === id)?.label ?? id;
}

/**
 * Best-effort, ordered destruction. A failing phase is recorded and the next
 * phase still runs (stopping half way would leave a half-destroyed vault), but
 * the report never claims more than what actually happened.
 */
export async function destroyWorkspace(
  deps: DestroyDeps,
  opts: DestroyOptions,
): Promise<DestroyReport> {
  const { specs, volume, holders, shredder, effects, now, log } = deps;
  let keysDestroyed = false;

  const handlers: Record<DestroyPhaseId, () => Promise<string | undefined>> = {
    quiesce: async () => {
      log(labelOf("quiesce"));
      await effects.quiesceServices();
      if (opts.remote) {
        const result = await effects.deleteRemoteRepo();
        if (result === "skipped-no-token") {
          log("  WARNING: No github-token found, cannot delete remote repo.");
        }
        if (result === "failed") throw new Error("remote repo deletion failed");
      }
      return undefined;
    },
    unmount: async () => {
      log(labelOf("unmount"));
      await holders.stopAll();
      const stuck: string[] = [];
      for (const spec of specs) {
        if (!(await volume.unmount(spec, "escalate", log))) stuck.push(spec.id);
      }
      if (stuck.length > 0) throw new Error(`could not release mount(s): ${stuck.join(", ")}`);
      return undefined;
    },
    "shred-keys": async () => {
      log(labelOf("shred-keys"));
      const failed: string[] = [];
      for (const spec of specs) {
        const gone = await shredder.shred(`${spec.cipherDir}/${KEY_FILE}`, KEY_SHRED_PASSES);
        if (!gone) failed.push(spec.id);
      }
      keysDestroyed = failed.length === 0;
      if (keysDestroyed) {
        log(`  *** POINT OF NO RETURN *** ${now()}`);
        return undefined;
      }
      log(
        `  !!! KEY ERASURE FAILED for: ${failed.join(", ")} (the vault may still be decryptable)`,
      );
      throw new Error(`key erasure failed: ${failed.join(", ")}`);
    },
    "shred-data": async () => {
      if (opts.skipShred) {
        log("[Phase 3] Skipped (--skip-shred).");
        return "skipped";
      }
      log(labelOf("shred-data"));
      for (const spec of specs) await effects.shredTree(spec.cipherDir);
      return undefined;
    },
    "scrub-credentials": async () => {
      log(labelOf("scrub-credentials"));
      await effects.scrubCredentials();
      return undefined;
    },
    "mark-complete": async () => {
      log(labelOf("mark-complete"));
      await effects.clearShellHistory();
      await effects.writeCompletionMarker(now());
      return undefined;
    },
  };

  const phases: PhaseReport[] = [];
  for (const phase of DESTROY_PHASES) {
    try {
      const detail = await handlers[phase.id]();
      phases.push(detail ? { phase: phase.id, ok: true, detail } : { phase: phase.id, ok: true });
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      phases.push({ phase: phase.id, ok: false, detail });
      log(`  WARNING: ${phase.id} failed: ${detail}`);
    }
  }

  const complete = phases.every((p) => p.ok);
  log(complete ? "destroyWorkspace complete." : "destroyWorkspace finished WITH FAILURES.");
  return { phases, keysDestroyed, complete };
}
