/**
 * gocryptfs adapter: the VolumeDriver port plus the shell primitives behind it.
 * Moved from cli/lib/vault.ts; the passphrase prompt stays in the CLI.
 */
import { existsSync, mkdirSync } from "node:fs";
import { DESTRUCT_COMPLETED_PATH } from "@sdw/core/constants";
import type { UnmountMode, VaultSpec, VaultState, VolumeDriver } from "@sdw/core/vault";
import { $ } from "bun";

/** Check if a path is a FUSE mountpoint. */
export async function isMounted(path: string): Promise<boolean> {
  const result = await $`mountpoint -q ${path}`.quiet().nothrow();
  return result.exitCode === 0;
}

/** Check if vault has been initialized (gocryptfs.conf exists). */
export function isInitialized(cipherDir: string): boolean {
  return existsSync(`${cipherDir}/gocryptfs.conf`);
}

/** Ensure both cipher and mount directories exist. */
export function ensureDirs(config: {
  cipherDir: string;
  mountPoint: string;
  label?: string;
}): void {
  mkdirSync(config.cipherDir, { recursive: true });
  mkdirSync(config.mountPoint, { recursive: true });
}

/** Initialize gocryptfs vault. Returns true on success. */
export async function initVault(cipherDir: string, passphrase?: string): Promise<boolean> {
  if (passphrase) {
    const proc = Bun.spawn(["gocryptfs", "-init", "-q", cipherDir], {
      stdin: new Response(`${passphrase}\n${passphrase}\n`).body!,
      stdout: "pipe",
      stderr: "pipe",
    });
    return (await proc.exited) === 0;
  }
  const result = await $`gocryptfs -init ${cipherDir}`.nothrow();
  return result.exitCode === 0;
}

/** Mount gocryptfs vault. Returns { ok, stderr }. */
export async function mountVault(
  cipherDir: string,
  mountPoint: string,
  passphrase: string,
): Promise<{ ok: boolean; stderr: string }> {
  mkdirSync(mountPoint, { recursive: true });
  const proc = Bun.spawn(["gocryptfs", "-q", "-nonempty", cipherDir, mountPoint], {
    stdin: new Response(`${passphrase}\n`).body!,
    stdout: "pipe",
    stderr: "pipe",
  });
  const exitCode = await proc.exited;
  const stderr = await new Response(proc.stderr).text();
  return { ok: exitCode === 0, stderr: stderr.trim() };
}

/**
 * Release a FUSE mount. Returns true when the mount point is free (including
 * when it was not mounted to begin with).
 *   graceful  fusermount -u
 *   lazy      fusermount -uz (detach now)
 *   escalate  graceful, then kill users of the mount and retry, then lazy
 */
export async function unmountVault(
  mountPoint: string,
  mode: UnmountMode = "escalate",
): Promise<boolean> {
  if (!(await isMounted(mountPoint))) return true;

  const attempt = async (lazy: boolean): Promise<boolean> => {
    const r = lazy
      ? await $`fusermount -uz ${mountPoint}`.quiet().nothrow()
      : await $`fusermount -u ${mountPoint}`.quiet().nothrow();
    return r.exitCode === 0;
  };

  if (mode === "lazy") return attempt(true);
  if (await attempt(false)) return true;
  if (mode === "graceful") return false;

  await $`fuser -k ${mountPoint}`.quiet().nothrow();
  await Bun.sleep(500);
  if (await attempt(false)) return true;

  await $`fuser -km ${mountPoint}`.quiet().nothrow();
  await Bun.sleep(1000);
  if (await attempt(false)) return true;

  return attempt(true);
}

/** Cleanup stale FUSE mount from previous container lifecycle. */
export async function cleanupStaleMount(mountPoint: string): Promise<void> {
  await $`fusermount -uz ${mountPoint}`.quiet().nothrow();
}

/** VolumeDriver port backed by gocryptfs + FUSE. */
export const gocryptfsVolume: VolumeDriver = {
  async state(spec: VaultSpec): Promise<VaultState> {
    if (existsSync(DESTRUCT_COMPLETED_PATH)) return "destroyed";
    if (await isMounted(spec.mountPoint)) return "unlocked";
    return isInitialized(spec.cipherDir) ? "locked" : "uninitialized";
  },
  init: (spec, passphrase) => initVault(spec.cipherDir, passphrase),
  async mount(spec, passphrase) {
    const { ok, stderr } = await mountVault(spec.cipherDir, spec.mountPoint, passphrase);
    return { ok, detail: stderr };
  },
  unmount: (spec, mode) => unmountVault(spec.mountPoint, mode),
};
