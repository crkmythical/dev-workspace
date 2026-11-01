# TODO

> **2026-10-30 更新**：运行时现在是全 Rust（`sdw` 多合一二进制 + `sync-service` 守护进程），镜像里不再有
> Bun。TS 的 `src/packages/{server,cli,infra}` 和 core 里被 Rust 重写的域逻辑已退役，只留浏览器 SPA
> （`src/packages/spa`）和它用的同步客户端（`src/packages/core/src/sync`）。详见 deploy/docs/rust-migration.md。
> 下面的「验证状态」是 cutover 之前的记录，smoke 数字（298）是当时的；Rust 迁移后的全量是 302/0。

## 验证状态

两轮验证，都在真实容器里。第一轮在重构（阶段 0–6）之后，用 Debian 基础镜像；第二轮把验证做成了仓库里的 smoke test（`src/tests/smoke/`，`make smoke`），在生产基础（Kali）镜像上跑，并补上了原来没法测的外部服务替身、真实浏览器、Cloudflare 隧道和几条真实的联网路径。跑起来才暴露的问题已修掉，见下面「验证中发现并修复的问题」。

环境：Colima + QEMU 软件模拟（宿主是没有嵌套虚拟化的 macOS 虚拟机，所以很慢），里面是真的 Ubuntu 24.04 内核（6.8、AppArmor 开启、arm64）、真的 FUSE、真的 systemd；浏览器是 Mac 上的 Chrome 155。上游代理用本地假服务（它把 CONNECT 转给真实互联网，所以容器真的能上网），另有真实的 Shadowsocks 节点。

### smoke test（`src/tests/smoke/`，细节见其中的 README）

最后一轮全量（`src/tests/smoke/run.sh --image <Kali 镜像> --services`）：**298 项通过、0 失败、0 跳过**，约 23 分钟；从零构建的 `-full` 镜像另跑了 boot / auth / 浏览器三项共 59 项，全部通过。每个检查在自己的 compose 项目、端口和网络里跑，不读 `.env`，所以可以放心在已有真实工作区的机器上跑。`--build` 先按生产基础构建；`.github/workflows/smoke.yml` 是对应的 CI 作业（手动触发，或改动了镜像 / compose / 测试本身的 PR；通过了 actionlint，**没有真正跑过**）。

### 已验证（真实容器）

- **构建**：默认（KasmVNC）镜像用仓库的 Dockerfile 在 Kali 基础上**从零一次性构建成功**（32 分钟）。官方重定向 `http.kali.org` 在这里选到过只有约 14 KB/s 的镜像，所以新增了 `KALI_MIRROR` 构建参数（用的是 `mirrors.tuna.tsinghua.edu.cn`）。`-full`（`DESKTOP_STACK=both`，Selkies + KasmVNC）同样从零一次性构建成功（27 分钟，6.4 GB），boot / auth / 浏览器检查全过，包括 Selkies 的 H.264 桌面画面。
- **启动**：首次启动（初始化并解锁 vault）、`docker restart`、`docker rm` 后用同一批卷重建（vault 还在、同一个口令、不再重复打印自毁口令）；`HEALTHCHECK` 变 healthy；横幅、`doctor` 各行；缺必填变量时拒绝启动。
- **出口**：confirmed、degraded（节点不通 / 订阅里没有节点）、down（Clash 配置损坏）；degraded 和 down 不阻止启动，横幅、`/var/run/egress-status`、vault 里的 `.notifications/egress-degraded.md`、`doctor` 都给出状态。**真实的 Clash 协议**：订阅里列一个 Shadowsocks 节点，Clash 加载、url-test 选中它、请求真的经它上网（节点自己记录了目标），启动探测 confirmed。
- **Vault**：磁盘上没有明文（内容和文件名）；`lock-vault`：被占用时拒绝并恢复被它停掉的桌面和备份监视器、干净锁定、强制锁定（如实说明已打开文件的进程仍可访问）、错口令、重建容器后数据还在；第二个 vault（pentest 插件）的 init / unlock / lock；`gocryptfs -passwd` 轮换口令。
- **服务**：code-server 登录、Caddy 认证、`/vnc/` 以及 KasmVNC 的 WebSocket 升级（无凭据 401，有凭据 101）、`/desktop/`（`-full`）、同步服务的完整协议流程（分块上传、finalize、带认证的列表、重放 / 伪造 / 过期 / 路径穿越都被拒）。
- **浏览器（真实 Chrome 155）**：同步页（WebCrypto 派生密钥、IndexedDB 保存、真实上传，在 vault 里逐字节一致、刷新后密钥恢复）、紧急页、`/vnc/`（KasmVNC）看到 XFCE 桌面画面并且点击能到达会话、`/desktop/`（Selkies，`-full` 镜像）看到 H.264 流出来的桌面。
- **Cloudflare 快速隧道**（trycloudflare.com，临时、不碰你自己的隧道配置）：经 Cloudflare 边缘的 https 访问，浏览器检查同样全部通过（含 WebSocket 与桌面画面）；curl 级别：`/` 302、`/vnc/` 无凭据 401 / 密码对 200、同步 API 401；自毁限流：同一来源 3 次后 429，**伪造 `X-Forwarded-For` 不增加额度，伪造 `CF-Connecting-IP` 被边缘直接 403**（直连源站时伪造该头只能换到全局 10 次/分钟的上限，80-destruct 里验证过）。
- **备份**：restic 初始化、监视器启动时的基线快照、一串写入只在静默 10 秒后产生一个快照、被排除的路径（`node_modules`、`.git`）不触发、恢复逐字节一致、`restic check`、prune、仓库消失后连续失败 3 次写通知并在 `doctor` 里报错、恢复后通知被清掉。**对 S3 端点**（`rclone serve s3`）：启动时建库、监视器写快照、恢复、口令错误和端点不可达都不会 `restic init`；IP 写法的端点不经 Clash（`no_proxy`），主机名写法的经 Clash。
- **vault 复制**：按 cron 的方式（无容器环境变量）跑 `vault-sync`——首次推送、无变化不提交、有新数据再提交、远端里只有加密的文件名且不含明文、远端不可用时连续失败并在第 3 次写通知、恢复后补推；**真实 sshd**（key 在 vault 里，首次信任并固定主机密钥，之后主机密钥变了就拒绝）和 **token 在 URL 里的 HTTP git**（像 GitHub 那样要 token）都通过；真实 GitHub 没有测。
- **自毁**：`self-destruct --force`（从 vault 内的目录运行）和 `POST /sync/api/destruct`（错口令 404、限流、对口令 200）；两个 vault 的密钥和数据（含隐藏条目）都被清空，之后进入 inert 模式，重启仍然 inert。
- **`desktop-install`**：用 stand-in 测参数、重复安装、失败路径；**真实**安装 Chromium（apt 经 Clash 联网），在远程桌面里打开，窗口出现在 `/vnc/` 的画面里。
- **pentest 插件**：stand-in 测 Kali 失败后回退 Debian、重跑、`pentest` 进入（PID 1、主机代理变量不泄漏、挂载不泄漏）、`lock-pentest`；**真实**的 `pentest-bootstrap`（35 分钟，这次 Kali 重定向没拿到 Release，回退到 Debian bookworm 成功，706 MB）和 `pentest`：nmap 能扫、tor 能起并通过 Clash 开始引导、apt 不再有 `_apt` 沙箱告警。
- **容器内命令**：`doctor`、`init-vault`（已初始化时）、`lock-vault` / `unlock-vault`、`init-pentest` / `unlock-pentest` / `lock-pentest` / `pentest-bootstrap` / `pentest`、`desktop-start` / `desktop-stop` / `desktop-install`、`backup-init`、`vault-sync`、`setup --auto`、`self-destruct`、`destruct-passphrase`。
- **停机**：`docker stop` 约 4 秒、退出码 0，supervisord 按顺序停掉所有程序。
- **宿主机**：Linux 的两个 systemd 用户单元在真实 systemd 下安装、启动、被 `kill -9` 后自动重启，并且「容器退出 + 自毁标记」触发的 `destroy.sh --force --paranoid` 能跑到最后（用 stand-in 的 `docker` / `cloudflared`，在 VM 里）；macOS launchd 用缩小版作业验证了清理脚本被其监督进程卸载时的行为（没有运行真实的 `destroy.sh`，它会删凭据和 shell 历史）。
- **amd64**：只做了经 qemu-user 能做的部分（`src/tests/smoke/manual/emulated.sh`，从真实的 Dockerfile 里原样取出下载那几段）：Bun x64 的 sha256 和 Caddy amd64 的 sha512 校验通过，bun / caddy / restic / clash 四个二进制的 ELF 机器类型都是 x86-64，`bun --version` 在 x86_64 下输出 1.4.3。Bun 在 qemu-user 下跑不了 JavaScript（`bun install` 在 libarchive 里报内存不足），Caddy（Go）也起不来（regexp 初始化 panic），所以没能在 amd64 上跑单元测试和服务；整个镜像没有在 amd64 上构建。
- **静态检查**：actionlint（`build-image.yml`、`ci.yml`、`smoke.yml`）、shellcheck（宿主机脚本和 smoke test 的全部脚本）、biome、`tsc`。

### 验证中发现并修复的问题

**第一轮（重构之后，Debian 基础镜像）**

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
- **vault 复制到 git 远端（`VAULT_GIT_REPO`）在默认流程下根本不工作**：启动流程自己创建 vault 时只做 `gocryptfs -init`，cipher 目录不是 git 仓库（只有交互式 `init-vault` 会 `git init`），cron 的 `vault-sync` 每 30 分钟抛一次异常；cron 又读不到容器环境变量，所以不能让它自己去读 `VAULT_GIT_REPO`。**原来的代码里就有这个问题。** → 启动新增 `prepare-replication` 步骤（有 `VAULT_GIT_REPO` 时把 cipher 目录变成以它为 origin 的仓库，幂等，不打印 URL）；`vault-sync` 只看有没有 origin，没有就静默退出；提交带备用身份，推送用显式的 `origin main`。
- **一次推送失败之后，那个提交再也不会被推送**：提交已经做了，下一轮看起来是「无变化」就跳过，既不重试也不累计失败次数，失败通知永远不会出现，最后的改动要等下一次编辑才会上去 → 计划里新增「有未推送的提交就只推送」。同时成功后会清掉旧的失败通知（备份监视器共用这个跟踪器），之前恢复后通知文件一直留着。
- **`setup --auto` 重启 Clash 后不退出**：Clash 子进程让 Bun 的事件循环一直活着（阶段 5 重构引入）→ 守护进程 `unref()`。
- **`backup-init` 对已存在的仓库报「初始化失败」**：脚本头写着重复运行安全，实际 `restic init` 对已有仓库退出 1 → 先探测（与启动共用同一个分类器和同一个适配器 `probeRepository`）：可读则报「已初始化」；口令错误或不可达则明确报错且不初始化。

**第二轮（smoke test）**

- **官方 Kali 重定向有时选到极慢的镜像**（约 14 KB/s，一个镜像索引下了十几分钟）→ 新增 `KALI_MIRROR` 构建参数（默认值不变）。
- **`desktop-stop` 之后桌面的辅助进程还在**：supervisord 的 stop 只给启动脚本发信号，XFCE 的 D-Bus 辅助进程（`at-spi-bus-launcher`、`xfconfd`、`xfsettingsd`、会话 dbus）留在后面继续占用 vault → 桌面程序加 `stopasgroup` / `killasgroup`。
- **桌面会话留下的 `ssh-agent` 让 `lock-vault` 永远不能干净卸载**：xfce4-session 启动的 `ssh-agent` 把自己放进新会话（组杀不到），套接字在会话的 HOME 里，也就是 vault 里；**监听中的 unix 套接字占着挂载却不是打开的文件，`fuser` 看不到**。表现为：没有任何进程打开任何文件，卸载却连续被拒 20 秒，每次启动桌面多一个残留的 agent。→ 镜像里把会话的 ssh-agent / gpg-agent 关掉（加进 xfce4-session 包自带的系统默认文件，保留它的 Failsafe 会话定义；一开始整个覆盖掉该文件是错的，会丢掉窗口管理器和面板的定义）。修复后卸载在 stop 命令返回后的第一次尝试就成功（量过 3 轮）。
- **`lock-vault` 被拒绝（回答 N）后，被它停掉的桌面和备份监视器不会回来**：保护悄悄丢了，没有任何提示 → 拒绝时按原样恢复，并说明（`Started again: ...`）；`HolderControl` 增加可选的 `restore`，用例 `cancelLock`。
- **自毁没删隐藏条目**：`rm -rf dir/*` 的通配符不匹配点文件，cipher 目录里的 `.git`（复制仓库）和 `.gitignore` 被留下（内容已被覆写成零，但目录结构和对象数还在）→ `find -mindepth 1 -delete`。
- **SSH 形式的 `VAULT_GIT_REPO` 在无人值守的 `vault-sync` 里必失败**：未知主机时 ssh 在没有终端的 cron 里直接拒绝（`Host key verification failed`），作业只说「push failed」→ 复制仓库设置 `core.sshCommand`：首次见到的主机密钥信任并固定（`accept-new`），之后变了就拒绝，`BatchMode` 防止卡住；已有的自定义命令不动。
- **pentest rootfs 里 tor 起不来**：vault 挂载没有 `allow_other`，FUSE 对 mount 者以外的 uid 一律 `Permission denied`，连可执行文件都读不了：`service tor start` 失败（`/var/lib/tor` 无法读取），apt 的 `_apt` 下载沙箱也失效。stand-in 的测试发现不了，是跑**真实**的 `pentest-bootstrap` 之后才暴露 → `VaultSpec.allowOther`，pentest vault 开启，工作区 vault 保持只有所有者可读。
- **`pentest` 的注释和横幅说「PID 隔离：内部进程从外部不可见」是反的**：PID 命名空间隐藏的是宿主的进程（从里面看不到），宿主仍然看得到里面的进程（量过）→ 改正注释和横幅。
- smoke test 自己的几处错误（定时、清理、断言）也在这个过程中修掉了；这些不是产品问题，不在此列。

### 仍未验证

- **Docker Desktop（macOS）和真实的 Linux 主机**：smoke test 就是为这个写的（`make smoke-build`，一条命令），但只在 arm64 的 Colima/QEMU 上跑过。
- **amd64**：见上，只验证了下载校验和二进制的架构，没有在 amd64 上构建或运行整个镜像。GitHub 的 amd64 runner 上跑 `smoke.yml` 会是第一次。
- **真实外部服务**：Cloudflare **Access 和命名隧道**（只测了快速隧道，它没有 Access 策略）；真实 S3 / R2（用的是 `rclone serve s3`，区域、签名、虚拟主机风格这些 R2 / AWS 的细节没测）；真实 GitHub（本地 sshd 和 git HTTP 替身）；真实机场订阅和节点（本地 Shadowsocks 节点，订阅里常见的 vmess / trojan / hysteria 等协议没测）。
- **GitHub 上的 CI**：三个 workflow 都通过了 actionlint，没有真正跑过（按你的决定，推送留到以后）。
- **Tor 的完整引导**：tor 能起，经 Clash 连上中继，引导到 30% 以上，没有等到 100%（这个网络下 Tor 能不能通未知）。
- **pentest 的 Kali 路径**：真实运行中 Kali 重定向这次取不到 Release，走的是 Debian bookworm 回退；第一次探测时 Kali 路径取到了 Release 并下载了索引，但我中断了，所以没有完整建成过一个 Kali rootfs。
- **macOS 上真实的 `host-watcher.sh --install` / `destroy.sh --paranoid`**：只验证了行为（见上），没有跑真实脚本。
- **在已装好的旧 LaunchAgent / systemd 单元上**：修复在脚本里，需要重启监视器（见 `deploy/linux-systemd/README.md`）。

## 已知缺口

### 安全与可靠性

- **出口是尽力而为，没有强制**（`deploy/docs/threat-model.md`）。要强制需要独立的防火墙层；容器内进程都是 root，按 uid 的规则无效，需要基于 mark 的方案。
- **主密码**：启动横幅不再打印它（2026-10-31）——只有在 PASSWORD 留空、容器自己生成口令时才打印（否则无从得知）；来自 `.env` 的口令永不进日志。它仍写进 code-server 的 `config.yaml`（code-server 要靠它认证，该文件在容器内、不随 `docker logs` 外泄）。vault 口令就是它，见 threat-model 的 Non-Guarantees。把它变成内存驻留 + 硬件信任根的方案见 `deploy/docs/design-memory-only-secrets.md`（设计，未本地 PoC——这台机器没有容器可见的 TPM2；验证脚本 `src/tests/smoke/manual/tpm-seal-verify.sh` 在真实 TPM / CI 上才跑）。
- **`docker-compose.apparmor.yml` 去掉容器最后一层 AppArmor 约束**。更好的做法是写一份只放行 FUSE 挂载的自定义 profile，目前没有。
- **restic 备份仓库不受自毁影响**，里面是明文工作区，只靠 `RESTIC_PASSWORD` 保护。`.credentials/`（ssh 密钥、同步口令）**已加入排除集**（2026-10-30，`BACKUP_EXCLUSION_SET`），不再进 restic 仓库；仍可从加密的 vault git 复制恢复（`VAULT_GITIGNORE` 不排除它）。自毁时连带删除 restic 仓库（需要仓库凭据）：**按决定不做**，把该仓库当作 vault 的一部分对待。
- **`lock-vault` 在 code-server 等仍占用挂载时需要强制（lazy）卸载**，之后已打开文件的进程仍能访问。空闲的 code-server 不占用（量过，干净卸载成功），打开过东西之后才占用。`lock-vault` 可能就跑在 code-server 的终端里，所以没有去停 code-server；彻底的办法是重启容器。
- **重放窗口不跨重启**（服务重启后 5 分钟容差内已见过的 nonce 可被重放一次）：**按决定接受**。被截获的请求本就在 Caddy 认证之后、E2E 加密之内，且只在重启那一刻的窗口里，重放一个上传无非重传同一文件。
- ~~远程自毁路由没有可注入依赖~~ **已解决**（Rust 迁移）：触发器是可注入的 `DestructTrigger`（`AppConfig.destruct`），`sync-service/tests/routes.rs` 用假触发器覆盖了成功路径，不再碰不可逆边界。
- ~~监听在 vault 里的 unix 套接字 `fuser` 看不到~~ **已解决**（2026-10-31）：卸载升级的 filesystem 阶段现在也读 `/proc/net/unix`，把挂载点下绑定套接字的属主（经 `/proc/<pid>/fd` 映射）并入 kill 集（纯解析 `socket_inodes_under_mount`，带单测）；调用者自身的进程谱系仍受保护，lazy 卸载仍是最后兜底。
- **`pentest-bootstrap` 的 Kali 镜像**：官方重定向会按位置挑镜像，挑到慢的或取不到 Release 的就回退到 Debian bookworm（已验证回退可用，得到的是 bookworm，不是 Kali；工具之后用 `install-kali-tools.sh` 补）。运行时现在可用 `KALI_MIRROR` 覆盖（2026-10-30，与镜像构建同名 build-arg 一致，默认 `http.kali.org`）。
- **debootstrap 每次启动都打印 `cannot create /pentest/rootfs/test-dev-null: Permission denied`**：vault 挂载是 `nodev`，debootstrap 的设备节点自检失败，但随后照常完成（真实运行验证过，rootfs 能用，`/dev` 由 `pentest` 另行挂载）。无害，吵。
- **`pentest` 里 `service tor stop` 报 `Is 32 not tor?`**：init 脚本核对 PID 时看到的是另一个命名空间的数字。会话结束时整个 PID 命名空间一起收掉，所以不留进程，但会话中途不能用它停 tor。

### 清理

已完成（2026-10-30，见 deploy/docs/rust-migration.md 与对应提交）：
- `desktop-audio` 在 `DESKTOP_AUDIO=0` 时不再被 supervisord 记成 FATAL（`autorestart=unexpected`、`exitcodes=0`、`startsecs=0`）。
- `destroy.sh` 改用带项目前缀的卷名清理（原来的不带前缀，什么也删不到）。
- `host-watcher.sh` 不再在清理后留孤儿 `docker events`（后台生产者写 FIFO，break 时按 PID kill）。
- 删除 KasmVNC 帧缓冲调试遗留（`image/Dockerfile.kasmvnc-test`、`docker-compose.kasmvnc-test.yml`、`image/scripts/start-kasmvnc-test.sh`）。
- `src/rust/sync-service` 早已不是 PoC：它是生产守护进程，`SYNC_ENABLED` 已实现并冒烟验证；现在只负责 `/sync/api/*`，静态 SPA 由 Caddy `file_server` 提供。
- `.env.example` 生成器已用 Rust 补回（`sdw env-example` / `make env-example`，带漂移测试）。
- 删除过时的 Parallels VM 代理变体（`parallels/`，2026-10-31）；维护中的独立代理是 `deploy/client/` 容器。
- Selkies web 客户端（约 6 MB）改为按 `DESKTOP_STACK` 条件复制（2026-10-31）：默认（KasmVNC）镜像不再携带它，`/usr/share/selkies/web` 为空。

仍待办：（暂无——此前的清理积压已清空。）

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
