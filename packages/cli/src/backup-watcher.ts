#!/usr/bin/env bun
/**
 * backup-watcher — Event-driven restic backup with debounce.
 *
 * Supervised program (autostart=false). Started on vault mount when
 * RESTIC_REPOSITORY is configured. Watches /workspace via inotify,
 * debounces 10s, then triggers `restic backup`.
 *
 * What to do when is decided by the pure state machine in @sdw/core/backup (loop.ts),
 * which is replayed over time in tests. This file owns the effects only.
 *
 * FUSE-unmount safety: this process holds inotify watches + read fds on
 * /workspace. It MUST be stopped before fusermount -u (lock-vault/destroy).
 */
import { type FSWatcher, watch } from "node:fs";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  BACKUP_EXCLUSION_SET,
  BACKUP_FAILURE_THRESHOLD,
  BACKUP_KEEP_DAILY,
  BACKUP_KEEP_HOURLY,
  BACKUP_KEEP_WEEKLY,
  BACKUP_STATE_DIR,
  decideNext,
  evaluateBackupGate,
  isExcludedPath,
  newLoop,
  onBackupFinished,
  onBackupStarted,
  onFsEvent,
  planPrune,
} from "@sdw/core/backup";
import { CLASH_HTTP_PORT } from "@sdw/core/egress";
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import { loadConfig } from "@sdw/core/workspace";
import { createFileFailureTracker } from "@sdw/infra";
import { $, type Subprocess } from "bun";

// ─── Pre-flight ──────────────────────────────────────────────────────────────

const gate = evaluateBackupGate(loadConfig(process.env).backup);
if (gate.status === "dormant") {
  console.log("Backup not configured (RESTIC_REPOSITORY absent). Exiting cleanly.");
  process.exit(0);
}
if (gate.status === "misconfigured") {
  console.error(`Backup misconfigured: missing ${gate.missing.join(", ")}. Exiting.`);
  process.exit(1);
}

const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mountCheck.exitCode !== 0) {
  console.error("Workspace not mounted. Exiting.");
  process.exit(1);
}

mkdirSync(BACKUP_STATE_DIR, { recursive: true });

// Counter, last-success marker and notification file live in the shared tracker (the
// same one vault-sync uses), so `doctor` reads one format for both jobs.
const tracker = createFileFailureTracker({
  stateDir: BACKUP_STATE_DIR,
  threshold: BACKUP_FAILURE_THRESHOLD,
  notificationFile: "backup-failure.md",
  notificationBody: (n) =>
    `# ⚠ Backup Failure\n\nFailed ${n} consecutive times. Run \`doctor\` to diagnose.\n`,
});

// ─── State ───────────────────────────────────────────────────────────────────

let loop = newLoop(tracker.consecutiveFailures());
let stopping = false;
let resticProc: Subprocess<"ignore", "pipe", "pipe"> | null = null;

/** restic goes through the same proxy as everything else. */
const PROXY_URL = `http://127.0.0.1:${CLASH_HTTP_PORT}`;
const resticEnv = () => ({
  ...process.env,
  http_proxy: PROXY_URL,
  https_proxy: PROXY_URL,
  HTTPS_PROXY: PROXY_URL,
  HTTP_PROXY: PROXY_URL,
});

// ─── inotify watcher ─────────────────────────────────────────────────────────

let watcher: FSWatcher;
try {
  watcher = watch(WORKSPACE_MOUNT, { recursive: true }, (_event, filename) => {
    if (stopping) return;
    if (filename && isExcludedPath(filename)) return;
    loop = onFsEvent(loop, Date.now());
  });
} catch (err) {
  console.error(`Failed to start filesystem watcher: ${err}`);
  process.exit(1);
}

console.log("Backup watcher started. Monitoring /workspace for changes.");

// ─── Main loop (1-second tick) ───────────────────────────────────────────────

const TICK_MS = 1000;
const timer = setInterval(async () => {
  if (stopping) return;
  if (decideNext(loop, Date.now()) === "trigger") {
    await runBackup();
  }
}, TICK_MS);

// ─── Graceful shutdown ───────────────────────────────────────────────────────

process.on("SIGTERM", gracefulStop);
process.on("SIGINT", gracefulStop);

async function gracefulStop() {
  if (stopping) return;
  stopping = true;
  clearInterval(timer);
  if (resticProc) {
    resticProc.kill("SIGTERM");
    const killTimeout = setTimeout(() => {
      try {
        resticProc?.kill("SIGKILL");
      } catch {}
    }, 5000);
    await resticProc.exited;
    clearTimeout(killTimeout);
  }
  watcher.close();
  console.log("Backup watcher stopped gracefully.");
  process.exit(0);
}

// ─── Backup execution ────────────────────────────────────────────────────────

async function runBackup() {
  if (loop.backupInFlight) return; // guard against re-entry from setInterval overlap
  loop = onBackupStarted(loop, Date.now());

  const excludeArgs = BACKUP_EXCLUSION_SET.flatMap((p) => ["--exclude", p]);
  let failures: number | null = null; // set when this run failed

  const fail = (detail: string) => {
    failures = tracker.recordFailure().count;
    console.error(`Backup failed (attempt ${failures}): ${detail.slice(0, 200)}`);
  };

  try {
    resticProc = Bun.spawn(["restic", "backup", WORKSPACE_MOUNT, ...excludeArgs], {
      stdout: "pipe",
      stderr: "pipe",
      env: resticEnv(),
    });

    // Save stderr ref before nulling resticProc (needed for failure reporting)
    const stderrStream = resticProc.stderr;
    const exitCode = await resticProc.exited;
    resticProc = null;

    if (exitCode === 0) {
      tracker.recordSuccess();
      // Prune is serialized with the backup: never concurrent with it.
      if (planPrune({ lastPruneMs: loadLastPrune(), nowMs: Date.now() }) === "prune-due") {
        await runPrune();
      }
    } else {
      fail(await new Response(stderrStream).text().catch(() => ""));
    }
  } catch (err) {
    resticProc = null;
    fail(String(err));
  }

  loop = onBackupFinished(
    loop,
    failures === null ? { ok: true } : { ok: false, failures },
    Date.now(),
  );
}

// ─── Prune ───────────────────────────────────────────────────────────────────

async function runPrune() {
  try {
    const proc = Bun.spawn(
      [
        "restic",
        "forget",
        "--prune",
        "--keep-hourly",
        String(BACKUP_KEEP_HOURLY),
        "--keep-daily",
        String(BACKUP_KEEP_DAILY),
        "--keep-weekly",
        String(BACKUP_KEEP_WEEKLY),
      ],
      { stdout: "pipe", stderr: "pipe", env: resticEnv() },
    );
    const exitCode = await proc.exited;
    if (exitCode === 0) {
      writeFileSync(`${BACKUP_STATE_DIR}/last-prune`, new Date().toISOString());
      console.log("Prune completed successfully.");
    } else {
      console.warn("Prune failed (non-fatal). Will retry next cadence.");
    }
  } catch (err) {
    console.warn(`Prune error (non-fatal): ${err}`);
  }
}

// ─── State helpers ───────────────────────────────────────────────────────────

function loadLastPrune(): number | null {
  try {
    const ts = readFileSync(`${BACKUP_STATE_DIR}/last-prune`, "utf-8").trim();
    return ts ? new Date(ts).getTime() : null;
  } catch {
    return null;
  }
}
