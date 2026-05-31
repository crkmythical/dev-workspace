/**
 * Desktop service control — single source of truth for the remote-desktop
 * supervisord process group, across BOTH desktop stacks.
 *
 * The active stack is fixed at image build time (DESKTOP_STACK build arg) and
 * recorded in /etc/sdw-desktop-stack. This module reads that marker once and
 * exposes a stack-aware topology so every caller (desktop-start, desktop-stop,
 * lock-vault, destroy, doctor) sees the correct program group, start order, and
 * stop order without knowing which stack is installed.
 *
 *   selkies  — [desktop-xvfb, desktop-audio, desktop-selkies, desktop-xfce]
 *              standalone Xvfb + WS H.264 encoder + XFCE session
 *   kasmvnc  — [desktop-xvnc, desktop-xfce]
 *              integrated Xkasmvnc (X server + WS VNC) + XFCE session
 *
 * Both stacks use XFCE as the desktop session. Stop order is always the exact
 * reverse of start order. XFCE (the primary HOME-fd holder on the gocryptfs
 * mount) is always stopped first, guaranteeing no process holds
 * /workspace/.desktop fds before FUSE unmount (EBUSY safety).
 * Both stacks bind the WebSocket on DESKTOP_STREAM_PORT (6080).
 */
import { existsSync, readFileSync } from "node:fs";
import { DESKTOP_STREAM_PORT } from "@sdw/core/constants";
import { $ } from "bun";

export type DesktopStack = "selkies" | "kasmvnc";

/** Path to the build-time stack marker written by the Dockerfile. */
const STACK_MARKER_PATH = "/etc/sdw-desktop-stack";

/**
 * The desktop stack baked into this image. Reads /etc/sdw-desktop-stack once;
 * defaults to "selkies" if the marker is absent (selkies is the default build).
 */
export function desktopStack(): DesktopStack {
  try {
    if (existsSync(STACK_MARKER_PATH)) {
      const v = readFileSync(STACK_MARKER_PATH, "utf-8").trim();
      if (v === "kasmvnc") return "kasmvnc";
    }
  } catch {
    // fall through to default
  }
  return "selkies";
}

export const DESKTOP_GROUP = "desktop";

/** Program members per stack (group-qualified names for supervisorctl). */
const STACK_PROGRAMS: Record<DesktopStack, readonly string[]> = {
  // Start order: X server → audio (optional, exits if disabled) → stream encoder → session.
  selkies: [
    `${DESKTOP_GROUP}:desktop-xvfb`,
    `${DESKTOP_GROUP}:desktop-audio`,
    `${DESKTOP_GROUP}:desktop-selkies`,
    `${DESKTOP_GROUP}:desktop-xfce`,
  ],
  // Start order: integrated X/VNC server → session.
  // Xkasmvnc is both X server and WS server (no separate Xvfb needed).
  kasmvnc: [
    `${DESKTOP_GROUP}:desktop-xvnc`,
    `${DESKTOP_GROUP}:desktop-xfce`,
  ],
};

/** Group-qualified name of the base program (the X server) for the active stack. */
export function desktopAnchor(stack: DesktopStack = desktopStack()): string {
  return STACK_PROGRAMS[stack][0];
}

/** Start order for the active stack (base/X server first). */
export function desktopStartOrder(stack: DesktopStack = desktopStack()): readonly string[] {
  return STACK_PROGRAMS[stack];
}

/** Stop order for the active stack: exact reverse of start order. */
export function desktopStopOrder(stack: DesktopStack = desktopStack()): readonly string[] {
  return [...STACK_PROGRAMS[stack]].reverse();
}

/** Whether the desktop X server (base of the dependency chain) is RUNNING. */
export async function isDesktopRunning(): Promise<boolean> {
  const r = await $`supervisorctl status ${desktopAnchor()}`.quiet().nothrow();
  return r.exitCode === 0 && r.text().includes("RUNNING");
}

/** Start the desktop group in dependency order. Caller handles readiness. */
export async function startDesktop(): Promise<void> {
  const order = desktopStartOrder();
  await $`supervisorctl start ${order[0]}`.quiet().nothrow();
  await Bun.sleep(2000);
  for (let i = 1; i < order.length; i++) {
    await $`supervisorctl start ${order[i]}`.quiet().nothrow();
  }
}

/** Stop the entire desktop group. Safe to call when not running. */
export async function stopDesktop(): Promise<void> {
  const order = desktopStopOrder();
  await $`supervisorctl stop ${order.join(" ")}`.quiet().nothrow();
}

/** Names of any desktop programs still RUNNING (for post-stop verification). */
export async function runningDesktopPrograms(): Promise<string[]> {
  const r = await $`supervisorctl status ${DESKTOP_GROUP}:*`.quiet().nothrow();
  return r
    .text()
    .split("\n")
    .filter((l) => l.includes("RUNNING"))
    .map((l) => l.split(/\s+/)[0])
    .filter(Boolean);
}

/** Poll until the desktop WebSocket port accepts TCP connections, or timeout. */
export async function waitForDesktopStream(timeoutSec = 30): Promise<boolean> {
  for (let i = 0; i < timeoutSec; i++) {
    const probe = await $`nc -z 127.0.0.1 ${DESKTOP_STREAM_PORT}`.quiet().nothrow();
    if (probe.exitCode === 0) return true;
    await Bun.sleep(1000);
  }
  return false;
}
