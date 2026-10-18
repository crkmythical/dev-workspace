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
# 桌面:    http://localhost:18080/vnc/        (KasmVNC，默认)
# Selkies: http://localhost:18080/desktop/   (仅 DESKTOP_STACK=selkies|both 的镜像)
# 认证:    user / <你的 PASSWORD>
```

端口 18080 是 `TUNNEL_HOST_PORT` 的默认值；改了它，启动横幅会打印实际端口。

**Linux 宿主机（Ubuntu、Debian 等带 AppArmor 的发行版）**：Docker 默认的 AppArmor 配置禁止 `mount(2)`，gocryptfs 挂载 vault 会失败（启动后 vault 保持锁定，报 `fusermount3: mount failed: Permission denied`；Docker Desktop 没有 AppArmor，不受影响）。在 `.env` 里取消注释 `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml`，或每次用 `docker compose -f docker-compose.yml -f docker-compose.apparmor.yml up -d`。这个覆盖文件会去掉容器最后一层 AppArmor 约束，是安全上的取舍，所以不是默认；见 `docs/threat-model.md`。

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

- **远程桌面** — 默认只装 KasmVNC；Selkies 为可选栈，两者可并行（见「构建选项」）：
  - **KasmVNC** (`/vnc/`，默认) — VNC-over-WebSocket，高延迟隧道下更稳定，镜像小、构建快
  - **Selkies** (`/desktop/`，`DESKTOP_STACK=selkies|both`) — H.264/WebCodecs 流式传输，低延迟本地访问最佳。自适应浏览器窗口大小，无需配置分辨率
- **Code Server** — 浏览器中的 VS Code
- **加密 Vault** — gocryptfs 自动初始化/解锁
- **代理** — Clash 全局模式（自动测速选最快节点，每小时刷新订阅，3 分钟健康检查）。尽力而为、不强制：读取代理环境变量的工具走 Clash，`noproxy` 或忽略代理设置的进程会直连。启动横幅、`doctor`、`/var/run/egress-status` 会显示出口状态（confirmed / degraded / down），出口异常不会阻止工作区启动
- **音频** — PulseAudio null-sink 桌面音频流
- **基础系统** — Kali Linux（可通过 apt 安装渗透测试工具）

## 构建选项

```bash
# 默认 — 仅 KasmVNC（.deb 约 8 MB，构建很快）
docker compose build

# 其它桌面栈
DESKTOP_STACK=selkies docker compose build   # 仅 Selkies（venv 约 420 MB，该层约 12 分钟）
DESKTOP_STACK=both docker compose build      # Selkies + KasmVNC 并行

# CI 预构建镜像：默认镜像 = KasmVNC；带 -full 后缀的镜像 = both
#   ghcr.io/<owner>/dev-workspace:<tag>        KasmVNC
#   ghcr.io/<owner>/dev-workspace:<tag>-full   Selkies + KasmVNC
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

受限网络（例如需要镜像仓库）：把 `BASE_IMAGE` 指向可达的镜像地址即可。Debian 系基础镜像的 apt 源由 `APT_MIRROR` 决定，默认 `mirrors.aliyun.com`（大陆内很快，境外很慢），境外构建请设 `APT_MIRROR=deb.debian.org`。Kali 基础镜像沿用 Kali 自己的源，主机由 `KALI_MIRROR` 决定，默认官方重定向 `http.kali.org`（按位置选镜像，个别网络下只有约 14 KB/s），慢的话设 `KALI_MIRROR=mirrors.tuna.tsinghua.edu.cn`。

也可以不改 `.env`，直接命令行覆盖构建另一个 tag：

```bash
docker build -t dev-workspace:trixie -f image/Dockerfile --build-arg BASE_IMAGE=debian:trixie-slim .
```

两个镜像除 OS 标识外工具链完全一致（Bun、Node、Java、Selkies、KasmVNC 等）。

## 容器内命令

```bash
doctor              # 健康检查（出口 / 已安装的桌面 / 备份状态）
noproxy <cmd>       # 绕过代理直连执行命令
lock-vault          # 锁定加密工作区（先停止桌面和备份；仍有进程占用时会询问是否强制卸载，
                    # 强制卸载后已打开文件的进程在退出前仍能访问，要彻底切断就重启容器）
unlock-vault        # 输入密码解锁
desktop-start       # 启动所有已安装桌面
desktop-start vnc   # 仅启动 KasmVNC 桌面
desktop-stop        # 停止所有桌面
desktop-stop selkies # 仅停止 Selkies 桌面
backup-init         # 初始化 restic 仓库（配置后首次启动自动执行）
destruct-passphrase # 显示远程自毁口令（由 vault 口令派生，随时可重新查看）
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
./scripts/setup-cloudflared.sh --hostname <hostname>   # 或在 .env 设置 TUNNEL_HOSTNAME

# 启动 tunnel（LaunchAgent，开机自启 + 崩溃自动重启）
launchctl load ~/Library/LaunchAgents/com.cloudflare.tunnel.plist

# 停止 tunnel
launchctl unload ~/Library/LaunchAgents/com.cloudflare.tunnel.plist

# 从任何地方访问（<hostname> 是你自己的域名）：
# https://<hostname>
# https://<hostname>/vnc/        (KasmVNC，默认)
# https://<hostname>/desktop/    (Selkies，仅 DESKTOP_STACK=selkies|both 的镜像)
```

`setup-cloudflared.sh` 会自动安装 cloudflared、登录 Cloudflare、创建 tunnel、配置 DNS，并在 macOS 上安装 LaunchAgent。没有默认域名，必须通过 `--hostname` 或 `.env` 指定。Linux 宿主机用 systemd 用户服务：`bash deploy/linux-systemd/install.sh --cloudflared`（见 `deploy/linux-systemd/README.md`；已在 Ubuntu 24.04 虚拟机的真实 systemd 下验证单元的安装、启动、自动重启和自毁后的清理链，未在真机和真实 Cloudflare Tunnel 上验证）。

## 环境变量 (.env)

完整的变量列表、默认值和说明在 [`.env.example`](.env.example)。它由代码里的 schema（`packages/core/src/workspace/env-schema.ts`，插件的变量由插件自己声明）生成，改了 schema 后运行 `make env-example`；测试会在示例文件、代码读取的变量和 docker-compose.yml 不一致时失败。

最常用的几个：

| 变量 | 说明 |
|------|------|
| CLASH_SUBSCRIPTION_URL | 必填。Clash 代理订阅地址 |
| PASSWORD | 统一密码：code-server、桌面和 vault 共用同一个。留空则首次启动随机生成并打印在启动横幅里，但这个密码只存在于该容器内：重启容器会保留，**重建容器（`docker compose up -d` 在重新构建或 pull 之后、`docker rm`）会生成新密码**，已有 vault 就打不开了（要用旧密码执行 `unlock-vault`）。务必保存横幅里的密码，或直接在 `.env` 里设置 |
| DESKTOP_STACK | 构建时选择桌面栈：`kasmvnc`（默认）/ `selkies` / `both` |
| SYNC_ENABLED | 默认关闭。启用浏览器文件同步页 `/sync/`；启用后每次解锁会把 vault 口令写入 `/workspace/.credentials/sync-passphrase`，关闭后下次解锁会删除它 |
| REMOTE_DESTRUCT_ENABLED | 默认关闭。启用远程自毁端点 `POST /sync/api/destruct`；口令用 `destruct-passphrase` 查看 |
| RESTIC_REPOSITORY | 设置后启用实时备份（还需 `RESTIC_PASSWORD` 与 S3 凭证）；仓库不存在时启动时自动创建。备份的是**明文**工作区（含 `.credentials/`），仅由 `RESTIC_PASSWORD` 保护，自毁不会删除它，要把它当作 vault 的一部分对待 |
| COMPOSE_FILE | 默认不设。Linux + AppArmor 的宿主机取消注释，见上文 |

## 验证（smoke test）

单元测试（`bun test`）不启动镜像。`tests/smoke/` 把真镜像和一个假上游一起起来，从外面检查启动、认证、vault、备份与复制、出口降级、自毁，加上 `--services` 时对真实的 S3 端点、sshd、git HTTP、Shadowsocks 节点，以及用真实 Chrome 看同步页和桌面画面。它有自己的 compose 项目、端口和网络，不读 `.env`，可以放心在已有真实工作区的机器上跑：

```bash
make smoke IMAGE=dev-workspace:latest     # 测一个已有的镜像
make smoke-build                          # 先按生产基础（Kali）构建再测
```

用法、检查清单和它证明 / 不证明什么见 [`tests/smoke/README.md`](tests/smoke/README.md)；CI 里对应 `.github/workflows/smoke.yml`（手动触发，或改动了镜像 / compose / 测试本身的 PR）。

## CI/CD（GitHub Actions）

镜像在推送 `v*` tag 时自动构建并推送，配置文件：`.github/workflows/build-image.yml`。

- **镜像变体**：默认镜像只含 KasmVNC；带 `-full` 后缀的镜像含 Selkies + KasmVNC（`DESKTOP_STACK=both`）。每次发布都会构建两种，保证 Selkies 一直可用
- **多架构**：`linux/amd64` + `linux/arm64`（原生 runner，无 QEMU 模拟）
- **Registry**：GHCR（始终推送）+ Docker Hub（配置 secrets 后自动推送）
- **缓存**：按变体与架构分离的 registry cache（GHCR 上的 `cache-<variant>-<arch>` tag）

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
# 从 GHCR（默认：KasmVNC；`-full`：Selkies + KasmVNC）
docker pull ghcr.io/<owner>/dev-workspace:latest
docker pull ghcr.io/<owner>/dev-workspace:latest-full

# 从 Docker Hub
docker pull <username>/dev-workspace:latest
```

也可通过 Actions → "Run workflow" 手动触发（同样构建两种变体）。
