#!/usr/bin/env bun
/**
 * backup-watcher — Event-driven restic backup with debounce.
 *
 * Supervised program (autostart=false). Started on vault mount when
 * RESTIC_REPOSITORY is configured. Watches /workspace via inotify,
 * debounces 10s, then triggers `restic backup`.
 *
 * FUSE-unmount safety: this process holds inotify watches + read fds on
 * /workspace. It MUST be stopped before fusermount -u (lock-vault/destroy).
 */
import { watch, type FSWatcher } from "node:fs";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  BACKUP_EXCLUSION_SET,
  BACKUP_KEEP_DAILY,
  BACKUP_KEEP_HOURLY,
  BACKUP_KEEP_WEEKLY,
  BACKUP_STATE_DIR,
  WORKSPACE_MOUNT,
  backoffDelayMs,
  evaluateBackupGate,
  isExcludedPath,
  planDebounce,
  planPrune,
  shouldNotify,
} from "@sdw/core";
import { $ } from "bun";

// ─── Pre-flight ──────────────────────────────────────────────────────────────

const gate = evaluateBackupGate(process.env);
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

// ─── State ───────────────────────────────────────────────────────────────────

let lastEventMs: number | null = null;
let lastTriggerMs: number | null = null;
let backupInFlight = false;
let pendingDuringFlight = false;
let consecutiveFailures = loadFailureCount();
let lastFailureMs: number | null = null;
let stopping = false;
let resticProc: ReturnType<typeof Bun.spawn> | null = null;

// ─── inotify watcher ─────────────────────────────────────────────────────────

let watcher: FSWatcher;
try {
  watcher = watch(WORKSPACE_MOUNT, { recursive: true }, (_event, filename) => {
    if (stopping) return;
    if (filename && isExcludedPath(filename)) return;
    lastEventMs = Date.now();
    if (backupInFlight) pendingDuringFlight = true;
  });
} catch (err) {
  console.error(`Failed to start filesystem watcher: ${err}`);
  process.exit(1);
}

console.log("Backup watcher started. Monitoring /workspace for changes.");

// ─── Main loop (1-second tick) ───────────────────────────────────────────────

const TICK_MS = 1000;
const loop = setInterval(async () => {
  if (stopping) return;
  const decision = planDebounce({
    lastEventMs,
    nowMs: Date.now(),
    backupInFlight,
    lastTriggerMs,
    consecutiveFailures,
    lastFailureMs,
  });
  if (decision === "trigger") {
    await runBackup();
  }
}, TICK_MS);

// ─── Graceful shutdown ───────────────────────────────────────────────────────

process.on("SIGTERM", gracefulStop);
process.on("SIGINT", gracefulStop);

async function gracefulStop() {
  if (stopping) return;
  stopping = true;
  clearInterval(loop);
  if (resticProc) {
    resticProc.kill("SIGTERM");
    const killTimeout = setTimeout(() => {
      try { resticProc?.kill("SIGKILL"); } catch {}
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
  backupInFlight = true;
  lastTriggerMs = Date.now();
  pendingDuringFlight = false;

  const excludeArgs = BACKUP_EXCLUSION_SET.flatMap((p) => ["--exclude", p]);
  const proxyUrl = "http://127.0.0.1:7890";

  try {
    resticProc = Bun.spawn(
      ["restic", "backup", WORKSPACE_MOUNT, ...excludeArgs],
      {
        stdout: "pipe",
        stderr: "pipe",
        env: {
          ...process.env,
          http_proxy: proxyUrl,
          https_proxy: proxyUrl,
          HTTPS_PROXY: proxyUrl,
          HTTP_PROXY: proxyUrl,
        },
      },
    );

    const exitCode = await resticProc.exited;
    resticProc = null;

    if (exitCode === 0) {
      onBackupSuccess();
    } else {
      const stderr = await new Response(resticProc?.stderr ?? "").text().catch(() => "");
      onBackupFailure(stderr);
    }
  } catch (err) {
    resticProc = null;
    onBackupFailure(String(err));
  }

  backupInFlight = false;

  // Re-enter debounce if events accumulated during this backup
  if (pendingDuringFlight) {
    lastEventMs = Date.now();
    pendingDuringFlight = false;
  }
}

function onBackupSuccess() {
  consecutiveFailures = 0;
  lastFailureMs = null;
  writeFileSync(`${BACKUP_STATE_DIR}/last-success`, new Date().toISOString());
  writeFileSync(`${BACKUP_STATE_DIR}/consecutive-failures`, "0");

  // Check if prune is due (serialized — never concurrent with backup)
  const pruneDecision = planPrune({
    lastPruneMs: loadLastPrune(),
    nowMs: Date.now(),
  });
  if (pruneDecision === "prune-due") {
    runPrune();
  }
}

function onBackupFailure(detail: string) {
  consecutiveFailures++;
  lastFailureMs = Date.now();
  writeFileSync(`${BACKUP_STATE_DIR}/consecutive-failures`, String(consecutiveFailures));
  console.error(`Backup failed (attempt ${consecutiveFailures}): ${detail.slice(0, 200)}`);

  if (shouldNotify(consecutiveFailures)) {
    try {
      mkdirSync(`${WORKSPACE_MOUNT}/.notifications`, { recursive: true });
      writeFileSync(
        `${WORKSPACE_MOUNT}/.notifications/backup-failure.md`,
        `# ⚠️ Backup Failure\n\nFailed ${consecutiveFailures} consecutive times. Run \`doctor\` to diagnose.\n`,
      );
    } catch {}
  }
}

// ─── Prune ───────────────────────────────────────────────────────────────────

async function runPrune() {
  const proxyUrl = "http://127.0.0.1:7890";
  try {
    const proc = Bun.spawn(
      [
        "restic", "forget", "--prune",
        "--keep-hourly", String(BACKUP_KEEP_HOURLY),
        "--keep-daily", String(BACKUP_KEEP_DAILY),
        "--keep-weekly", String(BACKUP_KEEP_WEEKLY),
      ],
      {
        stdout: "pipe",
        stderr: "pipe",
        env: {
          ...process.env,
          http_proxy: proxyUrl,
          https_proxy: proxyUrl,
          HTTPS_PROXY: proxyUrl,
          HTTP_PROXY: proxyUrl,
        },
      },
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

function loadFailureCount(): number {
  try {
    return parseInt(readFileSync(`${BACKUP_STATE_DIR}/consecutive-failures`, "utf-8").trim(), 10) || 0;
  } catch {
    return 0;
  }
}

function loadLastPrune(): number | null {
  try {
    const ts = readFileSync(`${BACKUP_STATE_DIR}/last-prune`, "utf-8").trim();
    return ts ? new Date(ts).getTime() : null;
  } catch {
    return null;
  }
}
