# TODO

> **2026-11-15 维护面 Rust 化更新**：`sdw-host` 继续接管 `update-tools` 与 `.env` 历史清理；
> tag 发布矩阵增加 Linux/macOS × amd64/arm64 四个原生宿主包、独立/汇总 SHA-256 和
> checksum 验证安装器。工具更新先完成全部计划再事务写入；历史清理要求 git-filter-repo，
> bundle 先创建并验证，重写后再验证所有 refs。本轮完成真实只读上游 check；安装器、
> Selkies 依赖重锁和历史改写分别只用 fake release、fake `uv`/`unzip` 与临时 Git 仓库验证。
> 没有改写本仓库历史、执行真实 Selkies 重锁或实际发布 GitHub Release。

> **2026-11-15 宿主 Rust 化更新**：新增 `sdw-host`，接管 destroy、self-destruct watcher、
> 命名 tunnel、临时 Cloudflare Access 实例、systemd 单元、Docker context 安全锁和
> secrets-init。原 Shell 路径仅保留定位项目后 `exec` 的兼容入口；Cloudflare 状态改为
> 原子 0600 JSON，失败删除会保留状态。全生命周期 fake Cloudflare/Docker/PID 测试通过。
> 真实 Cloudflare 账户和真实 Linux systemd 尚未在迁移后重跑。

> **2026-11-14 Rust-first 更新**：浏览器同步页和协议客户端已从 TypeScript/WebCrypto 迁到
> `src/rust/web`（Rust/WASM），原生 smoke probe 位于 `src/rust/smoke-client`，共同复用
> `sdw-core::sync::client`。Bun、Vite、Biome、TypeScript 配置和全部维护中的 `.ts` 源码已删除。
> 同批修复：同步失败不再误记成功、轮询不重入、流式哈希、请求超时、服务端同 file-id
> 串行与块序校验；远程销毁改为 202 scheduled 且幂等，未确认密钥擦除时不写 terminal marker；
> root bridge 收窄为五个固定目标生命周期命令；Clash 和 HTTP gate 不再保留无关密钥环境。
> 当前 Kali/KasmVNC review 镜像已构建；完整 smoke 首轮 563/2（仅下载器输出文案断言过期），
> 修正后 `45-plugins` 76/0、WASM 浏览器 17/0 重跑通过，565 个检查均有通过记录。
> 历史记录中的 TS/Bun 描述保留为当时状态，不代表当前架构。

> **2026-11-04 更新**：2026-11-03 两轮 review 提出的问题，已经按**根因**逐项修了一遍。下面「第三轮」按根因列出
> 做了什么、怎么验证的；「还没做」只列仍然开放的东西，其中大部分是这台机器上做不了的（没有嵌套虚拟化、没有 VPS、
> 没有真实的 Cloudflare Access / GitHub CI）。更早的记录（运行时迁到 Rust、两轮 review 的原文、历史验证）保留在后面，
> 每一条都标了现状。smoke 的最新数字在「第三轮」末尾。

## 还没做的

### 这里做不了（需要硬件、账号，或者你的决定）

- **microVM / gVisor**。Colima 的 vz 虚拟机里没有嵌套虚拟化，起不了 Kata / Firecracker；gVisor 不支持 nftables（出口防火墙
  默认开启，`nft -f -` 会失败，按设计拒绝启动），其 FUSE 支持还是实验性的（gocryptfs 能否挂载、`/proc/net/unix` 的 inode 是否
  对得上 `lock-vault` 的占用者检测，都没测）。Kata 要 KVM，且要用 QEMU / Cloud Hypervisor 而不是 Firecracker（后者没有
  virtio-fs，命名卷用不了）。需要一台真正的 Linux 主机。**这是现在最大的剩余边界问题**：用户代码已经是没有 capability 的
  `dev`，但容器里的 root 仍是带 `CAP_SYS_ADMIN` 的容器 root。
- **共享中继（交互卡顿）**。现状：CF 隧道墙内→海外边缘绕行，交互桌面卡。目标：一台好线路 VPS 跑 `frps` 或自建
  `Headscale` / DERP，`provision` 时每实例自动注册（生成密钥 + 子域，和现在 CF 流程同构，仍可批量化）；桌面可配合切
  Selkies（WebRTC）。验收：异地交互延迟明显下降。**需要一台线路好的 VPS 才能验证。** review 时记下的设计要点：加一个并列的
  `provision-relay.sh ID PORT`，写出与现在同名的状态键（`FQDN`、`ORIGIN_PORT`、连接器 PID 或 sidecar 名）并带自己的回滚；
  `up.sh` 按 `RELAY_MODE` 分支，端口挑选和 `docker compose` 调用不变。relay 模式下 `ACCESS_AUD` / `ACCESS_TEAM_DOMAIN` 留空
  （cookie 网关成为唯一边界，必须设强 PASSWORD，机器客户端的认证要在 VPS 的代理上自己加）。自毁限速取客户端地址靠
  `CF-Connecting-IP` → `X-Forwarded-For` 首跳，frps 前必须由中继**覆盖** `X-Forwarded-For`。Selkies（WebRTC）还要中继转发 UDP。
- **Oracle Cloud ARM 的 FUSE 兼容性**（见下面「多平台部署」）：需要一个 Oracle 账号和实例。
- **真实的外部服务**：Cloudflare Access 和命名隧道（Access 放行只用合成密钥做过单元测试）；**GitHub 上的 CI**（所有
  workflow 都通过了 actionlint 和本地的等价命令，没有真正在 GitHub 上跑过，包括这次新增的 `audit`、`scripts`、`fuzz`、
  `tools` 作业）；真实 R2 / S3；真实的机场订阅（只有本地的 HTTP 节点和 Shadowsocks 节点）。
- **amd64**：整个镜像没有在 amd64 上构建或运行过；GitHub 的 amd64 runner 上第一次跑 `smoke.yml` 才算数。
- **清理历史并轮换密钥（你的操作）**：`.env` 已经不再被 git 跟踪，但它曾被提交过 5 次，真实的 PASSWORD 和订阅 URL 在历史里
  （也在 `~/Downloads/dw.bundle`）。`make purge-env-history`（先演习，`YES=1` 才真做，会先备份）会改写历史，**我没有运行**，
  等你同意；PASSWORD（同时是 vault 口令，轮换步骤见 runbook）和订阅 URL 的轮换只能你做，且在清理历史之前就该做。
- **镜像签名（cosign）**：发布流水线现在有 smoke 门控、provenance 和 SBOM，没有签名——无密钥签名要先决定用哪个 OIDC
  身份、验证方的策略怎么写。
- **TPM / KMS**：vault 密钥仍然等于 `PASSWORD`。设计在 `design-memory-only-secrets.md`；这台机器没有容器可见的 TPM2。

### 做了，但没有验证到底

- **桌面里的 GUI 应用在出口防火墙后**：只验证了镜像自带的 Firefox ESR——无头模式，以工作区用户、带工作区的环境（代理变量），防火墙默认开着：它读代理环境变量，经 Clash 打开了 `https://example.com`（假节点的日志里有 `CONNECT example.com:443`，截图生成）。`desktop-install` 装的 Chromium、IDEA、Burp Suite 等没测；Chromium 在 Linux 上会读环境里的代理，Java 程序（IDEA、Burp）是否遵守要各自确认，不要假设。一个不遵守的应用在防火墙后表现为连不上，而不是泄漏。
- **规则模式 + 以 REJECT 打头的节点组**，对真实的订阅：只用本地假节点和 Shadowsocks 节点验证过（空订阅会失败而不是退回直连，
  有节点时被选中）。机场订阅里常见的其它协议（vmess、trojan、hysteria 等）没测。
- **Tor 引导到 100%**：能起、经 Clash 连上中继、引导到 30% 以上，没等到 100%；没完整建成过 Kali rootfs（Kali 重定向取不到
  Release 时按设计回退到 Debian bookworm）。
- **宿主上真实的 `host-watcher.sh --install` / `destroy.sh --paranoid`**：只验证了行为（用替身和缩小版作业），没有跑真实脚本。

### 已知的小缺口

- `destroy.sh --paranoid` 仍会删掉整个 `~/.cloudflared/`（所有隧道的凭据）和 Docker 构建缓存：它是显式的「深度清理」，不是默认。
- 出口防火墙开着时，容器里的域名解析走 Clash 的 DNS：同一个 compose 网络里的兄弟容器名字解析不到（用 IP + `EGRESS_ALLOW_CIDRS`，
  或 `extra_hosts`）。
- 多个写入方共用一个 vault 仓库的冲突处理没做（见下面「多平台部署」）。
- 桥接给 root 的命令名单里任何一个命令的漏洞都是提权的路（名单是生成的、很短，但它是边界的一部分）。

## 第三轮：按根因修复（2026-11-03 → 11-04）

用户的要求是「不要缝缝补补，要全面系统化彻底根治」。做法是先找每个发现的**根因**，再决定在哪一层修，然后让测试守住它。
下面每一项：发现了什么 → 根因 → 怎么修的 → 谁来证明它（测试都在仓库里）。

### 1. 密钥不该进 git；进了，也该有人拦

- **发现**：`.env` 被 git 跟踪，里面是真实的 PASSWORD（同时是 vault 口令）和订阅 URL。
- **根因**：没有任何一层阻止密钥文件被提交：`.gitignore` 里它被注释掉了，提交时不检查，CI 也不检查。
- **修**：`.env` 取消跟踪并写回 `.gitignore`。`deploy/scripts/check-tracked-secrets.sh` 拒绝被跟踪的密钥文件（`.env`、编排的
  `secrets.env`、`secrets/`、`instances/`）；pre-commit 钩子再加一层：暂存的改动里出现 `.env` 中任一密钥的**值**也拒绝（只报变量名，
  不打印值）；`make install-hooks` 在每个 clone 里启用；CI 的 `scripts` 作业跑同一个检查。
- **证明**：`src/tests/scripts/secrets-guard.test.sh`，在临时仓库里跑，不碰这个仓库。
- **没做**：清理历史（`make purge-env-history`，需要你同意）和轮换密钥（只有你能做），见「还没做」。

### 2. 供应链：下载没有校验，版本在浮动

- **发现**：KasmVNC `.deb`、mise 与 code-server 的安装脚本、IDEA 和 Burp 的下载没有校验和；`pnpm@latest`、`node@20` 浮动；
  `python-xlib` 取自 `master.zip`；Dockerfile 里的 `ARG` 版本靠人记得更新，固定的工具已经落后（mihomo、restic、KasmVNC、croc）。
- **根因**：每个下载点各写各的 `curl`，校验是可选的，版本散落在 Dockerfile 里。
- **修**：版本、URL 和校验和只写在一处：`src/image/tools/<名>.env`，由 `deploy/scripts/update-tools.sh` 生成；一个共用的下载器
  （镜像里的 `fetch-tool`）下载并校验，不符就失败。覆盖 Caddy、restic、croc、mihomo、code-server、KasmVNC、mise、IDEA、Burp Suite
  （Burp 的校验和是对 684 MB 的下载算的）。mise 的运行时固定到精确版本，Selkies 的 `python-xlib==0.33`，基础镜像按 digest 固定。
  Dependabot 管 Actions、crate、Bun 包和基础镜像；`tools.yml` 每周报告落后的固定工具；`cargo audit`（`audit.toml` 里每条被忽略的
  公告都写了理由）和 `bun audit` 进了 CI（vite 升到 6.4.5 后干净）；发布流水线先过 smoke 才推送，并带 provenance 和 SBOM。
- **没选**：用 Renovate 的正则管 `ARG`（它算不出校验和）。**没做**：镜像签名（cosign），见「还没做」。
- **证明**：构建本身（校验和不符构建失败）；`45-plugins` 用假的固定文件测 `desktop-install` 的校验（不符被拒绝，且不留下文件）。

### 3. Clash：配置会漂移，控制器没有 secret，空订阅会静默直连

- **发现**：独立的 `deploy/client` 的 Clash 根本不能用（配置里的 `${SUBURL}` 没有展开）；控制器 `127.0.0.1:9090` 没有 secret，
  容器里任何进程都能把 Clash 切到 DIRECT；Clash 模板放在 `/etc/clash` 卷里，而卷只在创建时从镜像填充，每个安装都冻结在第一次的模板上。
- **根因**：两份手写配置，渲染靠 `sed`，模板放在镜像更新不到的地方，节点选择靠运行时调 API 而不是声明。
- **修**：一个渲染器（Rust，单遍，缺任何占位符或读回不出 secret 就拒绝）、一个属于镜像的模板（`/usr/share/sdw/clash/config.yaml.template`）；
  工作区和独立客户端（`sdw proxy-only`，`clash-client` 构建阶段）用同一份。规则模式 + 一条 `MATCH,auto-select`；控制器要 secret
  （0600，`clash` 属主，每次开机轮换，`set-clash-dns` 热加载时保持）。**实测发现**：订阅里没有节点时节点组为空，mihomo 会补一个隐含的
  DIRECT 成员——请求全部直连而探测还通过；所以组里的第一个成员是 `REJECT`。没有渲染好的配置就不启动 Clash（否则它写一份默认配置，
  作为开放的 DIRECT 代理运行）。`clash-watcher` 用 Rust 重写，只从渲染器写的配置重启，僵尸进程不算在跑。
- **证明**：`15-clash`（配置的属主和权限、控制器要 secret、热加载不轮换、watcher 重启）、`70-egress-variants`（空订阅 → 失败而不是直连；
  模板被破坏 → 起不来）、`30-shadowsocks`（真实协议的节点被选中）、`services/50-clash-client`（独立客户端：订阅被用、流量过节点、
  控制器要 secret、Clash 以 `clash` 用户运行）。

### 4. 入口：没有安全头，cookie 与 WebSocket 的细节

- **发现**：Caddy 对所有页面一个安全响应头都不发；会话 cookie 在 HTTPS 下没有 `Secure`；`AUTH_SESSION_PERSIST` 的密钥是
  `HMAC(密码, 固定标签)`，拿到一个 cookie 的人可以离线猜密码；Access 的 JWKS 遇到未知 `kid` 不重取；WebSocket 没有 Origin 校验；
  上传的提交会跟随 vault 里的符号链接；restic 探测对不可达的 S3 最长会重试 15 分钟（实测 restic 0.19），把启动卡住。
- **根因**：这些都是入口各处的「默认值」，没有一处统一声明、也没有测试守着。
- **修**：基线安全头在 Caddyfile 里**每个头一个块**声明（实测：一个块里只要有一个头被预设为 `?`，整个块失效）；`/sync/` 有严格 CSP 和
  `X-Frame-Options: DENY`；HSTS 与 `Secure` 只在客户端是 HTTPS 时给（看 `CF-Visitor`、`X-Forwarded-Proto`；**不**用
  `trusted_proxies private_ranges`，那会让 XFF 可伪造、削弱登录限速）；跨站 WebSocket 握手回 403（审计事件 `gate.origin`）；
  `AUTH_SESSION_PERSIST` 用随机密钥，存在 `/var/lib/sdw/session.key`；JWKS 遇到未知 `kid` 最多每 60 秒重取一次；上传提交用
  `openat` / `renameat` + `O_NOFOLLOW`；restic 探测限时 45 秒。
- **证明**：`20-auth`（响应头、`Secure`、Origin、无第二个密码）、`access_rotation` 集成测试、`store` 的符号链接测试、`50-backup` 和 S3 检查。

### 5. 权限分离：用户代码是 root，拿得到全部密钥

- **发现**：code-server 终端和桌面（用户代码真正运行的地方）是 root，继承了 PASSWORD、RESTIC_PASSWORD 等全部密钥；主密码写在
  code-server 的 `config.yaml` 里；KasmVNC 有 `.kasmpasswd`；同步密钥的来源放在 vault 里。出口防火墙按 uid 做规则，在一切都是 root
  时没有意义。
- **根因**：从来没有「工作区用户」这个身份：密钥和用户代码在同一个 uid、同一个环境、同一个文件系统里。
- **修**：用户 `dev`（uid 1000，家目录 `/workspace/home` 在 vault 里）；`sdw workload` 是唯一的降权入口，环境变量是**白名单**
  （密钥即使在 `WORKLOAD_ENV` 里点名也拒绝）；vault 以 `-allow_other -force_owner 1000:1000` 挂载，挂载点 0700；密文目录每次启动设为
  0700（pentest 的是 0711，见下）；内层认证去掉（code-server `--auth none`，KasmVNC `-DisableBasicAuth`），生成的密码存 `/var/lib/sdw`
  （root）；同步密钥的来源搬到 `/run/sdw`；`opendoas` 桥接一份**生成的**短名单，其余 setuid / setgid 全部去掉；运营者的命令
  （`self-destruct`、`destruct-passphrase`、`backup-init`）对 `dev` 拒绝并提示 `docker exec`；mise 预装的运行时放在只读的 `/opt/mise`，
  `dev` 装自己的；远程自毁的密钥哈希原来是 0644（是 vault 口令的离线猜测依据），现在 0600、目录 0700。
- **证明**：`75-privilege`：谁以谁运行、`dev` 够不到的东西、工作区进程的环境里没有密钥、没有 capability、桥接只放行名单里的命令；
  `25-audit`（Caddy 和 Clash 读不到 vault）。
- **没做**：容器里的 root 仍带 `CAP_SYS_ADMIN`（microVM，见「还没做」）。
- **跑起来才发现的**：密文目录设成 0700 之后，pentest 的 rootfs 对 `debian-tor` 和 `_apt` 就不可访问了（挂载的根目录取自密文目录的属性）；
  pentest 的密文目录因此是 0711（别的用户只能穿过、不能列出，密钥文件 gocryptfs 本来就是 0400）。`45-plugins` 守着这一点。

### 6. 出口：从「尽力而为」改为「默认强制」

- **发现**：默认模式下忽略代理变量的进程直连外网（实测 `nc 1.1.1.1 443`、`curl --noproxy` 都成功）。对一个以隐私为卖点的产品，真实 IP
  会被任何不读代理变量的工具暴露；而 `EGRESS_ENFORCE` 以前没有任何 smoke 覆盖。
- **根因**：把「代理变量」当成了边界，而它只是工具自愿遵守的约定。
- **修**：nftables 默认丢弃的 OUTPUT 过滤器**默认开启**（`EGRESS_ENFORCE=0` 才关）：只放行回环、已建立连接的应答、`clash` 用户和
  `EGRESS_ALLOW_CIDRS` 里的网络（地址或 CIDR；比 /8（IPv6 /16）宽的、域名、写错的都让启动停下）；在 Clash 之前装载，装载不了就停止启动
  （不会在以为出口已封死的情况下启动）；`ssh` 不读代理变量，启动时写 `ssh_config.d` 让它走 Clash 的 SOCKS5（回环除外）；工作区进程的
  `no_proxy` 加上白名单网络；`doctor` 有「Egress firewall」一行，启动横幅有 Firewall 一行，关着时明说；pentest 的三个 capability
  （`SYS_CHROOT`、`MKNOD`、`NET_RAW`）搬出默认 compose，由 `docker-compose.pentest.yml` 补上，没有它时 `pentest-bootstrap` / `pentest`
  明说缺什么然后退出；`set-clash-dns` 的探测以 `clash` 用户运行（否则会被我们自己的防火墙挡掉，把「国外 DNS 可达」误判成不可达）。
- **证明**：`72-egress-enforce`：默认（该变量留空）就是开，root 和 `dev` 都不能直连（用网关口的 refused / timed out 区分，不依赖外网），
  Clash、白名单、经 Clash 的 ssh、由 Clash 解析的名字照常工作，`EGRESS_ENFORCE=0` 真的关掉并明说，过宽的白名单让启动停下，默认 compose 不带
  pentest 的 capability；`services/20-git-remotes` 在防火墙后再跑一遍：真实 sshd 和 token-in-URL 的 HTTP git 照常推送，假节点的日志里能看到
  两条 CONNECT。**手工验证**（没有放进套件，要联网、慢）：root 的 `apt-get update`（22 MB）和 `apt-get download` 在防火墙后经 Clash 正常
  （假节点日志里是 `CONNECT mirrors.aliyun.com:80`）；Firefox ESR 无头模式经 Clash 打开页面（见「做了，但没有验证到底」）。
- **代价**（README / runbook 里都写了）：局域网里的主机、同一个 compose 网络里的兄弟容器名、裸的 `nmap -sS` / `ping`，在防火墙后默认连不上。

### 7. 宿主脚本：密钥上命令行，清理范围过大，迁移其实没迁移

- **发现**：编排脚本把 Cloudflare token 放在 curl 的 argv、tunnel token 放在 cloudflared 的 argv，`set -a` 把 `secrets.env` 导出给所有子进程；
  `destroy.sh` 的卷匹配会碰到别的项目、默认删除整个 checkout、末尾无条件打印「removed」；`host-watcher.sh` 的 plist 没有 `PATH`，还会盯
  任何项目的 destruct-state 卷；`bootstrap.sh` 的迁移把密文克隆到一个没人用的宿主目录，然后新建了一个空 vault。
- **修**：`deploy/orchestration/lib.sh`（`secrets.env` 逐行读取而不 source；token 经 0600 的 curl 配置文件和 `TUNNEL_TOKEN` 环境变量，退出时清理）；
  `up.sh` 在 compose 失败时拆掉边缘；`destroy.sh` 按 compose 项目标签清理卷，删除 checkout 要显式 `--delete-project`，摘要问 Docker 而不是假设
  （还有东西没删掉就报 `!`、退出码非零、不删项目目录）；`host-watcher.sh` 只盯**它自己那个容器**的卷，`PATH` / `DOCKER_CONTEXT` 写进 plist；
  `bootstrap.sh` 迁移时先把克隆来的密文放进 vault 卷再第一次启动（已有 vault 时拒绝）。
- **证明**：`orchestration-lib.test.sh`（`$(…)`、反引号不会被执行，token 不在子进程环境里）、`destroy.test.sh`（用假 docker：只删本项目的卷、
  不删 checkout、摘要为真）；其余是 shellcheck。**`bootstrap.sh` 的迁移路径和 `host-watcher.sh` 没有自动化测试**，也没有在真机上完整跑过。

### 8. 运行时加固

- 新 vault 的 scrypt 成本 N=2^18（gocryptfs 默认 2^16）：`gocryptfs.conf` 随密文复制到 git 远端，它是弱口令和离线猜测之间唯一的东西。
  `init-vault`、`setup`、启动都走同一个 `init_vault`；已有的 vault 用 `gocryptfs -passwd -scryptn 18` 升级（实测可行，runbook 里写了）。
- `tune-inotify` 不再假装成功：试一次，然后如实说明当前的限制和在宿主上怎么提高。
- `vault-sync` 的输出进容器日志（由 Docker 的日志驱动轮转），不再写一个无限增长、没人读的 `/var/log/vault-sync.log`；桌面程序的日志
  （原来是 `/dev/null`）写到 `/var/log/desktop/<程序>.log`，每个 2 MB、留一份备份。
- 只文档化、没修：`MEMORY_LIMIT` 默认不限制；Docker 不会因 unhealthy 重启容器。
- 第一次真正跑独立客户端（`50-clash-client`）暴露了两个缺陷：compose 里 `healthcheck.start_interval` 没带 `start_period`，被 daemon 拒绝；
  以 root 启动时它写不进归 `clash` 所有的 `/etc/clash`（没有 `DAC_OVERRIDE`）。现在 `proxy-only` 先交出目录，再以 `clash` 用户重新运行自己，
  配置由将要读它的用户创建。

### 9. 工程

- CI：`ci.yml` 加了 `audit`（`cargo audit` + `bun audit`）和 `scripts`（shellcheck、密钥守卫、脚本测试）作业；`fuzz.yml` 每周对 6 个解析器跑覆盖率
  引导的模糊测试（上传体、路径、会话 cookie、Access JWT、WebSocket Origin、出口白名单）；`tools.yml` 每周报告落后的固定工具；
  `build-image.yml` 要先过 `smoke.yml` 才推送。这些都**没有在 GitHub 上真正跑过**。
- 新增的 Rust 测试覆盖：白名单解析与防火墙规则、`no_proxy` 合并、功能开关（只有明确的 no 才关）、capability 解析、密文目录模式、分发层
  （运营者命令对非 root 拒绝）、横幅和 doctor 的防火墙行等。
- 找出并修掉了烟测自己的两处**空转**：`75-privilege` 读 code-server 的环境用的是 root（读不到别的用户的 `/proc/<pid>/environ`，「没有密钥」的断言
  因此什么也没证明）；`50-clash-client` 的「主进程是 clash 用户」在容器反复重启时两边都是空串而「通过」。

### 10. Selkies 搬到 2.x（上游删掉了 1.x）

- **起因**：`-full` 镜像（`DESKTOP_STACK=selkies|both`）构建不出来。镜像固定的 Selkies 1.x 要 `pixelflux==1.6.4` / `pcmflux==1.0.8`，作者已把
  1.x 全部从 PyPI 和 GitHub 删掉（只剩 2.x），而那个 Selkies 提交又导入了 2.0 里不存在的类，所以不存在「再固定一次」。CI 的 `full` 变体
  （`build-image.yml`、`smoke.yml`）因此早就是坏的，而没人看到：默认镜像不含 Selkies。
- **做法**：迁到官方发布的 Selkies 2.2.0。`src/image/tools/selkies.env` 固定 release 里的 wheel（sha256 与 GitHub 元数据、项目自己的 `SHA256SUMS`
  三处一致，`fetch-tool` 校验）；它的 Python 依赖在 `selkies-requirements.txt`：`uv` 从 wheel 自己的元数据解析，每个版本带 PyPI 上所有文件的
  sha256，镜像用 `pip --require-hashes --only-binary=:all:` 安装，所以镜像里没有编译器，也装不进别的东西。`deploy/scripts/update-tools.sh`
  同时改 pin 和 lock（`--relock` 只重解依赖），`tools.yml` 每周检查是否落后。
- 2.x 在同一端口提供客户端、API 和 WebSocket（`/desktop/api/websockets`）：vendored 的 1.x 客户端（1237 个文件）、Dockerfile 里的 `selkies-web`
  选择阶段、Caddy 的 `file_server` 和 `SELKIES_WEB_ROOT` 都删了；Caddy 对 `/desktop/*` 只做反代，不改路径；`doctor` 改为问 Selkies 的
  `/desktop/api/health`，不再只看端口。镜像里的 PATH 也不再被 `/opt/selkies-venv/bin` 抢先（它让 root 的 `python`、`pip` 指向那个 venv）。
- **安全相关的设置**（`etc/desktop/start-selkies.sh`，固定的、不接受容器环境变量覆盖的）：2.x 默认开 Basic 认证、没有密码就拒绝启动，这里关掉，
  网关是唯一的登录；只监听 `127.0.0.1`；只用 WebSocket 传输（关掉 WebRTC：它要 UDP 和公网 STUN，既没发布也过不了出口防火墙）；关掉分享链接、
  打印；音频随 `DESKTOP_AUDIO`。它的 API（截图、会话列表、`~/Desktop` 的上传下载）都在网关之后，以 `dev` 运行，权限等同桌面里的终端
  （写在 threat-model 里）。
- **验证**（`dev-workspace:kali-full`，Python 3.14，arm64，真 Chrome）：`10-boot`（含 doctor 问到 Selkies）、`20-auth`（经网关的 WebSocket：无会话被拒、
  有会话 101、跨站 Origin 403、只监听 127.0.0.1:6080）、`55-browser`（`/desktop/` 出现真实的 XFCE 画面，点击面板会改变远端画面，即输入到达会话）、
  `75-privilege`、`40-vault`（Selkies 运行中能正常 `lock-vault`）、`72-egress`。日志里 `DESKTOP_AUDIO=1` 时 pcmflux 成功从 `output.monitor` 采集、
  创建 Opus 编码器；客户端驱动 RandR 改分辨率也有效。
- **第 6 项（桌面 HOME）的结论**：Selkies 组 18 个进程的 `HOME` 全是 `/workspace/.desktop`，KasmVNC 组是 `/workspace/.desktop-vnc`，
  没有一个落到 `/root`；`75-privilege` 现在对 dev 的**所有**进程断言 HOME 在 vault 内（并做过变异验证：起一个 `HOME=/root` 的进程会被抓到）。
- **顺带修的两处烟测空转**：浏览器检查里「点击后画面变了」比较的是整屏，而面板上的时钟每分钟自己变一次，点击什么都没做也可能「通过」，现在只比较菜单
  弹出的区域；我自己新加的「只监听 loopback」第一版用了镜像里没有的 `ss`，对空串做否定匹配，永远通过，改成直接读 `/proc/net/tcp*`。
- **没验证**：amd64；GPU 编码（容器里没有 GPU，用的是软件 x264，Selkies 只报 NVENC 不可用）；`DESKTOP_AUDIO=1` 时浏览器端是否真的出声（只看到服务端
  采集成功）；只用 Chrome 试过，Firefox 和 Safari 没试（WebCodecs 要 Chromium 94+ / Safari 16.4+）。

### smoke 的结果

- **默认镜像**（`dev-workspace:kali`，KasmVNC，本机 Colima/arm64，2026-11-04）：`run.sh --services` 跑全套（含真实 S3/MinIO、sshd、git-HTTP、
  Shadowsocks 节点、croc、独立 Clash 客户端，以及真 Chrome 的浏览器检查）——**553 通过 / 0 失败 / 0 跳过**。
  其中出口防火墙默认开启的那组（72-egress-enforce，45 项）覆盖：入站发布端口仍可达（302/200）、工作负载只能经 Clash 出网、`EGRESS_ALLOW_CIDRS`
  直连、ssh 经 Clash 且用户 `~/.ssh/config` 能对白名单主机覆盖；`20-git-remotes` 跑两遍（直连 + 防火墙后，SSH 走 Clash、HTTP 走 http.proxy，节点两个
  CONNECT 都看到）。本会话早先失败的两项（45-plugins 的 pentest 密文目录权限、50-clash-client 的 healthcheck/权限/分发）都已修复并在此全过。
- **-full 镜像**（`dev-workspace:kali-full`，Selkies + KasmVNC，同机，Selkies 2.2.0，Python 3.14）：跑 Selkies 相关子集——
  `10-boot` 23、`20-auth` 60、`55-browser` 22（真 Chrome 在 `/desktop/` 取到 XFCE 画面、点击面板远端有反应）、`75-privilege` 52、`40-vault` 43，
  全过 0 失败。详见上面第 10 节。（曾经一次 40-vault 在 `-full` 上失败，是我同时留着多个各带两个桌面的实例把那台 6 核 VM 压垮、`lock-vault`
  停两个并行桌面时撞上 XFCE D-Bus 辅助进程的竞态；清掉多余实例后独立重跑两次都 43/0，手动按同样时序也复现不出，判定为宿主超载下的偶发，不是回归。）
- 没在 GitHub Actions 上真跑过（`ci.yml`/`smoke.yml`/`build-image.yml`/`fuzz.yml`/`tools.yml` 都只在本机等价环境验证）；amd64 没构建或运行过。



## 架构 review backlog（2026-11-01 提出，2026-11-03 推进）

本周已完成并提交（背景，未 push）：统一鉴权网关（deny-by-default，会话 cookie + Cloudflare
Access JWT 合流）、Clash DNS 开机自动探测 + `set-clash-dns`、桌面 glycin/bwrap 崩溃修复、
一条命令的 Access-SSO 临时实例编排（`up.sh`/`down.sh`）、每实例资源上限（`pids_limit` 等）、
应用级健康检查。下面是 review 里「要啃才质变」的四项，按优先级。**2026-11-03 在本机（macOS arm64 +
Colima/vz）上推进了其中三项的第一步，每一项都写明做了什么、没做什么、哪些只是单元测试验证过。**

- [ ] **#2 隔离强化（高）**（Caddy/Clash 降权 + dev 用户已做；**嵌套虚拟化已开、Kata+CRI 已接进编排**——`setup-kata-runtime.sh` + `up-kata.sh`/`down-kata.sh` 把完整工作区起进 microVM，vault/防火墙/网关登录端到端通、稳定(runp 10/10)；剩裸机性能 + Mac 端口暴露 + 多实例边缘）— 现状：单容器、`CAP_SYS_ADMIN`、`seccomp:unconfined`、容器内服务绝大多数是 root，
      是安全主短板（见 threat-model 的「容器逃逸」「出口强制」）。目标：每个临时实例跑在 microVM
      （Kata / Firecracker）或 gVisor 里；容器内服务降到非 root uid。验收：逃逸不直接拿到宿主 root；
      egress 能按 uid 强制；smoke 仍全过。工作量大，分步做。
      - [x] 第一步（2026-11-03）：**Caddy 改为 `caddy` 用户**并用 `env -i` 清空环境（它是唯一面向网络的进程，
        原来是 root 且继承了 PASSWORD、RESTIC_PASSWORD 等全部密钥）；**Clash 始终以 `clash` 用户启动**
        （原来只在 `EGRESS_ENFORCE` 下；开机启动时只设了 uid，进程的 gid 仍是 0（root 组），现在 uid、gid 和
        清空附加组一起设，与 `clash-watcher.sh` 一致；`setup` 重启 Clash 也不再用 root）。
        登录接口加了限速并把 bcrypt 移出异步线程，`next` 的开放重定向（`/\host`、`/<TAB>/host`）已堵。
      - [x] 第二步（2026-11-04）：用户代码（code-server 终端和桌面）降到普通用户 `dev`（uid 1000）；做法、边界和验证见下面「第三轮」§5。
        上面当时列的前提都做了：工作区 vault 以 `-allow_other -force_owner 1000:1000` 挂载（`45-plugins` 和 `75-privilege` 断言了谁能进）、
        mise 的运行时挪到只读的 `/opt/mise`、`lock-vault` 的占用者检测照常工作（root 看得到别的 uid 的 `/proc/<pid>/fd`；`40-vault` 在
        `dev` 下通过）。同一 uid 内降能力不是边界（同 uid 的 ptrace 不需要 `CAP_SYS_PTRACE`），所以是**另一个** uid。
      - [x] 第三步（2026-11-04）：**嵌套虚拟化在本机可用，Kata microVM 原型跑通**（之前说"本机无法验证"是错的——这台是 Apple **M5** +
        macOS 27，不是 M1/M2）。做了什么：
        - **在 dev-workspace 这个 Colima VM 上开了嵌套虚拟化**（`colima start dev-workspace --nested-virtualization`，即 `nestedVirtualization: true`）。
          stop/start 保留磁盘：三个镜像（kali / kali-full / clash-client）、5 个卷（含 vault-data）、实时容器全部还在，容器自动回来且 healthy。
          VM 客户机里 `/dev/kvm` 出现（ARM64 内核只有虚拟化扩展 EL2 可用时才建这个设备）。colima 0.10.3 / lima 2.2.1 支持 `-z`。
        - **装了 Kata 4.3.1**（官方 `kata-static-4.3.1-arm64.tar.zst`，sha256 对 GitHub 元数据校验过 → `/opt/kata`），hypervisor 用 QEMU 11.0.1，
          客户机内核 vmlinux-6.18.56。用**系统 containerd + `ctr --runtime io.containerd.run.kata.v2`**（Docker 对 Kata 的 shim-v2 没有原生支持，
          daemon.json 的 runtimes 要 runc 式 OCI 二进制，而 Kata 4.x 只给 containerd shim）。
        - **坑**：arm64 嵌套 vz 下 QEMU 不认 `-cpu host,pmu=off`（`Property 'host-arm-cpu.pmu' not found`，QEMU 直接退出 → QMP 超时）。
          把配置 `cpu_features = "pmu=off"` 改成空即可;runtime-rs 的 `KATA_CONF_FILE` 只收"官方"路径,所以是改默认配置并修好那个断掉的 `configuration.toml` 符号链接。
        - **验证通过**：`ctr run --runtime io.containerd.run.kata.v2 alpine uname -r` → **6.18.56**(Kata 客户机内核),区别于宿主 VM 的 **6.8.0-117**
          ——容器确实在独立 microVM 里、有自己的内核(逃逸只到 microVM,不是宿主 VM,这就是 #2 的核心隔离属性)。`dev-workspace:kali` 镜像也能在 Kata 里启动
          (内核 6.18.56,自带 nft v1.1.7、gocryptfs 2.6.1)。**nftables 在 Kata microVM 里加规则成功**(`--cap-add CAP_NET_ADMIN`,`ip daddr 10.9.9.9 drop`
          真的进了 ruleset)——这正是 **gVisor 做不到的那条**,Kata 可以。
        - **`CAP_SYS_ADMIN → agent 崩` 的诊断与解法(2026-11-06)**:裸 `ctr run --cap-add CAP_SYS_ADMIN`(SingleContainer 模式,容器 spec 即 sandbox spec)
          会稳定触发 **sandbox agent 的 vsock 连接被重置**;对照 `ctr container create` 的 OCI spec,带/不带只差能力集里一个 `CAP_SYS_ADMIN`(无设备/挂载变化)。
          根因是 SingleContainer 下 SYS_ADMIN 直接作用到 sandbox 启动期。**解法:走 CRI**——pod sandbox(pause 容器,最小能力)先把 agent 连上,工作负载容器再用
          `add_capabilities: [SYS_ADMIN, NET_ADMIN, MKNOD]`(**不是 `privileged`**,privileged 会要求 sandbox 也 privileged 并再次触发问题)在**已运行的 sandbox**里拿到能力,
          agent 连接阶段永远不碰 SYS_ADMIN。装了 crictl v1.37.0、给 containerd 的 CRI 加了 `kata` 运行时句柄(`io.containerd.kata.v2`)。
        - **gocryptfs + nftables 在 microVM 内都跑通了(CRI + Kata)**:`crictl` 建 kata pod → 在其中建带 `add_capabilities` 的 `dev-workspace:kali` 容器 → exec 里
          `uname -r`=**6.18.56**(microVM 内核,宿主 VM 是 6.8.0-117)、`CapEff` 含 SYS_ADMIN、`nft add rule ... ip daddr 10.9.9.9 drop` 进了 ruleset、
          `gocryptfs -init`+挂载成功、写入 `vault-secret-under-kata-microvm` 再读回、密文 `gocryptfs.diriv` 落盘。**即 egress 防火墙的 nftables 和 vault 的 FUSE 这两项
          硬需求在 Kata microVM 里都成立**(gVisor 做不到 nftables)。
        - **稳定性调优(2026-11-07)**:嵌套 vz 下 agent 的 vsock 初始连接本来是概率性的(`crictl runp` 约 1/3~1/5 成,`reconnect_timeout_ms` 没用——
          那是另一个超时)。根因是客户机在套娃虚拟机里启动太慢、agent 没及时就绪。调 `default_vcpus=4`(启动快)+ `dial_timeout_ms=100`、`reconnect_timeout_ms=60000`
          (更耐心的重试窗口)后,**`runp` 实测 10/10**,每次 10–13s;再给启动器加 8 次 runp 重试兜底。这些固化在 `deploy/scripts/setup-kata-runtime.sh` 里。
      - [x] **第四步(2026-11-07):接进编排,完整工作区在 microVM 里端到端跑通**。
        - `deploy/scripts/setup-kata-runtime.sh`:幂等地装/配 Kata 4.3.1 + crictl v1.37.0 + CNI v1.9.1(bridge+portmap)+ containerd CRI 的 kata 句柄,
          带上 pmu 修复和稳定性调参;`--verify` 只检查不改。全部 pin 了版本和 sha256。
        - `deploy/orchestration/up-kata.sh <id> [port] [env_file]` / `down-kata.sh <id>`:把工作区起成一个 **Kata CRI pod**——
          pause sandbox 先连 agent,工作负载容器带 compose 里那 10 个能力(用 `add_capabilities`,非 privileged)+ `/dev/fuse` + 5 个 host-path 卷 +
          `seccomp:unconfined`;**CNI 给 pod 自己的 netns**(出口防火墙隔离在 microVM 内,不碰宿主)+ 宿主端口映射到网关;runp 带重试兜底嵌套 vsock 抖动。
        - **端到端验证(firewall 默认开启,真实配置)**:`up-kata.sh demo 18092` → 完整工作区在 microVM 里启动(内核 6.18.56)、**vault gocryptfs 挂载**、
          **egress nftables 防火墙加载(10 条规则,在 pod 独立 netns 内)**、从宿主端口**真实登录成功**(`POST /auth/login` → 303 + `sdw_session` cookie);
          `down-kata.sh` 干净拆除(pod + 卷)。连跑 2 个完整周期都稳。
      - [ ] #2 余下的工作(生产化,非本机能完全验证):(a) **性能**——嵌套 virtio 小文件读慢是唯一短板(真因见 (e)),裸机 Linux 无此问题。(b) **Mac 宿主暴露**——`up-kata.sh` 把网关映射到 VM 的回环端口,从 Mac 访问要 `colima ssh -L` 隧道(脚本已打印);Colima 不自动转发非 docker 端口。
        (c) **多实例**——现在每实例一个 host-path 卷目录 + 独立端口,但没接 `up.sh` 的 Cloudflare 边缘(隧道/Access);要并入需把 provision.sh 那套接到 CRI pod。
        (d) 在 Kata 下重跑整套 smoke(目前验了启动/网关/登录/vault/防火墙,桌面画面等受性能影响没验)。**gVisor 仍不适用**:netstack 无 nftables。
        (e) **Rust VMM / Cloud Hypervisor(可选,已跑通)**:官方静态包没编 clh 的 cargo feature;从源码 `cargo build --release -p runtime-rs --features cloud-hypervisor` 出带 clh 的 shim,
          装成独立 `kata-clh` 句柄(脚本 `up-kata-clh.sh` + `setup-kata-runtime.sh --with-clh`;`up-kata.sh` 加了 `SDW_KATA_RUNTIME` 开关,`down-kata.sh` 通用)。Dragonball 本 build experimental、未成。
        - **嵌套性能(定论,不再调)**:启动慢 / 网关短暂 502 的真因是**嵌套 virtio 的小文件读慢**——读 650MB bundle:L1 overlayfs 0.1–0.8s,嵌套 microVM ~30s 且反复读不缓存;
          CPU、时钟、裸盘都与 L1 相同(纯 CPU 0.507 vs 0.509s),只有 L2 的 virtio I/O 慢(virtio-fs 与块设备 devmapper 两条路都 ~30s,DAX/缓存调参无效)。Mac「VM 套 VM」的固有代价,裸机 Linux(单层)无此问题。
          (顺带修的一处独立问题:出口被 fail-closed 防火墙 DROP 时 code-server 启动外联挂起、反复重启 → `supervisord.conf` 加 `--disable-update-check --disable-telemetry`。)
        **定位(一句话)**:`up.sh`(docker/runc)是默认主线,精简/可靠/稳定/安全四项齐全、实测稳;**Kata/clh microVM 仅作裸机 Linux 上的可选额外硬件隔离**,Mac 嵌套下 I/O 慢、不作默认。
- [x] **#4 审计 / 可观测（中）** — 2026-11-03 完成。网关（`sync-service`）把每个值得追溯的事件写成一行 JSON：
      登录成功 / 失败 / 被限速、登出、Cloudflare Access 放行（带 Access 的 e-mail 或 subject，每个身份 15 分钟记一次）、
      自毁接口的每次调用（拒绝原因：`no_key` / `bad_request` / `wrong_passphrase` / `rate_limited`，触发成功或启动失败）、
      出口状态变化（开机时对比上次，之后由新的 `egress-monitor` 守护进程每分钟探测，连续两次探测一致才算变化）。
      写入 `/var/log/sdw/audit.jsonl`（0600，目录 0700，每条写盘即 `fsync`——自毁会很快杀掉容器；按 5 MiB 轮转保留 3 份；
      命名卷 `audit-data`，容器重建后还在，`docker compose down -v` 才清掉），同时打到 stdout（`docker logs`）。
      读取：`audit-log [--json] [N]`，`doctor` 里的 “Audit trail” 一行（24 小时内的登录/失败/限速/自毁/出口变化，
      连续 10 次失败、任何限速或自毁调用时给 ⚠）。可选 Prometheus 指标：`METRICS_ENABLED=1` 后 `GET /sync/api/metrics`
      （在登录网关之后，所以抓取方需要会话 cookie 或 Cloudflare Access service token）。
      防日志被灌满：攻击者能触发的事件（错误密码、被拒的自毁调用）每分钟最多写 30 条，其余合并成一条 `audit.suppressed`；
      成功登录、登出、自毁触发永不丢。口令从不进日志（有测试断言）。
      验证：core / infra / sync-service 单元与集成测试（含：日志里没有口令、限速、指标计数）；smoke 的
      `25-audit.sh` 在真实镜像上检查同样的事（结果见下方「2026-11-03 在本机跑 smoke」）。
      **没做 / 局限**：「谁」只是声明——地址来自 `CF-Connecting-IP` / `X-Forwarded-For`，直连源站时调用方可以伪造
      （因此限速另有全局上限）；主密码只有一个，没有用户名；Access 放行用合成的 RSA 密钥做过单元测试，
      **没有对真实的 Cloudflare Access 验证过**；没有外部告警，也没有把日志送到容器之外的收集器（stdout 的拷贝可被收集）。
- [ ] **#5 密钥脱离环境变量（中）** — 现状：PASSWORD / 订阅 / RESTIC_PASSWORD / AWS / VAULT_GIT_REPO
      token / 编排的 CF token 都在 env / `.env`。目标：改走 tmpfs 挂载或 Docker secrets 或启动时拉取；
      对齐 `deploy/docs/design-memory-only-secrets.md`（TPM/KMS + 把「登录凭据」与「vault 密钥」拆开）。
      - [x] 2026-11-03：八个密钥变量（PASSWORD、CLASH_SUBSCRIPTION_URL、VAULT_GIT_REPO、RESTIC_PASSWORD、AWS_ACCESS_KEY_ID、
        AWS_SECRET_ACCESS_KEY、PENTEST_VAULT_PASSWORD、CROC_PASS）都可以给 `<NAME>_FILE=路径`（Docker/Compose secrets 挂在
        `/run/secrets/`）。非空的 `<NAME>` 优先，所以 `.env` 里残留的值不会被悄悄忽略；`<NAME>_FILE` 读不了就**拒绝启动**
        （否则缺失的 PASSWORD 会变成随机生成的一个）。restic / croc 子进程由代码从文件取值后只放进它们自己的环境。
        `caddy hash-password --plaintext <密码>` 把主密码放在命令行上（任何进程都能从 `/proc` 读到）——改为进程内 bcrypt。
        提供 `docker-compose.secrets.yml` 覆盖文件（PASSWORD + 订阅 URL）和 `make secrets-init`（从 `.env` 写出
        `secrets/*`，0600，不打印值、不删任何东西）。
      - [ ] 没做：Compose 的文件型 secret 是**宿主文件的 bind mount，不是 tmpfs**（宿主盘上有 0600 文件，需放在加密卷）；vault 口令仍然
        等于 `PASSWORD`，**把登录凭据与 vault 密钥拆开 + TPM/KMS 没做**（设计文档的 Track B/C；Track A 的大半由网关和权限分离顺带做掉了，
        见该文档开头）；用 `.env`（旧方式）时，密钥仍被所有 root 子进程继承（工作区用户 `dev` 拿不到）——只有文件方式才让它们离开环境。
        （这里原来还列了：主密码写进 code-server 的 `config.yaml`、`deploy/orchestration` 的 token 在 argv 里、`set -a` 导出、
        `destroy.sh` / `migrate-clash-live.sh` 直接读 `.env`——已在第三轮 §5 和 §7 处理，`migrate-clash-live.sh` 已删除。）
- [ ] **交互卡顿：共享中继（中）** — 现状：CF 隧道墙内→海外边缘绕行，交互桌面卡。目标：一台好线路
      VPS 跑 `frps` 或自建 `Headscale`/DERP，`provision` 时每实例自动注册（生成密钥 + 子域，和现在
      CF 流程同构，仍可批量化）；桌面可配合切 Selkies（WebRTC）。验收：异地交互延迟明显下降。
      **未做，且需要一台线路好的 VPS 才能验证**，这台机器上做不了。review 时记下的设计要点：加一个并列的
      `provision-relay.sh ID PORT`，写出与现在同名的状态键（`FQDN`、`ORIGIN_PORT`、连接器 PID 或 sidecar 名）并带自己的回滚；
      `up.sh` 按 `RELAY_MODE` 分支，端口挑选和 `docker compose` 调用不变。relay 模式下 `ACCESS_AUD`/`ACCESS_TEAM_DOMAIN` 留空
      （cookie 网关成为唯一边界，必须设强 PASSWORD，机器客户端的认证要在 VPS 的代理上自己加）。自毁限速取客户端地址靠
      `CF-Connecting-IP` → `X-Forwarded-For` 首跳，frps 前必须由中继**覆盖** `X-Forwarded-For`。Selkies（WebRTC）还要中继转发 UDP。

## 2026-11-03 在本机构建与 review（macOS arm64 + Colima/vz）

**构建**：Kali 基础、默认 KasmVNC 栈，从零构建成功，约 15 分钟、5.5 GB（6 核 / 12 GiB 的专用 VM、走本机代理）。
做法和踩到的坑见 README「在 Mac 上本机构建」。这次发现并修掉的构建问题：

- `docker compose build` 在没有 buildx 插件时**悄悄退回经典构建器**，这个 Dockerfile 的 `COPY --chmod` 过不去（昨天
  的失败构建就是它在核心 apt 层之后留下的）。README 和 `setup-colima-build-vm.sh` 现在会检查并提示。
- 官方 Kali 重定向挑到坏镜像时核心 apt 层 `exit code: 100`；`KALI_MIRROR=mirrors.aliyun.com` 一次装完 198 个包。
  TUNA 的 Kali 镜像对 `curl` 回 403（`.env.example` 里原来推荐它）。
- 默认 2 CPU / 2 GiB 的 Colima VM 太小（LTO 的 Rust 编译、桌面都会被 OOM），上面还有别的容器：新增
  `deploy/scripts/setup-colima-build-vm.sh` 起一个专用 profile，不动原来的。
- 构建没法带代理：compose 只转发四个 build arg。新增 `BUILD_PROXY` / `BUILD_NO_PROXY`（BuildKit 的预定义 build arg：
  不进镜像和历史、不使缓存失效）。GitHub 下载加了 `--retry 5 --retry-all-errors`；mise 和 code-server 的安装循环
  原来以 `sleep` 的退出码 0 结束，三次都失败也「成功」，现改为失败即中止，并先下载到文件再执行。
- `Makefile` 的 `smoke` / `image-export` 默认标签是 `latest`，而 compose 构建出来的是 `dev-workspace:<WORKSPACE_VARIANT>`：
  现在从 `.env` 读。`bootstrap.sh` 的 `docker compose build 2>/dev/null` 把失败原因吞掉，已去掉；
  `setup-docker.sh` 的 `df -BG` 在 macOS 上直接让脚本退出，已改成 POSIX 的 `df -P -k`。

**smoke 套件在新的鉴权网关之后一直是过期的**（TODO 下面写的 298/0 是网关提交之前的数字）：`20-auth.sh` 期望 Basic Auth
语义、`40-vault.sh` 的 `sync_state`、`sync_e2e.ts` 和浏览器检查都带 `-u user:密码`，网关现在对这些回 302。已全部改为
先 `POST /auth/login` 取会话 cookie；`10-boot.sh` 的 `backup-watcher is running` 在快 VM 上会赶在它 5 秒的 `startsecs`
之前断言（容器在 Caddy 一起来就 healthy），改为等 STARTING 的稳定。

**review 发现、当时没有动的**（按严重程度；每条开头的「→」是 2026-11-04 的现状）：

【→ 已处理，历史清理和密钥轮换除外（第三轮 §1，「还没做」）。】 1. **`.env` 被 git 跟踪，里面是真实的 PASSWORD 和订阅 URL**（`775e5fb chore: add .env config and stop ignoring it`，
   `.gitignore` 里 `#.env` 被注释掉；共 5 个提交碰过它；权限 0644）。PASSWORD 同时是 vault 口令。`origin` 现在是本地文件
   `~/Downloads/dw.bundle`，历史在那个 bundle 里。**任何 push 或分享之前**：停止跟踪（`git rm --cached .env` + 取消
   `.gitignore` 的注释）、轮换这两个密钥、从历史里清掉。我没有碰，因为那个提交是你有意做的，而改写历史不可逆。
【→ 已处理（§7）；`--paranoid` 仍会删整个 `~/.cloudflared/`，它是显式的深度清理。】 2. `destroy.sh` 对开发 checkout 很危险：`rm -rf "$SCRIPT_DIR"` 删整个仓库（含 `.git`、未提交的改动）、`rm -rf ~/.cloudflared/`
   删**所有**隧道的凭据、`docker builder prune -af` 清掉整个构建缓存；卷名正则会匹配其它 compose 项目（含 `sdw-smoke*`）；
   末尾无条件打印「removed」。macOS 分支只卸载了 host-watcher 的 plist，漏了 `com.cloudflare.tunnel.plist`。
【→ 已处理（§7）。】 3. `host-watcher.sh`：LaunchAgent 的 plist 没有 `PATH`，launchd 默认 PATH 里没有 `/opt/homebrew/bin`，Homebrew 装的
   docker 会找不到而被 `KeepAlive` 无限重启；`docker volume ls --filter name=destruct-state | head -1` 会挑到任一项目
   的卷（含 smoke 的），可能在不该触发时触发 `destroy.sh --force --paranoid`。**测试期间不要安装它。**
【→ 已处理（§3）。】 4. Clash 控制器 `127.0.0.1:9090` 没有 `secret`：容器内任何进程（以及容器内浏览器里的页面，CORS 未核实）都能把
   Clash 切到 DIRECT 或加载别的配置。在用户代码仍是 root 的现在价值有限，降权之后就重要了。
【→ 已处理（§6）：三个 capability 搬到 `docker-compose.pentest.yml`；`seccomp:unconfined` 仍在（bwrap 的 `pivot_root`），没有自定义 profile。】 5. `cap_add` 比需要的多：`NET_RAW` / `SYS_CHROOT` / `MKNOD` 只服务 pentest 插件，`NET_ADMIN` 只在 `EGRESS_ENFORCE` 下需要；
   `seccomp:unconfined` 只为 bwrap 的 `pivot_root`（glycin 垫片已经在没有 userns 时降级），可以换成「默认 profile +
   pivot_root」的自定义 profile。这些改动会影响 pentest 插件和桌面，需要对应的 smoke 才能动。
【→ 所有桌面程序现在经 `sdw workload --home <目录>` 启动，HOME 由启动器设定（§5）；在 `-full` 镜像里实测：Selkies 组 18 个进程的 HOME 都是 `/workspace/.desktop`，没有落到 `/root`，`75-privilege` 对 dev 的所有进程断言了这一点（§10）】 6. `desktop-selkies.conf` 的 `desktop-xfce` 没有 `environment=`，`start-xfce.sh` 的 `HOME` 默认值被 Docker 注入的
   `HOME=/root` 盖住，XFCE 的配置/浏览器 profile 会落到容器的明文可写层而不是 vault（违反 threat-model 不变量 1）。
   只影响 `DESKTOP_STACK=selkies|both`，需要在该镜像里读 `/proc/<pid>/environ` 确认后再改。
【→ 已处理（§2）。】 7. 供应链：KasmVNC `.deb` 没有校验和；mise、code-server 的安装脚本直接执行（无哈希）；`pnpm@latest`、`node@20` 浮动；
   `python-xlib` 取自 `master.zip`。其余下载（caddy / restic / croc / mihomo）有 sha 校验。
【→ 前几条（`toolchain:`、`bun-version`、`smoke.yml` 的 `setup-bun`）已改，smoke 成了发布的门；CI 仍**没有在 GitHub 上真正跑过**；`build-image.yml` 的基础镜像仍与本地 compose 的默认不同（设计如此，`BASE_IMAGE`）。】 8. CI 从未在 GitHub 上跑过：`ci.yml` 用 SHA 固定了 `dtolnay/rust-toolchain` 但没给 `toolchain:`（该 action 靠 ref 名取
   工具链，SHA 引用多半会失败）；`bun-version: latest` 与 Dockerfile / 本机的 1.4.3 不一致；`build-image.yml` 的基础镜像
   永远是 Kali，本地 compose 默认是 Debian trixie，所以本地与 CI 的镜像不同；`smoke.yml` 没有 `setup-bun` 而 `30-sync.sh`
   要在宿主上跑 bun。
【→ 已处理：`threat-model.md`、`architecture.md`、runbook、README、QUICKSTART 都按现状重写过。】 9. 文档与代码的出入：`threat-model.md`、本文件下面「已知缺口」里「容器内全是 root，按 uid 的规则无效」是 `EGRESS_ENFORCE`
   出现之前的描述（现在 root 进程也会被 nftables 挡住，真正的缺口是持有 `NET_ADMIN` 的 root 可以删表）；`architecture.md`
   的请求流程图还写着 “Caddy basic_auth”。

## 第二轮 review（2026-11-03 晚）：每一项都有证据，括号里写明怎么得到的

**不是读代码猜的，已实测**

【→ 已处理并验证（§3、§8；`services/50-clash-client`）。】 1. **`deploy/client` 的独立 Clash 容器不能用**（实测）。mihomo 不展开配置里的 `${VAR}`：provider 去请求字面量
   `${SUBURL}`，报 `unsupported protocol scheme`。而它的 Dockerfile 把模板原样当 `config.yaml`，没有任何一步替换
   `CLASH_SUBSCRIPTION_URL`（主镜像由 Rust 的 `render_clash_config` 替换，所以不受影响）。顺带：mihomo 下载没有校验、
   `docker-compose.yml` 的 `version:` 已过时、基础镜像没固定、以 root 运行。
【→ 已处理（§4；`20-auth`）。】 2. **Caddy 对所有页面一个安全响应头都不发**（实测 `/`、`/sync/` 只有 `Server: Caddy`）。登录页自己带了头，其余没有。
   `/sync/` 是处理 vault 口令的静态页，可以直接加严格 CSP；code-server 要谨慎（webview 用同源 iframe，`frame-ancestors 'self'`）。
【→ 已处理（§3）。】 3. **Clash 控制器无 secret**（实测：容器里 `GET 127.0.0.1:9090/configs` 回 200）。
【→ 已处理：防火墙默认开启（§6；`72-egress-enforce`）。】 4. **默认模式下忽略代理的进程直连外网**（对照实测：未启用时 `nc 1.1.1.1 443`、`curl --noproxy example.com` 成功；
   `EGRESS_ENFORCE=1` 时两者都被拒）。对一个以隐私为卖点的产品，真实 IP 会被任何不读代理变量的工具暴露。
   `EGRESS_ENFORCE` 此前**没有任何 smoke 覆盖**，现在有了 `72-egress-enforce.sh`（9/9），把它设为默认（pentest 除外）
   的风险降低了，但 `noproxy` 命令、`apt`/pentest 直连会受影响，需要先决定。
5. **监听面很小**（实测 `/proc/net/tcp`）：只有 Caddy 监听全部接口（`:8080`），其余全在回环；进程 32 个 root、`caddy`、`clash` 各 1。
【→ 已处理（§2：校验和固定 + `tools.yml` 每周检查 + Dependabot）。】 6. **固定的上游版本落后**（对比 GitHub 最新版）：mihomo 1.18.0（2024-01）对 1.19.33、restic 0.17.3 对 0.19.1、
   KasmVNC 1.4.0 对 1.5.2、croc 11.5.5 对 11.6.3、code-server 4.141.0 对 4.144.0；Caddy 已是最新。没有 Dependabot / Renovate，
   Dockerfile 里的 `ARG` 版本、两个基础镜像的 digest、GitHub Actions 的 SHA 都靠人记得更新。
【→ 已处理（§2：`audit.toml` 写明理由、`cargo audit` 与 `bun audit` 进 CI、vite 升到 6.4.5）。】 7. **依赖漏洞扫描**（`cargo audit`，RustSec 1402 条，170 个 crate）：1 条，`rsa 0.9.10` 的 RUSTSEC-2023-0071（Marvin，
   时序侧信道，无修复版本）。我们只用它**验证**签名（公钥运算），不涉及私钥时序，不适用；应在 `audit.toml` 写明理由后忽略，
   并把 `cargo audit` 放进 CI。`bun audit`：12 条（5 高），全部在 SPA 的构建工具链（vite 6.4.2、postcss、nanoid、launch-editor），
   不进运行镜像，升级 vite 即可。
8. **shellcheck**：39 个脚本只有 1 个 warning（`dr-drill.sh:49`，已修）。
9. **资源**：桌面栈（KasmVNC）空闲时 565 MiB、157 个进程、CPU 0.07%；`mem_limit` 默认 0，`mem_reservation` 只是软限制。

**读代码得出的结论（没有单独复现）**

【→ 符号链接的缺口已堵（§4：`openat` / `renameat` + `O_NOFOLLOW`）。】 10. 上传存储没有发现问题：路径校验、先认证后落盘、重放窗口、整体校验和、总大小上限都在。唯一值得一提：`commit` 会跟随
    `/workspace/shared` 里的符号链接（先要在 vault 里种一个链接并持有同步密钥才能利用）。
【→ IDEA 和 Burp 的下载已有校验和（§2）。】 11. `desktop-install` 的 `sh -c` 只拼接编译期常量（用户参数只是查表键），**不可注入**（推翻了子代理的怀疑）。但 IDEA（固定在 2024.1）
    和 Burp 的下载没有校验和。
12. 浏览器端（SPA）：`innerHTML` 全是静态模板，日志用 `textContent`，文件名不会变成 HTML；同步密钥是不可导出的 `CryptoKey`，
    PBKDF2 600k 次；盐是全局常量（线上格式已固定，改它是破坏兼容）。
【→ 已处理（§4）。】 13. 会话 cookie 没有 `Secure`（经 Cloudflare 的 HTTPS 访问时应该加，可以看 `X-Forwarded-Proto`）；`AUTH_SESSION_PERSIST` 的密钥是
    `HMAC(密码, 固定标签)`，拿到一个 cookie 的人可以离线猜密码，应改成随机生成并持久化的密钥；Access 的 JWKS 遇到未知 `kid`
    不立刻重取（轮换后最长一小时失效）；WebSocket 没有 Origin 校验（`SameSite=Lax` 挡跨站，挡不住同站的子域名）。
【→ 已处理（§8：新 vault N=2^18，旧 vault 可升级）；口令仍建议 20 位以上的随机串。】 14. 新建 vault 用 gocryptfs 默认的 scrypt 成本（2^16），而 vault 口令就是登录口令（目前 `.env` 里是 12 个字符）：
    新 vault 可以加 `-scryptn 18`，口令建议 20 位以上的随机串。
【→ 部分：加了测试（分发层、doctor、横幅、各纯函数）和每周的 fuzz 作业、smoke 门控、SBOM；没做：镜像签名、amd64 覆盖；Docker 不因 unhealthy 重启只文档化；桌面和 code-server 的日志不再写 `/dev/null`（§8）。】 15. 工程：CLI 的 30 个命令文件里 19 个没有测试模块（含 469 行的 `entrypoint.rs`，只靠 smoke 覆盖）；fuzz 目标不在 CI；CI 从未在 GitHub 上跑过；
    发布流水线没有 smoke 门控、没有 SBOM 和签名；没有任何 amd64 覆盖；Docker 的重启策略不看健康状态（unhealthy 不会被重启）；
    code-server 和桌面的日志写到 `/dev/null`。

## 验证状态

### 2026-11-03：在真机 Apple Silicon Mac 上跑 smoke（Colima + macOS Virtualization.Framework，不是 QEMU）

环境：Apple M5（32 GiB）、macOS 27.0.1，专用 Colima profile（6 CPU / 12 GiB，vz + virtiofs，Ubuntu 24.04 内核，
AppArmor 开启），`linux/arm64` 的 Kali 镜像、默认 KasmVNC 栈；真实 Chrome 来自宿主机。从零构建约 15 分钟。

最终镜像上的一次完整运行（`src/tests/smoke/run.sh --image dev-workspace:kali --services`）：**367 通过、0 失败、2 跳过**
（两个跳过是同一件事：这个网络直连不到 `https://www.google.com`，启动探测按设计报 degraded，“Clash 路径通”的断言单独通过）。
各组：10-boot 21 · 20-auth 38 · 25-audit 31 · 30-sync 4 · 40-vault 43 · 45-plugins 64 · 50-backup 23 · 55-browser 17 ·
60-replication 27 · 65-secrets 12 · 70-egress-variants 15 · 80-destruct 36（另有之后新增的 72-egress-enforce 9，单独跑过）· services（S3 14 · git 远端 12 ·
Shadowsocks 6 · croc 4）。

这一轮改了什么才跑到全绿（第一次完整运行是 349 / 6 失败，都不是同一个原因）：

- 网关之后一直过期的断言：`20-auth.sh` 重写（期望 302 到登录页、会话 cookie、KasmVNC 自带的 Basic 认证在网关之后），
  `40-vault.sh`（`sync_state` 改成在容器内直连 sync-service 的回环口，不再每次轮询都登录）、`sync_e2e.ts`、浏览器检查改为先登录。
- `10-boot.sh` 的 `backup-watcher is running` 与它自己 5 秒的 `startsecs` 抢跑（容器在 Caddy 一起来就 healthy）：改为等 STARTING 稳定。
- `55-browser` 的 KasmVNC 标题断言写死 “KasmVNC”，连上会话后实际是 “x11 - KasmVNC”：两种都接受。
- **产品里的不一致**（`70-egress-variants` 的 “doctor warns about it” 暴露的）：开机探测的是 `https://www.google.com`，
  `doctor` 的实时探测却是 `http://www.gstatic.com/generate_204`，很多网络不经代理就能直连到后者——订阅里没有节点时开机报 degraded、
  `doctor` 却报 confirmed。现在 `doctor` 和新的 egress-monitor 与开机用同一个目标（HEAD 请求，每分钟一次也不费流量）。
- `20-git-remotes` 的 SSH 四项：测试用的小镜像在 Alpine 上 `apk add` 偶发失败（`docker build -q` 还把原因吞了）：辅助镜像构建加三次重试，
  最后一次打印原因。

新增 `25-audit.sh`（审计日志、登录限速、Caddy / Clash 的运行用户与环境、自毁调用的记录）和 `65-secrets.sh`（文件型密钥：环境里没有口令
和订阅 URL、用文件里的口令建 vault 并登录、读不了的文件让容器拒绝启动）。


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

- **Docker Desktop（macOS）本身和真实的 Linux 主机**：Apple Silicon Mac 上的 Colima（vz，真机）已在 2026-11-03 全量跑过（见上）；Docker Desktop 没跑，Linux 主机没跑。
- **amd64**：见上，只验证了下载校验和二进制的架构，没有在 amd64 上构建或运行整个镜像。GitHub 的 amd64 runner 上跑 `smoke.yml` 会是第一次。
- **真实外部服务**：Cloudflare **Access 和命名隧道**（只测了快速隧道，它没有 Access 策略）；真实 S3 / R2（用的是 `rclone serve s3`，区域、签名、虚拟主机风格这些 R2 / AWS 的细节没测）；真实 GitHub（本地 sshd 和 git HTTP 替身）；真实机场订阅和节点（本地 Shadowsocks 节点，订阅里常见的 vmess / trojan / hysteria 等协议没测）。
- **GitHub 上的 CI**：三个 workflow 都通过了 actionlint，没有真正跑过（按你的决定，推送留到以后）。
- **Tor 的完整引导**：tor 能起，经 Clash 连上中继，引导到 30% 以上，没有等到 100%（这个网络下 Tor 能不能通未知）。
- **pentest 的 Kali 路径**：真实运行中 Kali 重定向这次取不到 Release，走的是 Debian bookworm 回退；第一次探测时 Kali 路径取到了 Release 并下载了索引，但我中断了，所以没有完整建成过一个 Kali rootfs。
- **macOS 上真实的 `host-watcher.sh --install` / `destroy.sh --paranoid`**：只验证了行为（见上），没有跑真实脚本。
- **在已装好的旧 LaunchAgent / systemd 单元上**：修复在脚本里，需要重启监视器（见 `deploy/linux-systemd/README.md`）。

## 已知缺口

### 安全与可靠性

- **出口防火墙默认开启**（2026-11-04，`deploy/docs/threat-model.md` 的 Egress；这一条原来写的是「尽力而为、没有强制」）。局限：持有 `NET_ADMIN` 的容器 root 能删表（`dev` 不能）；去往白名单网络的流量带着真实 IP；经 Clash 的目的地只和代理节点一样私密；`EGRESS_ENFORCE=0` 时回到只有代理变量。
- **主密码**：启动横幅不再打印它（2026-10-31）——只有在 PASSWORD 留空、容器自己生成口令时才打印（否则无从得知）；来自 `.env` 的口令永不进日志。它不再写进 code-server 的任何文件（code-server 以 `--auth none` 运行，登录只有网关一道；生成的口令存 root 才能读的 `/var/lib/sdw/master-password`）。vault 口令仍然就是它，见 threat-model 的 Non-Guarantees。把它变成内存驻留 + 硬件信任根的方案见 `deploy/docs/design-memory-only-secrets.md`（设计，未本地 PoC——这台机器没有容器可见的 TPM2；验证脚本 `src/tests/smoke/manual/tpm-seal-verify.sh` 在真实 TPM / CI 上才跑）。
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
- [ ] 测试 Oracle Cloud ARM 实例的 FUSE 兼容性 — **需要一个 Oracle 账号和实例，这里做不了**。在新实例上跑 `make smoke-build`（或 `src/tests/smoke/run.sh --image …`）即是这项的验收；`40-vault.sh` / `45-plugins.sh` 里的 FUSE 与 `allow_other` 检查最先会暴露不兼容。
- [x] 评估 Hetzner CAX 系列（ARM）vs CPX 系列（x86）性价比（2026-11-03，官方价目页，不含 VAT；
      [cost-optimized](https://www.hetzner.com/cloud/cost-optimized/)、[regular performance](https://www.hetzner.com/cloud/regular-performance/)）。
      同为 4 vCPU / 8 GB：**CAX21（Ampere ARM，80 GB NVMe）€10.99/月**，CPX32（AMD，160 GB NVMe）€35.99/月（欧洲机房）；
      美国机房的 CPX31 €62.99/月。ARM 约为三分之一的价钱，而镜像本来就是 `linux/arm64` + `linux/amd64` 双架构、arm64 是
      这次实际构建和跑 smoke 的架构，所以首选 CAX。但**抓取时这几档在价目页上都标着「暂不可订」**，今年 4 月和 6 月 15 日
      各涨过一次价（第三方统计 CPX22 从 €5.99 涨到约 €19.5–20），价格和可订状态都要在下单时重看；下单前也要先确认 FUSE
      （上一条）。CAX 是共享 vCPU，持续高负载（桌面 + 编译）的表现没有测过。
- [x] 考虑多实例同步策略（多台机器共用一个 vault repo）— 结论：**同一时刻只允许一个写入方**（主/备，不是多主）。
      原因：gocryptfs 密文目录里每个目录有自己的 `gocryptfs.diriv`，文件名由「父目录的 diriv + 明文名」确定性地加密。
      两台机器各自在同一个明文位置新建同名目录，会得到同一个密文目录名，里面却是两份**不同**的 `gocryptfs.diriv`；
      git 把它们当作 add/add 冲突，随便选一边，另一边写进去的文件名就再也解不出来（文件内容没丢，名字丢了）。
      同一个文件两边都改则是整块密文的二进制冲突。只改不同的已有文件时 git 合并没问题，但无法保证。
      做法：备机平时 `git pull --ff-only` 保持追平但不解锁、不 `vault-sync`；切换时在主机 `lock-vault` + 最后一次
      `vault-sync`，备机再拉取、解锁。没有写这个流程的脚本。`vault-sync` 对第二个写入方只处理了简单的情形：推送被拒时做一次
      `git pull --rebase` 再推（`infra/src/replication.rs` 的 `push_to_origin`）；**冲突它不处理**——rebase 失败就返回 false，
      不会 `git rebase --abort`，密文仓库留在 rebase 中途；之后几轮会怎样没有测过（很可能一直失败，直到有人手动处理）。
      真要跑两个实例，先补上 abort 和一条点明冲突的通知，并用两个实例造一次 diriv 冲突实测。
