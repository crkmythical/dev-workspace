# TODO

## 已知缺口

重构（阶段 0–6）时发现、但没有处理或无法在开发机上验证的事项。

### 没有在真实 Docker 里验证过

开发机没有 Docker，下面这些只做了类型检查、单元测试和 `bash -n`：

- 容器启动流程（`entrypoint-main.ts` 的各步骤）、解锁钩子、supervisord 联动
- Dockerfile 的改动（版本固定、`install-bins.ts`、`DESKTOP_STACK` 默认值）和 `build-image.yml` 的变体矩阵
- `deploy/linux-systemd/` 的 systemd 单元
- `backup-watcher.ts` 的 I/O 部分（状态机本身有回放测试）

**建议**：CI 里加一个 smoke job：构建默认镜像，`docker run` 起来，检查 `doctor` 与横幅里的各项状态。

### 安全与可靠性

- **出口是尽力而为，没有强制**（`docs/threat-model.md`）。要强制需要独立的防火墙层；容器内进程都是 root，按 uid 的规则无效，需要基于 mark 的方案。
- **优雅停机**：`entrypoint-main.ts` 不把 SIGTERM 转给 supervisord，`docker stop` 实际等于超时后 SIGKILL。
- **主密码会进容器日志**（启动横幅）和 code-server 的 `config.yaml`，且 vault 口令就是它，见 threat-model 的 Non-Guarantees。
- **重放窗口不跨重启**：服务重启后，5 分钟容差内已见过的 nonce 可以被重放。
- **远程自毁路由没有可注入依赖**，成功路径（会杀 PID 1）无法测试。把 `createDestroyDeps` 通过 `createApp` 注入即可。

### 清理

- Selkies 静态客户端（约 6 MB）被复制进所有镜像，包括只含 KasmVNC 的默认镜像；可移到构建阶段按需复制。
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
- [ ] ≥ 8GB RAM
- [ ] 能运行 cloudflared
- [ ] 出站 443 可达（Clash + Cloudflare Tunnel）

### 待办事项

- [x] Dockerfile 支持 multi-arch（CI 构建 `linux/amd64` + `linux/arm64`）
- [x] 宿主机脚本适配 Linux（`deploy/linux-systemd/`，未在真实主机上验证）
- [ ] 测试 Oracle Cloud ARM 实例的 FUSE 兼容性
- [ ] 评估 Hetzner CAX 系列（ARM）vs CPX 系列（x86）性价比
- [ ] 考虑多实例同步策略（多台机器共用一个 vault repo）
