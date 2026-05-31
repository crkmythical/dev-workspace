#!/usr/bin/env bun
import {
  CLASH_CONTROLLER_PORT,
  CLASH_HTTP_PORT,
  CLASH_SOCKS_PORT,
  DESKTOP_DEFAULT_RESOLUTION,
} from "@sdw/core/constants";
/**
 * entrypoint-main — Container startup logic (called from entrypoint.sh)
 *
 * 1. Cleanup stale FUSE mounts
 * 2. Fetch Clash subscription
 * 3. Configure git globals
 * 4. Tune inotify
 * 5. Start Clash and wait for readiness
 * 6. Probe egress
 * 7. exec supervisord
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

// 2. Fetch subscription (must bypass proxy since Clash isn't ready yet)
const subUrl = process.env.CLASH_SUBSCRIPTION_URL;
if (subUrl) {
  const fetchProc = Bun.spawn(
    [
      "curl",
      "-fsSL",
      "--noproxy",
      "*",
      "--max-time",
      "30",
      subUrl,
      "-o",
      "/etc/clash/config.yaml.new",
    ],
    {
      stdout: "pipe",
      stderr: "pipe",
      env: {
        ...process.env,
        http_proxy: "",
        https_proxy: "",
        all_proxy: "",
        HTTP_PROXY: "",
        HTTPS_PROXY: "",
        ALL_PROXY: "",
      },
    },
  );
  const fetchExit = await fetchProc.exited;
  if (fetchExit === 0) {
    await $`mv /etc/clash/config.yaml.new /etc/clash/config.yaml`.quiet();
    // Force global mode in the config file (survives Clash restarts)
    await $`sed -i 's/^mode:.*/mode: global/' /etc/clash/config.yaml`.quiet().nothrow();
    console.log("Clash subscription updated.");
  } else {
    console.warn("WARNING: Subscription fetch failed, using cached config.");
    await $`rm -f /etc/clash/config.yaml.new`.quiet();
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

// 6. Set global mode + auto-select proxy node
if (ready) {
  // Switch to global mode: ALL traffic goes through the selected proxy node
  await fetch(`http://127.0.0.1:${CLASH_CONTROLLER_PORT}/configs`, {
    method: "PATCH",
    body: JSON.stringify({ mode: "global" }),
    headers: { "Content-Type": "application/json" },
  }).catch(() => {});

  try {
    const resp = await fetch(`http://127.0.0.1:${CLASH_CONTROLLER_PORT}/proxies`);
    if (resp.ok) {
      const data = (await resp.json()) as any;
      const selector = data.proxies?.["🚀 节点选择"];
      if (selector && selector.now === "🎯 全球直连" && selector.all?.length > 1) {
        const autoGroup = selector.all.find(
          (n: string) => n.includes("自动") || n.includes("auto") || n.includes("url-test"),
        );
        // Prefer real proxy nodes with region identifiers over generic "优选域名" nodes
        const regionNode = selector.all.find(
          (n: string) =>
            (n.includes("HKG") || n.includes("SGP") || n.includes("US") || n.includes("JP") ||
             n.includes("香港") || n.includes("新加坡") || n.includes("美国") || n.includes("日本") ||
             n.includes("移动") || n.includes("联通") || n.includes("电信")) &&
            !n.includes("直连") && !n.includes("DIRECT"),
        );
        const proxyNode =
          autoGroup ||
          regionNode ||
          selector.all.find(
            (n: string) =>
              !n.includes("直连") &&
              !n.includes("DIRECT") &&
              !n.includes("拦截") &&
              !n.includes("REJECT") &&
              !n.includes("净化") &&
              !n.includes("漏网"),
          );
        if (proxyNode) {
          await fetch(
            `http://127.0.0.1:${CLASH_CONTROLLER_PORT}/proxies/%F0%9F%9A%80%20%E8%8A%82%E7%82%B9%E9%80%89%E6%8B%A9`,
            {
              method: "PUT",
              body: JSON.stringify({ name: proxyNode }),
            },
          );
          console.log(`Proxy auto-selected: ${proxyNode}`);
        }
      }
    }
  } catch {}
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
console.log(egress ? "Egress confirmed." : "WARNING: Egress probe failed (proxy nodes may be down).");

// 8. Generate auth for desktop and code-server
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
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

// 8b. Auto-vault: init (if needed) + unlock (if not mounted) using masterPassword
import { initVault, isInitialized, mountVault } from "./lib/vault.ts";
import { VAULT_CIPHER_DIR, WORKSPACE_MOUNT } from "@sdw/core/constants";
import { startDesktop, waitForDesktopStream } from "./lib/desktop.ts";

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
    // 8c. Auto-start desktop: enable autostart so supervisord starts it automatically
    mkdirSync(`${WORKSPACE_MOUNT}/.desktop/.local/share/applications`, { recursive: true });
    mkdirSync("/tmp/.desktop-cache", { recursive: true });
    // Patch supervisor desktop config to autostart=true (one-shot sed)
    await $`sed -i 's/autostart=false/autostart=true/g' /etc/supervisor/conf.d/desktop.conf`.quiet().nothrow();
  } else {
    console.warn("WARNING: Vault unlock failed (password mismatch?). Run 'unlock-vault' manually.");
  }
}

// 9. exec supervisord (explicitly pass the current environment so Caddy, a
// supervisord child, sees DESKTOP_BCRYPT_HASH for its basic_auth directive).
console.log("");
console.log("╔══════════════════════════════════════════════════╗");
console.log("║          Secure Dev Workspace Ready             ║");
console.log("╠══════════════════════════════════════════════════╣");
console.log(`║  Password:  ${masterPassword.padEnd(36)}║`);
console.log(`║  Code:      http://localhost:18080               ║`);
console.log(`║  Desktop:   http://localhost:18080/desktop/      ║`);
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
