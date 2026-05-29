#!/usr/bin/env bun
import { existsSync } from "node:fs";
import { VAULT_CIPHER_DIR, WORKSPACE_MOUNT } from "@sdw/core/constants";
/**
 * init-vault — One-time gocryptfs vault initialization
 */
import { $ } from "bun";
import { planInit } from "./lib/vault-lifecycle.ts";

// Idempotency check
if (planInit(existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`)).action === "noop") {
  console.log("Vault already initialized.");
  process.exit(0);
}

console.log("=== Vault Initialization ===\n");
console.log("Choose a strong passphrase (16+ chars, mix case/numbers/symbols).");
console.log("Store it in a password manager immediately.\n");

// Init gocryptfs
const initResult = await $`gocryptfs -init ${VAULT_CIPHER_DIR}`.nothrow();
if (initResult.exitCode !== 0) {
  console.error("gocryptfs init failed.");
  process.exit(1);
}

// Init git repo
await $`git config --global user.email "${process.env.GIT_USER_EMAIL || "vault@local"}"`.quiet();
await $`git config --global user.name "${process.env.GIT_USER_NAME || "Vault"}"`.quiet();
await $`cd ${VAULT_CIPHER_DIR} && git init -b main`.quiet();

// Write .gitignore
const gitignore = `node_modules/
target/
build/
dist/
.cache/
**/*.log
.idea/
.vscode/
__pycache__/
.DS_Store
*.tmp
*.swp
shared/
`;
await Bun.write(`${VAULT_CIPHER_DIR}/.gitignore`, gitignore);

// Write .gitattributes
await Bun.write(
  `${VAULT_CIPHER_DIR}/.gitattributes`,
  "*.bin filter=lfs diff=lfs merge=lfs -text\n*.jar filter=lfs diff=lfs merge=lfs -text\n",
);

// Initial commit
await $`cd ${VAULT_CIPHER_DIR} && git add -A && git commit -q -m "init vault"`.quiet();

// Push to remote if configured
const vaultRepo = process.env.VAULT_GIT_REPO;
if (vaultRepo) {
  await $`cd ${VAULT_CIPHER_DIR} && git remote add origin ${vaultRepo} && git push -u origin main`
    .quiet()
    .nothrow();
  console.log(`Pushed to ${vaultRepo}`);
} else {
  console.log("No VAULT_GIT_REPO configured — skipping remote push.");
  console.log("Set VAULT_GIT_REPO in .env to enable auto-sync later.");
}

console.log("\n✓ Vault initialized. Run 'unlock-vault' to mount.\n");
console.log("⚠ SAVE YOUR PASSPHRASE IN A PASSWORD MANAGER NOW.");
