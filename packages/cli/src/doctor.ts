#!/usr/bin/env bun
import { existsSync, readFileSync } from "node:fs";
import {
  CLASH_HTTP_PORT,
  CODE_SERVER_PORT,
  DESKTOP_WEB_ROOT,
  PENTEST_CIPHER_DIR,
  PENTEST_MOUNT,
  SELKIES_DISPLAY,
  SELKIES_STREAM_PORT,
  SYNC_SERVICE_PORT,
  VAULT_CIPHER_DIR,
  VAULT_SYNC_STATE_DIR,
  VNC_DISPLAY,
  VNC_STREAM_PORT,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
import type { DoctorResult } from "@sdw/core/types";
/**
 * doctor — Workspace health check
 */
import { $ } from "bun";
import { type DesktopTarget, desktopAnchor, installedStacks } from "./lib/desktop.ts";

const results: DoctorResult[] = [];

// 1. Clash
const clashProbe =
  await $`curl -s --connect-timeout 2 -x http://127.0.0.1:${CLASH_HTTP_PORT} http://www.gstatic.com/generate_204 -o /dev/null`
    .quiet()
    .nothrow();
results.push(
  clashProbe.exitCode === 0
    ? { component: "Clash proxy", status: "ok", detail: `port ${CLASH_HTTP_PORT} responding` }
    : {
        component: "Clash proxy",
        status: "fail",
        detail: `port ${CLASH_HTTP_PORT} not responding`,
      },
);

// 2. Vault mount
const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
results.push(
  mountCheck.exitCode === 0
    ? { component: "Vault mount", status: "ok", detail: "/workspace mounted" }
    : { component: "Vault mount", status: "warn", detail: "not mounted — run unlock-vault" },
);

// 3. code-server
const csProbe =
  await $`curl -s --connect-timeout 2 http://127.0.0.1:${CODE_SERVER_PORT}/healthz -o /dev/null`
    .quiet()
    .nothrow();
results.push(
  csProbe.exitCode === 0
    ? { component: "code-server", status: "ok" }
    : {
        component: "code-server",
        status: "fail",
        detail: `port ${CODE_SERVER_PORT} not responding`,
      },
);

// 4. Sync service
const syncProbe =
  await $`curl -s --connect-timeout 2 http://127.0.0.1:${SYNC_SERVICE_PORT}/sync/api/doctor -o /dev/null`
    .quiet()
    .nothrow();
results.push(
  syncProbe.exitCode === 0
    ? { component: "Sync service", status: "ok" }
    : {
        component: "Sync service",
        status: "fail",
        detail: `port ${SYNC_SERVICE_PORT} not responding`,
      },
);

// 5. Last vault sync
const lastSyncFile = `${VAULT_SYNC_STATE_DIR}/last-success`;
if (existsSync(lastSyncFile)) {
  const ts = readFileSync(lastSyncFile, "utf-8").trim();
  const age = Date.now() - new Date(ts).getTime();
  results.push(
    age < 7200_000
      ? { component: "Vault sync", status: "ok", detail: `last: ${ts}` }
      : { component: "Vault sync", status: "warn", detail: `last: ${ts} (>2h ago)` },
  );
} else {
  results.push({ component: "Vault sync", status: "warn", detail: "no sync recorded" });
}

// 6. Disk usage
if (existsSync(VAULT_CIPHER_DIR)) {
  const du = await $`du -sh ${VAULT_CIPHER_DIR}`.text();
  const size = du.split("\t")[0];
  results.push({ component: "Vault disk", status: "ok", detail: size });
}

// 7. Desktop(s) (optional, on-demand) — one report per installed stack.
async function isAnyAnchorRunning(): Promise<boolean> {
  for (const t of installedStacks()) {
    const r = await $`supervisorctl status ${desktopAnchor(t)}`.quiet().nothrow();
    if (r.exitCode === 0 && r.text().includes("RUNNING")) return true;
  }
  return false;
}
type StackMeta = {
  xLabel: string;
  streamLabel: string;
  port: number;
  display: string;
  servesOwnClient: boolean;
};
const STACK_META: Record<DesktopTarget, StackMeta> = {
  selkies: {
    xLabel: "Desktop X server (Xvfb)",
    streamLabel: "Desktop stream (Selkies)",
    port: SELKIES_STREAM_PORT,
    display: SELKIES_DISPLAY,
    servesOwnClient: false,
  },
  vnc: {
    xLabel: "VNC X/server (Xkasmvnc)",
    streamLabel: "VNC stream (KasmVNC)",
    port: VNC_STREAM_PORT,
    display: VNC_DISPLAY,
    servesOwnClient: true,
  },
};

for (const target of installedStacks()) {
  const meta = STACK_META[target];
  const anchorStatus = await $`supervisorctl status ${desktopAnchor(target)}`.quiet().nothrow();
  if (anchorStatus.exitCode === 0 && anchorStatus.text().includes("RUNNING")) {
    results.push({ component: meta.xLabel, status: "ok", detail: "running" });

    const streamProbe = await $`nc -z 127.0.0.1 ${meta.port}`.quiet().nothrow();
    results.push(
      streamProbe.exitCode === 0
        ? { component: meta.streamLabel, status: "ok", detail: `port ${meta.port} listening` }
        : { component: meta.streamLabel, status: "fail", detail: `port ${meta.port} not responding` },
    );

    // Static client reachability — selkies only. Selkies is WS-only, so a missing
    // static client is invisible to a port probe yet fatal (426/white-screen).
    // KasmVNC serves its own client from the same port, so this check is N/A there.
    if (!meta.servesOwnClient) {
      const clientExists = existsSync(`${DESKTOP_WEB_ROOT}/index.html`);
      results.push(
        clientExists
          ? { component: "Desktop client (static)", status: "ok", detail: `${DESKTOP_WEB_ROOT}/index.html present` }
          : { component: "Desktop client (static)", status: "warn", detail: `${DESKTOP_WEB_ROOT}/index.html MISSING — browser will get 426` },
      );
    }

    const displayCheck = await $`DISPLAY=${meta.display} xdpyinfo`.quiet().nothrow();
    results.push(
      displayCheck.exitCode === 0
        ? { component: `Desktop Display ${meta.display}`, status: "ok", detail: "active" }
        : { component: `Desktop Display ${meta.display}`, status: "warn", detail: "not responding" },
    );
  } else {
    results.push({ component: `Desktop [${target}]`, status: "ok", detail: "not started (optional)" });
  }
}

// Window-manager / panel health (shared XFCE binaries across both stacks).
// Detects the "fake-alive" session: X server up but WM/panel crashed (black
// screen in browser even though supervisord reports RUNNING).
if ((await isAnyAnchorRunning())) {
  const [wmName, panelName] = ["xfwm4", "xfce4-panel"];
  const wm = await $`pgrep -x ${wmName}`.quiet().nothrow();
  const panel = await $`pgrep -x ${panelName}`.quiet().nothrow();
  const wmOk = wm.exitCode === 0 && panel.exitCode === 0;
  results.push(
    wmOk
      ? { component: "Desktop session", status: "ok", detail: `${wmName} + ${panelName} running` }
      : {
          component: "Desktop session",
          status: "warn",
          detail: `${wmName}/${panelName} not running — restart: desktop-stop && desktop-start`,
        },
  );
}

// 8. Pentest vault
if (existsSync(`${PENTEST_CIPHER_DIR}/gocryptfs.conf`)) {
  const pentestMount = await $`mountpoint -q ${PENTEST_MOUNT}`.quiet().nothrow();
  if (pentestMount.exitCode === 0) {
    const du = await $`du -sh ${PENTEST_MOUNT}`.text();
    const size = du.split("\t")[0];
    results.push({ component: "Pentest vault", status: "ok", detail: `mounted (${size})` });
  } else {
    results.push({ component: "Pentest vault", status: "ok", detail: "locked (initialized)" });
  }
} else {
  results.push({ component: "Pentest vault", status: "ok", detail: "not initialized (optional)" });
}

// 8. Backup (S3/restic)
import {
  evaluateBackupGate,
  planDoctorBackupStatus,
  BACKUP_STATE_DIR,
} from "@sdw/core";

const backupGate = evaluateBackupGate(process.env);
const backupLastSuccessFile = `${BACKUP_STATE_DIR}/last-success`;
const backupFailuresFile = `${BACKUP_STATE_DIR}/consecutive-failures`;
const backupLastSuccess = existsSync(backupLastSuccessFile)
  ? readFileSync(backupLastSuccessFile, "utf-8").trim()
  : null;
const backupFailures = existsSync(backupFailuresFile)
  ? parseInt(readFileSync(backupFailuresFile, "utf-8").trim(), 10) || 0
  : 0;

const backupStatus = planDoctorBackupStatus({
  gateResult: backupGate,
  lastSuccessMs: backupLastSuccess ? new Date(backupLastSuccess).getTime() : null,
  consecutiveFailures: backupFailures,
  nowMs: Date.now(),
});

const backupDetail =
  backupStatus === "not-configured"
    ? "not configured (optional)"
    : backupStatus === "ok"
      ? `last: ${backupLastSuccess}`
      : backupStatus === "warn"
        ? backupLastSuccess
          ? `last: ${backupLastSuccess} (stale)`
          : "no backup recorded"
        : `${backupFailures} consecutive failures`;

results.push({
  component: "Backup (S3/restic)",
  status: backupStatus === "not-configured" ? "ok" : backupStatus,
  detail: backupDetail,
});

// Output
console.log("=== Workspace Health Check ===");
let hasFailure = false;
for (const r of results) {
  const icon = r.status === "ok" ? "✓" : r.status === "warn" ? "⚠" : "✗";
  console.log(`  ${icon} ${r.component}${r.detail ? ` — ${r.detail}` : ""}`);
  if (r.status === "fail") hasFailure = true;
}
console.log(hasFailure ? "\nSome checks failed." : "\nAll checks passed.");

// Warnings
if (!process.env.VAULT_GIT_REPO) {
  console.log("\n⚠ VAULT_GIT_REPO not configured — vault data is NOT backed up.");
  console.log("  If Docker Desktop updates or volumes are lost, data is gone.");
  console.log("  Set VAULT_GIT_REPO in .env to enable auto-backup.");
}
process.exit(hasFailure ? 1 : 0);
