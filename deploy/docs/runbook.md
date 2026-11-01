# Operations Runbook

## First-Time Setup

```bash
cp .env.example .env          # then set CLASH_SUBSCRIPTION_URL (required) and, better, PASSWORD
docker compose build          # ~5 min first time
docker compose up -d          # creates AND unlocks the vault on first start; the banner shows the state
./deploy/scripts/setup-cloudflared.sh --hostname <hostname>
```

Set `PASSWORD` before the first start. Left empty, a random one is generated and printed in the
banner, but it lives only inside that container: a restart keeps it, recreating the container
(`docker compose up -d` after a rebuild or pull, `docker rm`) generates a new one, and the vault
made under the old one then stays locked until you give it the old password
(`docker exec -it dev-workspace unlock-vault`). Save the printed password, or set it in `.env`.

## Daily Operations

```bash
docker exec -it dev-workspace unlock-vault   # Start of day
docker exec -it dev-workspace lock-vault     # End of day
docker exec -it dev-workspace doctor         # Health check
```

## Access

- Local: http://localhost:18080
- Remote: https://<hostname> (the domain you gave `deploy/scripts/setup-cloudflared.sh`)
- File upload (one-way push into the vault): https://<hostname>/sync/ (needs `SYNC_ENABLED=1`)

## Password Rotation

The vault, code-server, the desktops and the sync key share one password, so rotating it means
changing the vault passphrase and `PASSWORD` together. There is no `change-password` command
(older versions had one). Needs an explicit `PASSWORD` in `.env`: a generated one stays in
code-server's config and would still be the old one at the next start.

```bash
docker exec -it dev-workspace lock-vault
docker exec -it dev-workspace gocryptfs -passwd /vault/cipher   # asks for the old, then the new password
# set PASSWORD=<new password> in .env, then recreate so every service takes it:
docker compose up -d --force-recreate
docker exec -it dev-workspace destruct-passphrase                # the remote self-destruct passphrase changes with it
# Then: Sync Page → Clear key → Set passphrase (new password)
```

Checked in a container: after these steps the vault opens with the new password and keeps its
data, `/vnc/` accepts the new password and rejects the old one, code-server's config and the sync
key file carry the new one. The Sync Page step (browser side) was not exercised.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Clash not connecting | Check subscription URL; verify provider loaded: `curl -s http://127.0.0.1:9090/providers/proxies/subscription \| jq '.proxies \| length'`; restart container if needed |
| Start-up banner says `Egress: DEGRADED` or `DOWN` | The workspace started anyway. `doctor` shows the live state; `cat /var/run/egress-status` shows what the start-up probe saw; with the vault open, `/workspace/.notifications/egress-degraded.md` explains. DOWN: nothing listens on :7890 (check the Clash config and subscription URL). DEGRADED: proxy up but the probe failed, usually nodes still loading; retry `doctor` in a minute |
| Vault unlock fails, `Password incorrect` | Wrong passphrase. If `PASSWORD` is empty and the container was recreated, the start-up generated a new password and the vault still wants the old one: run `docker exec -it dev-workspace unlock-vault` with the old one, then put it in `PASSWORD` in `.env` (the start-up prints this hint) |
| Vault unlock fails, `fusermount3: mount failed: Permission denied` | The host's AppArmor profile forbids the FUSE mount (Ubuntu, Debian). Uncomment `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml` in `.env` and `docker compose up -d`; see the AppArmor note in `deploy/docs/threat-model.md` before doing so |
| `lock-vault` asks `Force unmount?` | Some process still has the vault in use: files open (code-server once it has opened something there, a shell inside it) or a program listening on a unix socket inside it (`grep ' /workspace' /proc/net/unix`; `fuser` does not show those). `y` detaches the mount at once, but those processes keep what they already opened until they exit; restart the container to be sure. Answering `N` leaves the vault mounted and starts the desktops and the backup watcher again, which the attempt had stopped |
| Vault unlock fails, other | Check `gocryptfs.conf` exists in the cipher volume; the start-up prints what gocryptfs said |
| Sync Page idle | Modify a file in the synced folder; check browser console |
| code-server 502 | Wait 10s after container start; check `docker logs` |

## Destroy

```bash
./destroy.sh --force              # Standard
./destroy.sh --force --paranoid   # Deep clean (history, DNS, creds)
```

## Emergency Self-Destruct

三种触发方式，任选其一：

### 1. 容器内 CLI（最快）

```bash
docker exec -it dev-workspace self-destruct --force
# 可选: --skip-shred (跳过数据覆写，更快) --remote (同时删除 GitHub vault repo)
```

### 2. 远程 API（无需 SSH/exec 权限）

```bash
curl -X POST https://<hostname>/sync/api/destruct \
  -H "Content-Type: application/json" \
  -d '{"passphrase":"<your-destruct-passphrase>"}'
# 200 = 成功; 404 = 密码错误; 429 = 速率限制
```

### 3. 浏览器 SPA

访问 `https://<hostname>/sync/#emergency`，输入 destruct passphrase 和确认词 "DESTROY"。

### Destruct Passphrase

- 首次 `unlock-vault` 时自动打印（仅一次）
- 派生自 vault passphrase (HKDF-SHA256)
- 密码轮换后自动更新（下次 unlock 静默刷新）
- 保存到密码管理器

### 销毁后行为

1. 容器进入 inert 模式 (`sleep infinity`)
2. 宿主机 host-watcher 检测到 marker → 自动执行 `destroy.sh --force --paranoid`（在监视器的服务之外运行：`--paranoid` 会卸载监视器自己的服务，作为子进程运行会被一起杀掉，清理在这一步被截断；已修复，升级后需要重启监视器）
3. 清理: `docker compose down -v` 即可移除 inert 容器

### 安装 host-watcher

```bash
bash deploy/scripts/host-watcher.sh --install    # 安装 LaunchAgent
bash deploy/scripts/host-watcher.sh --uninstall  # 卸载
```

## Remote Desktop

桌面环境按需启动，不影响容器空闲时的资源占用。

### 启动/停止

```bash
docker exec -it dev-workspace desktop-start   # 启动，等待 KasmVNC 就绪
docker exec -it dev-workspace desktop-stop    # 停止所有桌面进程
```

启动后通过浏览器访问: `/vnc/`（KasmVNC，默认）；`DESKTOP_STACK=selkies|both` 的镜像另有 `/desktop/`（Selkies）

### 安装 GUI 应用

```bash
docker exec -it dev-workspace desktop-install firefox
docker exec -it dev-workspace desktop-install chromium
docker exec -it dev-workspace desktop-install wireshark
docker exec -it dev-workspace desktop-install idea
docker exec -it dev-workspace desktop-install burpsuite
```

### 性能调优

- 降低分辨率: 在 `.env` 中设置 `DESKTOP_RESOLUTION=1600x900`
- 调整帧率: 编辑 `/workspace/.desktop/.config/kasmvnc/kasmvnc.yaml` 中的 `frame_rate`
- 降低画质: 调低 `dynamic_quality_max` (默认 9，可降到 6)

### 与 Pentest 环境联动

桌面启动后，pentest 环境内的 GUI 程序可直接显示在宿主桌面上:

```bash
docker exec -it dev-workspace pentest
# inside chroot:
DISPLAY=:1 wireshark &
```
