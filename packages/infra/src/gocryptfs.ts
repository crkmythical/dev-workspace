/**
 * gocryptfs adapter: the VolumeDriver port plus the shell primitives behind it.
 * Moved from cli/lib/vault.ts; the passphrase prompt stays in the CLI.
 */
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import {
  DESTRUCT_COMPLETED_PATH,
  type UnmountMode,
  type VaultSpec,
  type VaultState,
  type VolumeDriver,
  killable,
  lineage,
  parseFuserPids,
} from "@sdw/core/vault";
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

/** Parent of `pid` per /proc, or undefined once the process is gone. */
function parentOf(pid: number): number | undefined {
  try {
    const stat = readFileSync(`/proc/${pid}/stat`, "utf-8");
    // "pid (comm) state ppid ...": comm may hold spaces and parentheses, so cut after the last ")".
    const ppid = Number(stat.slice(stat.lastIndexOf(")") + 2).split(" ")[1]);
    return Number.isInteger(ppid) ? ppid : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Leave the mount if this process's working directory is inside it. Left in place it is a
 * holder of its own (the unmount reports "busy"), and once the mount is detached `getcwd`
 * fails, so every subprocess started afterwards dies with ENOENT: self-destruct run from a
 * terminal inside the vault got as far as unmounting and then could not erase the keys.
 */
export function stepOutOf(mountPoint: string): void {
  try {
    const cwd = process.cwd();
    if (cwd !== mountPoint && !cwd.startsWith(`${mountPoint}/`)) return;
  } catch {
    // The working directory is already unreachable: leave it all the same.
  }
  try {
    process.chdir("/");
  } catch {
    // Nowhere to go; the unmount will report what it finds.
  }
}

/**
 * SIGKILL the processes using a mount, except this one and everything above it (see
 * killable()). "directory" is whatever uses the mount point itself (a shell whose cwd is
 * in it); "filesystem" is anything with a file open anywhere on the mounted filesystem.
 * Needs `fuser` (psmisc); without it nothing is found and nothing is killed.
 */
async function killHolders(
  mountPoint: string,
  scope: "directory" | "filesystem",
  report?: (message: string) => void,
): Promise<void> {
  const listing =
    scope === "filesystem"
      ? await $`fuser -m ${mountPoint}`.quiet().nothrow()
      : await $`fuser ${mountPoint}`.quiet().nothrow();
  const victims = killable(
    parseFuserPids(listing.stdout.toString()),
    lineage(process.pid, parentOf),
  );
  for (const pid of victims) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Already gone.
    }
  }
  if (victims.length > 0) {
    report?.(`  killed ${victims.length} process(es) using ${mountPoint}: ${victims.join(" ")}`);
  }
}

/**
 * Release a FUSE mount. Returns true when the mount point is free (including
 * when it was not mounted to begin with).
 *   graceful  fusermount -u
 *   lazy      fusermount -uz (detach now)
 *   escalate  graceful, then kill users of the mount and retry, then lazy
 *             (never the calling process or its ancestors)
 */
export async function unmountVault(
  mountPoint: string,
  mode: UnmountMode = "escalate",
  report?: (message: string) => void,
): Promise<boolean> {
  if (!(await isMounted(mountPoint))) return true;
  stepOutOf(mountPoint);

  const attempt = async (lazy: boolean): Promise<boolean> => {
    const r = lazy
      ? await $`fusermount -uz ${mountPoint}`.quiet().nothrow()
      : await $`fusermount -u ${mountPoint}`.quiet().nothrow();
    return r.exitCode === 0;
  };

  if (mode === "lazy") return attempt(true);
  if (await attempt(false)) return true;
  if (mode === "graceful") return false;

  await killHolders(mountPoint, "directory", report);
  await Bun.sleep(500);
  if (await attempt(false)) return true;

  await killHolders(mountPoint, "filesystem", report);
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
  unmount: (spec, mode, report) => unmountVault(spec.mountPoint, mode, report),
};
