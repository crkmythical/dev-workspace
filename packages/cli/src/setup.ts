#!/usr/bin/env bun
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  CLASH_CONFIG_PATH,
  CLASH_PROVIDERS_DIR,
  CLASH_TEMPLATE_PATH,
  bringUpEgress,
  describeEgress,
  renderClashConfig,
} from "@sdw/core/egress";
import { VAULT_CIPHER_DIR, VAULT_GITATTRIBUTES, VAULT_GITIGNORE } from "@sdw/core/vault";
import { loadConfig } from "@sdw/core/workspace";
import { clashEgressPorts } from "@sdw/infra";
/**
 * setup — Interactive or automated workspace configuration
 *
 * Usage:
 *   setup                    # Interactive mode (prompts for each setting)
 *   setup --auto             # Auto mode (reads from env vars, no prompts)
 *   setup --clash-url URL    # Set specific values non-interactively
 */
import { $ } from "bun";

const config = loadConfig(process.env);

// --- Parse args ---
const args = process.argv.slice(2);
const isAuto = args.includes("--auto");
const getArg = (flag: string) => {
  const idx = args.indexOf(flag);
  return idx >= 0 && args[idx + 1] ? args[idx + 1] : null;
};

// --- Helpers ---
function ask(question: string, defaultVal = ""): string {
  if (isAuto) return defaultVal;
  const answer = prompt(`${question}${defaultVal ? ` [${defaultVal}]` : ""}: `);
  return answer?.trim() || defaultVal;
}

function askSecret(question: string): string {
  if (isAuto) return "";
  const answer = prompt(`${question}: `);
  return answer?.trim() || "";
}

// --- Main ---
console.log("\n=== Secure Dev Workspace Setup ===\n");

// Step 1: Clash subscription
console.log("[1/4] Clash 代理订阅");
let clashUrl = getArg("--clash-url") || config.clashSubscriptionUrl;
if (!clashUrl && !isAuto) clashUrl = ask("  Clash 订阅链接 (回车跳过)");
if (clashUrl) {
  // Rendered from the same template as the container start-up: the template defines the
  // url-test group, DNS and rules, and the subscription is only its provider URL.
  if (!existsSync(CLASH_TEMPLATE_PATH)) {
    console.log(`  ✗ 找不到 Clash 模板 ${CLASH_TEMPLATE_PATH}`);
  } else {
    mkdirSync(CLASH_PROVIDERS_DIR, { recursive: true });
    const template = readFileSync(CLASH_TEMPLATE_PATH, "utf-8");
    writeFileSync(CLASH_CONFIG_PATH, renderClashConfig(template, clashUrl));
    console.log("  ✓ 订阅配置已更新");
    await $`pkill -x clash`.quiet().nothrow();
    await Bun.sleep(1000);
    const report = await bringUpEgress(clashEgressPorts(() => {}));
    console.log(
      report.status === "confirmed"
        ? "  ✓ 代理连接正常"
        : `  ⚠ 代理已启动，出口未确认: ${describeEgress(report)}`,
    );
  }
} else {
  console.log("  跳过 — 设置 CLASH_SUBSCRIPTION_URL，或使用 --clash-url");
}

// Step 2: Vault
console.log("\n[2/4] Vault 加密存储");
if (existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`)) {
  console.log("  ✓ Vault 已初始化");
} else {
  const initVault = isAuto ? "y" : ask("  初始化加密 vault？(y/n)", "y");
  if (initVault.toLowerCase() === "y") {
    console.log("  正在初始化...");
    // The vault passphrase is the master password: PASSWORD in auto mode; gocryptfs prompts otherwise
    if (isAuto && config.password) {
      const pw = config.password;
      const proc = Bun.spawn(["gocryptfs", "-init", "-q", VAULT_CIPHER_DIR], {
        stdin: new Response(`${pw}\n${pw}\n`).body!,
        stdout: "pipe",
        stderr: "pipe",
      });
      const exitCode = await proc.exited;
      if (exitCode === 0) console.log("  ✓ Vault 初始化完成");
      else console.log("  ✗ 初始化失败");
    } else {
      const r = await $`gocryptfs -init ${VAULT_CIPHER_DIR}`.nothrow();
      if (r.exitCode === 0) console.log("  ✓ Vault 初始化完成");
    }
    // Init git repo
    await $`git config --global user.email "${config.git.userEmail ?? "vault@local"}"`.quiet();
    await $`git config --global user.name "${config.git.userName ?? "Vault"}"`.quiet();
    await $`cd ${VAULT_CIPHER_DIR} && git init -b main`.quiet();
    writeFileSync(`${VAULT_CIPHER_DIR}/.gitignore`, VAULT_GITIGNORE);
    writeFileSync(`${VAULT_CIPHER_DIR}/.gitattributes`, VAULT_GITATTRIBUTES);
    await $`cd ${VAULT_CIPHER_DIR} && git add -A && git commit -q -m "init vault"`
      .quiet()
      .nothrow();
  }
}

// Step 3: GitHub backup
console.log("\n[3/4] GitHub 备份 (可选)");
let vaultRepo = getArg("--vault-repo") || config.git.vaultRepo || "";
if (!vaultRepo && !isAuto) {
  console.log("  备份 vault 密文到 GitHub，丢失数据可恢复。");
  console.log("  格式: https://ghp_TOKEN@github.com/user/repo.git");
  vaultRepo = ask("  GitHub repo URL (回车跳过)");
}
if (vaultRepo) {
  // Check if remote already configured
  const hasRemote = await $`cd ${VAULT_CIPHER_DIR} && git remote get-url origin`.quiet().nothrow();
  if (hasRemote.exitCode !== 0) {
    await $`cd ${VAULT_CIPHER_DIR} && git remote add origin "${vaultRepo}"`.quiet().nothrow();
  }
  const pushResult = await $`cd ${VAULT_CIPHER_DIR} && git push -u origin main`.quiet().nothrow();
  if (pushResult.exitCode === 0) {
    console.log("  ✓ 已推送到 GitHub");
  } else {
    console.log("  ⚠ 推送失败（检查 token 权限或仓库是否存在）");
  }
}
if (!vaultRepo) {
  console.log("  跳过 — 后续可在 .env 中配置 VAULT_GIT_REPO");
}

// Step 4: Git identity
console.log("\n[4/4] Git 身份");
let gitName = getArg("--git-name") || config.git.userName || "";
let gitEmail = getArg("--git-email") || config.git.userEmail || "";
if (!gitName && !isAuto) gitName = ask("  你的名字", "Developer");
if (!gitEmail && !isAuto) gitEmail = ask("  你的邮箱", "dev@local");
if (gitName) await $`git config --global user.name "${gitName}"`.quiet();
if (gitEmail) await $`git config --global user.email "${gitEmail}"`.quiet();
console.log(`  ✓ ${gitName || "Developer"} <${gitEmail || "dev@local"}>`);

// Done
console.log("\n=== 配置完成 ===");
console.log(`  code-server: http://localhost:${config.hostPort}`);
console.log(
  "  远程访问:    经 Cloudflare Tunnel 暴露的域名（宿主机 scripts/setup-cloudflared.sh 配置）",
);
console.log("  健康检查:    docker exec dev-workspace doctor");
if (existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`)) {
  console.log("  解锁 vault:  docker exec -it dev-workspace unlock-vault");
}
console.log("");
