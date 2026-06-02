# Secure Dev Workspace

加密、云端就绪的开发环境，集成远程桌面、Code Server 和代理。

## 快速开始

```bash
# 1. 配置环境变量
cp .env.example .env
# 设置 PASSWORD（留空则自动生成）
# 设置 CLASH_SUBSCRIPTION_URL（必填）

# 2a. 本地构建运行
docker compose build
docker compose up -d

# 2b. 或拉取 CI 预构建镜像
docker compose pull
docker compose up -d

# 3. 访问
# Code:    http://localhost:18080
# 桌面:    http://localhost:18080/desktop/   (Selkies, H.264)
# VNC:     http://localhost:18080/vnc/        (KasmVNC)
# 认证:    user / <你的 PASSWORD>
```

## 重建 / 重置

```bash
# 停止
docker compose down

# 重建（代码变更后）
docker compose build
docker compose up -d

# 完全重置（清除 vault 数据）
docker compose down
docker volume rm dev-workspace_vault-data
docker compose build --no-cache
docker compose up -d
```

## 功能

- **双远程桌面** — 两个独立桌面并行运行：
  - **Selkies** (`/desktop/`) — H.264/WebCodecs 流式传输，低延迟本地访问最佳。自适应浏览器窗口大小，无需配置分辨率。
  - **KasmVNC** (`/vnc/`) — VNC-over-WebSocket，高延迟隧道下更稳定
- **Code Server** — 浏览器中的 VS Code
- **加密 Vault** — gocryptfs 自动初始化/解锁
- **代理** — Clash 全局模式（所有流量走代理）
- **音频** — PulseAudio null-sink 桌面音频流
- **基础系统** — Kali Linux（可通过 apt 安装渗透测试工具）

## 构建选项

```bash
# 双桌面（默认）— selkies + kasmvnc 并行
docker compose build

# 单栈（备选）
DESKTOP_STACK=selkies docker compose build   # 仅 selkies
DESKTOP_STACK=kasmvnc docker compose build   # 仅 kasmvnc
```

### 基础镜像切换

通过 `.env` 中的 `BASE_IMAGE` 和 `WORKSPACE_VARIANT` 控制：

```bash
# Kali（默认）— 自带渗透测试工具 apt 源
BASE_IMAGE=kalilinux/kali-rolling
WORKSPACE_VARIANT=latest

# Debian trixie — 更精简，适合纯开发
BASE_IMAGE=debian:trixie-slim
WORKSPACE_VARIANT=trixie
```

修改后重新构建即可：

```bash
docker compose build
docker compose up -d
```

也可以不改 `.env`，直接命令行覆盖构建另一个 tag：

```bash
docker build -t dev-workspace:trixie -f image/Dockerfile --build-arg BASE_IMAGE=debian:trixie-slim .
```

两个镜像除 OS 标识外工具链完全一致（Bun、Node、Java、Selkies、KasmVNC 等）。

## 容器内命令

```bash
doctor              # 健康检查（报告双桌面 + 备份状态）
noproxy <cmd>       # 绕过代理直连执行命令
lock-vault          # 锁定加密工作区（先停止桌面和备份）
unlock-vault        # 输入密码解锁
desktop-start       # 启动所有已安装桌面
desktop-start vnc   # 仅启动 KasmVNC 桌面
desktop-stop        # 停止所有桌面
desktop-stop selkies # 仅停止 Selkies 桌面
backup-init         # 初始化 restic 仓库（配置后首次启动自动执行）
```

## 认证

所有端点共享相同凭证（`user` / 你的 `PASSWORD`）：

- **Code-server** 和 **Selkies** (`/desktop/`) 由 Caddy 网关认证（realm `restricted`）
- **KasmVNC** (`/vnc/`) 使用原生认证（realm `Websockify`）

由于 KasmVNC 拥有独立的认证域，浏览器会单独弹出一次认证提示 — 用户名密码相同，只是多一次输入。这是必要的：浏览器不会将 Caddy 的 basic-auth 凭证转发给 VNC WebSocket 握手。

## 远程访问（Cloudflare Tunnel）

通过 Cloudflare Tunnel 暴露工作区到公网（零入站端口）：

```bash
# 一次性配置（在宿主机执行，非容器内）
./scripts/setup-cloudflared.sh

# 启动 tunnel（LaunchAgent，开机自启 + 崩溃自动重启）
launchctl load ~/Library/LaunchAgents/com.cloudflare.tunnel.plist

# 停止 tunnel
launchctl unload ~/Library/LaunchAgents/com.cloudflare.tunnel.plist

# 从任何地方访问：
# https://workspace.cicd.dpdns.org
# https://workspace.cicd.dpdns.org/desktop/   (Selkies)
# https://workspace.cicd.dpdns.org/vnc/        (KasmVNC)
```

`setup-cloudflared.sh` 会自动安装 cloudflared、登录 Cloudflare、创建 tunnel、配置 DNS 并安装 macOS LaunchAgent。

## 环境变量 (.env)

| 变量 | 默认值 | 说明 |
|------|--------|------|
| BASE_IMAGE | kalilinux/kali-rolling | 构建基础镜像（`debian:trixie-slim` 可选） |
| WORKSPACE_VARIANT | latest | 镜像 tag 后缀（Kali 用 `latest`，Debian 用 `trixie`） |
| PASSWORD | (自动生成) | 统一密码，用于 code-server、桌面和 vault |
| CLASH_SUBSCRIPTION_URL | (必填) | Clash 代理订阅地址 |
| DESKTOP_RESOLUTION | 1920x1080 | KasmVNC (`/vnc/`) 分辨率。Selkies (`/desktop/`) 忽略此项（自适应）。 |
| DESKTOP_AUDIO | 1 | 启用 PulseAudio（0 禁用） |
| TUNNEL_HOST_PORT | 18080 | 宿主机端口映射 |
| RESTIC_REPOSITORY | (空) | S3 备份仓库 URL，设置后启用实时备份 |
| RESTIC_PASSWORD | (空) | 备份仓库加密密码 |
| AWS_ACCESS_KEY_ID | (空) | S3 备份凭证 |
| AWS_SECRET_ACCESS_KEY | (空) | S3 备份凭证 |

## CI/CD（GitHub Actions）

镜像在推送 `v*` tag 时自动构建并推送，配置文件：`.github/workflows/build-image.yml`。

- **多架构**：`linux/amd64` + `linux/arm64`（原生 runner，无 QEMU 模拟）
- **Registry**：GHCR（始终推送）+ Docker Hub（配置 secrets 后自动推送）
- **缓存**：按架构分离的 registry cache（GHCR 上的 `cache-amd64` / `cache-arm64` tag）

### 配置步骤

1. 在 GitHub repo **Settings → Secrets and variables → Actions** 中添加：
   - `DOCKERHUB_USERNAME` — Docker Hub 用户名
   - `DOCKERHUB_TOKEN` — Docker Hub Access Token（在 hub.docker.com → Account Settings → Security 生成）
   - GHCR 无需额外配置，`GITHUB_TOKEN` 自带 `packages:write` 权限

2. 打 tag 触发构建：

```bash
git tag v1.0.0
git push origin v1.0.0
```

3. 拉取构建好的镜像：

```bash
# 从 GHCR
docker pull ghcr.io/<owner>/dev-workspace:latest

# 从 Docker Hub
docker pull <username>/dev-workspace:latest
```

也可通过 Actions → "Run workflow" 手动触发，支持选择 desktop stack。
