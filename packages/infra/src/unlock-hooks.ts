/**
 * Post-unlock hooks. The same hook implementations serve the container's
 * boot-time auto-unlock and the interactive `unlock-vault` command; each
 * composition root only chooses WHICH hooks to run.
 *
 * Feature-gated hooks (sync credential, remote-destruct key) are idempotent
 * "make the state match the flag": enabled → provision, disabled → remove stale
 * material left behind by an earlier run.
 */
import { chmodSync, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { CREDENTIALS_DIR, WORKSPACE_MOUNT } from "@sdw/core/shared/layout";
import { SYNC_PASSPHRASE_PATH } from "@sdw/core/sync/layout";
import {
  DESTRUCT_KEY_HASH_PATH,
  DESTRUCT_STATE_DIR,
  STATE_SOCKET_PATH,
  type UnlockHook,
} from "@sdw/core/vault";
import type { FeatureFlags } from "@sdw/core/workspace";
import { $ } from "bun";
import { deriveDestructKey, destructKeyToPassphrase } from "./destruct-key.ts";

type Announce = (message: string) => void;

/** Create the shared sync directory in the workspace vault. */
export function ensureSharedDirHook(): UnlockHook {
  return {
    name: "shared-dir",
    async run({ spec }) {
      if (spec.id === "workspace") mkdirSync(`${spec.mountPoint}/shared`, { recursive: true });
    },
  };
}

/** Link ~/.ssh to the credentials stored inside the vault, if any. */
export function linkSshCredentialsHook(announce: Announce): UnlockHook {
  return {
    name: "ssh-credentials",
    async run({ spec }) {
      if (spec.id !== "workspace") return;
      const sshCreds = `${CREDENTIALS_DIR}/ssh`;
      if (existsSync(sshCreds)) {
        await $`rm -f ~/.ssh && ln -sf ${sshCreds} ~/.ssh`.quiet().nothrow();
        announce("SSH credentials linked.");
      } else {
        announce(`No SSH credentials at ${sshCreds} — set up later.`);
      }
    },
  };
}

/** Point git's credential helper at the token stored inside the vault, if any. */
export function gitCredentialsHook(announce: Announce): UnlockHook {
  return {
    name: "git-credentials",
    async run({ spec }) {
      if (spec.id !== "workspace") return;
      const tokenFile = `${CREDENTIALS_DIR}/github-token`;
      if (!existsSync(tokenFile)) return;
      await $`git config --global credential.helper "store --file=${tokenFile}"`.quiet();
      announce("Git credential helper configured.");
    },
  };
}

/**
 * SYNC_ENABLED: write the vault passphrase to the sync key source (inside the
 * vault, so it disappears when the vault locks). Disabled: remove a stale copy.
 */
export function syncCredentialHook(opts: {
  enabled: boolean;
  path?: string;
  announce?: Announce;
}): UnlockHook {
  const target = opts.path ?? SYNC_PASSPHRASE_PATH;
  return {
    name: "sync-credential",
    async run({ spec, passphrase }) {
      if (spec.id !== "workspace") return;
      if (!opts.enabled) {
        if (existsSync(target)) {
          rmSync(target, { force: true });
          opts.announce?.("Sync disabled: removed stale sync credential.");
        }
        return;
      }
      mkdirSync(path.dirname(target), { recursive: true });
      writeFileSync(target, passphrase, { mode: 0o600 });
      chmodSync(target, 0o600); // mode only applies on create; enforce on overwrite too
    },
  };
}

/**
 * REMOTE_DESTRUCT_ENABLED: derive the remote-destruct passphrase from the vault
 * passphrase and store only its bcrypt hash. The passphrase is announced the
 * first time the hash is created; `destruct-passphrase` can re-derive it later.
 * Disabled: remove a stale hash so the endpoint stays dormant.
 */
export function remoteDestructKeyHook(opts: {
  enabled: boolean;
  announce: Announce;
  hashPath?: string;
  stateDir?: string;
  /** bcrypt cost; lowered in tests only. */
  cost?: number;
}): UnlockHook {
  const hashPath = opts.hashPath ?? DESTRUCT_KEY_HASH_PATH;
  const stateDir = opts.stateDir ?? DESTRUCT_STATE_DIR;
  return {
    name: "remote-destruct-key",
    async run({ spec, passphrase }) {
      if (spec.id !== "workspace") return;
      if (!opts.enabled) {
        if (existsSync(hashPath)) {
          rmSync(hashPath, { force: true });
          opts.announce("Remote self-destruct disabled: removed stale key hash.");
        }
        return;
      }
      const destructPassphrase = destructKeyToPassphrase(deriveDestructKey(passphrase));
      const hashFileExisted = existsSync(hashPath);
      const hash = await Bun.password.hash(destructPassphrase, {
        algorithm: "bcrypt",
        cost: opts.cost ?? 12,
      });
      mkdirSync(stateDir, { recursive: true });
      await Bun.write(hashPath, hash);
      if (!hashFileExisted) {
        opts.announce(`\nDestruction passphrase (save this): ${destructPassphrase}\n`);
      }
    },
  };
}

/** Tell listeners on the state socket that the vault changed state. */
export async function broadcastState(state: "locked" | "unlocked"): Promise<void> {
  await $`echo ${JSON.stringify({ state })} | socat - UNIX-CONNECT:${STATE_SOCKET_PATH}`
    .quiet()
    .nothrow();
}

export function broadcastStateHook(): UnlockHook {
  return {
    name: "broadcast-state",
    async run({ spec }) {
      if (spec.id === "workspace") await broadcastState("unlocked");
    },
  };
}

/** Install the user's mise-managed tools if the vault carries a .mise.toml. */
export function miseInstallHook(): UnlockHook {
  return {
    name: "mise-install",
    async run({ spec }) {
      if (spec.id !== "workspace") return;
      if (existsSync(`${WORKSPACE_MOUNT}/.mise.toml`)) {
        await $`cd ${WORKSPACE_MOUNT} && mise install`.quiet().nothrow();
      }
    },
  };
}

/** Start the backup watcher under supervisord (only when a restic repo is configured). */
export function startBackupWatcherHook(opts: { enabled: boolean }): UnlockHook {
  return {
    name: "start-backup-watcher",
    async run({ spec }) {
      if (spec.id !== "workspace" || !opts.enabled) return;
      await $`supervisorctl start backup-watcher`.quiet().nothrow();
    },
  };
}

/**
 * Hooks every unlock of the workspace vault runs, whether at container boot or
 * from the command line.
 */
export function workspaceUnlockHooks(opts: {
  flags: FeatureFlags;
  announce: Announce;
}): UnlockHook[] {
  return [
    ensureSharedDirHook(),
    linkSshCredentialsHook(opts.announce),
    gitCredentialsHook(opts.announce),
    syncCredentialHook({ enabled: opts.flags.sync, announce: opts.announce }),
    remoteDestructKeyHook({ enabled: opts.flags.remoteDestruct, announce: opts.announce }),
  ];
}

/**
 * Extra hooks for an interactive unlock only. They need a running supervisord
 * (or are slow), so the container's boot path deliberately leaves them out.
 */
export function interactiveUnlockExtras(opts: { backupEnabled: boolean }): UnlockHook[] {
  return [
    broadcastStateHook(),
    miseInstallHook(),
    startBackupWatcherHook({ enabled: opts.backupEnabled }),
  ];
}
