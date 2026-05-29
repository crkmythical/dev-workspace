# TODO

## 多平台部署（待实施）

当前部署在家用 Mac Mini，后续考虑扩展部署位置以提升安全性和隐私性。

### 候选方案

| 优先级 | 方案 | 特点 | 备注 |
|--------|------|------|------|
| 高 | Hetzner VPS (德国/芬兰) | GDPR 保护、KVM、便宜 | 安全+隐私最大化 |
| 高 | Oracle Cloud Free Tier | ARM A1 4C/24G 永久免费 | 需 Dockerfile 支持 linux/arm64 |
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

- [ ] Dockerfile 支持 multi-arch（`linux/amd64` + `linux/arm64`）
- [ ] 测试 Oracle Cloud ARM 实例的 FUSE 兼容性
- [ ] 评估 Hetzner CAX 系列（ARM）vs CPX 系列（x86）性价比
- [ ] 编写部署脚本适配不同环境（systemd 替代 LaunchAgent）
- [ ] 考虑多实例同步策略（多台机器共用一个 vault repo）

## 远程桌面 — KasmVNC PoC（待验证）

浏览器内运行完整 Linux GUI 桌面。Spec 已创建: `.kiro/specs/remote-desktop/`

### 下一步: PoC 测试

- [ ] 构建 `image/Dockerfile.desktop-poc` 验证 KasmVNC 在 Docker Desktop macOS 下的可用性
- [ ] 验证 llvmpipe OpenGL 渲染可用 (`glxinfo`)
- [ ] 测量空闲桌面内存基线 (target < 250MB)
- [ ] 主观评估帧率 (target ≥ 25fps through Caddy)
- [ ] 确认中文字体渲染正常
- [ ] PoC 通过后进入正式实现

### 技术选型

- KasmVNC (WebSocket native, WebP 自适应编码, 30fps)
- Xvfb + Openbox + tint2 (轻量桌面)
- CPU 软渲染 llvmpipe (Mac Mini Docker 无 GPU passthrough)
- 按需启动 (supervisord autostart=false)

## 容器内自毁 — Self-Destruct（待实施）

紧急数据销毁能力。Spec 已创建: `.kiro/specs/self-destruct/`

- [ ] 实现 `destroyCore()` 核心逻辑
- [ ] 实现 `self-destruct` CLI 命令
- [ ] 实现远程触发 API (`POST /sync/api/destruct`)
- [ ] 实现 host-watcher 宿主机联动
- [ ] SPA 紧急面板 (`#emergency`)
