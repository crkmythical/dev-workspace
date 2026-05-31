#!/usr/bin/env bun
/**
 * desktop-start — Start the on-demand remote desktop.
 */
import { mkdirSync } from "node:fs";
import {
  DESKTOP_CACHE_DIR,
  DESKTOP_HOME,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
import { $ } from "bun";
import { isDesktopRunning, startDesktop, waitForDesktopStream } from "./lib/desktop.ts";

const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mountCheck.exitCode !== 0) {
  console.error("ERROR: Vault not mounted. Run 'unlock-vault' first.");
  process.exit(1);
}

if (await isDesktopRunning()) {
  console.log("Desktop already running. Access at: /desktop/");
  process.exit(0);
}

mkdirSync(`${DESKTOP_HOME}/.local/share/applications`, { recursive: true });
mkdirSync(DESKTOP_CACHE_DIR, { recursive: true });

console.log("Starting desktop...");
await startDesktop();

if (await waitForDesktopStream(30)) {
  console.log("\n✓ Desktop ready. Access at: /desktop/\n");
} else {
  console.warn("⚠ Desktop started but stream port not ready after 30s. Check logs.");
}
