/**
 * In-memory fakes for the vault ports. Every side effect is appended to a shared
 * `calls` log, so a test can assert both WHAT happened and in WHAT ORDER without a
 * container, gocryptfs or root.
 */
import type {
  DestroyEffects,
  HolderControl,
  RemoteRepoResult,
  Shredder,
  UnmountMode,
  VaultSpec,
  VaultState,
  VolumeDriver,
} from "../../src/vault/index.ts";

export const WORKSPACE: VaultSpec = {
  id: "workspace",
  cipherDir: "/c/ws",
  mountPoint: "/m/ws",
};
export const PENTEST: VaultSpec = { id: "pentest", cipherDir: "/c/pt", mountPoint: "/m/pt" };
export const SPECS: readonly VaultSpec[] = [WORKSPACE, PENTEST];

export interface FakeVolumeOptions {
  mountOk?: boolean;
  mountDetail?: string;
  /** Result per unmount mode; defaults to success. */
  unmount?: Partial<Record<UnmountMode, boolean>>;
  throwOnUnmount?: boolean;
}

export function fakeVolume(
  initial: VaultState,
  calls: string[],
  opts: FakeVolumeOptions = {},
): VolumeDriver {
  let state = initial;
  return {
    async state() {
      return state;
    },
    async init(spec) {
      calls.push(`init:${spec.id}`);
      state = "locked";
      return true;
    },
    async mount(spec) {
      calls.push(`mount:${spec.id}`);
      if (opts.mountOk === false)
        return { ok: false, detail: opts.mountDetail ?? "bad passphrase" };
      state = "unlocked";
      return { ok: true };
    },
    async unmount(spec, mode) {
      calls.push(`unmount:${spec.id}:${mode}`);
      if (opts.throwOnUnmount) throw new Error("fusermount exploded");
      const released = opts.unmount?.[mode] ?? true;
      if (released) state = "locked";
      return released;
    },
  };
}

export function fakeHolders(calls: string[]): HolderControl {
  return {
    async stopAll() {
      calls.push("holders.stopAll");
    },
  };
}

/** `failOn` lists the paths whose shred should report "still there". */
export function fakeShredder(calls: string[], failOn: ReadonlySet<string> = new Set()): Shredder {
  return {
    async shred(path, passes) {
      calls.push(`shred:${path}:${passes}`);
      return !failOn.has(path);
    },
  };
}

export function fakeEffects(
  calls: string[],
  opts: { remote?: RemoteRepoResult } = {},
): DestroyEffects {
  return {
    async quiesceServices() {
      calls.push("effects.quiesce");
    },
    async deleteRemoteRepo() {
      calls.push("effects.deleteRemote");
      return opts.remote ?? "deleted";
    },
    async shredTree(dir) {
      calls.push(`effects.shredTree:${dir}`);
    },
    async scrubCredentials() {
      calls.push("effects.scrubCredentials");
    },
    async clearShellHistory() {
      calls.push("effects.clearHistory");
    },
    async writeCompletionMarker(iso) {
      calls.push(`effects.marker:${iso}`);
    },
  };
}
