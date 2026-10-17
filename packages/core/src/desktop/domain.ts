/**
 * Desktop domain: the supervisord topology of the remote-desktop stacks and the
 * start/stop ordering rules. Pure (no IO); the supervisorctl calls live in
 * @sdw/infra.
 *
 *   selkies  group "desktop": [desktop-xvfb, desktop-audio, desktop-selkies, desktop-xfce]
 *            standalone Xvfb (:1) + WS H.264 encoder (6080) + XFCE. Served at /desktop/.
 *   kasmvnc  group "vnc":     [desktop-xvnc, desktop-xfce-vnc]
 *            integrated Xkasmvnc (:2, X server + WS VNC on 6081) + XFCE. Served at /vnc/.
 *
 * Stop order is always the exact reverse of start order, with XFCE (the
 * primary HOME-fd holder on the gocryptfs mount) stopped first. That guarantees
 * no process holds /workspace/.desktop* fds before a FUSE unmount (EBUSY safety).
 */
import { SELKIES_STREAM_PORT, VNC_STREAM_PORT } from "./layout.ts";

/** A single desktop stack target. */
export type DesktopTarget = "selkies" | "vnc";

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
 * Which stacks an image contains, from the build-time marker file contents:
 *   "both"    → ["selkies", "vnc"]
 *   "selkies" → ["selkies"]
 *   "kasmvnc" → ["vnc"]
 * An absent or unrecognised marker means a legacy selkies-only image.
 */
export function parseStackMarker(raw: string | undefined): readonly DesktopTarget[] {
  switch (raw?.trim()) {
    case "both":
      return ["selkies", "vnc"];
    case "kasmvnc":
      return ["vnc"];
    default:
      return ["selkies"];
  }
}

/** Supervisord group name for a stack. */
export function desktopGroup(target: DesktopTarget): string {
  return GROUP[target];
}

/** WebSocket stream port for a stack. */
export function desktopStreamPort(target: DesktopTarget): number {
  return STREAM_PORT[target];
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
