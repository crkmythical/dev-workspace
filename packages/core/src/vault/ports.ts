/**
 * Vault ports: what the use cases need from the outside world.
 * Implemented by adapters in @sdw/infra; replaced by fakes in tests.
 */
import type { VaultSpec, VaultState } from "./domain.ts";

/**
 * How hard to try to release a mount:
 *   graceful  plain unmount; refuses when files are open
 *   lazy      detach now, clean up when the last handle closes
 *   escalate  graceful, then kill users of the mount and retry, then lazy
 */
export type UnmountMode = "graceful" | "lazy" | "escalate";

export interface VolumeDriver {
  /** Observe the vault's current state. */
  state(spec: VaultSpec): Promise<VaultState>;
  init(spec: VaultSpec, passphrase: string): Promise<boolean>;
  mount(spec: VaultSpec, passphrase: string): Promise<{ ok: boolean; detail?: string }>;
  /**
   * Returns true when the mount point has been released (or was not mounted).
   * `report` hears of anything done to get there that the user would want to know, such as
   * the processes killed in "escalate" mode.
   */
  unmount(spec: VaultSpec, mode: UnmountMode, report?: (message: string) => void): Promise<boolean>;
}

/**
 * Anything that may hold fds or inotify watches on the plaintext mount
 * (desktop sessions, the backup watcher, ...). They must all be stopped before
 * an unmount, or it fails with EBUSY.
 */
export interface HolderControl {
  /** Idempotent. Must not throw when nothing is running. */
  stopAll(): Promise<void>;
  /**
   * Start again what `stopAll` stopped, for a lock that then did not happen (the unmount was
   * busy and the user declined to force it). Returns what it started, for the user to be told.
   * Optional: a vault with nothing to stop has nothing to restore.
   */
  restore?(): Promise<string[]>;
}

export interface Shredder {
  /** Irrecoverably remove a file. Returns true if the file is gone afterwards. */
  shred(path: string, passes: number): Promise<boolean>;
}

export interface UnlockContext {
  spec: VaultSpec;
  passphrase: string;
}

/** A step that runs after a successful mount. A failing hook never undoes the unlock. */
export interface UnlockHook {
  name: string;
  run(ctx: UnlockContext): Promise<void>;
}

export type RemoteRepoResult = "deleted" | "failed" | "skipped-no-repo" | "skipped-no-token";

/** Side effects of destruction that are not volume, holder or shred operations. */
export interface DestroyEffects {
  /** Stop periodic replication (vault-sync cron). */
  quiesceServices(): Promise<void>;
  deleteRemoteRepo(): Promise<RemoteRepoResult>;
  /** Shred every file under a directory, then remove what is left. */
  shredTree(dir: string): Promise<void>;
  /** Proxy config, ssh keys, git credentials, remote-destruct key hash. */
  scrubCredentials(): Promise<void>;
  clearShellHistory(): Promise<void>;
  /** Written last: its presence puts the container into inert mode on restart. */
  writeCompletionMarker(isoTimestamp: string): Promise<void>;
}
