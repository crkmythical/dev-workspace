# Operations Runbook

## First-Time Setup

```bash
cp .env.example .env          # Subscription URL pre-filled
docker compose build          # ~5 min first time
docker compose up -d
docker exec -it dev-workspace init-vault
docker exec -it dev-workspace unlock-vault
./scripts/setup-cloudflared.sh
```

## Daily Operations

```bash
docker exec -it dev-workspace unlock-vault   # Start of day
docker exec -it dev-workspace lock-vault     # End of day
docker exec -it dev-workspace doctor         # Health check
```

## Access

- Local: http://localhost:18080
- Remote: https://workspace.cicd.dpdns.org
- File sync: https://workspace.cicd.dpdns.org/sync/

## Password Rotation

```bash
docker exec -it dev-workspace lock-vault
docker exec -it dev-workspace change-password
docker exec -it dev-workspace unlock-vault  # Use new password
# Then: Sync Page → Clear key → Set passphrase (new password)
```

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Clash not connecting | Check subscription URL; restart container |
| Vault unlock fails | Verify passphrase; check gocryptfs.conf exists |
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
curl -X POST https://workspace.cicd.dpdns.org/sync/api/destruct \
  -H "Content-Type: application/json" \
  -d '{"passphrase":"<your-destruct-passphrase>"}'
# 200 = 成功; 404 = 密码错误; 429 = 速率限制
```

### 3. 浏览器 SPA

访问 `https://workspace.cicd.dpdns.org/sync/#emergency`，输入 destruct passphrase 和确认词 "DESTROY"。

### Destruct Passphrase

- 首次 `unlock-vault` 时自动打印（仅一次）
- 派生自 vault passphrase (HKDF-SHA256)
- 密码轮换后自动更新（下次 unlock 静默刷新）
- 保存到密码管理器

### 销毁后行为

1. 容器进入 inert 模式 (`sleep infinity`)
2. 宿主机 host-watcher 检测到 marker → 自动执行 `destroy.sh --force --paranoid`
3. 清理: `docker compose down -v` 即可移除 inert 容器

### 安装 host-watcher

```bash
bash scripts/host-watcher.sh --install    # 安装 LaunchAgent
bash scripts/host-watcher.sh --uninstall  # 卸载
```

## Remote Desktop (KasmVNC)

桌面环境按需启动，不影响容器空闲时的资源占用。

### 启动/停止

```bash
docker exec -it dev-workspace desktop-start   # 启动，等待 KasmVNC 就绪
docker exec -it dev-workspace desktop-stop    # 停止所有桌面进程
```

启动后通过浏览器访问: `https://workspace.cicd.dpdns.org/desktop/`

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
