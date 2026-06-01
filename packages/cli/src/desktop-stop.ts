#!/usr/bin/env bun
/**
 * desktop-stop — Stop the remote desktop session(s).
 *
 * Usage:
 *   desktop-stop            stop every installed stack
 *   desktop-stop selkies    stop only the selkies stack
 *   desktop-stop vnc        stop only the kasmvnc stack
 */
import { type DesktopTarget, runningDesktopPrograms, stopDesktop } from "./lib/desktop.ts";

const arg = process.argv[2] as DesktopTarget | undefined;
if (arg && arg !== "selkies" && arg !== "vnc") {
  console.error(`ERROR: unknown target '${arg}'. Use 'selkies' or 'vnc'.`);
  process.exit(1);
}

await stopDesktop(arg);

// Confirm all desktop processes stopped
const stillRunning = await runningDesktopPrograms();
if (stillRunning.length > 0) {
  console.warn("⚠ Some desktop processes still running:");
  for (const p of stillRunning) console.warn(`  ${p}`);
} else {
  console.log("Desktop stopped.");
}
