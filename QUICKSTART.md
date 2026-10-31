# Secure Dev Workspace — 启动指南

## 0. 前置条件
- **Docker**:Docker Desktop(macOS,无 AppArmor,最省心)或 Colima(`colima start --cpu 4 --memory 8`)。建议给 Docker ≥ 12 GB 内存、≥ 100 GB 磁盘。
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

**首次启动会自动做什么**(容器入口是 `sdw entrypoint`):校验必填变量 → 起 Clash 探测出口 → **用 PASSWORD 自动创建并解锁 vault** → 起 Caddy / code-server / 桌面 / 同步服务 → 打印横幅。所以**全新启动不需要手动 init**。横幅示例:

```
╔══════════════════════════════════════════════════╗
║          Secure Dev Workspace Ready              ║
╠══════════════════════════════════════════════════╣
║  Password:  <你的密码 或 "<set via PASSWORD in .env>">
║  Code:      http://localhost:18080               ║
║  VNC:       http://localhost:18080/vnc/          ║
║  Vault:     unlocked                             ║
║  Egress:    confirmed                            ║
╚══════════════════════════════════════════════════╝
```
> PASSWORD 留空时,横幅里会打印那串随机生成的密码——**一定要记下来**(它只存在于这个容器里,重建容器会变)。

---

## 2. 访问与登录
| 入口 | 地址 | 认证 |
|---|---|---|
| VS Code(code-server) | http://localhost:18080 | `user` / 你的 PASSWORD |
| 远程桌面(KasmVNC) | http://localhost:18080/vnc/ | 同上(KasmVNC 会再弹一次,正常) |
| 同步页(需 `SYNC_ENABLED=1`) | http://localhost:18080/sync/ | 页面内输 vault 口令 |

端口 18080 来自 `TUNNEL_HOST_PORT`(默认值);改了它横幅会打印实际端口。桌面默认 `autostart=false`,浏览器打开 `/vnc/` 即会按需拉起;或在容器内 `desktop-start`。

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
./scripts/setup-cloudflared.sh --hostname workspace.example.com
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
它会:查依赖 → `cp .env.example .env` 并让你编辑 → `docker compose build` → 跑 `setup-docker.sh`/`setup-cloudflared.sh` → `up -d` → 问是否装 host-watcher(自毁后宿主清理)。

---

## 7. 构建慢 / 受限网络
镜像要下 Kali/Debian apt 包 + caddy/clash/restic/croc/code-server/kasmvnc + mise 的 java/node/python,首次较久。
- Debian 基:`APT_MIRROR`(默认 `mirrors.aliyun.com`;境外设 `deb.debian.org`)
- Kali 基:`KALI_MIRROR`(默认 `http.kali.org`;慢则 `mirrors.tuna.tsinghua.edu.cn`)

在 `.env` 设置,或 `APT_MIRROR=... docker compose build`。

---
更深入的架构与运维见 `docs/architecture.md`、`docs/runbook.md`、`docs/threat-model.md`;完整环境变量见 `.env.example`。
