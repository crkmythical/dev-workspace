#!/usr/bin/env bun
import {
  CLASH_CONTROLLER_PORT,
  CLASH_HTTP_PORT,
  DESKTOP_DEFAULT_RESOLUTION,
} from "@sdw/core/constants";
/**
 * entrypoint-main — Container startup logic (called from entrypoint.sh)
 *
 * 1. Cleanup stale FUSE mounts
 * 2. Generate Clash config from template
 * 3. Configure git globals
 * 4. Tune inotify
 * 5. Start Clash and wait for readiness
 * 6. Wait for provider to load (url-test auto-selects fastest node)
 * 7. Probe egress
 * 8. Generate auth + vault + desktop + backup
 * 9. exec supervisord
 */
import { $ } from "bun";
import { envValidationError, findMissingRequiredEnv } from "./lib/env-validate.ts";
import { cleanupStaleMount } from "./lib/vault.ts";

// 0. Defense-in-depth env validation (mirrors entrypoint.sh pre-flight).
const missingEnv = findMissingRequiredEnv(process.env);
const envError = envValidationError(missingEnv);
if (envError) {
  console.error(envError);
  process.exit(1);
}

// 0b. Guarantee DESKTOP_RESOLUTION is set. supervisord expands
// `%(ENV_DESKTOP_RESOLUTION)s` in the desktop-xvnc command and FAILS to start
// the program if the variable is unset (it has no default-value syntax). The
// container env normally provides it (docker-compose), but a bare `docker run`
// or an edited compose would otherwise silently break the desktop. supervisord
// inherits this process env, so setting it here is the single safety net.
if (!process.env.DESKTOP_RESOLUTION) {
  process.env.DESKTOP_RESOLUTION = DESKTOP_DEFAULT_RESOLUTION;
}

// 1. Cleanup stale FUSE mounts from previous container lifecycle
await cleanupStaleMount("/workspace");
await cleanupStaleMount("/pentest/rootfs");

// 2. Generate Clash config from template
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
const subUrl = process.env.CLASH_SUBSCRIPTION_URL;
if (subUrl) {
  const templatePath = "/etc/clash/config.yaml.template";
  if (existsSync(templatePath)) {
    const template = readFileSync(templatePath, "utf-8");
    const config = template.replace("{{CLASH_SUBSCRIPTION_URL}}", subUrl);
    mkdirSync("/etc/clash/providers", { recursive: true });
    writeFileSync("/etc/clash/config.yaml", config);
    console.log("Clash config generated from template.");
  }
}

// 3. Git globals (optional — skip if not configured)
if (process.env.GIT_USER_NAME) {
  await $`git config --global user.name "${process.env.GIT_USER_NAME}"`.quiet();
}
if (process.env.GIT_USER_EMAIL) {
  await $`git config --global user.email "${process.env.GIT_USER_EMAIL}"`.quiet();
}
await $`git config --global http.proxy http://127.0.0.1:${CLASH_HTTP_PORT}`.quiet();
await $`git config --global https.proxy http://127.0.0.1:${CLASH_HTTP_PORT}`.quiet();

// 4. inotify
await $`sysctl -w fs.inotify.max_user_watches=524288`.quiet().nothrow();

// 5. Start Clash
console.log("Starting Clash...");
Bun.spawn(["/usr/bin/clash", "-d", "/etc/clash"], { stdout: "ignore", stderr: "ignore" });

let ready = false;
for (let i = 0; i < 30; i++) {
  const probe = await $`nc -z 127.0.0.1 ${CLASH_HTTP_PORT}`.quiet().nothrow();
  if (probe.exitCode === 0) {
    ready = true;
    break;
  }
  await Bun.sleep(1000);
}
console.log(ready ? "Clash ready." : "WARNING: Clash not ready after 30s.");

// 6. Wait for provider to load nodes (url-test auto-selects fastest)
if (ready) {
  let providerLoaded = false;
  for (let i = 0; i < 15; i++) {
    try {
      const resp = await fetch(
        `http://127.0.0.1:${CLASH_CONTROLLER_PORT}/providers/proxies/subscription`,
      );
      if (resp.ok) {
        const data = (await resp.json()) as any;
        const nodeCount = Object.keys(data.proxies || {}).length;
        if (nodeCount > 0) {
          providerLoaded = true;
          console.log(`Provider loaded: ${nodeCount} nodes (url-test auto-selecting fastest).`);
          break;
        }
      }
    } catch {}
    await Bun.sleep(2000);
  }
  if (!providerLoaded) {
    console.warn("WARNING: Provider did not load nodes within 30s (will retry via interval).");
  }

  // Point GLOBAL to auto-select (GLOBAL defaults to DIRECT which bypasses proxy)
  await fetch(`http://127.0.0.1:${CLASH_CONTROLLER_PORT}/proxies/GLOBAL`, {
    method: "PUT",
    body: JSON.stringify({ name: "auto-select" }),
    headers: { "Content-Type": "application/json" },
  }).catch(() => {});
}

// 7. Egress probe (after node selection, give Clash a moment to establish connection)
await Bun.sleep(2000);
let egress = false;
for (let i = 0; i < 3; i++) {
  const probe =
    await $`curl -fsSL --proxy http://127.0.0.1:${CLASH_HTTP_PORT} --max-time 10 https://www.google.com -o /dev/null`
      .quiet()
      .nothrow();
  if (probe.exitCode === 0) {
    egress = true;
    break;
  }
  await Bun.sleep(3000);
}
console.log(
  egress ? "Egress confirmed." : "WARNING: Egress probe failed (proxy nodes may be down).",
);

// 8. Generate auth for desktop and code-server
const csConfigPath = "/root/.config/code-server/config.yaml";
const envPassword = process.env.PASSWORD;
let masterPassword: string;
if (!existsSync(csConfigPath) || envPassword) {
  mkdirSync("/root/.config/code-server", { recursive: true });
  masterPassword = envPassword || crypto.randomUUID().replace(/-/g, "").slice(0, 24);
  writeFileSync(
    csConfigPath,
    `bind-addr: 127.0.0.1:8082\nauth: password\npassword: ${masterPassword}\ncert: false\n`,
  );
  if (!envPassword) {
    console.log(`Generated password: ${masterPassword}`);
    console.log("  (set PASSWORD in .env to use a fixed password)");
  }
} else {
  const csConfig = readFileSync(csConfigPath, "utf-8");
  const m = csConfig.match(/^password:\s*(.+)$/m);
  masterPassword = m ? m[1].trim() : crypto.randomUUID().replace(/-/g, "").slice(0, 24);
}

// Generate bcrypt hash for Caddy basic_auth
const hashProc = Bun.spawn(["caddy", "hash-password", "--plaintext", masterPassword], {
  stdout: "pipe",
  stderr: "pipe",
});
const hashOut = await new Response(hashProc.stdout).text();
await hashProc.exited;
const bcryptHash = hashOut.trim();
if (bcryptHash) {
  process.env.DESKTOP_BCRYPT_HASH = bcryptHash;
  console.log(`Auth configured (user: user, password: ${masterPassword}).`);
}

import { SELKIES_HOME, VAULT_CIPHER_DIR, VNC_HOME, WORKSPACE_MOUNT } from "@sdw/core/constants";
import { ensureKasmPasswd, installedStacks } from "./lib/desktop.ts";
// 8b. Auto-vault: init (if needed) + unlock (if not mounted) using masterPassword
import { initVault, isInitialized, mountVault } from "./lib/vault.ts";

const vaultInitialized = isInitialized(VAULT_CIPHER_DIR);
if (!vaultInitialized) {
  const ok = await initVault(VAULT_CIPHER_DIR, masterPassword);
  if (ok) console.log("Vault initialized (password = same as code-server).");
  else console.warn("WARNING: Vault init failed.");
}

const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
if (mountCheck.exitCode !== 0 && (vaultInitialized || isInitialized(VAULT_CIPHER_DIR))) {
  const { ok } = await mountVault(VAULT_CIPHER_DIR, WORKSPACE_MOUNT, masterPassword);
  if (ok) {
    console.log("Vault unlocked.");
    // 8c. Auto-start desktop(s): create per-stack HOME dirs + flip autostart=true
    // on every installed desktop group's supervisor conf so supervisord starts
    // them automatically. "both" mode → both desktop.conf and vnc.conf present.
    mkdirSync("/tmp/.desktop-cache", { recursive: true });
    const stacks = installedStacks();
    if (stacks.includes("selkies")) {
      mkdirSync(`${SELKIES_HOME}/.local/share/applications`, { recursive: true });
    }
    if (stacks.includes("vnc")) {
      mkdirSync(`${VNC_HOME}/.local/share/applications`, { recursive: true });
      // KasmVNC native HTTP Basic Auth reads $VNC_HOME/.kasmpasswd. Provision it
      // with the master password so the /vnc/ WS upgrade authenticates with the
      // same credentials as code-server/selkies (KasmVNC owns its WS auth).
      await ensureKasmPasswd(masterPassword);
    }
    // Patch every desktop group conf (one-shot sed; only files that exist).
    await $`sh -c "sed -i 's/autostart=false/autostart=true/g' /etc/supervisor/conf.d/desktop.conf /etc/supervisor/conf.d/vnc.conf 2>/dev/null || true"`
      .quiet()
      .nothrow();

    // 8d. Auto-start backup watcher (if configured via RESTIC_REPOSITORY)
    if (process.env.RESTIC_REPOSITORY) {
      // Auto-init repo if not yet initialized (idempotent).
      const initCheck = await $`restic snapshots --no-lock`.quiet().nothrow();
      if (initCheck.exitCode !== 0) {
        const stderr = (await new Response(initCheck.stderr).text()).toLowerCase();
        if (stderr.includes("wrong password") || stderr.includes("unable to open config")) {
          console.warn(
            "WARNING: Backup repo exists but RESTIC_PASSWORD does not match. Check .env.",
          );
        } else {
          const initResult = await $`restic init`.quiet().nothrow();
          if (initResult.exitCode === 0) {
            console.log("Backup repo initialized.");
          } else {
            console.warn("WARNING: Backup repo init failed (will retry on next start).");
          }
        }
      }
      // Flip autostart so supervisord starts the watcher
      await $`sed -i 's/autostart=false/autostart=true/' /etc/supervisor/conf.d/backup-watcher.conf`
        .quiet()
        .nothrow();
    }
  } else {
    console.warn("WARNING: Vault unlock failed (password mismatch?). Run 'unlock-vault' manually.");
  }
}

// 9. exec supervisord (explicitly pass the current environment so Caddy, a
// supervisord child, sees DESKTOP_BCRYPT_HASH for its basic_auth directive).
const stacks = installedStacks();
console.log("");
console.log("╔══════════════════════════════════════════════════╗");
console.log("║          Secure Dev Workspace Ready             ║");
console.log("╠══════════════════════════════════════════════════╣");
console.log(`║  Password:  ${masterPassword.padEnd(36)}║`);
console.log("║  Code:      http://localhost:18080               ║");
if (stacks.includes("selkies")) {
  console.log("║  Desktop:   http://localhost:18080/desktop/      ║");
}
if (stacks.includes("vnc")) {
  console.log("║  VNC:       http://localhost:18080/vnc/          ║");
}
console.log(`║  Auth:      user / ${masterPassword.padEnd(29)}║`);
console.log("╚══════════════════════════════════════════════════╝");
console.log("");
console.log("Starting supervisord...");
const proc = Bun.spawn(["/usr/bin/supervisord", "-c", "/etc/supervisor/supervisord.conf"], {
  stdout: "inherit",
  stderr: "inherit",
  env: { ...process.env },
});
await proc.exited;
