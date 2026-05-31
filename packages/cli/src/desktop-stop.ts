#!/usr/bin/env bun
/**
 * desktop-stop — Stop the remote desktop session.
 */
import { runningDesktopPrograms, stopDesktop } from "./lib/desktop.ts";

await stopDesktop();

// Confirm all desktop processes stopped
const stillRunning = await runningDesktopPrograms();
if (stillRunning.length > 0) {
  console.warn("⚠ Some desktop processes still running:");
  for (const p of stillRunning) console.warn(`  ${p}`);
} else {
  console.log("Desktop stopped.");
}
