#!/usr/bin/env bun
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import {
  CLASH_SOCKS_PORT,
  CREDENTIALS_DIR,
  SYNC_PASSPHRASE_PATH,
  VAULT_CIPHER_DIR,
  WORKSPACE_MOUNT,
} from "@sdw/core/constants";
/**
 * setup — Interactive or automated workspace configuration
 *
 * Usage:
 *   setup                    # Interactive mode (prompts for each setting)
 *   setup --auto             # Auto mode (reads from env vars, no prompts)
 *   setup --clash-url URL    # Set specific values non-interactively
 */
import { $ } from "bun";

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

async function testClashUrl(url: string): Promise<boolean> {
  try {
    const r = await $`curl -fsSL --max-time 10 "${url}" -o /dev/null`.quiet().nothrow();
    return r.exitCode === 0;
  } catch {
    return false;
  }
}

async function testProxy(): Promise<boolean> {
  try {
    const r =
      await $`curl -s --proxy socks5://127.0.0.1:${CLASH_SOCKS_PORT} --max-time 10 https://www.google.com -o /dev/null`
        .quiet()
        .nothrow();
    return r.exitCode === 0;
  } catch {
    return false;
  }
}

const DEFAULT_CLASH_URL =
  "https://cfnew.cicd.dpdns.org/372074d4-385f-4205-9071-7e61186dffd4/sub?target=clash";
const DEFAULT_TUNNEL_HOSTNAME = "workspace.cicd.dpdns.org";
const DEFAULT_TUNNEL_NAME = "dev-workspace";
const DEFAULT_TUNNEL_PORT = "18080";

// --- Main ---
console.log("\n=== Secure Dev Workspace Setup ===\n");

// Step 1: Clash subscription
console.log("[1/4] Clash 代理订阅");
let clashUrl = getArg("--clash-url") || process.env.CLASH_SUBSCRIPTION_URL || "";
if (!clashUrl && !isAuto) {
  clashUrl = ask("  Clash 订阅链接 (回车使用默认)", DEFAULT_CLASH_URL);
}
if (!clashUrl) clashUrl = DEFAULT_CLASH_URL;
if (clashUrl) {
  // Write subscription directly (don't test — network may not be available yet)
  const fetchResult = await $`curl -fsSL --max-time 15 "${clashUrl}" -o /etc/clash/config.yaml.new`
    .quiet()
    .nothrow();
  if (fetchResult.exitCode === 0) {
    await $`mv /etc/clash/config.yaml.new /etc/clash/config.yaml`.quiet();
    console.log("  ✓ 订阅配置已更新");
    // Restart clash
    await $`pkill -x clash`.quiet().nothrow();
    await Bun.sleep(1000);
    Bun.spawn(["/usr/bin/clash", "-d", "/etc/clash"], { stdout: "ignore", stderr: "ignore" });
    await Bun.sleep(3000);
    if (await testProxy()) {
      console.log("  ✓ 代理连接正常");
    } else {
      console.log("  ⚠ 代理已启动（连接测试未通过，可能需要等待节点就绪）");
    }
  } else {
    console.log("  ⚠ 订阅拉取失败（使用默认配置）");
  }
}

// Step 2: Vault
console.log("\n[2/4] Vault 加密存储");
if (existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`)) {
  console.log("  ✓ Vault 已初始化");
} else {
  const initVault = isAuto ? "y" : ask("  初始化加密 vault？(y/n)", "y");
  if (initVault.toLowerCase() === "y") {
    console.log("  正在初始化...");
    // In auto mode, use env var VAULT_PASSWORD; in interactive, gocryptfs prompts
    if (isAuto && process.env.VAULT_PASSWORD) {
      const pw = process.env.VAULT_PASSWORD;
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
    await $`git config --global user.email "${process.env.GIT_USER_EMAIL || "vault@local"}"`.quiet();
    await $`git config --global user.name "${process.env.GIT_USER_NAME || "Vault"}"`.quiet();
    await $`cd ${VAULT_CIPHER_DIR} && git init -b main`.quiet();
    const gitignore =
      "node_modules/\ntarget/\nbuild/\ndist/\n.cache/\n**/*.log\n.idea/\n.vscode/\n__pycache__/\n.DS_Store\n*.tmp\n*.swp\nshared/\n";
    writeFileSync(`${VAULT_CIPHER_DIR}/.gitignore`, gitignore);
    writeFileSync(
      `${VAULT_CIPHER_DIR}/.gitattributes`,
      "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    );
    await $`cd ${VAULT_CIPHER_DIR} && git add -A && git commit -q -m "init vault"`
      .quiet()
      .nothrow();
  }
}

// Step 3: GitHub backup
console.log("\n[3/4] GitHub 备份 (可选)");
let vaultRepo = getArg("--vault-repo") || process.env.VAULT_GIT_REPO || "";
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
let gitName = getArg("--git-name") || process.env.GIT_USER_NAME || "";
let gitEmail = getArg("--git-email") || process.env.GIT_USER_EMAIL || "";
if (!gitName && !isAuto) gitName = ask("  你的名字", "Developer");
if (!gitEmail && !isAuto) gitEmail = ask("  你的邮箱", "dev@local");
if (gitName) await $`git config --global user.name "${gitName}"`.quiet();
if (gitEmail) await $`git config --global user.email "${gitEmail}"`.quiet();
console.log(`  ✓ ${gitName || "Developer"} <${gitEmail || "dev@local"}>`);

// Done
console.log("\n=== 配置完成 ===");
console.log(`  code-server: http://localhost:${DEFAULT_TUNNEL_PORT}`);
console.log(`  远程访问:    https://${DEFAULT_TUNNEL_HOSTNAME}`);
console.log(`  文件同步:    https://${DEFAULT_TUNNEL_HOSTNAME}/sync/`);
console.log("  健康检查:    docker exec dev-workspace doctor");
if (existsSync(`${VAULT_CIPHER_DIR}/gocryptfs.conf`)) {
  console.log("  解锁 vault:  docker exec -it dev-workspace unlock-vault");
}
console.log("");
