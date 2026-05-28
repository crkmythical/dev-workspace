# Secure Dev Workspace

加密的、容器化的远程开发环境。部署在 Mac Mini 上，通过浏览器从任何设备访问。所有数据加密、所有流量走代理、公司监控看不到任何内容。

## 系统拓扑

```
┌──────────────────────────────────────────────────────────────────────┐
│  公司电脑 / 家里 / iPad / 网吧                                         │
│                                                                      │
│  浏览器 ──── HTTPS ────→ Cloudflare CDN                              │
│                             │                                        │
│  公司监控只看到:             │ (加密，看不到内容)                       │
│  "访问了某个 HTTPS 网站"    │                                        │
└─────────────────────────────┼────────────────────────────────────────┘
                              │
                              ▼
┌──────────── Cloudflare Edge ─────────────────────────────────────────┐
│  Cloudflare Access (OAuth) → Cloudflare Tunnel (加密隧道)            │
└──────────────────────────────────────────────────────────────────────┘
                              │
                              ▼ (出站隧道，Mac Mini 无入站端口)
┌──────────── Mac Mini (公司机房) ─────────────────────────────────────┐
│                                                                      │
│  cloudflared (宿主机服务)                                             │
│      │                                                               │
│      ▼ localhost:18080                                               │
│  ┌────────────────── Docker Container ────────────────────────────┐  │
│  │                                                                │  │
│  │  Caddy (:8080) ─── 反向代理                                    │  │
│  │    ├── /        → code-server (:8082)  ← VS Code Web IDE      │  │
│  │    └── /sync/*  → sync-service (:8081) ← 文件同步 + 加密传输   │  │
│  │                                                                │  │
│  │  Clash (:7890/:7891) ← 所有出站流量加密代理                     │  │
│  │    └── DNS fake-ip (防 DNS 泄漏)                               │  │
│  │                                                                │  │
│  │  gocryptfs (FUSE) ← 磁盘加密                                  │  │
│  │    /vault/cipher (密文) ↔ /workspace (明文，只在内存)           │  │
│  │                                                                │  │
│  │  supervisord ← 进程管理 | cron ← vault-sync (30min)           │  │
│  └────────────────────────────────────────────────────────────────┘  │
│                                                                      │
│  磁盘上只有: Docker.raw (VM 虚拟磁盘, 内部全是密文)                   │
└──────────────────────────────────────────────────────────────────────┘
                              │
                              │ Clash 加密隧道 (VLESS+WS+TLS)
                              ▼
┌──────────── 互联网 ──────────────────────────────────────────────────┐
│  GitHub (vault 密文备份) | npm/Maven/PyPI | 任何网站                  │
└──────────────────────────────────────────────────────────────────────┘
```

## 核心功能

| cloudflared tunnel route dns dev-workspace workspace.cicd.dpdns.org 2>&1


### 1. 加密存储 (gocryptfs)
- 一个密码保护所有数据
- 文件名也加密
- 容器停止后零明文残留
- 4KB 块加密，git diff 友好

### 2. 网络代理 (Clash/mihomo)
- 所有出站流量走加密隧道
- DNS fake-ip 防泄漏
- 自动选最快节点 + 故障切换
- git push/pull 透明走代理

### 3. 远程访问 (Cloudflare Tunnel)
- 零入站端口
- OAuth 认证
- 任何设备浏览器可用

### 4. 浏览器文件同步 (Sync Service)
- File System Access API 双向同步
- 端到端加密 (AES-256-GCM)
- PNG 伪装绕过 DLP
- 断点续传

### 5. 数据持久化 (vault-sync)
- 每 30 分钟自动推送密文到 GitHub
- 换机器 git clone + 输入密码即恢复
- 完整版本历史

### 6. 一键部署/迁移/销毁
- `setup` 交互式引导
- `bootstrap.sh` 自动化部署
- `destroy.sh --force` 30 秒清除所有痕迹

## 安全防御层级

```
Layer 5: 端到端加密 (WebCrypto AES-GCM)        ← 防 TLS 中间人
Layer 4: Cloudflare Access (OAuth)              ← 防未授权访问
Layer 3: Cloudflare Tunnel (TLS)                ← 防网络嗅探
Layer 2: gocryptfs (AES-256)                    ← 防磁盘取证
Layer 1: Clash 代理 (VLESS+WS+TLS)              ← 防流量识别
```

## 快速开始

### 前置条件
- Docker Desktop
- Git

### 部署（3 步）

```bash
# 1. 克隆
git clone <repo-url> ~/dev-workspace && cd ~/dev-workspace

# 2. 配置（只需一个参数，或直接用默认）
cp .env.example .env

# 3. 启动
docker compose build && docker compose up -d
```

### 交互式配置（推荐首次使用）

```bash
docker exec -it dev-workspace setup
```

引导你配置：Clash 代理 → Vault 加密 → GitHub 备份 → Git 身份。每步都有验证。

### 自动化部署（CI/脚本）

```bash
docker exec -e VAULT_PASSWORD=xxx dev-workspace setup --auto \
  --git-name "Name" --git-email "email@x.com"
```

### 解锁 vault

```bash
docker exec -it dev-workspace unlock-vault
```

### 健康检查

```bash
docker exec -it dev-workspace doctor
```

## 日常操作

| 操作 | 命令 |
|------|------|
| 解锁 | `docker exec -it dev-workspace unlock-vault` |
| 锁定 | `docker exec -it dev-workspace lock-vault` |
| 健康检查 | `docker exec -it dev-workspace doctor` |
| 查看日志 | `docker compose logs -f --tail=20` |
| 重启 | `docker compose restart` |
| 密码轮换 | `lock-vault` → `change-password` → `unlock-vault` |
| 销毁 | `./destroy.sh --force` |

## 渗透测试环境（双 Vault 架构）

独立的加密分区存放完整 Kali rootfs，通过 namespace 隔离进入。锁定后磁盘上零痕迹。

### 架构

```
┌─── dev-workspace container ────────────────────────────────┐
│                                                             │
│  Vault 1 (workspace):  /vault/cipher → /workspace          │
│    ├── 代码、项目、配置（git 自动备份）                       │
│    └── 与渗透环境完全独立                                    │
│                                                             │
│  Vault 2 (pentest):  /pentest/cipher → /pentest/rootfs     │
│    ├── 完整 Kali rootfs（不备份，可重建）                     │
│    ├── 独立密码 (plausible deniability)                      │
│    └── 通过 unshare+chroot 隔离进入                          │
│                                                             │
│  网络:                                                       │
│    开发流量 → Clash 代理 (VLESS)                             │
│    渗透流量 → Tor/proxychains (inside chroot)                │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 首次配置

```bash
# 1. 初始化渗透 vault（设置独立密码）
docker exec -it dev-workspace init-pentest

# 2. 解锁
docker exec -it dev-workspace unlock-pentest

# 3. 安装 Kali rootfs（约 2-4GB，需要 5-15 分钟）
docker exec -it dev-workspace pentest-bootstrap

# 4. 进入
docker exec -it dev-workspace pentest
```

### 日常使用

| 操作 | 命令 |
|------|------|
| 解锁渗透环境 | `docker exec -it dev-workspace unlock-pentest` |
| 进入渗透环境 | `docker exec -it dev-workspace pentest` |
| 锁定渗透环境 | `docker exec -it dev-workspace lock-pentest` |
| 安装更多工具 | `(inside) /opt/tools/install-kali-tools.sh` |
| 启动 Tor | `(inside) service tor start` |
| 验证匿名性 | `(inside) tor-check` |
| 使用代理链 | `(inside) proxychains4 nmap target` |

### 安全特性

- **双密码隔离**：workspace 和 pentest 用不同密码，解锁一个看不到另一个
- **独立网络**：渗透流量走 Tor，不经过日常用的 Clash 节点
- **不备份**：rootfs 不进 git（避免 GitHub 体积爆炸 + 内容暴露）
- **PID 隔离**：chroot 内进程对外不可见
- **锁定即消失**：`lock-pentest` 后磁盘上只有无法识别的密文

## 迁移到新机器

```bash
./bootstrap.sh --vault-repo git@github.com:you/dev-vault.git
docker exec -it dev-workspace unlock-vault
# 完全恢复，所有代码、配置、扩展都在
```

## 本地开发

```bash
bun install              # 安装依赖
bun test                 # 29 tests, 306ms
bun run packages/server/src/index.ts  # 本地启动 sync-service
```

## 项目结构

```
packages/
├── core/       共享逻辑（类型、常量、AAD、reconcile、ReplayWindow）
├── server/     Hono + Bun HTTP/WS 服务 (12,640 req/s)
├── cli/        容器内命令行工具（Bun TS）
└── spa/        浏览器端文件同步 SPA

image/          Docker 构建资产（Dockerfile, supervisord, Caddy, Clash config）
scripts/        宿主机 shell 脚本（entrypoint, setup-docker, setup-cloudflared）
docs/           详细文档
```

## 技术栈

| 组件 | 技术 |
|------|------|
| Runtime | Bun |
| HTTP | Hono + Bun.serve() |
| 加密(磁盘) | gocryptfs (AES-256) |
| 加密(传输) | WebCrypto AES-256-GCM |
| 代理 | Clash/mihomo |
| IDE | code-server |
| 反向代理 | Caddy |
| 隧道 | Cloudflare Tunnel |
| 进程管理 | supervisord |
| 测试 | bun test + fast-check |
| Lint | Biome |

## 详细文档

- [架构设计](docs/architecture.md)
- [操作手册](docs/runbook.md)
- [威胁模型](docs/threat-model.md)
- [迁移指南](docs/migration.md)
