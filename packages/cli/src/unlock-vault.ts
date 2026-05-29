#!/usr/bin/env bun
import { existsSync, mkdirSync } from "node:fs";
import {
  CREDENTIALS_DIR,
  STATE_SOCKET_PATH,
  SYNC_PASSPHRASE_PATH,
  VAULT_CIPHER_DIR,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
/**
 * unlock-vault — Mount the gocryptfs vault at /workspace
 */
import { $ } from "bun";
import { planUnlock } from "./lib/vault-lifecycle.ts";

// 1-3. Gather observed state, then decide via the pure planner.
const mountCheck = await $`mountpoint -q ${WORKSPACE_MOUNT}`.quiet().nothrow();
const mounted = mountCheck.exitCode === 0;
const initialized = existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`);

let passphrase = "";
if (!mounted && initialized) {
  if (process.stdin.isTTY) {
    passphrase = prompt("Vault passphrase: ") ?? "";
  } else {
    // Read from stdin (piped input)
    passphrase = (await new Response(process.stdin).text()).trim();
  }
}

const plan = planUnlock({ mounted, initialized, passphrase });
if (plan.action === "noop") {
  console.log("Vault already unlocked.");
  process.exit(plan.exitCode);
}
if (plan.action === "error") {
  if (plan.reason === "not-initialized") {
    console.error("ERROR: Vault not initialized. Run 'init-vault' first.");
  } else {
    console.error("ERROR: Passphrase cannot be empty.");
  }
  process.exit(plan.exitCode);
}
if (plan.warnShort) {
  console.warn("⚠ WARNING: Passphrase is very short (<8 chars). Use 16+ for production.");
}

// 4. Mount — pipe passphrase to gocryptfs stdin (-nonempty allows README placeholder)
const gocryptfs = Bun.spawn(["gocryptfs", "-q", "-nonempty", VAULT_CIPHER_DIR, WORKSPACE_MOUNT], {
  stdin: new Response(`${passphrase}\n`).body!,
  stdout: "pipe",
  stderr: "pipe",
});
const exitCode = await gocryptfs.exited;
if (exitCode !== 0) {
  console.error("ERROR: Failed to unlock vault (wrong passphrase or mount error).");
  process.exit(1);
}

// 5. Post-unlock hooks
// 5a. Create shared directory for sync
mkdirSync(`${WORKSPACE_MOUNT}/shared`, { recursive: true });

// 5b. SSH symlink
const sshCreds = `${CREDENTIALS_DIR}/ssh`;
if (existsSync(sshCreds)) {
  await $`rm -f ~/.ssh && ln -sf ${sshCreds} ~/.ssh`.quiet().nothrow();
  console.log("SSH credentials linked.");
} else {
  console.log(`No SSH credentials at ${sshCreds} — set up later.`);
}

// 5b. Git credential helper
const tokenFile = `${CREDENTIALS_DIR}/github-token`;
if (existsSync(tokenFile)) {
  await $`git config --global credential.helper "store --file=${tokenFile}"`.quiet();
  console.log("Git credential helper configured.");
}

// 5c. Write sync-passphrase (always overwrite for rotation correctness)
mkdirSync(CREDENTIALS_DIR, { recursive: true });
await Bun.write(SYNC_PASSPHRASE_PATH, passphrase);
await $`chmod 600 ${SYNC_PASSPHRASE_PATH}`.quiet();

// 5c-2. Derive and store destruct key hash (for remote self-destruct API)
import { deriveDestructKey, destructKeyToPassphrase } from "@sdw/core/crypto-utils";
import { DESTRUCT_KEY_HASH_PATH, DESTRUCT_STATE_DIR } from "@sdw/core/constants";
{
  const destructKey = deriveDestructKey(passphrase);
  const destructPassphrase = destructKeyToPassphrase(destructKey);
  const hashFileExisted = existsSync(DESTRUCT_KEY_HASH_PATH);
  const hash = await Bun.password.hash(destructPassphrase, { algorithm: "bcrypt", cost: 12 });
  mkdirSync(DESTRUCT_STATE_DIR, { recursive: true });
  await Bun.write(DESTRUCT_KEY_HASH_PATH, hash);
  if (!hashFileExisted) {
    console.log(`\nDestruction passphrase (save this): ${destructPassphrase}\n`);
  }
}

// 5d. Broadcast state
await $`echo '{"state":"unlocked"}' | socat - UNIX-CONNECT:${STATE_SOCKET_PATH}`.quiet().nothrow();

// 5e. Load user mise config if exists in vault
if (existsSync(`${WORKSPACE_MOUNT}/.mise.toml`)) {
  await $`cd ${WORKSPACE_MOUNT} && mise install`.quiet().nothrow();
}

console.log("\n✓ Vault unlocked. Workspace available at /workspace\n");
