# Implementation Plan: self-destruct

## Overview

为 dev-workspace 实现容器内紧急数据销毁功能。按依赖顺序构建：constants → 核心逻辑 → CLI → server route → entrypoint 变更 → docker-compose 变更 → host-watcher → SPA → 集成验证。

每个 task 产出可独立验证的增量。核心原则：`destroyCore()` 是纯逻辑函数（不调用 process.exit，不杀 PID 1），CLI 和 API 各自处理进程终止。

## Task Dependency Graph

```json
{
  "waves": [
    {"tasks": ["1.1", "1.3"]},
    {"tasks": ["1.2"]},
    {"tasks": ["2.1", "2.2", "2.3", "2.4", "3.1"]},
    {"tasks": ["3.2", "3.3"]},
    {"tasks": ["4.1", "4.2", "5.1", "5.2"]},
    {"tasks": ["4.3", "4.4"]},
    {"tasks": ["6.1", "6.2"]}
  ]
}
```

## Tasks

- [x] 1. 基础设施与共享逻辑
  - [x] 1.1 新增 constants
    - 在 `packages/core/src/constants.ts` 中添加 self-destruct 相关常量:
      - `DESTRUCT_STATE_DIR = "/var/run/self-destruct"`
      - `DESTRUCT_KEY_HASH_PATH = "/var/run/self-destruct/key-hash"`
      - `DESTRUCT_COMPLETED_PATH = "/var/run/self-destruct/completed"`
      - `DESTRUCT_HKDF_SALT = "destruct-key-v1"`
      - `DESTRUCT_HKDF_INFO = "self-destruct"`
      - `DESTRUCT_RATE_LIMIT_WINDOW_MS = 60_000`
      - `DESTRUCT_RATE_LIMIT_MAX_ATTEMPTS = 3`
      - `DESTRUCT_RESPONSE_DELAY_MS = 200`
    - _Requirements: 4.2, 4.8, 5.5_
  - [x] 1.2 实现 `packages/cli/src/lib/destroy.ts` — destroyCore()
    - 导出 `DestroyOptions` 接口: `{ force, skipShred, remote, silent }`
    - 导出 `destroyCore(opts): Promise<void>` — 执行 Phase 0-5 后 **return**（不 exit，不 kill PID 1）
    - Phase 0: `$\`supervisorctl stop vault-sync-cron\`.quiet().nothrow()`; if `remote` && vault mounted: read `VAULT_GIT_REPO` from env, read token from `CREDENTIALS_DIR/github-token`, `$\`gh repo delete --yes ${repo}\`.nothrow()`; log warning if unavailable
    - Phase 1: `await unmountVault(WORKSPACE_MOUNT)` + `await unmountVault(PENTEST_MOUNT)` (imported from `lib/vault.ts`)
    - Phase 2: `await shredFile(VAULT_CIPHER_DIR + "/gocryptfs.conf")` + `await shredFile(PENTEST_CIPHER_DIR + "/gocryptfs.conf")`; print "POINT OF NO RETURN" + ISO timestamp
    - Phase 3 (unless skipShred): `$\`find ${dir} -type f -exec shred -n1 -z {} +\`` + `$\`rm -rf ${dir}/*\`` for both cipher dirs
    - Phase 4: shred `/etc/clash/config.yaml`; `rm -rf /root/.ssh /root/.gitconfig /root/.git-credentials`; `rm -f ${DESTRUCT_KEY_HASH_PATH}`
    - Phase 5: `$\`> ~/.bash_history\``; `await Bun.write(DESTRUCT_COMPLETED_PATH, new Date().toISOString())`
    - **destroyCore returns here** — does NOT stop services or kill processes (caller responsibility)
    - Helper: `shredFile(path)` = `$\`shred -n3 -z ${path} && rm -f ${path}\`.quiet().nothrow()` (idempotent: no error if file missing)
    - Progress output: if `!opts.silent` → `console.log()` after each phase
    - _Requirements: 1.3, 1.5, 1.6, 1.7, 1.8, 1.9, 2.1, 2.2, 2.3, 2.4, 2.5, 5.1, 5.3_
  - [x] 1.3 实现 destruct key 派生工具函数
    - 新建 `packages/core/src/crypto-utils.ts`:
      - `import crypto from "node:crypto"`
      - `import { DESTRUCT_HKDF_SALT, DESTRUCT_HKDF_INFO, AES_KEY_LENGTH } from "./constants.ts"`
      - `export function deriveDestructKey(passphrase: string): Buffer` — `crypto.hkdfSync("sha256", passphrase, DESTRUCT_HKDF_SALT, DESTRUCT_HKDF_INFO, AES_KEY_LENGTH)`
      - `export function destructKeyToPassphrase(key: Buffer): string` — `key.toString("base64")`
    - 在 `packages/core/src/index.ts` 中 re-export
    - `packages/cli` (unlock-vault.ts) 和 `packages/server` (routes/destruct.ts) 均从 `@sdw/core/crypto-utils` 导入
    - _Requirements: 4.2_

- [x] 2. CLI 命令与 Dockerfile 集成
  - [x] 2.1 实现 `packages/cli/src/self-destruct.ts`
    - 解析 argv: `--force`, `--skip-shred`, `--remote`
    - 非 force 时: `const answer = prompt("Type 'destroy' to confirm: ")`; 若不匹配则 `process.exit(0)`
    - `await destroyCore({ force: true, skipShred, remote, silent: false })`
    - destroyCore 返回后:
      - `await $\`supervisorctl stop all\`.quiet().nothrow()` (停所有服务)
      - `await $\`kill -TERM 1\`.quiet().nothrow()` (终止容器, tini 转发 SIGTERM)
    - 注意: kill 1 后当前进程也会被 tini 的 SIGTERM 传播杀死，这是预期行为
    - _Requirements: 1.1, 1.2, 1.4, 1.8_
  - [x] 2.2 在 Dockerfile 中注册 self-destruct 命令
    - 在 `image/Dockerfile` 的 CLI symlink 循环中添加 `self-destruct`
    - 具体位置: `for cmd in init-pentest unlock-pentest lock-pentest pentest-bootstrap` 循环的同一块中新增
    - _Requirements: 1.1_
  - [x] 2.3 变更 docker-compose.yml
    - 在 `volumes:` 顶级 section 新增 `destruct-state:`
    - 在 workspace service 的 volumes 列表中添加 `- destruct-state:/var/run/self-destruct`
    - _Requirements: 4.3, 5.5_
  - [x] 2.4 变更 entrypoint — Inert Mode 检查
    - 在 `scripts/entrypoint.sh` 中（env var 检查之前，作为脚本的最第一个逻辑）添加:
      ```bash
      # Inert mode: container was self-destructed
      if [ -f /var/run/self-destruct/completed ]; then
        echo "Environment destroyed. Container is inert. Clean up: docker compose down -v"
        exec sleep infinity
      fi
      ```
    - 位置: 在 `set -euo pipefail` 之后、`for v in CLASH_SUBSCRIPTION_URL` 之前
    - _Requirements: 5.5, 5.6, 1.7_

- [x] 3. 远程触发 — unlock hook + API endpoint
  - [x] 3.1 修改 `packages/cli/src/unlock-vault.ts` 的 post-unlock hook
    - 在 step 5d (broadcast state) 之前，新增 step 5d-pre:
      - `import { deriveDestructKey, destructKeyToPassphrase } from "@sdw/core/crypto-utils"`
      - `const destructKey = deriveDestructKey(passphrase)`
      - `const destructPassphrase = destructKeyToPassphrase(destructKey)`
      - `const hashFileExisted = existsSync(DESTRUCT_KEY_HASH_PATH)`
      - `const hash = await Bun.password.hash(destructPassphrase, { algorithm: "bcrypt", cost: 12 })`
      - `mkdirSync(DESTRUCT_STATE_DIR, { recursive: true })`
      - `await Bun.write(DESTRUCT_KEY_HASH_PATH, hash)`
      - if `!hashFileExisted`: `console.log(\`\nDestruction passphrase (save this): ${destructPassphrase}\n\`)`
      - 如果已存在（后续 unlock），静默更新（密码轮换后自动同步）
    - _Requirements: 4.3, 4.4_
  - [x] 3.2 实现 `packages/server/src/routes/destruct.ts`
    - 导出 `destructRoute` 作为 Hono handler (`(c: Context) => Response | Promise<Response>`)
    - Import: `import { destroyCore } from "../../../cli/src/lib/destroy.ts"` (follows existing monorepo relative import convention)
    - 模块级速率限制器: `const attempts = new Map<string, { count: number; resetAt: number }>()`
    - `getClientIP(c)`: 从 `X-Forwarded-For` header 或 connection info 获取 IP
    - 逻辑流程:
      1. 检查 rate limit → 超过则 return `c.body(null, 429)`
      2. 读取 `DESTRUCT_KEY_HASH_PATH` → 不存在则 return `c.body(null, 404)`
      3. 解析 body: `const { passphrase } = await c.req.json()` → 格式错误则 404
      4. `Bun.password.verify(passphrase, hash)` → false 则 increment counter, return 404
      5. 验证成功 → `setTimeout(async () => { await destroyCore({...}); process.kill(1, "SIGTERM"); }, DESTRUCT_RESPONSE_DELAY_MS)`
         - 注意: 这里直接 kill 1，不用 supervisorctl stop all（因为 sync-service 自己就在 supervisord 下，stop all 会杀死自己导致 destroyCore 中断）
      6. return `c.json({ status: "destroyed" }, 200)`
    - _Requirements: 4.1, 4.6, 4.7, 4.8, 4.9, 5.4_
  - [x] 3.3 注册 destruct route 到 server
    - 在 `packages/server/src/index.ts` 中添加:
      - `import { destructRoute } from "./routes/destruct.ts"`
      - `app.post("/sync/api/destruct", destructRoute)`
    - _Requirements: 4.1, 4.9_

- [x] 4. 宿主机联动
  - [x] 4.1 实现 `scripts/host-watcher.sh`
    - 解析子命令: `--install`, `--uninstall`, `--once`, 或默认 daemon 模式
    - Daemon 模式:
      ```bash
      CONTAINER="dev-workspace"
      # Dynamically find the destruct-state volume (handles any docker compose project name)
      DESTRUCT_VOLUME=$(docker volume ls --filter "name=destruct-state" --format '{{.Name}}' | head -1)
      docker events --filter "container=${CONTAINER}" --filter "event=die" | while read -r line; do
        if [ -n "$DESTRUCT_VOLUME" ] && docker run --rm -v "${DESTRUCT_VOLUME}:/s" alpine test -f /s/completed 2>/dev/null; then
          echo "[$(date -Iseconds)] Self-destruct confirmed."
          "${SCRIPT_DIR}/destroy.sh" --force --paranoid
          break
        fi
      done
      ```
    - `--once`: 执行一次检查后退出（用于测试）
    - `--install`: 生成 plist, cp 到 `~/Library/LaunchAgents/`, `launchctl load`
    - `--uninstall`: `launchctl unload`, rm plist
    - 脚本开头: `command -v docker &>/dev/null || { echo "docker not found"; exit 1; }`
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.8_
  - [x] 4.2 创建 LaunchAgent plist 模板
    - 内嵌在 `host-watcher.sh --install` 逻辑中（通过 heredoc 或 printf 生成）
    - KeepAlive: true, RunAtLoad: true
    - ProgramArguments: ["/path/to/host-watcher.sh"] (使用实际安装路径)
    - WorkingDirectory: 项目目录
    - StandardOutPath: /tmp/dev-workspace-watcher.log
    - StandardErrorPath: /tmp/dev-workspace-watcher.log
    - _Requirements: 3.5_
  - [x] 4.3 集成到 bootstrap.sh
    - 在 step 8 (docker compose up) 之后、step 9 (instructions) 之前:
      ```bash
      read -rp "Install host-watcher (auto-cleanup on self-destruct)? [y/N]: " watcher
      if [[ "${watcher:-}" =~ ^[Yy]$ ]]; then
        bash scripts/host-watcher.sh --install
      fi
      ```
    - _Requirements: 3.7_
  - [x] 4.4 更新 destroy.sh --paranoid 模式
    - 在 paranoid 清理步骤中添加: `launchctl unload ~/Library/LaunchAgents/com.dev-workspace.host-watcher.plist 2>/dev/null; rm -f ~/Library/LaunchAgents/com.dev-workspace.host-watcher.plist`
    - 确保 host-watcher LaunchAgent 在完整销毁时被移除
    - _Requirements: 3.4 (cleanup completeness)_

- [x] 5. SPA 紧急面板
  - [x] 5.1 实现 emergency panel UI
    - 在 `packages/spa/src/main.ts` 中添加逻辑:
      - 监听 `hashchange` + 初始检查 `location.hash`
      - 当 hash === '#emergency' 时渲染 emergency panel (替换或覆盖正常 UI)
      - 当 hash 变为其他值时隐藏 panel
    - Panel 内容 (纯 DOM，与现有 SPA 风格一致):
      - 红色 banner: "⚠ EMERGENCY DESTRUCT — This will permanently destroy all workspace data"
      - Input: passphrase (type=password, placeholder="Destruction passphrase")
      - Input: confirmation (type=text, placeholder='Type "DESTROY" to confirm')
      - Button: "Execute" (disabled unless passphrase non-empty AND confirm === "DESTROY")
      - Status area: pending spinner / success message / error message
    - _Requirements: 4.10, 4.11_
  - [x] 5.2 实现请求逻辑
    - 点击 Execute:
      - disable all inputs
      - `fetch("/sync/api/destruct", { method: "POST", headers: {"Content-Type":"application/json"}, body: JSON.stringify({passphrase}) })`
      - 200 → 显示 "✓ Environment destroyed" (green)
      - 404 → 显示 "✗ Incorrect passphrase or not configured" (orange)
      - 429 → 显示 "✗ Rate limited. Wait 60 seconds." (orange)
      - network error → 显示 "✗ Connection failed" (red)
    - 不重新启用输入（防止重复触发）
    - _Requirements: 4.11, 4.12_

- [x] 6. 文档与集成验证
  - [x] 6.1 更新文档
    - `README.md`: 日常操作表格添加 `self-destruct [--force] [--skip-shred]`; 添加 "紧急销毁" 小节含三种触发方式
    - `docs/runbook.md`: 添加紧急销毁操作章节
    - `docs/threat-model.md`: 添加 self-destruct 攻击面分析
  - [x] 6.2 端到端验证清单
    - 在测试环境中:
      1. `unlock-vault` → 确认 destruct passphrase 首次打印; `key-hash` 文件存在于 destruct-state volume
      2. 再次 `lock-vault` + `unlock-vault` → destruct passphrase 不再打印（静默更新）
      3. `self-destruct --force --skip-shred` → 确认: gocryptfs.conf 不存在, completed marker 存在, 容器随后退出
      4. 重启容器 → 确认进入 inert mode (`docker logs` 显示 "inert" 消息, 进程列表只有 sleep)
      5. 远程: POST `/sync/api/destruct` with correct passphrase → 200
      6. 远程: POST with wrong passphrase → 404 (无信息泄露)
      7. 远程: 4 次错误尝试 within 1 minute → 第 4 次 429
      8. host-watcher: `./scripts/host-watcher.sh --once` 在容器退出后确认检测到 marker 并调用 destroy.sh
