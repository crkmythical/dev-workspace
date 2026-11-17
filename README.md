# Secure Dev Workspace

加密、云端就绪的开发环境：远程桌面 + Code Server + 出口防火墙，一道登录全部打通。业务代码 **Rust-first**（Linux 运行时、CLI、守护进程、鉴权/同步服务、浏览器界面编译为 WebAssembly），Shell 只做宿主安装与容器编排。

> 深入文档见 [`deploy/docs/`](deploy/docs/)：上手 [`QUICKSTART.md`](deploy/docs/QUICKSTART.md)、架构 [`architecture.md`](deploy/docs/architecture.md)、威胁模型 [`threat-model.md`](deploy/docs/threat-model.md)、运维 [`runbook.md`](deploy/docs/runbook.md)。

## 快速开始

三步：拿镜像 → 配 `.env` → 启动。

```bash
# 1. 获取镜像 —— 二选一
# A) 下载预构建镜像（GitHub Release，按架构选一个导入）
docker load < dev-workspace-kali-kasmvnc-linux-arm64.tar.gz   # ARM Linux / Android Podroid
docker load < dev-workspace-kali-kasmvnc-linux-amd64.tar.gz   # x86_64 Linux
# 导入后镜像名是 dev-workspace-kali-kasmvnc，给它打上 compose 期望的标签：
docker tag dev-workspace-kali-kasmvnc dev-workspace:kali-kasmvnc
# B) 或本地构建（见「本地构建」）
docker compose build

# 2. 配置（全部可选，可直接用默认）
install -m 600 .env.example .env

# 3. 启动
docker compose up -d
```

访问（端口 18080 是 `TUNNEL_HOST_PORT` 的默认值，改了启动横幅会打印实际端口）：

- **Code**：`http://localhost:18080`
- **桌面**：`http://localhost:18080/vnc/`（KasmVNC，默认）
- **Selkies**：`http://localhost:18080/desktop/`（`DESKTOP_STACK=selkies|both` 的镜像）
- **登录**：任一地址先到登录页，输入 `PASSWORD`（没有用户名），之后所有入口共用这一个会话

**什么都不填也能起**：`CLASH_SUBSCRIPTION_URL` 可留空——工作区照常启动（code-server、桌面、vault 都是本地的，不需要代理）。只是出口防火墙默认开着，没有订阅时 Clash 没有节点，联网操作（git / curl / npm / 浏览器）会被挡住，直到你填上订阅（之后 `docker compose up -d` 重启生效），或设 `EGRESS_ENFORCE=0` 允许直连。出口**不会**偷偷退回直连。`PASSWORD` 留空则首次启动随机生成并打印在横幅里。

**Linux / Colima（带 AppArmor）宿主**：Docker 默认 AppArmor 禁止 FUSE 挂载，vault 会一直锁着并报 `mount failed: Permission denied`（Docker Desktop 不受影响）。在 `.env` 里取消注释 `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml`。可叠加的覆盖文件（`COMPOSE_FILE` 用冒号连接）：

| 覆盖文件 | 何时需要 |
|---|---|
| `docker-compose.apparmor.yml` | Linux / Colima（AppArmor）宿主，否则 vault 挂载被拒 |
| `docker-compose.secrets.yml` | 把 `PASSWORD`、订阅地址改成文件密钥（`make secrets-init`） |
| `docker-compose.pentest.yml` | pentest 插件：补上 `SYS_CHROOT` / `MKNOD` / `NET_RAW` |

## 启动参数（`.env`）

完整列表、默认值和说明在 [`.env.example`](.env.example)（由 `src/rust/core/src/workspace/env_schema.rs` 生成，改后运行 `make env-example`）。**全部可选**，最常用的：

| 变量 | 默认 | 说明 |
|---|---|---|
| `CLASH_SUBSCRIPTION_URL` | 空 | Clash 订阅地址，工作区的出口通道。留空照常启动；防火墙开着时无法联网，直到填上或设 `EGRESS_ENFORCE=0`。不回退直连 |
| `PASSWORD` | 随机 | 登录网关与 vault 的统一口令。留空＝首次启动随机生成并打印在横幅；**重建容器会换新口令**，已有 vault 需用旧口令 `unlock-vault` |
| `TUNNEL_HOST_PORT` | `18080` | Web UI 发布在 `127.0.0.1` 的宿主端口 |
| `DESKTOP_RESOLUTION` | `1920x1080` | KasmVNC 分辨率（Selkies 自适应浏览器窗口，忽略此项） |
| `EGRESS_ENFORCE` | 开 | 出口防火墙。`0` / `false` / `no` / `off` 关闭；其它值（含写错的）保持开启 |
| `EGRESS_ALLOW_CIDRS` | 空 | 防火墙开着时允许**直连**（不经 Clash）的地址 / CIDR，逗号或空格分隔，最宽 /8（IPv6 /16） |
| `MEMORY_LIMIT` / `CPU_LIMIT` / `PIDS_LIMIT` | 不限 / 不限 / `4096` | 容器资源上限，批量起多实例时按实例设 |
| `AUTH_SESSION_PERSIST` | 关 | `1`＝登录会话在容器重启后保留 |
| `SYNC_ENABLED` | 关 | 浏览器**单向上传**页 `/sync/`（把所选文件夹加密推进 vault，不是双向同步） |
| `REMOTE_DESTRUCT_ENABLED` | 关 | 远程自毁端点 `POST /sync/api/destruct` |
| `RESTIC_REPOSITORY` | 空 | 设置即启用实时备份（还需 `RESTIC_PASSWORD` + S3 凭证） |
| `COMPOSE_FILE` | 空 | 叠加可选覆盖文件，见上表 |
| `<NAME>_FILE` | 空 | 任一密钥改用「文件路径」提供，环境里不留明文（`docker-compose.secrets.yml` + `make secrets-init`） |

受限网络的构建代理 / 镜像源（`APT_MIRROR`、`KALI_MIRROR`、`BUILD_PROXY`、`SDW_GH_MIRROR`）只用于 `docker compose build`，说明见 `.env.example` 的 “IMAGE BUILD”。

## 容器内命令

```bash
doctor                   # 健康检查（出口 / 已装桌面 / 备份）
unlock-vault             # 输入密码解锁加密工作区
lock-vault               # 锁定（先停桌面和备份）
desktop-start [vnc]      # 启动全部桌面 / 仅 KasmVNC
desktop-stop  [selkies]  # 停止全部桌面 / 仅 Selkies
set-clash-dns …          # 查看 / 修改 Clash 解析器（持久、热加载，不用重建）
audit-log [--json] [N]   # 最近 N 条审计事件（默认 20）
```

code-server / 桌面里的 `dev` 用户只有一个很窄的无密码桥接：`doctor`、`lock-vault`、`unlock-vault`、`desktop-start/stop`。其余接受任意路径、装软件、改代理、读审计、自毁的命令必须由运营者从宿主执行：

```bash
docker exec -u root dev-workspace <命令>              # setup / set-clash-dns / init-* / pentest 等
docker exec -it  dev-workspace self-destruct --force  # 自毁，见 deploy/docs/runbook.md
docker exec -it  dev-workspace destruct-passphrase    # 远程自毁口令（由 vault 口令派生，随时可重看）
```

## 构建选项

每次选一个基础系统 + 一组兼容套件；发布工作流分别构建 amd64 / arm64。兼容清单见 [`src/image/profiles.json`](src/image/profiles.json)。

| 参数 | 可选值 | 默认 |
|---|---|---|
| `BASE_PROFILE` | `kali` / `debian` / `alpine` | `kali` |
| `DESKTOP_STACK` | `none` / `kasmvnc` / `selkies` / `both` | `kasmvnc` |
| `EDITOR_SUITE` | `none` / `code-server` | `code-server` |
| `DEV_RUNTIMES` | `none` / `standard`（Java、Node、pnpm、Python） | `standard` |
| `DESKTOP_APPS` | `none` / `firefox` | `firefox` |
| `PENTEST_SUITE` | `none` / `bootstrap` | `bootstrap` |
| `BACKUP_TRANSFER` | `none` / `standard`（restic、croc） | `standard` |

Kali / Debian 支持上表全部选择；Alpine 当前只支持所有套件为 `none`。不兼容组合在下载和编译前直接失败。

```bash
make build                       # 默认：Kali + KasmVNC + code-server + 标准工具

BASE_PROFILE=debian BASE_IMAGE=debian:trixie-slim WORKSPACE_VARIANT=debian-selkies \
  DESKTOP_STACK=selkies DESKTOP_APPS=none make build       # Debian + Selkies

BASE_PROFILE=alpine BASE_IMAGE=alpine:3.22 WORKSPACE_VARIANT=alpine-minimal \
  DESKTOP_STACK=none EDITOR_SUITE=none DEV_RUNTIMES=none DESKTOP_APPS=none \
  PENTEST_SUITE=none BACKUP_TRANSFER=none make build        # Alpine 最小镜像
```

`WORKSPACE_VARIANT` 只控制本地镜像标签，不决定内容（实际配置写在 `/etc/sdw-build.json` 和 OCI labels）。

## 本地构建（Mac + Colima）

已在 Apple Silicon（Colima + vz/virtiofs，Docker 29）从零构建 Kali 默认栈成功，约 15 分钟。三件事缺一不可：**buildx 插件**（否则退回经典构建器，Dockerfile 的 `COPY --chmod` 会失败）、**够大的 VM**（默认 2C/2G 会 OOM，给工作区单独起一个 profile）、**能到 GitHub / Docker Hub / crates.io / apt 的网络**。

```bash
brew install colima docker docker-compose docker-buildx
ln -sfn /opt/homebrew/lib/docker/cli-plugins/docker-buildx ~/.docker/cli-plugins/docker-buildx
deploy/scripts/setup-colima-build-vm.sh --proxy http://192.168.5.2:1080   # 专用 VM，--proxy 可选
export DOCKER_CONTEXT=colima-dev-workspace   # 每个跑 docker/make 的 shell 都要；别用 docker context use
make build    # 从零约 15 分钟；改 Rust 增量约 3-5 分钟
make smoke    # 对刚构建的镜像跑完整 smoke（本机 Colima 全跑特权段）
```

国内网络在 `.env` 按需加 `KALI_MIRROR=mirrors.aliyun.com`、`BUILD_PROXY=http://192.168.5.2:1080`、`COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml`。详细排错见 [`deploy/docs/QUICKSTART.md`](deploy/docs/QUICKSTART.md)。

## 重建 / 重置

```bash
docker compose down                              # 停止
docker compose build && docker compose up -d     # 代码变更后重建
# 完全重置（清除 vault 数据）：
docker compose down && docker volume rm dev-workspace_vault-data \
  && docker compose build --no-cache && docker compose up -d
```

## 远程访问（Cloudflare Tunnel）

零入站端口把工作区暴露到公网：

```bash
brew install cloudflared                                      # 宿主先装
./deploy/scripts/setup-cloudflared.sh --hostname <你的域名>   # 登录你的 Cloudflare 账号，创建/复用 tunnel + DNS
# 然后 https://<你的域名>（/vnc/、/desktop/ 同快速开始）
```

脚本写入独立的 `~/.cloudflared/sdw-<name>.yml`，不覆盖其它 tunnel；macOS 装并加载独立 LaunchAgent。没有默认域名，必须用 `--hostname` 或 `.env` 的 `TUNNEL_HOSTNAME` 指定。Linux 另运行 `bash deploy/linux-systemd/install.sh --cloudflared`。

## 安全模型（简述）

- **出口防火墙（默认开启）** — 工作区一切只能走 Clash：nftables 默认丢弃所有不经 Clash 的出站，忽略代理的进程、UDP/QUIC、自己解析域名的程序都被拦，不会带本机真实 IP 直连。订阅没有可用节点时请求失败，**不退回直连**。`EGRESS_ENFORCE=0` 关闭，`EGRESS_ALLOW_CIDRS` 放行指定网络直连。需要 `NET_ADMIN`，装不上时启动会停在 `enforce-egress`，而不是带着以为封死的状态启动。
- **统一认证** — 只有一道登录（Caddy 前的会话网关），签名 cookie 被 code-server / `/vnc/` / `/desktop/` / `/sync/` 共用（含 WebSocket）；后端自身无密码（code-server `--auth none`、KasmVNC `-DisableBasicAuth`）。可选接受 Cloudflare Access 注入的 JWT。登录限速、异源 WebSocket 拒绝、审计日志全程记录。
- **权限分离** — 你运行的一切都是普通用户 `dev`（uid 1000，家目录在 vault 里），无 capability、看不到任何密钥；少数需要 root 的命令经 `doas` 窄桥接。要给工具看某个 `.env` 变量用 `WORKLOAD_ENV`，密钥类会被拒。
- **供应链** — 镜像下载的每个工具都固定版本和 sha256，不符即构建失败。

完整边界、做了与没做什么见 [`deploy/docs/threat-model.md`](deploy/docs/threat-model.md)。

## 功能

远程桌面（KasmVNC 默认 / Selkies 可选 / 两者并行）、Code Server、gocryptfs 加密 vault（自动初始化解锁）、Clash 代理 + 出口防火墙、权限分离、固定校验和的供应链、PulseAudio 桌面音频、Kali 基础（apt 装渗透工具）。

## 迁移与销毁

```bash
./bootstrap.sh --vault-repo <VAULT_GIT_REPO>   # 迁移到新机器（先克隆密文再首启）；--init 全新 vault
./destroy.sh  --force                          # 删本项目容器/卷/镜像（按 compose 项目标签，不碰别的项目）
./destroy.sh  --force --delete-project         # 连这份 checkout 一起删
```

迁移时 `.env` 的 `PASSWORD` 必须是旧 vault 口令；卷里已有 vault 时 `bootstrap.sh` 拒绝覆盖。

## 验证与 CI/CD

Actions 页面三个手动构建入口（**构建镜像 — Kali / Debian / Alpine**）调用同一个 [共享构建流程](.github/workflows/build-image.yml)，关联规则来自 [`src/image/profiles.json`](src/image/profiles.json)；另有 **测试全部镜像选项（覆盖矩阵）**。每次运行只产出两个 Actions artifact（保留 7 天）：

```text
dev-workspace-<base>-<desktop>-linux-amd64.tar.gz
dev-workspace-<base>-<desktop>-linux-arm64.tar.gz
```

editor 或任一套件偏离基础系统默认值时，文件名和加载后的镜像名会追加全部选择以免互相覆盖。推送 `v*` tag 用 Kali 默认选项构建，并把这两个文件放进 GitHub Release（不再生成 GHCR/Docker Hub 标签、`-full` 变体、`sdw-host` 包、SBOM 或截图）。

本地验证最权威：`make check`（fmt + clippy + test + shellcheck + 宿主测试）、`make build`、`make smoke`（托管 runner 的 CI smoke 会跳过特权挂载段，本机 Colima 不跳）。`v1.0.0` 两文件 Release 已发布；最新一次[覆盖矩阵运行](https://github.com/crkmythical/dev-workspace/actions/runs/43895296656)在固定的 Ubuntu 24.04 runner 上 40/40 jobs 通过，覆盖清单定义的 108 个两两选项组合。

## 维护

| 命令 | 作用 |
|---|---|
| `make check-tools` / `make update-tools` | 核对发布方 checksum 后事务式更新 `src/image/tools/*` 的工具 pin；CI 每周只读检查 |
| `make audit` | 全部 Rust 依赖的已知漏洞扫描 |
| `make lint-sh` | shellcheck 全部 shell 脚本 |
| `make install-hooks` | 每个 clone 做一次：提交前拒绝带进密钥文件或 `.env` 里出现过的值 |
| `make env-example` | 改 `env_schema.rs` 后重新生成 `.env.example`（两者不一致测试会失败） |

宿主高风险逻辑统一在 `sdw-host`（销毁、watcher、Cloudflare、systemd、密钥初始化、工具 pin 更新、Git 历史清理）；`make host` / `make install-host` 构建安装，Shell 入口只定位项目并 `exec`。
