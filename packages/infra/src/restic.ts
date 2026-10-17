/**
 * restic adapter: asking a repository what state it is in.
 *
 * The decision (init it, warn, or leave it alone) is made from restic's exit code and message
 * by classifyRepoProbe() in core; this only runs restic and hands the answer over. Used by the
 * container's start-up and by `backup-init`, so both treat the same repository the same way.
 */
import { type RepoProbe, classifyRepoProbe } from "@sdw/core/backup";
import { CLASH_HTTP_PORT } from "@sdw/core/egress";

export interface RepoStatus {
  verdict: RepoProbe;
  /** What restic printed on stderr, for a diagnostic. May contain the repository URL. */
  detail: string;
}

/** restic goes through the same proxy as everything else. */
export function resticEnv(env: Record<string, string | undefined>): Record<string, string> {
  const proxy = `http://127.0.0.1:${CLASH_HTTP_PORT}`;
  const merged = {
    ...env,
    http_proxy: proxy,
    https_proxy: proxy,
    HTTP_PROXY: proxy,
    HTTPS_PROXY: proxy,
  };
  return Object.fromEntries(
    Object.entries(merged).filter((entry): entry is [string, string] => entry[1] !== undefined),
  );
}

/** `restic snapshots`, read-only (no lock), classified. Never throws. */
export async function probeRepository(
  env: Record<string, string | undefined>,
): Promise<RepoStatus> {
  try {
    const proc = Bun.spawn(["restic", "snapshots", "--no-lock"], {
      stdout: "ignore",
      stderr: "pipe",
      env: resticEnv(env),
    });
    const stderr = await new Response(proc.stderr).text();
    const exitCode = await proc.exited;
    return { verdict: classifyRepoProbe({ exitCode, stderr }), detail: stderr.trim() };
  } catch (err) {
    // restic is not there at all, or could not be started: not "missing", so nothing is initialized.
    return { verdict: "unreachable", detail: err instanceof Error ? err.message : String(err) };
  }
}

/** `restic init`. True on success. */
export async function initRepository(env: Record<string, string | undefined>): Promise<boolean> {
  try {
    const proc = Bun.spawn(["restic", "init"], {
      stdout: "inherit",
      stderr: "inherit",
      env: resticEnv(env),
    });
    return (await proc.exited) === 0;
  } catch {
    return false;
  }
}
