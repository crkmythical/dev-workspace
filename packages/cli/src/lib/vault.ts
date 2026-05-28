import { existsSync, mkdirSync } from "node:fs";
/**
 * Shared vault operations — DRY primitives for init/unlock/lock/mount workflows.
 * Used by both workspace vault and pentest vault.
 */
import { $ } from "bun";

export interface VaultConfig {
  cipherDir: string;
  mountPoint: string;
  label: string; // Human-readable name for messages (e.g., "workspace", "pentest")
}

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
export function ensureDirs(config: VaultConfig): void {
  mkdirSync(config.cipherDir, { recursive: true });
  mkdirSync(config.mountPoint, { recursive: true });
}

/** Read passphrase from env, TTY prompt, or piped stdin. */
export async function readPassphrase(envVar?: string, promptMsg = "Passphrase: "): Promise<string> {
  // 1. Environment variable (automation)
  if (envVar && process.env[envVar]) {
    return process.env[envVar]!;
  }
  // 2. Interactive prompt
  if (process.stdin.isTTY) {
    return prompt(promptMsg) ?? "";
  }
  // 3. Piped stdin
  return (await new Response(process.stdin).text()).trim();
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
  // Interactive — gocryptfs handles passphrase prompt
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

/** Unmount a FUSE mount with progressive escalation. Returns true on success. */
export async function unmountVault(mountPoint: string): Promise<boolean> {
  // Attempt 1: clean unmount
  const r1 = await $`fusermount -u ${mountPoint}`.quiet().nothrow();
  if (r1.exitCode === 0) return true;

  // Attempt 2: kill processes, then retry
  await $`fuser -k ${mountPoint}`.quiet().nothrow();
  await Bun.sleep(500);
  const r2 = await $`fusermount -u ${mountPoint}`.quiet().nothrow();
  if (r2.exitCode === 0) return true;

  // Attempt 3: aggressive kill + retry
  await $`fuser -km ${mountPoint}`.quiet().nothrow();
  await Bun.sleep(1000);
  const r3 = await $`fusermount -u ${mountPoint}`.quiet().nothrow();
  if (r3.exitCode === 0) return true;

  // Attempt 4: lazy unmount (always succeeds)
  await $`fusermount -uz ${mountPoint}`.quiet();
  return true;
}

/** Cleanup stale FUSE mount from previous container lifecycle. */
export async function cleanupStaleMount(mountPoint: string): Promise<void> {
  await $`fusermount -uz ${mountPoint}`.quiet().nothrow();
}
