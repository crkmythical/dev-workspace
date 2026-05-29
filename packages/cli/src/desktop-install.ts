#!/usr/bin/env bun
/**
 * desktop-install — Install GUI applications into the remote desktop.
 *
 * Usage: desktop-install <app-name>
 * Supported: firefox, chromium, idea, burpsuite, wireshark
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { DESKTOP_APPS_DIR, DESKTOP_APPS_REGISTRY, DESKTOP_HOME } from "@sdw/core/constants";
import { $ } from "bun";

interface AppDef {
  name: string;
  type: "apt" | "download";
  package?: string;
  url?: string;
  extractTo?: string;
  binary?: string;
  desktopEntry: string;
}

const APPS: Record<string, AppDef> = {
  firefox: {
    name: "Firefox",
    type: "apt",
    package: "firefox-esr",
    binary: "firefox-esr",
    desktopEntry: `[Desktop Entry]
Name=Firefox
Exec=firefox-esr %u
Icon=firefox-esr
Type=Application
Categories=Network;WebBrowser;`,
  },
  chromium: {
    name: "Chromium",
    type: "apt",
    package: "chromium",
    binary: "chromium",
    desktopEntry: `[Desktop Entry]
Name=Chromium
Exec=chromium --no-sandbox %u
Icon=chromium
Type=Application
Categories=Network;WebBrowser;`,
  },
  wireshark: {
    name: "Wireshark",
    type: "apt",
    package: "wireshark",
    binary: "wireshark",
    desktopEntry: `[Desktop Entry]
Name=Wireshark
Exec=wireshark
Icon=wireshark
Type=Application
Categories=Network;`,
  },
  idea: {
    name: "IntelliJ IDEA",
    type: "download",
    url: "https://download.jetbrains.com/idea/ideaIC-2024.1.tar.gz",
    extractTo: `${DESKTOP_APPS_DIR}/idea`,
    binary: `${DESKTOP_APPS_DIR}/idea/bin/idea.sh`,
    desktopEntry: `[Desktop Entry]
Name=IntelliJ IDEA
Exec=${DESKTOP_APPS_DIR}/idea/bin/idea.sh
Icon=${DESKTOP_APPS_DIR}/idea/bin/idea.svg
Type=Application
Categories=Development;IDE;`,
  },
  burpsuite: {
    name: "Burp Suite",
    type: "download",
    url: "https://portswigger-cdn.net/burp/releases/download?product=community&type=Jar",
    extractTo: `${DESKTOP_APPS_DIR}/burpsuite`,
    binary: `java -jar ${DESKTOP_APPS_DIR}/burpsuite/burpsuite.jar`,
    desktopEntry: `[Desktop Entry]
Name=Burp Suite
Exec=java -jar ${DESKTOP_APPS_DIR}/burpsuite/burpsuite.jar
Icon=burp
Type=Application
Categories=Network;Security;`,
  },
};

const appName = process.argv[2]?.toLowerCase();
if (!appName || !APPS[appName]) {
  console.log("Usage: desktop-install <app>");
  console.log(`Available: ${Object.keys(APPS).join(", ")}`);
  process.exit(1);
}

const app = APPS[appName];

// Check Clash proxy is available
const proxyCheck = await $`nc -z 127.0.0.1 7890`.quiet().nothrow();
if (proxyCheck.exitCode !== 0) {
  console.warn("⚠ Clash proxy not available. Downloads may fail.");
}

console.log(`Installing ${app.name}...`);

if (app.type === "apt") {
  const result =
    await $`apt-get update && apt-get install -y --no-install-recommends ${app.package}`.nothrow();
  if (result.exitCode !== 0) {
    console.error(`ERROR: Failed to install ${app.package}`);
    process.exit(1);
  }
} else if (app.type === "download" && app.url && app.extractTo) {
  mkdirSync(app.extractTo, { recursive: true });
  if (app.url.endsWith(".tar.gz") || app.url.includes("tar.gz")) {
    const dl =
      await $`curl -fsSL ${app.url} | tar xz -C ${app.extractTo} --strip-components=1`.nothrow();
    if (dl.exitCode !== 0) {
      console.error(`ERROR: Failed to download/extract ${app.name}`);
      process.exit(1);
    }
  } else {
    // JAR or single file download
    const filename = appName === "burpsuite" ? "burpsuite.jar" : appName;
    const dl = await $`curl -fsSL ${app.url} -o ${app.extractTo}/${filename}`.nothrow();
    if (dl.exitCode !== 0) {
      console.error(`ERROR: Failed to download ${app.name}`);
      process.exit(1);
    }
  }
}

// Create .desktop file
const appsDir = `${DESKTOP_HOME}/.local/share/applications`;
mkdirSync(appsDir, { recursive: true });
writeFileSync(`${appsDir}/${appName}.desktop`, app.desktopEntry);

// Update registry
let registry: string[] = [];
try {
  registry = JSON.parse(readFileSync(DESKTOP_APPS_REGISTRY, "utf-8"));
} catch {}
if (!registry.includes(appName)) {
  registry.push(appName);
  writeFileSync(DESKTOP_APPS_REGISTRY, JSON.stringify(registry, null, 2));
}

console.log(`✓ Installed ${app.name}. Launch from desktop menu or: ${app.binary}`);
