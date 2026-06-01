/**
 * Desktop service control — single source of truth for the remote-desktop
 * supervisord process groups, across BOTH desktop stacks running in parallel.
 *
 * The image is built with DESKTOP_STACK={both|selkies|kasmvnc} (default both),
 * recorded in /etc/sdw-desktop-stack. This module reads that marker once and
 * exposes a stack-aware topology so every caller (desktop-start, desktop-stop,
 * lock-vault, destroy, doctor) sees the correct program groups, start order,
 * and stop order without knowing which stack(s) are installed.
 *
 *   selkies  — group "desktop": [desktop-xvfb, desktop-audio, desktop-selkies, desktop-xfce]
 *              standalone Xvfb (:1) + WS H.264 encoder (6080) + XFCE session. Served at /desktop/.
 *   kasmvnc  — group "vnc":     [desktop-xvnc, desktop-xfce-vnc]
 *              integrated Xkasmvnc (:2, X server + WS VNC on 6081) + XFCE session. Served at /vnc/.
 *
 * In "both" mode both groups exist and run simultaneously on independent
 * displays with independent HOME dirs. Stop order is always the exact reverse
 * of start order, with XFCE (the primary HOME-fd holder on the gocryptfs mount)
 * stopped first, guaranteeing no process holds /workspace/.desktop* fds before
 * FUSE unmount (EBUSY safety).
 */
import { existsSync, readFileSync } from "node:fs";
import { SELKIES_STREAM_PORT, VNC_HOME, VNC_STREAM_PORT } from "@sdw/core/constants";
import { $ } from "bun";

/** A single desktop stack target. */
export type DesktopTarget = "selkies" | "vnc";

/** Path to the build-time stack marker written by the Dockerfile. */
const STACK_MARKER_PATH = "/etc/sdw-desktop-stack";

/** Per-target supervisord group name. */
const GROUP: Record<DesktopTarget, string> = {
  selkies: "desktop",
  vnc: "vnc",
};

/** Per-target WebSocket stream port (for readiness probes). */
const STREAM_PORT: Record<DesktopTarget, number> = {
  selkies: SELKIES_STREAM_PORT,
  vnc: VNC_STREAM_PORT,
};

/**
 * Program members per stack, in START order (group-qualified for supervisorctl).
 * Stop order is the exact reverse. XFCE is always last to start / first to stop.
 */
const STACK_PROGRAMS: Record<DesktopTarget, readonly string[]> = {
  // X server → audio (optional, exits if disabled) → stream encoder → session.
  selkies: [
    "desktop:desktop-xvfb",
    "desktop:desktop-audio",
    "desktop:desktop-selkies",
    "desktop:desktop-xfce",
  ],
  // Integrated X/VNC server (:2) → session. Xkasmvnc is both X server and WS server.
  vnc: ["vnc:desktop-xvnc", "vnc:desktop-xfce-vnc"],
};

/**
 * Which stacks are installed in this image. Reads /etc/sdw-desktop-stack:
 *   "both"    → ["selkies", "vnc"]
 *   "selkies" → ["selkies"]
 *   "kasmvnc" → ["vnc"]
 * Defaults to ["selkies"] if the marker is absent (selkies-only legacy images).
 */
export function installedStacks(): readonly DesktopTarget[] {
  try {
    if (existsSync(STACK_MARKER_PATH)) {
      const v = readFileSync(STACK_MARKER_PATH, "utf-8").trim();
      if (v === "both") return ["selkies", "vnc"];
      if (v === "kasmvnc") return ["vnc"];
      if (v === "selkies") return ["selkies"];
    }
  } catch {
    // fall through to default
  }
  return ["selkies"];
}

/** Resolve a caller target argument to the concrete list of stacks to act on. */
function resolveTargets(target?: DesktopTarget): readonly DesktopTarget[] {
  const installed = installedStacks();
  if (!target) return installed;
  return installed.includes(target) ? [target] : [];
}

/** Group-qualified name of the base program (X server) for a stack. */
export function desktopAnchor(target: DesktopTarget): string {
  return STACK_PROGRAMS[target][0];
}

/** Start order for a stack (base/X server first). */
export function desktopStartOrder(target: DesktopTarget): readonly string[] {
  return STACK_PROGRAMS[target];
}

/** Stop order for a stack: exact reverse of start order. */
export function desktopStopOrder(target: DesktopTarget): readonly string[] {
  return [...STACK_PROGRAMS[target]].reverse();
}

/** Whether a specific stack's X server (base of the chain) is RUNNING. */
export async function isDesktopRunning(target: DesktopTarget): Promise<boolean> {
  const r = await $`supervisorctl status ${desktopAnchor(target)}`.quiet().nothrow();
  return r.exitCode === 0 && r.text().includes("RUNNING");
}

/** Whether ANY installed stack is currently running. */
export async function isAnyDesktopRunning(): Promise<boolean> {
  for (const t of installedStacks()) {
    if (await isDesktopRunning(t)) return true;
  }
  return false;
}

/** Start one stack in dependency order. Caller handles readiness. */
async function startStack(target: DesktopTarget): Promise<void> {
  const order = desktopStartOrder(target);
  await $`supervisorctl start ${order[0]}`.quiet().nothrow();
  await Bun.sleep(2000);
  for (let i = 1; i < order.length; i++) {
    await $`supervisorctl start ${order[i]}`.quiet().nothrow();
  }
}

/**
 * Start desktop(s). With no argument, starts every installed stack (default
 * for "both" mode). Pass a target to start only that stack.
 */
export async function startDesktop(target?: DesktopTarget): Promise<void> {
  for (const t of resolveTargets(target)) {
    await startStack(t);
  }
}

/** Stop one stack (reverse dependency order). Safe when not running. */
async function stopStack(target: DesktopTarget): Promise<void> {
  const order = desktopStopOrder(target);
  await $`supervisorctl stop ${order.join(" ")}`.quiet().nothrow();
}

/**
 * Stop desktop(s). With no argument, stops every installed stack. Pass a target
 * to stop only that stack.
 */
export async function stopDesktop(target?: DesktopTarget): Promise<void> {
  for (const t of resolveTargets(target)) {
    await stopStack(t);
  }
}

/**
 * Stop ALL desktop stacks before a FUSE unmount (lock-vault / destroy).
 * Critical for EBUSY safety: every XFCE session holding a HOME fd on the
 * gocryptfs mount must exit first.
 */
export async function stopAllDesktops(): Promise<void> {
  for (const t of installedStacks()) {
    await stopStack(t);
  }
}

/** Names of any desktop programs still RUNNING across all groups (post-stop check). */
export async function runningDesktopPrograms(): Promise<string[]> {
  const groups = [...new Set(installedStacks().map((t) => GROUP[t]))];
  const running: string[] = [];
  for (const g of groups) {
    const r = await $`supervisorctl status ${g}:*`.quiet().nothrow();
    running.push(
      ...r
        .text()
        .split("\n")
        .filter((l) => l.includes("RUNNING"))
        .map((l) => l.split(/\s+/)[0])
        .filter(Boolean),
    );
  }
  return running;
}

/** Poll until a stack's WebSocket port accepts TCP connections, or timeout. */
export async function waitForDesktopStream(
  target: DesktopTarget,
  timeoutSec = 30,
): Promise<boolean> {
  const port = STREAM_PORT[target];
  for (let i = 0; i < timeoutSec; i++) {
    const probe = await $`nc -z 127.0.0.1 ${port}`.quiet().nothrow();
    if (probe.exitCode === 0) return true;
    await Bun.sleep(1000);
  }
  return false;
}

/**
 * Write KasmVNC's native HTTP Basic Auth password file at $VNC_HOME/.kasmpasswd
 * (user "user") from the given plaintext password. KasmVNC owns the /vnc/ and
 * /websockify auth end-to-end (Caddy does not gate it — browsers won't replay
 * Caddy creds to a JS WebSocket handshake), so this file must exist for the VNC
 * desktop to authenticate. Idempotent: `kasmvncpasswd -w -r` overwrites cleanly.
 *
 * Shared by the entrypoint (auto-start path) and `desktop-start` (manual path)
 * so the credential-provisioning logic lives in exactly one place.
 */
export async function ensureKasmPasswd(password: string): Promise<void> {
  if (!password) return;
  const kasmpasswd = `${VNC_HOME}/.kasmpasswd`;
  const proc = Bun.spawn(["kasmvncpasswd", "-u", "user", "-w", "-r", kasmpasswd], {
    stdin: "pipe",
    stdout: "ignore",
    stderr: "ignore",
  });
  proc.stdin.write(`${password}\n${password}\n`);
  await proc.stdin.end();
  await proc.exited;
  await $`chmod 0600 ${kasmpasswd}`.quiet().nothrow();
}
