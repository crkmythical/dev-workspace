#!/usr/bin/env bun
/**
 * desktop-start — Start the on-demand KasmVNC remote desktop.
 */
import { cpSync, existsSync, mkdirSync } from "node:fs";
import {
  DESKTOP_CACHE_DIR,
  DESKTOP_CONFIG_DIR,
  DESKTOP_HOME,
  KASMVNC_PORT,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
import { $ } from "bun";

// 1. Check vault is mounted
const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mountCheck.exitCode !== 0) {
  console.error("ERROR: Vault not mounted. Run 'unlock-vault' first.");
  process.exit(1);
}

// 2. Check if desktop is already running
const statusCheck = await $`supervisorctl status desktop:desktop-xvnc`.quiet().nothrow();
if (statusCheck.exitCode === 0 && statusCheck.text().includes("RUNNING")) {
  console.log("Desktop already running. Access at: /desktop/");
  process.exit(0);
}

// 3. Initialize desktop config directories
const configDirs = [
  `${DESKTOP_CONFIG_DIR}/openbox`,
  `${DESKTOP_CONFIG_DIR}/tint2`,
  `${DESKTOP_CONFIG_DIR}/kasmvnc`,
  `${DESKTOP_HOME}/.local/share/applications`,
  DESKTOP_CACHE_DIR,
];
for (const dir of configDirs) mkdirSync(dir, { recursive: true });

// Copy default configs if not present
const defaults = "/opt/workspace/image/etc/desktop";
if (
  !existsSync(`${DESKTOP_CONFIG_DIR}/openbox/rc.xml`) &&
  existsSync(`${defaults}/openbox-rc.xml`)
) {
  cpSync(`${defaults}/openbox-rc.xml`, `${DESKTOP_CONFIG_DIR}/openbox/rc.xml`);
}
if (!existsSync(`${DESKTOP_CONFIG_DIR}/tint2/tint2rc`) && existsSync(`${defaults}/tint2rc`)) {
  cpSync(`${defaults}/tint2rc`, `${DESKTOP_CONFIG_DIR}/tint2/tint2rc`);
}
if (
  !existsSync(`${DESKTOP_CONFIG_DIR}/kasmvnc/kasmvnc.yaml`) &&
  existsSync(`${defaults}/kasmvnc-defaults.yaml`)
) {
  cpSync(`${defaults}/kasmvnc-defaults.yaml`, `${DESKTOP_CONFIG_DIR}/kasmvnc/kasmvnc.yaml`);
}

// 4. Start desktop group
console.log("Starting desktop...");
await $`supervisorctl start desktop:desktop-xvnc desktop:desktop-openbox desktop:desktop-tint2`.quiet().nothrow();

// 5. Wait for KasmVNC port
let ready = false;
for (let i = 0; i < 30; i++) {
  const probe = await $`nc -z 127.0.0.1 ${KASMVNC_PORT}`.quiet().nothrow();
  if (probe.exitCode === 0) {
    ready = true;
    break;
  }
  await Bun.sleep(1000);
}

if (ready) {
  console.log("\n✓ Desktop ready. Access at: /desktop/\n");
} else {
  console.warn("⚠ Desktop started but KasmVNC port not ready after 30s. Check logs.");
}
