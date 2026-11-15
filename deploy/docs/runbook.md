# Operations Runbook

## First-Time Setup

```bash
install -m 600 .env.example .env  # then set CLASH_SUBSCRIPTION_URL and, better, PASSWORD
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

The login gate, the vault and the sync key share one password, so rotating it means changing the
vault passphrase and `PASSWORD` together. There is no `change-password` command (older versions
had one). Needs an explicit `PASSWORD` in `.env`: a generated one is kept in the container's
`/var/lib/sdw/master-password` and would still be the old one at the next start.

```bash
docker exec -it dev-workspace lock-vault
docker exec -it dev-workspace gocryptfs -passwd /vault/cipher   # asks for the old, then the new password
# set PASSWORD=<new password> in .env, then recreate so every service takes it:
docker compose up -d --force-recreate
docker exec -it dev-workspace destruct-passphrase                # the remote self-destruct passphrase changes with it
# Then: Sync Page → Clear key → Set passphrase (new password)
```

After these steps the vault opens with the new password and keeps its data; the login page accepts
the new password and rejects the old one (the gate takes a bcrypt hash of `PASSWORD` at every
start); `/run/sdw/sync-passphrase` is rewritten at the next unlock. Every browser session ends with
the recreate. The Sync Page step (browser side) is not exercised by the smoke suite.

### Raise the scrypt cost of an existing vault

A new vault wraps its master key with scrypt N=2^18; a vault made before that has gocryptfs's
default N=2^16. `gocryptfs.conf` is replicated with the ciphertext, so the cost is what stands
between a weak passphrase and an offline guess. The same command raises it, keeping the master key
and the data (enter the SAME passphrase for the old and the new one):

```bash
docker exec -it dev-workspace lock-vault
docker exec -it dev-workspace gocryptfs -passwd -scryptn 18 /vault/cipher
docker exec -it dev-workspace unlock-vault
```

The old `gocryptfs.conf` stays in the git history of the ciphertext remote (`VAULT_GIT_REPO`), so a
vault whose passphrase was weak should get a strong one (rotate it, above), not just a higher cost.

## Disaster Recovery (restore from the off-host backup)

If the host or the vault volumes are lost, the workspace is recoverable from the restic backup
(`RESTIC_REPOSITORY`) — provided `PASSWORD` was set (a generated one dies with its container). The
git ciphertext remote (`VAULT_GIT_REPO`) is a second, independent copy.

```bash
# 1. Recreate the stack on a fresh host, same .env (PASSWORD, RESTIC_*, ...).
docker compose up -d                              # a FRESH, empty vault is created and unlocked

# 2. Pick the snapshot to restore. Do NOT use `latest`: the fresh container's own backup watcher
#    has already written a new EMPTY snapshot to the same repo, so `latest` is that empty one.
docker exec -it dev-workspace restic snapshots            # choose the id from BEFORE the loss
docker exec -it dev-workspace restic ls <id> | grep <a-file-you-remember>   # confirm it

# (recommended) stop the watcher so it cannot add snapshots mid-restore:
docker exec -it dev-workspace supervisorctl stop backup-watcher

# 3. Restore that snapshot over /workspace (the unlocked vault), BY ID:
docker exec -it dev-workspace restic restore <id> --target /
docker exec -it dev-workspace supervisorctl start backup-watcher
```

Verified by `src/tests/smoke/dr-drill.sh`: it writes a canary, backs it up off-host, destroys the
container AND every volume, recreates a fresh stack, restores the pre-loss snapshot by id, and
checks the canary returns byte-for-byte. Run it against any image:
`SMOKE_IMAGE=dev-workspace:<img> SMOKE_PASSWORD=<pw> bash src/tests/smoke/dr-drill.sh`.

## Who did what (audit trail)

Logins, logouts, Cloudflare Access sign-ins, WebSocket handshakes refused for their `Origin`
(`gate.origin`), every call to the self-destruct endpoint and every
change of the egress status are written, one JSON object per line, to `/var/log/sdw/audit.jsonl`
(root only; rotated at 5 MiB, three files; on the `audit-data` volume, so it survives a container
re-creation but not `docker compose down -v`) and to the container's stdout.

```bash
docker exec dev-workspace audit-log            # the last 20 events, one line each
docker exec dev-workspace audit-log 200        # more
docker exec dev-workspace audit-log --json 50  # the raw lines, for jq
docker exec dev-workspace doctor               # the "Audit trail" row summarises the last 24 hours
docker logs dev-workspace | grep '"event":"auth.login"'   # the stdout copy
```

Reading it: the address is what the proxies in front *claim* (`CF-Connecting-IP`, else the first
`X-Forwarded-For` hop), so it is evidence, not proof. There is one master password, hence no user
name; behind Cloudflare Access the `who` field of `auth.access` is the Access e-mail or subject.
A `throttled` login means more than 10 attempts a minute from one address (or 60 in all): the
right password is refused too until the minute is over. `audit.suppressed` lines stand for events a
flood of wrong passwords or refused destruct calls would otherwise have written (at most 30 a
minute are kept). Optional Prometheus counters: `METRICS_ENABLED=1`, then `GET /sync/api/metrics`
(behind the login gate).

## Egress firewall

Everything the workspace sends out goes through Clash, and an nftables filter drops the rest. It is
on by default, loaded at every start before Clash comes up, and `doctor` has a row for it
(`Egress firewall`: failed if it is wanted and missing, a warning if you turned it off).

What it changes for the people using the workspace:

* A tool that ignores `http_proxy` / `all_proxy` (a raw socket, UDP, QUIC, a program that does its
  own DNS) cannot connect: it hangs until its own timeout. Most tools honour the proxy; `ssh` does
  not, so the start-up writes `/etc/ssh/ssh_config.d/10-sdw-egress.conf`, which sends it through
  Clash (loopback hosts excepted). `noproxy <cmd>` is dropped as well.
* A host on your LAN or in another Docker network is unreachable, unless you list it:
  `EGRESS_ALLOW_CIDRS=192.168.1.0/24,10.0.0.5` in `.env` (addresses or CIDR blocks; names are not
  accepted, use `extra_hosts:` in the compose file for a name). Traffic to those networks leaves
  with this machine's real address and bypasses Clash; tools are told through `no_proxy`, and for
  `ssh` you put `ProxyCommand none` for that host in `~/.ssh/config`. A block wider than /8 (IPv6:
  /16), a name, or a typo stops the boot with the entry named, rather than being skipped.
* Names inside a compose network (`db`, a sibling container) do not resolve: the resolver is Clash.
* `EGRESS_ENFORCE=0` turns it off. The banner, the boot log and `doctor` then say so on every
  start. Docker without `NET_ADMIN`, or a host without nftables, makes the boot stop with
  `nft load failed` and this variable named; that is deliberate (it will not boot believing its
  egress is closed).
* `pentest-bootstrap` and `pentest` need `SYS_CHROOT` and `MKNOD`, which the default container does
  not have: add `docker-compose.pentest.yml` to `COMPOSE_FILE` (they refuse to start, and say so,
  otherwise). Tor is set up to reach the network through Clash, so it should work behind the
  firewall (that combination has not been run); a raw `nmap -sS` or `ping` does not, so put the
  lab network in `EGRESS_ALLOW_CIDRS`.

## Troubleshooting

| Issue | Fix |
|-------|-----|
| Boot stops with `EGRESS_ALLOW_CIDRS: '…': …` | The value has an entry that is not an IPv4/IPv6 address or CIDR block, or is wider than /8 (IPv6: /16). Fix it in `.env` and `docker compose up -d` |
| Boot stops with `nft load failed` | The container has no `NET_ADMIN`, or nftables does not work on this host (gVisor, some rootless setups). Add the capability, or set `EGRESS_ENFORCE=0` and accept the consequences above |
| A command hangs when it connects to an address in the LAN | The egress firewall dropped it. List the network in `EGRESS_ALLOW_CIDRS`. A command that ignores the proxy variables for an Internet address is dropped too, by design |
| Start-up warns `fs.inotify.max_user_watches is …` | The host's limit is low and a container cannot raise it. On the Docker host (inside the VM on Docker Desktop / Colima): `sudo sysctl -w fs.inotify.max_user_watches=524288`, and put it in `/etc/sysctl.d/` to keep it |
| `pentest-bootstrap` says the pentest plugin needs capabilities | `COMPOSE_FILE=docker-compose.yml:docker-compose.pentest.yml` in `.env` (add `:docker-compose.apparmor.yml` on a Linux or Colima host), then `docker compose up -d` |
| A desktop does not start or its screen stays black | The desktop programs write to `/var/log/desktop/<program>.log` (2 MB, one backup): `docker exec dev-workspace tail -n 50 /var/log/desktop/*.log` |
| The container keeps restarting right after start | `docker logs dev-workspace` ends with the step that stopped the boot (`boot aborted at "…"`). A secret given as `<NAME>_FILE` that cannot be read, a bad `EGRESS_ALLOW_CIDRS` and a missing capability all stop it there |
| The workspace seems hung (`docker ps` says unhealthy) | Docker reports it but does not restart on it. `docker restart dev-workspace`; the vault is locked on stop and unlocked again at start |
| Clash not connecting | Check the subscription URL. `docker exec dev-workspace doctor` shows how many nodes loaded. The controller (127.0.0.1:9090) wants the secret that is in Clash's rendered config, which only root can read: `docker exec dev-workspace sh -c 'curl -s -H "Authorization: Bearer $(sed -n "s/^secret: \"\(.*\)\"$/\1/p" /etc/clash/config.yaml)" http://127.0.0.1:9090/providers/proxies/subscription' \| jq '.proxies \| length'`. The config is rendered again at every start from the image's template, so edit the subscription in `.env`, not in the file; restart the container if needed |
| Start-up banner says `Egress: DEGRADED` or `DOWN` | The workspace started anyway. `doctor` shows the live state; `cat /var/run/egress-status` shows what the start-up probe saw; with the vault open, `/workspace/.notifications/egress-degraded.md` explains. DOWN: nothing listens on :7890 (check the Clash config and subscription URL). DEGRADED: proxy up but the probe failed, usually nodes still loading; retry `doctor` in a minute |
| Vault unlock fails, `Password incorrect` | Wrong passphrase. If `PASSWORD` is empty and the container was recreated, the start-up generated a new password and the vault still wants the old one: run `docker exec -it dev-workspace unlock-vault` with the old one, then put it in `PASSWORD` in `.env` (the start-up prints this hint) |
| Vault unlock fails, `fusermount3: mount failed: Permission denied` | The host's AppArmor profile forbids the FUSE mount (Ubuntu, Debian). Uncomment `COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml` in `.env` and `docker compose up -d`; see the AppArmor note in `deploy/docs/threat-model.md` before doing so |
| `lock-vault` asks `Force unmount?` | Some process still has the vault in use: files open (code-server once it has opened something there, a shell inside it) or a program listening on a unix socket inside it (`grep ' /workspace' /proc/net/unix`; `fuser` does not show those). `y` detaches the mount at once, but those processes keep what they already opened until they exit; restart the container to be sure. Answering `N` leaves the vault mounted and starts the desktops and the backup watcher again, which the attempt had stopped |
| Vault unlock fails, other | Check `gocryptfs.conf` exists in the cipher volume; the start-up prints what gocryptfs said |
| Sync Page idle | Modify a file in the synced folder; check browser console |
| code-server 502 | Wait 10s after container start; check `docker logs` |

## Destroy

```bash
./destroy.sh --force              # Standard: this project's containers, volumes and images, the tunnel
./destroy.sh --force --paranoid   # Deep clean (history, DNS, creds)
./destroy.sh --force --delete-project   # ... and this checkout itself (nothing deletes it unless you say so)
```

These compatibility commands execute the typed Rust `sdw-host destroy` implementation. It locks to
`SDW_DOCKER_CONTEXT` when configured, verifies actual Docker leftovers before deleting the checkout,
and keeps the project/state when cleanup is incomplete.

The volumes removed are the ones of THIS compose project (matched by label, not by name), so a
second workspace on the same machine keeps its vault.

## Move to a new machine

```bash
./bootstrap.sh --vault-repo <VAULT_GIT_REPO url>   # clones the ciphertext, builds, restores
```

The clone is copied into the vault volume BEFORE the container first starts (a start with an empty
volume would create a NEW vault, not open the old one); the script refuses to do it when a vault
already exists there. Set the same `PASSWORD` in `.env`: it is the vault passphrase. A fresh
workspace instead: `./bootstrap.sh --init`.

## Keep the pinned tools current

Every program the image downloads (Caddy, restic, croc, Clash/mihomo, code-server, KasmVNC, mise,
IDEA, Burp Suite) is pinned to a version and a checksum in `src/image/tools/<name>.env`, and the
build fails if a download does not match. `make check-tools` lists the ones behind their project's
latest release (CI runs it weekly, `.github/workflows/tools.yml`); `make update-tools` writes the
new pins, and the diff is reviewed like any dependency bump. `make audit` checks the Cargo
workspace against the Rust advisory database.

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
# 202 = 已启动销毁任务; 404 = 密码错误; 429 = 速率限制
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
2. 宿主 `sdw-host watch` 检测到 marker 后，在监视服务之外启动 `sdw-host destroy --force --paranoid --delete-project`。macOS 使用新 session，Linux 使用 transient systemd unit，所以卸载 watcher 不会截断清理。它只读取配置容器的 destruct-state 卷，并受 `SDW_DOCKER_CONTEXT` 约束。
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
