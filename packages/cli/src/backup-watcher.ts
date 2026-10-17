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
import { type FSWatcher, watch } from "node:fs";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  BACKUP_EXCLUSION_SET,
  BACKUP_FAILURE_THRESHOLD,
  BACKUP_KEEP_DAILY,
  BACKUP_KEEP_HOURLY,
  BACKUP_KEEP_WEEKLY,
  BACKUP_STATE_DIR,
  backoffDelayMs,
  evaluateBackupGate,
  isExcludedPath,
  planDebounce,
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

let lastEventMs: number | null = null;
let lastTriggerMs: number | null = null;
let backupInFlight = false;
let pendingDuringFlight = false;
let consecutiveFailures = tracker.consecutiveFailures();
let lastFailureMs: number | null = null;
let stopping = false;
let resticProc: Subprocess<"ignore", "pipe", "pipe"> | null = null;

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
  if (backupInFlight) return; // guard against re-entry from setInterval overlap
  backupInFlight = true;
  lastTriggerMs = Date.now();
  // Clear pending flag for THIS backup cycle — we only care about events
  // that arrive DURING this backup, not before it.
  pendingDuringFlight = false;

  const excludeArgs = BACKUP_EXCLUSION_SET.flatMap((p) => ["--exclude", p]);
  const proxyUrl = `http://127.0.0.1:${CLASH_HTTP_PORT}`;

  try {
    resticProc = Bun.spawn(["restic", "backup", WORKSPACE_MOUNT, ...excludeArgs], {
      stdout: "pipe",
      stderr: "pipe",
      env: {
        ...process.env,
        http_proxy: proxyUrl,
        https_proxy: proxyUrl,
        HTTPS_PROXY: proxyUrl,
        HTTP_PROXY: proxyUrl,
      },
    });

    // Save stderr ref before nulling resticProc (needed for failure reporting)
    const stderrStream = resticProc.stderr;
    const exitCode = await resticProc.exited;
    resticProc = null;

    if (exitCode === 0) {
      await onBackupSuccess();
    } else {
      const stderr = await new Response(stderrStream).text().catch(() => "");
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

async function onBackupSuccess() {
  consecutiveFailures = 0;
  lastFailureMs = null;
  tracker.recordSuccess();

  // Check if prune is due (serialized — never concurrent with backup)
  const pruneDecision = planPrune({
    lastPruneMs: loadLastPrune(),
    nowMs: Date.now(),
  });
  if (pruneDecision === "prune-due") {
    await runPrune();
  }
}

function onBackupFailure(detail: string) {
  lastFailureMs = Date.now();
  consecutiveFailures = tracker.recordFailure().count;
  console.error(`Backup failed (attempt ${consecutiveFailures}): ${detail.slice(0, 200)}`);
}

// ─── Prune ───────────────────────────────────────────────────────────────────

async function runPrune() {
  const proxyUrl = `http://127.0.0.1:${CLASH_HTTP_PORT}`;
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

function loadLastPrune(): number | null {
  try {
    const ts = readFileSync(`${BACKUP_STATE_DIR}/last-prune`, "utf-8").trim();
    return ts ? new Date(ts).getTime() : null;
  } catch {
    return null;
  }
}
