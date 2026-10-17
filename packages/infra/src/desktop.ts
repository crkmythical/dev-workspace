/**
 * Desktop adapter: supervisorctl control of the remote-desktop process groups.
 * The topology and ordering rules are pure and live in @sdw/core/desktop.
 *
 * The image is built with DESKTOP_STACK={both|selkies|kasmvnc}, recorded in
 * /etc/sdw-desktop-stack; installedStacks() reads that marker so every caller
 * (desktop-start, desktop-stop, lock-vault, destroy, doctor) sees the stacks
 * that actually exist without knowing how the image was built.
 */
import { existsSync, readFileSync } from "node:fs";
import { VNC_HOME } from "@sdw/core/constants";
import {
  type DesktopTarget,
  desktopAnchor,
  desktopGroup,
  desktopStartOrder,
  desktopStopOrder,
  desktopStreamPort,
  parseStackMarker,
} from "@sdw/core/desktop";
import { $ } from "bun";

/** Path to the build-time stack marker written by the Dockerfile. */
const STACK_MARKER_PATH = "/etc/sdw-desktop-stack";

/** Which stacks are installed in this image. */
export function installedStacks(): readonly DesktopTarget[] {
  try {
    const raw = existsSync(STACK_MARKER_PATH)
      ? readFileSync(STACK_MARKER_PATH, "utf-8")
      : undefined;
    return parseStackMarker(raw);
  } catch {
    return parseStackMarker(undefined);
  }
}

/** Resolve a caller target argument to the concrete list of stacks to act on. */
function resolveTargets(target?: DesktopTarget): readonly DesktopTarget[] {
  const installed = installedStacks();
  if (!target) return installed;
  return installed.includes(target) ? [target] : [];
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
  const groups = [...new Set(installedStacks().map((t) => desktopGroup(t)))];
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
  const port = desktopStreamPort(target);
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
