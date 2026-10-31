# Smoke test

单元测试看不到的东西，只有把真镜像跑起来才看得到：supervisord 的真实行为、FUSE 挂载、真实的 git / ssh / S3 / Shadowsocks、真实的 Chrome。这里的 smoke test 就是把镜像和一个假上游一起起来，然后从外面（HTTP、`docker exec`、浏览器）逐项检查。

```bash
tests/smoke/run.sh --build                   # 用 image/Dockerfile 构建（默认生产基础 kalilinux/kali-rolling）再测
tests/smoke/run.sh --image dev-workspace:latest --services
tests/smoke/run.sh --image dev-workspace:latest --reuse --only '^50'   # 只重跑一个检查，复用已起好的实例
```

## 需要什么

- Docker 和 Compose **≥ 2.24**（用到 `env_file: !override`）
- 约 4 GB 内存（主实例约 1 GB；`70`、`80`、`services/` 会再临时起一个实例）
- Linux 宿主机需要 `/dev/fuse`；开了 AppArmor 的发行版自动叠加 `docker-compose.apparmor.yml`（`SMOKE_APPARMOR=yes|no|auto`）
- 浏览器检查（`checks/55-browser.sh`）需要 `node` 和 Chrome / Chromium（找不到就整项跳过）；Playwright 只用它的 JS 包，不下载浏览器

## 它不会碰什么

每个实例是独立的 compose 项目（容器、网络、卷、端口都带 `sdw-smoke` 前缀），**不读仓库的 `.env`**，用 `smoke.env` 里的占位值。所以可以放心在已经有真实工作区的机器上跑：端口 18090–18096、网络 `172.30.77.0/24`–`172.30.83.0/24` 被占用时用 `--port`、`--net`、`--project` 改。退出时按项目名清理全部容器、卷、网络（`--keep` / `--reuse` 除外）。

## 结构

| 文件 | 做什么 |
|------|--------|
| `run.sh` | 入口：预检、（可选）构建、起主实例、依次跑 `checks/*.sh`，`--services` 再跑 `services/*.sh`，汇总 |
| `lib.sh`, `stack.sh` | 断言、`docker exec` 包装、起停实例；`instance` 临时切到另一个实例（另一个项目、端口、网络） |
| `docker-compose.smoke.yml` | 叠在仓库的 `docker-compose.yml` 上：换镜像、换 `env_file`、独立名字和端口、加假上游 |
| `fixtures/fake_upstream.py` | 假的 Clash 订阅 + 假的 HTTP 代理节点（把 CONNECT 转给真实互联网并记录），可选再列一个 Shadowsocks 节点 |
| `fixtures/sync_e2e.ts` | 真实的同步协议客户端（core）对真实服务，在容器里跑 |
| `fixtures/stubs/` | `debootstrap`、`apt-get`、`curl` 的替身（两个几百 MB 的下载和 2 GB 的 rootfs），`45-plugins.sh` 用 |
| `fixtures/git_http_server.py` | 像 GitHub 那样要 token 的 git smart-HTTP 服务 |
| `browser/` | Playwright + 系统 Chrome：同步页、紧急页、`/vnc/` 和 `/desktop/` 的画面与输入 |
| `manual/` | 手动跑的验证脚本（Cloudflare 快速隧道、另一种 CPU 架构），见下 |

| 检查 | 内容 |
|------|------|
| `10-boot` | 启动日志、healthy、横幅、出口、各服务在跑、`doctor` |
| `20-auth` | code-server 登录、`/vnc/`（`-full` 还有 `/desktop/`）的认证与 WebSocket 升级、同步页、自毁端点对错误口令一律 404 |
| `30-sync` | 同步协议全流程（分块、finalize、带认证的列表、重放 / 伪造 / 过期 / 路径穿越） |
| `40-vault` | 磁盘上没有明文、被占用时 `lock-vault` 的行为（拒绝后恢复被停掉的服务）、干净锁定、错口令、强制锁定、重建容器后数据还在 |
| `45-plugins` | `desktop-install`、pentest 插件（init / unlock / bootstrap 含 Kali 失败回退 / `pentest` 进入 / lock） |
| `50-backup` | restic 监视器：基线、防抖、被排除的路径、恢复、仓库消失 3 次后写通知、恢复后清通知 |
| `55-browser` | 真实 Chrome：同步页上传、紧急页、桌面画面和输入 |
| `60-replication` | `vault-sync` 按 cron 的方式跑（无容器环境变量）：推送、无变化不提交、远端不可用 3 次后写通知、恢复后补推 |
| `70-egress-variants` | 出口 degraded（订阅无节点）、down（Clash 起不来）、缺必填变量时拒绝启动 |
| `80-destruct` | 自毁两条路（HTTP 端点含限流和伪造头、CLI 从 vault 内运行），之后容器 inert、密钥和密文都没了 |
| `services/10-s3-backup` | restic 对 S3 端点（`rclone serve s3`）：启动建库、监视器写快照、恢复、IP / 主机名两种写法的代理路径 |
| `services/20-git-remotes` | 无人值守的 `vault-sync` 对真实 sshd（首次信任并固定主机密钥，之后变了就拒绝）和 token-in-URL 的 HTTP git |
| `services/30-shadowsocks` | 订阅里列一个真实的 Shadowsocks 节点：Clash 加载、url-test 选中、请求真的经它上网 |

## 手动做的验证（不在默认运行里）

这些要么依赖本机的网络出口，要么太慢，所以不放进 `run.sh`。先 `tests/smoke/run.sh --image <镜像> --keep --only '^10'` 起一个保留的实例（容器名 `sdw-smoke-workspace`，端口 18090）。

- `manual/tunnel.sh`：把这个一次性实例通过 Cloudflare 快速隧道（trycloudflare.com）暴露出去，经边缘测认证、WebSocket、自毁限流（伪造 `X-Forwarded-For` / `CF-Connecting-IP` 买不到额度），再让真实 Chrome 经 https 跑一遍浏览器检查。需要 `cloudflared`；`HOME` 指向临时目录，不会读你自己的 `~/.cloudflared`。
- `manual/emulated.sh [linux/amd64]`：另一种 CPU 架构，经 qemu-user。从真实的 Dockerfile 里原样取出下载并校验的那几段（Bun + sha256、Caddy + sha512、restic、Clash），在目标平台上构建，装上仓库的包（冻结的 lockfile）并跑 core 和 cli 的单元测试。其余的 apt 和 mise 安装与架构无关，在模拟下要跑几个小时，不做。
- 真实的 `pentest-bootstrap`（`stubs/` 里的 `debootstrap` 只是替身）：`docker exec sdw-smoke-workspace init-pentest && docker exec sdw-smoke-workspace unlock-pentest && docker exec sdw-smoke-workspace pentest-bootstrap`。约 300 MB 下载，软件模拟下要 35 分钟。然后 `printf '%s\n' 'nmap --version' 'service tor start' 'exit' | docker exec -i sdw-smoke-workspace pentest`。官方 `http.kali.org` 重定向偶尔取不到 Release，这时会按设计回退到 Debian bookworm。tor 起不来多半是 vault 挂载缺 `allow_other`（见 `45-plugins`）。
- 真实的 `desktop-install`：`docker exec sdw-smoke-workspace desktop-install chromium`（apt 经 Clash 联网），再 `docker exec -d -e DISPLAY=:2 -e HOME=/workspace/.desktop-vnc sdw-smoke-workspace chromium --no-sandbox`，在 `/vnc/` 里能看到窗口。

## 它证明什么，不证明什么

证明：镜像按预期起来、运行起来的行为和文档一致（见上表），Chrome 里能看到桌面画面并且输入能到达会话。

不证明：真实的 Cloudflare Tunnel / Access、R2 / AWS S3、GitHub、机场节点（这里用的是等价的本地替身：`rclone serve s3`、本地 sshd 和 git 服务、本地 Shadowsocks）；真实的 Docker Desktop（在另一台机器上跑一遍 `run.sh` 即可）。

## 加一个检查

在 `checks/` 放一个 `NN-name.sh`：`source ../lib.sh`，用 `step` 分段，`expect_eq / expect_match / expect_true / expect_code`，最后调用 `finish`（它打印 `RESULT passed= failed= skipped=`，`run.sh` 靠这行汇总）。需要另起实例的检查用 `instance`（见 `70-egress-variants.sh`），每个实例用自己的网段，因为 Clash 用自己的 DNS，假上游必须有固定地址。文件名不要叫 `*.test.ts` / `*.spec.ts`，否则根目录的 `bun test` 会去执行它。
