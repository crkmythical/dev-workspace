#!/usr/bin/env bun
/**
 * entrypoint-main — container start-up (called from entrypoint.sh).
 *
 * The ORDER of the steps, what a failure of each one means (abort / degrade) and which
 * ones need the vault are decided in core (BOOT_PLAN) and tested there. This file only
 * binds each step id to its effects; the compiler insists on exactly one per id.
 *
 * Egress (Clash) is best effort: if it is not confirmed the workspace still starts,
 * and the banner, /var/run/egress-status, `doctor` and (once the vault is open) a
 * note in .notifications say so.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { classifyRepoProbe } from "@sdw/core/backup";
import { DESKTOP_CACHE_DIR, SELKIES_HOME, VNC_HOME } from "@sdw/core/desktop";
import {
  CLASH_CONFIG_PATH,
  CLASH_HTTP_PORT,
  CLASH_PROVIDERS_DIR,
  CLASH_TEMPLATE_PATH,
  bringUpEgress,
  renderClashConfig,
} from "@sdw/core/egress";
import { WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import { PENTEST_MOUNT, VAULT_CIPHER_DIR, WORKSPACE_VAULT, unlockVault } from "@sdw/core/vault";
import {
  BOOT_PLAN,
  type BootState,
  type BootStepImpls,
  CODE_SERVER_CONFIG_DIR,
  CODE_SERVER_CONFIG_PATH,
  CODE_SERVER_PORT,
  SUPERVISORD_BIN,
  SUPERVISORD_CONF,
  envValidationError,
  findMissingRequiredEnv,
  loadConfig,
  renderBanner,
  renderCodeServerConfig,
  resolveMasterPassword,
  runBoot,
  supervisorConf,
} from "@sdw/core/workspace";
import {
  caddyPasswordHash,
  clashEgressPorts,
  cleanupStaleMount,
  enableAutostart,
  ensureKasmPasswd,
  gocryptfsVolume,
  initVault,
  installedStacks,
  isInitialized,
  syncEgressNotice,
  workspaceUnlockHooks,
  writeEgressStatus,
} from "@sdw/infra";
import { $ } from "bun";

const config = loadConfig(process.env);

/** Set by resolve-auth; every later step that needs the password reads it from here. */
let masterPassword = "";

async function gitConfig(key: string, value: string): Promise<void> {
  const r = await $`git config --global ${key} ${value}`.quiet().nothrow();
  if (r.exitCode !== 0) console.warn(`WARNING: git config ${key} failed.`);
}

const steps: BootStepImpls = {
  async preflight() {
    // Defense in depth behind scripts/entrypoint.sh, which checks the same variables.
    const envError = envValidationError(findMissingRequiredEnv(process.env));
    if (envError) {
      console.error(envError);
      throw new Error("missing required environment");
    }
    // supervisord expands `%(ENV_DESKTOP_RESOLUTION)s` in the desktop-xvnc command and
    // FAILS to start the program if the variable is unset (it has no default-value
    // syntax). The container env normally provides it (docker-compose), but a bare
    // `docker run` or an edited compose would otherwise silently break the desktop.
    // supervisord inherits this process env, so setting it here is the safety net.
    if (!process.env.DESKTOP_RESOLUTION) {
      process.env.DESKTOP_RESOLUTION = config.desktopResolution;
    }
  },

  async "cleanup-stale-mounts"() {
    await cleanupStaleMount(WORKSPACE_MOUNT);
    await cleanupStaleMount(PENTEST_MOUNT);
  },

  async "render-clash-config"() {
    if (!config.clashSubscriptionUrl || !existsSync(CLASH_TEMPLATE_PATH)) return;
    const template = readFileSync(CLASH_TEMPLATE_PATH, "utf-8");
    mkdirSync(CLASH_PROVIDERS_DIR, { recursive: true });
    writeFileSync(CLASH_CONFIG_PATH, renderClashConfig(template, config.clashSubscriptionUrl));
    console.log("Clash config generated from template.");
  },

  async "git-globals"() {
    // Optional identity; the proxy settings are always applied.
    if (config.git.userName) await gitConfig("user.name", config.git.userName);
    if (config.git.userEmail) await gitConfig("user.email", config.git.userEmail);
    await gitConfig("http.proxy", `http://127.0.0.1:${CLASH_HTTP_PORT}`);
    await gitConfig("https.proxy", `http://127.0.0.1:${CLASH_HTTP_PORT}`);
  },

  async "tune-inotify"() {
    await $`sysctl -w fs.inotify.max_user_watches=524288`.quiet().nothrow();
  },

  async "bring-up-egress"(state) {
    state.egress = await bringUpEgress(clashEgressPorts(console.log));
    // The observation stands even if it cannot be left for `doctor` to read back.
    try {
      writeEgressStatus(state.egress);
    } catch (err) {
      console.warn(`WARNING: could not record the egress status: ${err}`);
    }
  },

  async "resolve-auth"() {
    const existingConfig = existsSync(CODE_SERVER_CONFIG_PATH)
      ? readFileSync(CODE_SERVER_CONFIG_PATH, "utf-8")
      : undefined;
    const password = resolveMasterPassword({
      envPassword: config.password,
      existingConfig,
      generate: () => crypto.randomUUID().replace(/-/g, "").slice(0, 24),
    });
    if (password.writeConfig) {
      mkdirSync(CODE_SERVER_CONFIG_DIR, { recursive: true });
      writeFileSync(
        CODE_SERVER_CONFIG_PATH,
        renderCodeServerConfig(password.value, CODE_SERVER_PORT),
      );
    }
    if (password.source === "generated") {
      console.log(`Generated password: ${password.value}`);
      console.log("  (set PASSWORD in .env to use a fixed password)");
    }
    masterPassword = password.value;

    // bcrypt hash for Caddy's basic_auth. supervisord (and so Caddy) inherits this env.
    const hash = await caddyPasswordHash(masterPassword);
    if (hash) {
      process.env.DESKTOP_BCRYPT_HASH = hash;
      console.log("Auth configured (user: user).");
    } else {
      console.warn("WARNING: could not hash the password for Caddy; desktop auth may not start.");
    }
  },

  async "init-vault"() {
    if (isInitialized(VAULT_CIPHER_DIR)) return;
    if (await initVault(VAULT_CIPHER_DIR, masterPassword)) {
      console.log("Vault initialized (password = same as code-server).");
    } else {
      console.warn("WARNING: Vault init failed.");
    }
  },

  async "unlock-vault"(state) {
    // One unlock path, shared with the interactive `unlock-vault` command, so the
    // post-unlock hooks (ssh / git credential links, feature-gated credentials) run
    // on boot too. Hooks that need a running supervisord are interactive-only.
    const unlock = await unlockVault(WORKSPACE_VAULT, async () => masterPassword, {
      volume: gocryptfsVolume,
      hooks: workspaceUnlockHooks({ flags: config.features, announce: console.log }),
    });
    switch (unlock.status) {
      case "unlocked":
        state.vaultMounted = true;
        console.log("Vault unlocked.");
        for (const failure of unlock.hookFailures) {
          console.warn(`WARNING: post-unlock step failed: ${failure}`);
        }
        break;
      case "noop":
        state.vaultMounted = true;
        console.log("Vault already unlocked.");
        break;
      case "failed":
        console.warn(
          "WARNING: Vault unlock failed (password mismatch?). Run 'unlock-vault' manually.",
        );
        break;
      case "rejected":
        console.warn(`WARNING: Vault not unlocked (${unlock.reason}).`);
        break;
    }
  },

  async "notify-egress"(state) {
    if (!state.egress) return;
    if (syncEgressNotice(state.egress) === "written") {
      console.warn("Egress is not confirmed: see .notifications/egress-degraded.md in the vault.");
    }
  },

  async "prepare-desktops"() {
    // Per-stack HOME dirs (inside the vault) and autostart on every installed stack.
    mkdirSync(DESKTOP_CACHE_DIR, { recursive: true });
    const stacks = installedStacks();
    if (stacks.includes("selkies")) {
      mkdirSync(`${SELKIES_HOME}/.local/share/applications`, { recursive: true });
    }
    if (stacks.includes("vnc")) {
      mkdirSync(`${VNC_HOME}/.local/share/applications`, { recursive: true });
      // KasmVNC's native HTTP Basic Auth reads $VNC_HOME/.kasmpasswd. Provision it with
      // the master password so the /vnc/ WS upgrade authenticates with the same
      // credentials as code-server/selkies (KasmVNC owns its WS auth).
      await ensureKasmPasswd(masterPassword);
    }
    // "both" mode: both group files exist; a single-stack image has only one.
    enableAutostart(supervisorConf("desktop"));
    enableAutostart(supervisorConf("vnc"));
  },

  async "prepare-backup"() {
    if (!config.backup.RESTIC_REPOSITORY) return;
    const probe = await $`restic snapshots --no-lock`.quiet().nothrow();
    const verdict = classifyRepoProbe({
      exitCode: probe.exitCode,
      stderr: probe.stderr.toString(),
    });
    switch (verdict) {
      case "ready":
        break;
      case "wrong-password":
        console.warn("WARNING: Backup repo exists but RESTIC_PASSWORD does not match. Check .env.");
        break;
      case "unreachable":
        console.warn("WARNING: Backup repo not reachable (will retry on next start).");
        break;
      case "missing": {
        // Only restic's explicit "no repository here" gets here, never a transient error.
        const init = await $`restic init`.quiet().nothrow();
        if (init.exitCode === 0) console.log("Backup repo initialized.");
        else console.warn("WARNING: Backup repo init failed (will retry on next start).");
        break;
      }
    }
    // The watcher retries and reports on its own, so it starts either way.
    enableAutostart(supervisorConf("backup-watcher"));
  },

  async banner(state) {
    for (const line of renderBanner({
      password: masterPassword,
      stacks: installedStacks(),
      vaultMounted: state.vaultMounted,
      egress: state.egress,
      degraded: state.degraded,
    })) {
      console.log(line);
    }
    console.log("Starting supervisord...");
  },

  async supervisord() {
    // Explicit env, so Caddy (a supervisord child) sees DESKTOP_BCRYPT_HASH.
    const proc = Bun.spawn([SUPERVISORD_BIN, "-c", SUPERVISORD_CONF], {
      stdout: "inherit",
      stderr: "inherit",
      env: { ...process.env },
    });
    const code = await proc.exited;
    if (code !== 0) throw new Error(`supervisord exited with code ${code}`);
  },
};

const state: BootState = { vaultMounted: false, degraded: [] };
const report = await runBoot(BOOT_PLAN, steps, state, console.warn);
process.exit(report.aborted ? 1 : 0);
