# Secure Dev Workspace — 启动指南

## 0. 前置条件
- **Docker**:Docker Desktop(macOS,无 AppArmor,最省心)或 Colima。建议给 Docker ≥ 12 GB 内存、≥ 100 GB 磁盘(Colima 默认的 2 CPU / 2 GiB 编译 Rust 会被 OOM 杀掉)。**必须装 buildx 插件**:没有它 `docker compose build` 会悄悄退回经典构建器,这个 Dockerfile 的 `COPY --chmod` 过不去。Mac 上用 Colima 的完整步骤(含专用 VM、国内网络、代理)见 README 的「在 Mac 上本机构建」。
- `git`、`curl`。
- **一个 Clash 订阅地址**(`CLASH_SUBSCRIPTION_URL`,唯一必填项)。

> ⚠️ Colima / Linux 且发行版带 AppArmor:Docker 默认 profile 会挡住 gocryptfs 的 FUSE 挂载,vault 会解不开(日志报 `fusermount3: mount failed: Permission denied`)。解决:在 `.env` 里取消注释 `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml`。Docker Desktop(macOS)无此问题。

---

## 1. 最快路径(本地、全新 vault)

```bash
# 1) 配置:只有 CLASH_SUBSCRIPTION_URL 必填;强烈建议同时设 PASSWORD
cp .env.example .env
#   编辑 .env:
#     CLASH_SUBSCRIPTION_URL=https://你的订阅?target=clash
#     PASSWORD=选一个强密码           # 留空=首次启动随机生成并打印在日志里
#   (可选)要 Kali 渗透工具基:再设
#     BASE_IMAGE=kalilinux/kali-rolling
#     WORKSPACE_VARIANT=latest

# 2) 构建镜像(默认 Debian trixie 精简基;首次构建较久,国内走 aliyun 源)
docker compose build

# 3) 启动
docker compose up -d

# 4) 看启动横幅(拿密码/确认 vault 已解锁/看访问地址)
docker logs -f dev-workspace
```

**首次启动会自动做什么**(容器入口是 `sdw entrypoint`):校验必填变量 → **装载出口防火墙(默认开启)** → 起 Clash 探测出口 → **用 PASSWORD 自动创建并解锁 vault** → 起 Caddy / code-server / 桌面 / 同步服务 → 打印横幅。所以**全新启动不需要手动 init**。横幅示例:

```
╔══════════════════════════════════════════════════╗
║          Secure Dev Workspace Ready              ║
╠══════════════════════════════════════════════════╣
║  Password:  <你的密码 或 "<set via PASSWORD in .env>">
║  Code:      http://localhost:18080               ║
║  VNC:       http://localhost:18080/vnc/          ║
║  Vault:     unlocked                             ║
║  Egress:    confirmed                            ║
║  Firewall:  on (direct egress blocked)           ║
╚══════════════════════════════════════════════════╝
```
> PASSWORD 留空时,横幅里会打印那串随机生成的密码——**一定要记下来**(它只存在于这个容器里,重建容器会变)。

> **出口防火墙默认开启**:工作区里的一切只能经 Clash 出去,忽略代理变量的进程(原始 socket、UDP/QUIC、自己解析域名的程序)和 `noproxy` 会被丢弃,而不是带着本机真实 IP 直连。局域网或另一个 Docker 网络里的主机默认也连不上:把它们写进 `.env` 的 `EGRESS_ALLOW_CIDRS=192.168.1.0/24,10.0.0.5`。宿主不允许容器有 `NET_ADMIN`(或没有 nftables)时启动会停下并指向 `EGRESS_ENFORCE=0`。细节见 `deploy/docs/runbook.md` 的「Egress firewall」。

---

## 2. 访问与登录
| 入口 | 地址 | 认证 |
|---|---|---|
| VS Code(code-server) | http://localhost:18080 | 打开后是登录页,输入 PASSWORD(没有用户名) |
| 远程桌面(KasmVNC) | http://localhost:18080/vnc/ | 同一个登录会话,**不会再弹第二次** |
| 上传页/单向(需 `SYNC_ENABLED=1`) | http://localhost:18080/sync/ | 同一个登录会话;页面内再输 vault 口令来派生上传密钥 |

登录网关是唯一的一道登录:code-server 以 `--auth none`、KasmVNC 以 `-DisableBasicAuth` 运行,不再各有一个密码。你在 code-server 终端和桌面里运行的一切都以**普通用户 `dev`**(uid 1000,家目录 `/workspace/home`,在 vault 里)运行,看不到 `PASSWORD`、备份和订阅密钥。无密码 `doas` 桥接只保留 `doctor`、`lock-vault`、`unlock-vault`、`desktop-start`、`desktop-stop`;安装软件、任意路径传输、pentest rootfs、审计读取和代理配置命令都必须由宿主执行 `docker exec -u root dev-workspace <命令>`。要让普通开发工具看到某个 `.env` 变量,把名字写进 `WORKLOAD_ENV=GITHUB_TOKEN,NPM_TOKEN`(工作区自身密钥会被拒绝)。

端口 18080 来自 `TUNNEL_HOST_PORT`(默认值);改了它横幅会打印实际端口。桌面默认 `autostart=false`,浏览器打开 `/vnc/` 即会按需拉起;或在容器内 `desktop-start`。桌面程序的日志在 `/var/log/desktop/<程序>.log`(各 2 MB,留一份备份)。

可选的 compose 覆盖文件(加进 `.env` 的 `COMPOSE_FILE=docker-compose.yml:…`,用冒号连接):

| 文件 | 用途 |
|---|---|
| `docker-compose.apparmor.yml` | Linux / Colima(带 AppArmor)宿主,否则 vault 挂载被拒 |
| `docker-compose.secrets.yml` | 把 PASSWORD 和订阅地址改成文件密钥(`make secrets-init`) |
| `docker-compose.pentest.yml` | pentest 插件需要的 `SYS_CHROOT` / `MKNOD` / `NET_RAW`;不加它,`pentest-bootstrap` 会明说缺什么然后退出 |

---

## 3. 常用命令(在容器里跑)
```bash
docker exec -it dev-workspace doctor         # 健康检查(出口/桌面/备份)
docker exec -it dev-workspace unlock-vault   # 手动解锁(若启动时没自动解开)
docker exec -it dev-workspace lock-vault     # 锁定(会先停桌面+备份)
docker exec -it dev-workspace desktop-start  # 启动桌面
docker exec -it dev-workspace sdw            # 看所有子命令
```

---

## 4. 停止 / 重启 / 重置
```bash
docker compose down                 # 停止(保留 vault 数据卷)
docker compose up -d                # 再次启动(用同一密码自动解锁)

# 代码改动后重建
docker compose build && docker compose up -d

# 彻底重置(⚠️ 清空 vault 数据,不可逆)
docker compose down
docker volume rm dev-workspace_vault-data
docker compose up -d
```

---

## 5. 远程访问(可选,Cloudflare Tunnel,零入站端口)
```bash
# 宿主机执行(不是容器内)
./deploy/scripts/setup-cloudflared.sh --hostname workspace.example.com
# macOS:
launchctl load ~/Library/LaunchAgents/com.cloudflare.tunnel.plist
# Linux: bash deploy/linux-systemd/install.sh --cloudflared
```
之后从任何地方访问 `https://workspace.example.com`(及 `/vnc/`)。

---

## 6. 一键脚本(等价于上面的 1–3 + 宿主设置)
```bash
./bootstrap.sh --init                        # 全新 vault
./bootstrap.sh --vault-repo git@host:you/dev-vault.git   # 从已有加密 vault 仓库迁移
```
它会:查依赖 → `cp .env.example .env` 并让你编辑 → `docker compose build` → 跑 `setup-docker.sh`/`setup-cloudflared.sh` → 迁移模式下**先把克隆来的密文放进 vault 卷**(在容器第一次启动之前;卷里已有 vault 时拒绝覆盖) → `up -d` → 问是否装 host-watcher(自毁后宿主清理)。迁移时 `.env` 里的 `PASSWORD` 必须是旧 vault 的口令。

---

## 7. 构建慢 / 受限网络
镜像要下 Kali/Debian apt 包 + caddy/clash/restic/croc/code-server/kasmvnc + mise 的 java/node/python,首次较久。
- Debian 基:`APT_MIRROR`(默认 `mirrors.aliyun.com`;境外设 `deb.debian.org`)
- Kali 基:`KALI_MIRROR`(默认 `http.kali.org`;国内用 `mirrors.aliyun.com`。TUNA 的 Kali 镜像回 403,不要用)
- GitHub / crates.io / npm / Docker Hub 时通时断:`BUILD_PROXY=http://<VM 看到的宿主地址>:<端口>`(构建步骤走它,不写进镜像);Docker 守护进程拉基础镜像要在守护进程上配代理,`deploy/scripts/setup-colima-build-vm.sh --proxy …` 会做
- GitHub 下载已带重试(`--retry 5 --retry-all-errors`)

在 `.env` 设置,或 `KALI_MIRROR=... docker compose build`。

---
更深入的架构与运维见 `deploy/docs/architecture.md`、`deploy/docs/runbook.md`、`deploy/docs/threat-model.md`;完整环境变量见 `.env.example`。
