#!/usr/bin/env bun
/**
 * desktop-start — Start the on-demand remote desktop(s).
 *
 * Usage:
 *   desktop-start            start every installed stack (selkies + vnc in "both" mode)
 *   desktop-start selkies    start only the selkies stack (/desktop/)
 *   desktop-start vnc        start only the kasmvnc stack (/vnc/)
 */
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import {
  DESKTOP_CACHE_DIR,
  SELKIES_HOME,
  VNC_HOME,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
import { $ } from "bun";
import {
  type DesktopTarget,
  ensureKasmPasswd,
  installedStacks,
  isDesktopRunning,
  startDesktop,
  waitForDesktopStream,
} from "./lib/desktop.ts";

const URL_FOR: Record<DesktopTarget, string> = {
  selkies: "/desktop/",
  vnc: "/vnc/",
};

/**
 * Read the master password from code-server's config (the runtime SSOT) so a
 * manual `desktop-start vnc` can provision KasmVNC's .kasmpasswd if missing
 * (the entrypoint already does this on auto-start). Returns "" if unavailable.
 */
function masterPasswordFromConfig(): string {
  const csConfigPath = "/root/.config/code-server/config.yaml";
  if (!existsSync(csConfigPath)) return "";
  const m = readFileSync(csConfigPath, "utf-8").match(/^password:\s*(.+)$/m);
  return m ? m[1].trim() : "";
}

const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mountCheck.exitCode !== 0) {
  console.error("ERROR: Vault not mounted. Run 'unlock-vault' first.");
  process.exit(1);
}

// Resolve which stacks to start from the CLI arg (default: all installed).
const arg = process.argv[2] as DesktopTarget | undefined;
if (arg && arg !== "selkies" && arg !== "vnc") {
  console.error(`ERROR: unknown target '${arg}'. Use 'selkies' or 'vnc'.`);
  process.exit(1);
}
const installed = installedStacks();
if (arg && !installed.includes(arg)) {
  console.error(`ERROR: stack '${arg}' is not installed in this image.`);
  process.exit(1);
}
const targets = arg ? [arg] : installed;

// Shared cache dir + per-stack HOME dirs.
mkdirSync(DESKTOP_CACHE_DIR, { recursive: true });
if (targets.includes("selkies")) {
  mkdirSync(`${SELKIES_HOME}/.local/share/applications`, { recursive: true });
}
if (targets.includes("vnc")) {
  mkdirSync(`${VNC_HOME}/.local/share/applications`, { recursive: true });
  // Provision KasmVNC's native auth file if missing (entrypoint does this on
  // auto-start; this covers a manual `desktop-start vnc`).
  if (!existsSync(`${VNC_HOME}/.kasmpasswd`)) {
    await ensureKasmPasswd(masterPasswordFromConfig());
  }
}

for (const target of targets) {
  if (await isDesktopRunning(target)) {
    console.log(`Desktop [${target}] already running. Access at: ${URL_FOR[target]}`);
    continue;
  }
  console.log(`Starting desktop [${target}]...`);
  await startDesktop(target);
  if (await waitForDesktopStream(target, 30)) {
    console.log(`✓ Desktop [${target}] ready. Access at: ${URL_FOR[target]}`);
  } else {
    console.warn(`⚠ Desktop [${target}] started but stream port not ready after 30s.`);
  }
}
