# TODO

## 验证状态

重构（阶段 0–6）之后，在真实容器里做过一轮完整验证。环境：Colima + QEMU 软件模拟（宿主是没有嵌套虚拟化的 macOS 虚拟机，所以很慢），里面是真的 Ubuntu 24.04 内核（6.8、AppArmor 开启、arm64）、真的 FUSE、真的 systemd；上游代理用本地假服务。跑起来才暴露的问题已修掉，见下面「验证中发现并修复的问题」。

### 已验证（真实容器）

- **构建**：默认（KasmVNC）镜像从仓库的 Dockerfile 构建成功；`-full`（Selkies）用仓库 Dockerfile 里的 Selkies 块叠在默认镜像上构建（层顺序与真实 `DESKTOP_STACK=both` 不同，内容相同）。
- **启动**：首次启动（初始化并解锁 vault）、`docker restart`、`docker rm` 后用同一批卷重建；`HEALTHCHECK` 变 healthy；横幅、`doctor` 各行。
- **出口**：confirmed、degraded（节点不通 / 订阅里没有节点）、down（Clash 配置损坏）；degraded 和 down 不阻止启动，横幅、`/var/run/egress-status`、vault 里的 `.notifications/egress-degraded.md`、`doctor` 都给出状态。
- **Vault**：加解锁、`lock-vault`（干净卸载 / 被占用时强制）、第二个 vault（pentest 插件）的 init / unlock / lock、`gocryptfs -passwd` 轮换口令。
- **服务**：code-server 登录页、Caddy 认证（无凭据 401，有凭据 200）、`/vnc/`、`/desktop/`（`-full`）、同步服务的完整协议流程（分块上传、finalize、带认证的列表、重放 / 伪造 / 过期 / 路径穿越都被拒）。
- **备份**：restic 初始化、监视器启动时的基线快照、空闲不产生快照（无风暴）、一串编辑只在静默 10 秒后产生一个快照、被排除的路径不触发、无事件时 5 分钟兜底快照、prune、口令错误和仓库不可达两种情形（不会 `restic init` 覆盖、连续失败 3 次后写通知并在 `doctor` 里报错）。
- **自毁**：`self-destruct --force`（从 vault 外和 vault 内的目录运行）、`POST /sync/api/destruct`（错口令 404，对口令 200）；两个 vault 的密钥和数据都被清空，重启后进入 inert 模式。
- **停机**：`docker stop` 约 4 秒、退出码 0，supervisord 按顺序停掉所有程序。
- **宿主机**：Linux 的两个 systemd 用户单元在真实 systemd 下安装、启动、被 `kill -9` 后自动重启，并且「容器退出 + 自毁标记」触发的 `destroy.sh --force --paranoid` 能跑到最后（用 stand-in 的 `docker` / `cloudflared`，在 VM 里）；macOS launchd 用缩小版作业验证了清理脚本被其监督进程卸载时的行为（没有运行真实的 `destroy.sh`，它会删凭据和 shell 历史）。
- **静态检查**：actionlint（原来的 `build-image.yml` 无效，新文件通过）、shellcheck。

### 验证中发现并修复的问题

- `bunx` 不存在：固定版本的 Bun 压缩包里只有 `bun`，SPA 构建会失败 → Dockerfile 里补符号链接。
- 镜像里没有 `fuser`：`unmountVault` 升级阶梯里的 `fuser -k` / `-km` 一直是空操作 → 装 `psmisc`。但装上之后它会**杀掉调用者自己**：在 vault 内的终端里运行 `self-destruct`，`fuser` 杀掉 shell 和调用者，密钥没来得及销毁 → 改为永不杀调用者及其祖先进程。
- 在 vault 内的目录运行 `self-destruct`：lazy 卸载之后 `getcwd` 失败，后续每一步（包括销毁密钥）都报 ENOENT → 卸载前先离开挂载点。**这个问题在原来的代码里就有**（没有 `fuser` 时同样发生）。
- 宿主机 `host-watcher` 触发的清理会被自己的服务管理器杀掉：`destroy.sh --paranoid` 卸载 / 停止监视器服务，launchd 和 systemd 随即杀掉监视器启动的全部进程，凭据、shell 历史、DNS、项目目录都没清到 → 清理改在监视器的服务之外运行（macOS 新会话，Linux 临时单元）。macOS 上用缩小版作业做了 A/B 对照。另外 `destroy.sh` 现在先 `cd` 到项目目录，从别处运行时 `docker compose down -v` 不再静默失败。
- Docker 默认的 AppArmor 配置禁止 FUSE 挂载（Ubuntu / Debian 宿主机）：vault 保持锁定，旧提示是「口令不匹配？」→ 解锁失败时给出真正的原因，并提供可选的 `docker-compose.apparmor.yml`（`COMPOSE_FILE`）。
- Clash 只在启动时拉一次订阅，竞态时 provider 为空要等一小时 → 无节点时主动让 Clash 再拉一次。
- `-full` 镜像里 Selkies 一启动就崩：`pixelflux` / `pcmflux` 没有固定版本，pip 取了 2.1.0，而固定的 Selkies 提交需要 1.x → 固定 `pixelflux==1.6.4`、`pcmflux==1.0.8`（x86_64 和 aarch64 的 cp312 wheel 都存在），并在构建期加 `import selkies.selkies` 冒烟检查，之前这个镜像能构建成功但桌面是坏的。
- `PASSWORD` 留空时生成的密码只存在于容器内，重建容器会生成新密码，已有 vault 打不开 → 失败提示、`.env.example`、README、runbook、threat-model 里写明，并提示用旧密码 `unlock-vault`。
- 启动横幅写死 18080，`TUNNEL_HOST_PORT` 改了也不变 → 读取该变量。
- `docker stop` 实际是 tini 把 SIGTERM 传给 Bun，Bun 直接退出，其余进程被内核 SIGKILL（不是原来写的「等超时」）→ 把 SIGTERM 转给 supervisord，优雅停机。
- 镜像构建默认用的阿里云 apt 源在境外只有约 14 KB/s → 新增 `APT_MIRROR` 构建参数，默认值不变。
- `lock-vault` 强制卸载后仍说「Plaintext data is no longer accessible」→ 强制时如实说明已打开文件的进程仍可访问。
- runbook 里的 `change-password` 命令不存在 → 改为验证过的 `gocryptfs -passwd` 流程。

### 仍未验证

- **Docker Desktop（macOS）、真实 Linux 主机、amd64**：只在 arm64 的 Colima/QEMU 上跑过。amd64 只确认了 PyPI 上有对应的 wheel，没有在 amd64 上构建过。
- **浏览器里的真实使用**：KasmVNC / Selkies 只验证了进程常驻、端口、路由和认证，没有连上去看画面（Selkies 在软件模拟下也无法测画质）；同步页 SPA 的界面没有在浏览器里跑过（服务端协议用脚本验证了）。
- **真实外部服务**：Cloudflare Tunnel / Access、真实 S3 / R2、真实 Clash 订阅和节点（用的是本地假上游和本地 restic 仓库）。
- **GitHub 上的 CI**：workflow 通过了 actionlint，但没有真正跑过。
- **macOS 上真实的 `host-watcher.sh --install` / `destroy.sh --paranoid`**：只验证了行为（见上），没有跑真实脚本。
- **在已装好的旧 LaunchAgent / systemd 单元上**：修复在脚本里，需要重启监视器（见 `deploy/linux-systemd/README.md`）。

**建议**：把这次用的夹具（假上游、compose 覆盖、检查脚本）收进仓库，在 CI 里加一个 smoke job：构建默认镜像，起来，检查 `doctor`、横幅、认证、备份、自毁。这次发现的问题大部分只有这样才抓得到。

## 已知缺口

### 安全与可靠性

- **出口是尽力而为，没有强制**（`docs/threat-model.md`）。要强制需要独立的防火墙层；容器内进程都是 root，按 uid 的规则无效，需要基于 mark 的方案。
- **主密码会进容器日志**（启动横幅）和 code-server 的 `config.yaml`，且 vault 口令就是它，见 threat-model 的 Non-Guarantees。留空由容器生成的话，重建容器会丢，见上。
- **`docker-compose.apparmor.yml` 去掉容器最后一层 AppArmor 约束**。更好的做法是写一份只放行 FUSE 挂载的自定义 profile，目前没有。
- **restic 备份仓库不受自毁影响，而且里面是明文工作区（含 `.credentials/`）**，只靠 `RESTIC_PASSWORD` 保护。要么在自毁里加删除（需要仓库凭据），要么把 `.credentials/` 加进排除集。
- **`lock-vault` 在 code-server 等仍占用挂载时需要强制（lazy）卸载**，之后已打开文件的进程仍能访问。`lock-vault` 可能就跑在 code-server 的终端里，所以没有去停 code-server；彻底的办法是重启容器。
- **重放窗口不跨重启**：服务重启后，5 分钟容差内已见过的 nonce 可以被重放。
- **远程自毁路由没有可注入依赖**，单元测试覆盖不到成功路径（会杀 PID 1）。现在它在真实容器里端到端验证过，但把 `createDestroyDeps` 通过 `createApp` 注入仍然更好。

### 清理

- Selkies 静态客户端（约 6 MB）被复制进所有镜像，包括只含 KasmVNC 的默认镜像；可移到构建阶段按需复制。
- `-full` 镜像里 `desktop-audio` 在 `DESKTOP_AUDIO=0`（默认）时立刻退出 0，supervisord 把它记成 FATAL，启动日志里有几行 `exited ... not expected`，无害但吵。把它改成 `startsecs=0`、`autorestart=unexpected`、`exitcodes=0` 即可。
- `destroy.sh` 里 `docker volume rm vault-data cache-data clash-config destruct-state` 用的是不带项目前缀的名字，实际什么也删不到；真正删卷的是前面的 `docker compose down -v`。
- `host-watcher.sh` 的 `docker events | while read` 在清理触发后会留下一个孤儿 `docker events` 进程，直到下一个事件时因 SIGPIPE 退出。
- `image/Dockerfile.kasmvnc-test`、`docker-compose.kasmvnc-test.yml`、`image/scripts/start-kasmvnc-test.sh` 是 KasmVNC 帧缓冲问题的调试遗留。
- `parallels/` 的 VM 方案没有随重构更新。
- `rust/sync-service` 只是 PoC：要么毕业成为真正的守护进程，要么移除。它没有实现 `SYNC_ENABLED` 开关。

## 多平台部署

当前部署在家用 Mac Mini，后续考虑扩展部署位置以提升安全性和隐私性。

### 候选方案

| 优先级 | 方案 | 特点 | 备注 |
|--------|------|------|------|
| 高 | Hetzner VPS (德国/芬兰) | GDPR 保护、KVM、便宜 | 安全+隐私最大化 |
| 高 | Oracle Cloud Free Tier | ARM A1 4C/24G 永久免费 | 镜像已是 linux/arm64 |
| 中 | 家庭 NAS (群晖 x86/QNAP) | 数据物理可控、功耗低 | 需确认 Docker FUSE 权限 |
| 中 | 旧笔记本/台式机 (Ubuntu Server) | 零成本、放任意位置 | 配 UPS + WoL |
| 低 | 国内云 (阿里云/腾讯云) | 延迟低 | 实名制+合规风险，Clash 出站受限 |

### 宿主机硬性要求

- [ ] KVM 虚拟化或物理机（`/dev/fuse` 支持）
- [ ] Docker + `SYS_ADMIN` capability
- [ ] Ubuntu / Debian 等带 AppArmor 的发行版：启用 `docker-compose.apparmor.yml`（否则 vault 无法挂载）
- [ ] ≥ 8GB RAM
- [ ] 能运行 cloudflared
- [ ] 出站 443 可达（Clash + Cloudflare Tunnel）

### 待办事项

- [x] Dockerfile 支持 multi-arch（CI 构建 `linux/amd64` + `linux/arm64`）
- [x] 宿主机脚本适配 Linux（`deploy/linux-systemd/`，已在 Ubuntu 24.04 VM 的真实 systemd 下验证，未在真机上验证）
- [ ] 测试 Oracle Cloud ARM 实例的 FUSE 兼容性
- [ ] 评估 Hetzner CAX 系列（ARM）vs CPX 系列（x86）性价比
- [ ] 考虑多实例同步策略（多台机器共用一个 vault repo）
