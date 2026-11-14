# Secure Dev Workspace

加密、云端就绪的开发环境，集成远程桌面、Code Server 和代理。

> 详细中文上手指南见 [`deploy/docs/QUICKSTART.md`](deploy/docs/QUICKSTART.md)；架构与运维见 [`deploy/docs/`](deploy/docs/)。

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
# 登录:    打开任何一个地址都会先到登录页，输入 PASSWORD（没有用户名）；之后所有入口共用这一个会话
```

端口 18080 是 `TUNNEL_HOST_PORT` 的默认值；改了它，启动横幅会打印实际端口。

**出口防火墙默认开启**：工作区里的一切只能经 Clash 出去，忽略代理设置的进程不会带着本机真实 IP 直连，而是被丢弃（见下文「出口防火墙」）。局域网里的主机默认连不上，要用 `EGRESS_ALLOW_CIDRS` 列出来；宿主不让容器有 `NET_ADMIN` 时启动会停下，并指向 `EGRESS_ENFORCE=0`。

**可选的 compose 覆盖文件**（在 `.env` 里用 `COMPOSE_FILE=docker-compose.yml:…` 叠加，冒号连接）：

| 文件 | 什么时候要 |
|------|-----------|
| `docker-compose.apparmor.yml` | Linux / Colima（带 AppArmor）宿主，否则 vault 挂载被拒 |
| `docker-compose.secrets.yml` | 把 PASSWORD 和订阅地址改成文件密钥（`make secrets-init`） |
| `docker-compose.pentest.yml` | 用 pentest 插件：补上 `SYS_CHROOT` / `MKNOD` / `NET_RAW`（默认容器没有这三项；不加它，`pentest-bootstrap` 会明说缺什么然后退出） |

**Linux 宿主机（Ubuntu、Debian 等带 AppArmor 的发行版）**：Docker 默认的 AppArmor 配置禁止 `mount(2)`，gocryptfs 挂载 vault 会失败（启动后 vault 保持锁定，报 `fusermount3: mount failed: Permission denied`；Docker Desktop 没有 AppArmor，不受影响）。在 `.env` 里取消注释 `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml`，或每次用 `docker compose -f docker-compose.yml -f docker-compose.apparmor.yml up -d`。这个覆盖文件会去掉容器最后一层 AppArmor 约束，是安全上的取舍，所以不是默认；见 `deploy/docs/threat-model.md`。

## 在 Mac 上本机构建（Colima，含国内网络）

已在 Apple Silicon（Apple M5、macOS 27.0.1，Colima 0.10.3 + vz/virtiofs，Docker 29）上从零构建成功：Kali 基础、默认 KasmVNC 栈，
6 核 / 12 GiB 的 VM、走本机代理，约 15 分钟，镜像 5.5 GB。三件事缺一不可：

1. **buildx 插件**。没有它 `docker compose build` 会悄悄退回经典构建器（日志里只有一行
   `buildx Docker CLI plugin not found: falling back to the classic builder`），而这个 Dockerfile 用了
   BuildKit 才有的 `COPY --chmod`，构建会在中途失败。
2. **足够大的 VM**。Colima 默认 2 CPU / 2 GiB：带 LTO 的 Rust 编译会被 OOM 杀掉，桌面也跑不动，而且这个 VM 上
   通常还有别的容器。给工作区单独起一个 profile，原来的 VM 不动：
3. **能到 GitHub / Docker Hub / crates.io / apt 的网络**。国内网络下这几处时通时断，见下。

```bash
brew install colima docker docker-compose docker-buildx
ln -sfn /opt/homebrew/lib/docker/cli-plugins/docker-buildx ~/.docker/cli-plugins/docker-buildx

# 一次性：专用 VM（已存在的 profile 不会被改动或重建）。--proxy 让 Docker 守护进程经代理拉基础镜像；
# 没有代理就省略。Colima 里宿主机是 192.168.5.2，宿主 127.0.0.1:1080 的代理经它可达。
deploy/scripts/setup-colima-build-vm.sh --proxy http://192.168.5.2:1080

# 在运行 docker / make 的 shell 里（不要 `docker context use`，那会改掉你其它容器用的 VM）：
export DOCKER_CONTEXT=colima-dev-workspace
```

`.env` 里按需加（说明见 `.env.example` 的 “IMAGE BUILD”）：

```bash
KALI_MIRROR=mirrors.aliyun.com        # 官方重定向 http.kali.org 在这里只有约 14 KB/s；TUNA 的 Kali 镜像回 403
BUILD_PROXY=http://192.168.5.2:1080   # 可选。构建步骤（curl / cargo / bun / mise / apt）走它，不写进镜像或其历史
COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml   # Colima 的 VM 是 Ubuntu + AppArmor，否则 vault 挂载被拒
```

```bash
make build     # = docker compose build；从零约 15 分钟，之后只重做改动的层（改 Rust 约 3-5 分钟）
make smoke     # 对刚构建的镜像（默认 dev-workspace:$WORKSPACE_VARIANT）跑 smoke test，见下文「验证」
```

| 现象 | 原因 / 处理 |
|------|------------|
| `falling back to the classic builder` | 没装 buildx，见上 |
| 核心 apt 层 `exit code: 100` | 官方 Kali 重定向挑到了坏镜像：设 `KALI_MIRROR=mirrors.aliyun.com`（已验证整层 198 个包 170 MB 一次装完） |
| `pull access denied` / 拉 `rust`、`oven/bun`、`kali-rolling` 超时 | 拉镜像是 Docker 守护进程做的，`BUILD_PROXY` 管不到：用 `--proxy` 重新跑上面的脚本 |
| GitHub release 下载偶发失败 | 下载已带 `--retry 5 --retry-all-errors`；仍失败就设 `BUILD_PROXY` |
| `mount failed: Permission denied`（vault 一直锁着） | 没加 AppArmor 覆盖文件，见上 `COMPOSE_FILE` |

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
- **代理与出口防火墙** — Clash 规则模式（一条规则指向自动测速的节点组，每小时刷新订阅，3 分钟健康检查；订阅里没有可用节点时请求失败，**不会**退回直连）。nftables 默认丢弃一切不经 Clash 的出站连接（默认开启，`EGRESS_ENFORCE=0` 关闭），`ssh` 也被引到 Clash。启动横幅、`doctor`、`/var/run/egress-status` 会显示出口状态（confirmed / degraded / down），出口异常不会阻止工作区启动
- **权限分离** — 你在 code-server 终端和桌面里运行的一切都是普通用户 `dev`（家目录在 vault 里），看不到 PASSWORD、备份和订阅的密钥，也没有任何 capability；需要 root 的少数命令经 `doas` 桥接
- **供应链** — 镜像里下载的每个工具（Caddy、restic、croc、Clash、code-server、KasmVNC、mise、IDEA、Burp）都固定了版本和校验和（`src/image/tools/*.env`），下载不符构建即失败
- **音频** — PulseAudio null-sink 桌面音频流
- **基础系统** — Kali Linux（可通过 apt 安装渗透测试工具）

## 构建选项

```bash
# 默认 — 仅 KasmVNC（.deb 约 8 MB，构建很快）
docker compose build

# 其它桌面栈
DESKTOP_STACK=selkies docker compose build   # 仅 Selkies（Python 环境约 160 MB，该层约 4 分钟）
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
docker build -t dev-workspace:trixie -f src/image/Dockerfile --build-arg BASE_IMAGE=debian:trixie-slim ./src
```

两个镜像除 OS 标识外工具链完全一致（Node、Python、Java、Selkies、KasmVNC 等；Bun 仅在构建时用于编译同步页，运行镜像里没有）。

## 容器内命令

```bash
doctor              # 健康检查（出口 / 已安装的桌面 / 备份状态）
noproxy <cmd>       # 不带代理变量运行命令，让它直连（出口防火墙开着时会被丢弃；EGRESS_ENFORCE=0 才有意义）
lock-vault          # 锁定加密工作区（先停止桌面和备份；仍有进程占用时会询问是否强制卸载，
                    # 强制卸载后已打开文件的进程在退出前仍能访问，要彻底切断就重启容器）
unlock-vault        # 输入密码解锁
desktop-start       # 启动所有已安装桌面
desktop-start vnc   # 仅启动 KasmVNC 桌面
desktop-stop        # 停止所有桌面
desktop-stop selkies # 仅停止 Selkies 桌面
set-clash-dns [dns,…|--clear] # 查看 / 修改 Clash 用的解析器（持久、热加载，不用重建）
audit-log [--json] [N] # 最近 N 条审计事件（默认 20）：登录成功/失败/被限速、登出、Cloudflare Access 放行、
                    # 被拒的跨站 WebSocket、自毁接口的每次调用、出口状态变化。每行：时间、事件、结果、声明的来源地址 / User-Agent
```

以上在 code-server 终端里直接敲就行：以 `dev` 运行的会话里，需要 root 的几个（`doctor`、`lock-vault`、`unlock-vault`、`desktop-*`、`set-clash-dns`、`setup`、`push`/`pull`、`audit-log`、pentest 系列、`init-*`）由 `doas` 桥接，桥接名单是生成的（`sdw doas-rules`）。下面三个**只能由运营者在宿主上 `docker exec` 运行**，`dev` 运行会被拒绝并提示：

```bash
docker exec -it dev-workspace self-destruct --force   # 自毁，见 runbook
docker exec -it dev-workspace destruct-passphrase     # 显示远程自毁口令（由 vault 口令派生，随时可重新查看）
docker exec -it dev-workspace backup-init             # 初始化 restic 仓库（配置后首次启动自动执行）
```

## 认证

只有一道登录：Caddy 前面的会话网关（`forward_auth` 到 sync-service 的 `/auth/verify`）。输入 `PASSWORD`（没有用户名）后得到一个签名的会话 cookie，**所有入口共用它**——code-server、`/vnc/`、`/desktop/`、`/sync/`，包括 WebSocket 升级（同源 cookie 会随握手一起发出，这是 Basic Auth 做不到的）。放在 Cloudflare Access 后面时，网关也接受边缘注入的 `Cf-Access-Jwt-Assertion`（用团队 JWKS 验证 RS256），不用再登录一次。

后端自己不再有密码：code-server 以 `--auth none` 运行，KasmVNC 以 `-DisableBasicAuth`，所以**没有第二次弹窗**，容器里也没有主密码的副本可供工作区里的代码读取。

- 会话 cookie 用随机密钥签名（每次启动随机；`AUTH_SESSION_PERSIST=1` 时是存在 `/var/lib/sdw/session.key`、仅 root 可读的随机密钥，**不是**从密码派生的），经 HTTPS 访问时带 `Secure`
- 来自别的站点的 WebSocket 握手（`Origin` 不是本站）即使带着有效 cookie 也会被拒（审计事件 `gate.origin`）
- 每个响应带基线安全头（`X-Content-Type-Options`、`Referrer-Policy`，HTTPS 下有 HSTS；`/sync/` 有严格的 CSP 和 `X-Frame-Options: DENY`）
- 登录限速：每个声明的来源地址每分钟 10 次、所有地址合计 60 次，超过回 429，哪怕密码是对的
- 每次登录/登出/自毁调用都进审计日志 `/var/log/sdw/audit.jsonl`（只有 root 可读、自动轮转、放在命名卷 `audit-data` 里，容器重建后还在；同时打到 `docker logs`）。日志里没有口令。`doctor` 的 “Audit trail” 一行汇总最近 24 小时

## 出口防火墙（默认开启）

工作区里发出的一切只能走 Clash。除了 `http_proxy` / `all_proxy` 这些变量（工具自愿遵守），启动时还会装一份 nftables 的默认丢弃过滤器：只放行回环、已建立连接的应答、`clash` 用户，以及你用 `EGRESS_ALLOW_CIDRS` 列出的网络。所以忽略代理的进程、UDP/QUIC、`noproxy`、自己解析域名的程序，**不会**带着本机真实 IP 出去，而是被丢弃；域名由 Clash 的 DNS 解析，查询也不会绕过它。`ssh` 不读代理变量，启动时会写 `/etc/ssh/ssh_config.d/10-sdw-egress.conf` 让它经 Clash（回环地址除外）。

| 你要 | 做法 |
|------|------|
| 访问局域网 / 另一个 Docker 网络里的主机 | `.env` 里写 `EGRESS_ALLOW_CIDRS=192.168.1.0/24,10.0.0.5`（地址或 CIDR；比 /8（IPv6 /16）更宽的、域名、写错的都会让启动停下并指明哪一项）。去那里的流量**带着真实 IP、不经 Clash**；工具通过 `no_proxy` 得知，`ssh` 要在 `~/.ssh/config` 给该主机写 `ProxyCommand none`。域名不会替你解析，用 compose 的 `extra_hosts` |
| 关掉防火墙 | `EGRESS_ENFORCE=0`（也认 `false` / `no` / `off`；无法识别的值保持开启）。横幅、启动日志、`doctor` 每次都会明说它关着 |
| 用 pentest 插件 | 加 `docker-compose.pentest.yml`。Tor 配置成经 Clash 出去（torrc 的 `HTTPSProxy`），所以在防火墙后应当照常工作——这个组合（Tor + 防火墙开着）**没有实际跑过**；裸的 `nmap -sS`、`ping` 不行，把靶场网络写进 `EGRESS_ALLOW_CIDRS` |
| 确认它真的在 | `doctor` 有一行 “Egress firewall”：该开却没装上是失败；`nft list ruleset` |

要有 `NET_ADMIN` 能力和 nftables；没有时启动会停在 `enforce-egress` 并告诉你怎么办，而不是在以为出口已封死的情况下启动（gVisor 不支持 nftables，所以在那里要设 `EGRESS_ENFORCE=0`）。git（HTTPS 和 SSH）、restic、apt、Tor 和镜像自带的 Firefox ESR 在防火墙后验证过；`desktop-install` 装的 Chromium、IDEA、Burp 没测，一个不遵守代理变量的应用在防火墙后表现为连不上，而不是泄漏。

## 工作区用户（`dev`）

你在 code-server 终端和桌面里运行的一切（包括包的安装脚本）都是 `dev`（uid 1000）：家目录 `/workspace/home`（在 vault 里，所以 shell 历史、工具缓存加密落盘、锁定即消失）；环境变量是白名单，**没有**工作区的任何密钥；没有 capability，也没有除 `doas` 外的 setuid 程序。网关、备份、同步、supervisord 是 root，Caddy 是 `caddy`，Clash 是 `clash`。要让工具看到某个 `.env` 变量，把名字写进 `WORKLOAD_ENV=GITHUB_TOKEN,NPM_TOKEN`；密钥类变量（`PASSWORD`、`RESTIC_*`、`AWS_*`、`CLASH_SUBSCRIPTION_URL`……）会被拒绝并警告。预装的运行时（Node 20、JDK 17 等）在只读的 `/opt/mise`，`dev` 用 `mise` 把自己的版本装进家目录。

局限：桥接给 root 的命令名单虽短，其中任何一个命令的漏洞都是提权的路；容器里的 root 仍是带 `CAP_SYS_ADMIN` 的容器 root。完整的边界、做了什么、没做什么见 `deploy/docs/threat-model.md`。

## 迁移与销毁

```bash
./bootstrap.sh --vault-repo <VAULT_GIT_REPO 的地址>   # 迁移到新机器：先克隆密文，再在容器第一次启动之前放进 vault 卷
./bootstrap.sh --init                                  # 全新 vault
./destroy.sh --force                 # 删掉本项目的容器、卷和镜像（只按 compose 项目标签匹配，不会碰别的项目）
./destroy.sh --force --delete-project # 同时删掉这份 checkout（不加这个就不会删）
```

迁移时 `.env` 里的 `PASSWORD` 必须是旧 vault 的口令；卷里已经有 vault 时 `bootstrap.sh` 拒绝覆盖。

## 远程访问（Cloudflare Tunnel）

通过 Cloudflare Tunnel 暴露工作区到公网（零入站端口）：

```bash
# 一次性配置（在宿主机执行，非容器内）
./deploy/scripts/setup-cloudflared.sh --hostname <hostname>   # 或在 .env 设置 TUNNEL_HOSTNAME

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

完整的变量列表、默认值和说明在 [`.env.example`](.env.example)。它由代码里的 schema（`src/rust/core/src/workspace/env_schema.rs`，插件的变量由插件自己声明）生成，改了 schema 后运行 `make env-example`；测试会在示例文件、代码读取的变量和 docker-compose.yml 不一致时失败。

最常用的几个：

| 变量 | 说明 |
|------|------|
| CLASH_SUBSCRIPTION_URL | 必填。Clash 代理订阅地址 |
| PASSWORD | 统一密码：登录网关和 vault 口令是同一个。留空则首次启动随机生成并打印在启动横幅里（存在容器内 root 才能读的 `/var/lib/sdw/master-password`），但这个密码只存在于该容器内：重启容器会保留，**重建容器（`docker compose up -d` 在重新构建或 pull 之后、`docker rm`）会生成新密码**，已有 vault 就打不开了（要用旧密码执行 `unlock-vault`）。务必保存横幅里的密码，或直接在 `.env` 里设置 |
| EGRESS_ENFORCE | 出口防火墙，**默认开启**。`0` / `false` / `no` / `off` 关闭；其它值（包括写错的）保持开启。见上文「出口防火墙」 |
| EGRESS_ALLOW_CIDRS | 防火墙开着时允许**直连**（不经 Clash）的地址或 CIDR，逗号或空格分隔。最宽 /8（IPv6 /16），不接受域名；写错会让启动停下 |
| WORKLOAD_ENV | 要传给工作区用户（`dev`）的 `.env` 变量名，逗号分隔，如 `GITHUB_TOKEN,NPM_TOKEN`；密钥类变量被拒绝 |
| AUTH_SESSION_PERSIST | 默认关闭。`1` 时登录会话在容器重启后保留（签名密钥是随机的，存在 `/var/lib/sdw/session.key`，不由密码派生） |
| MEMORY_LIMIT / CPU_LIMIT / PIDS_LIMIT | 容器资源上限，默认不限内存和 CPU、PID 上限 4096。批量起多个实例时按实例设（如 `MEMORY_LIMIT=8g`）；低于约 1 GiB 可能连解锁都失败（gocryptfs 的 scrypt 瞬间要 256 MiB）。Docker 不会因为 unhealthy 而重启容器，卡死时 `docker restart` |
| DESKTOP_STACK | 构建时选择桌面栈：`kasmvnc`（默认）/ `selkies` / `both` |
| SYNC_ENABLED | 默认关闭。启用浏览器**单向上传**页 `/sync/`(把所选本地文件夹加密推送进 vault——**不是双向同步**:无下载、无删除、无冲突处理);启用后每次解锁会把 vault 口令写入 `/run/sdw/sync-passphrase`（仅 root 可读、不在 vault 里，`lock-vault` 会删掉它） |
| REMOTE_DESTRUCT_ENABLED | 默认关闭。启用远程自毁端点 `POST /sync/api/destruct`；口令用 `destruct-passphrase` 查看 |
| RESTIC_REPOSITORY | 设置后启用实时备份（还需 `RESTIC_PASSWORD` 与 S3 凭证）；仓库不存在时启动时自动创建。备份的是**明文**工作区，但**排除** `.credentials/`（SSH 密钥、同步口令）、`node_modules`、各类缓存/构建目录（清单见 `src/rust/core/src/backup/domain.rs` 的 `BACKUP_EXCLUSION_SET`）。**注意**：只配 restic 时 `.credentials/` 不在任何备份里——它只随加密 vault 的 git 复制（`VAULT_GIT_REPO`）走。restic 仓库是**明文**、仅由 `RESTIC_PASSWORD` 保护，自毁不删除它，要当作 vault 的一部分对待 |
| COMPOSE_FILE | 默认不设。叠加可选的覆盖文件（AppArmor、文件型密钥、pentest），见「快速开始」里的表 |
| `<NAME>_FILE` | 任一密钥（PASSWORD、CLASH_SUBSCRIPTION_URL、VAULT_GIT_REPO、RESTIC_PASSWORD、AWS_*、PENTEST_VAULT_PASSWORD、CROC_PASS）都可以给成「装着它的文件的路径」，这样环境里只有路径，`docker inspect` 和各进程的 `/proc/<pid>/environ` 里没有值。`docker-compose.secrets.yml` + `make secrets-init` 是现成的做法（PASSWORD 与订阅 URL）；非空的 `<NAME>` 仍然优先，所以要把它从 `.env` 删掉；读不了的文件会让容器拒绝启动。Compose 的文件型 secret 是宿主文件的 bind mount，不是 tmpfs |
| METRICS_ENABLED | 默认关闭。开启后 `GET /sync/api/metrics` 给出 Prometheus 文本指标（登录、网关放行/拒绝、自毁请求计数），在登录网关之后 |
| BUILD_PROXY / BUILD_NO_PROXY | 只用于 `docker compose build`：构建步骤走的 HTTP 代理（不写进镜像）。见「在 Mac 上本机构建」 |

## 验证（smoke test）

单元测试（`bun test`）不启动镜像。`src/tests/smoke/` 把真镜像和一个假上游一起起来，从外面检查启动、认证、vault、备份与复制、出口降级、自毁，加上 `--services` 时对真实的 S3 端点、sshd、git HTTP、Shadowsocks 节点，以及用真实 Chrome 看同步页和桌面画面。它有自己的 compose 项目、端口和网络，不读 `.env`，可以放心在已有真实工作区的机器上跑：

```bash
make smoke                                # 测刚用 `make build` 构建的镜像（dev-workspace:$WORKSPACE_VARIANT，取自 .env）
make smoke IMAGE=dev-workspace:trixie     # 测另一个已有的镜像
make smoke-build                          # 先按生产基础（Kali）另外构建一份再测（构建时间翻倍）
```

用法、检查清单和它证明 / 不证明什么见 [`src/tests/smoke/README.md`](src/tests/smoke/README.md)；CI 里对应 `.github/workflows/smoke.yml`（手动触发，或改动了镜像 / compose / 测试本身的 PR）。

## 维护

| 命令 | 作用 |
|------|------|
| `make check-tools` / `make update-tools` | 镜像里下载的第三方工具（`src/image/tools/*.env`）是否落后于各自项目的最新版 / 移到最新版并写入新的校验和（改动像依赖升级一样审阅）。CI 每周跑一次检查 |
| `make audit` | Rust 与 Bun 依赖的已知漏洞（`src/rust/.cargo/audit.toml` 里每条被忽略的公告都写明了理由） |
| `make lint-sh` | 全部 shell 脚本过一遍 shellcheck |
| `make install-hooks` | 每个 clone 做一次：提交前拒绝带进密钥文件，或带进 `.env` 里出现过的值 |
| `make secrets-init` | 把 PASSWORD 和订阅地址从 `.env` 写成 `secrets/*`（0600），配合 `docker-compose.secrets.yml` |
| `make env-example` | 改了 `env_schema.rs` 之后重新生成 `.env.example`（测试会在两者不一致时失败） |
| `make purge-env-history` | 把 `.env` 从整个 git 历史里清掉。**改写历史**：默认只演习，`YES=1` 才真做，会先备份。推送或分享之前，同时要轮换里面出现过的密钥 |

`.env` 不再被 git 跟踪（`.gitignore` 里有它），已经泄漏进历史的值要当作泄漏处理。

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



已验证：Apple Silicon Mac 上的 Colima（vz，真机）从零构建，并跑完整套 smoke。默认镜像（KasmVNC）`run.sh --services` **553 通过 / 0 失败 / 0 跳过**（2026-11-04）；`-full` 镜像（Selkies 2.2.0 + KasmVNC）的 Selkies 相关子集（启动、认证、真 Chrome 的桌面画面、权限分离、vault 锁定）也全过。详见 `deploy/docs/TODO.md`。

当前没验证的：
```yml
Docker Desktop 本身（Colima 已验证）和真实的 Linux 主机。
amd64（整个镜像没在 amd64 上构建或运行过）。
GitHub Actions（CI / smoke / build-image / fuzz / tools 都只在本机等价环境跑过）。
Cloudflare Access 和命名隧道（Access 放行只用合成密钥做过单元测试）。
真实 R2 和 GitHub，用的是本地替身。
机场节点的其他协议、真实订阅下的 mode:rule / REJECT。
Tor 引导到 30% 以上，没等到 100%；出口防火墙开启时的 Tor。
没完整建成过 Kali rootfs。
Kata microVM 隔离已接进编排并本机端到端跑通（`deploy/scripts/setup-kata-runtime.sh` + `deploy/orchestration/up-kata.sh`/`down-kata.sh`：完整工作区起进 microVM，vault/egress 防火墙/网关登录都通，runp 实测 10/10 稳定）。**未在生产环境验证的**：裸机 Linux 上的性能（嵌套 vz 下 virtio-fs I/O 慢，重型 GUI 会卡）、Mac 宿主端口暴露（需 `colima ssh -L` 隧道）、多实例 + Cloudflare 边缘的接入、Kata 下重跑整套 smoke。gVisor 仍不适用（netstack 无 nftables）。
Selkies：GPU 硬件编码（容器里用软件 x264）、浏览器端出声、Firefox/Safari 客户端（只试过 Chrome）。
防火墙后的 GUI 应用：只验证了镜像自带的 Firefox ESR；Chromium/IDEA/Burp 没测。
```