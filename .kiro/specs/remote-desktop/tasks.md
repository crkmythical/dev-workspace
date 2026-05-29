# Implementation Plan: remote-desktop

## Overview

为 dev-workspace 添加浏览器可访问的 KasmVNC 远程桌面。按依赖顺序：PoC 验证 → Dockerfile 改造 → supervisord 配置 → CLI 命令 → Caddy 路由 → 现有系统集成 → desktop-install 工具 → 文档。

核心约束：桌面环境按需启动（`autostart=false`），所有配置加密存储，与现有安全模型无缝集成。

## Task Dependency Graph

```json
{
  "waves": [
    {"tasks": ["1.1"]},
    {"tasks": ["2.1", "2.2"]},
    {"tasks": ["2.3", "2.4", "3.1"]},
    {"tasks": ["3.2", "3.3", "3.4"]},
    {"tasks": ["4.1", "4.2"]},
    {"tasks": ["5.1", "5.2"]},
    {"tasks": ["6.1", "6.2"]}
  ]
}
```

## Tasks

- [ ] 1. PoC 验证
  - [ ] 1.1 构建 PoC 验证 KasmVNC 在 Docker Desktop macOS 下的可用性
    - 创建 `image/Dockerfile.desktop-poc`：基于 debian:bookworm-slim，安装 Xvfb + KasmVNC + Openbox + mesa-utils
    - 创建 `docker-compose.desktop-poc.yml`：映射 6080 端口
    - 验证项:
      - `glxinfo | grep "OpenGL renderer"` 应显示 "llvmpipe"
      - KasmVNC web UI 可访问（localhost:6080）
      - 帧率测试：运行 `glxgears` 或拖动窗口，主观评估流畅度
      - 内存基线：`docker stats` 观察空闲桌面内存 (target < 250MB)
      - 中文字体渲染：打开 xterm，输入中文，确认正常显示
    - PoC 通过后删除临时文件，进入正式实现
    - _Requirements: 1.1, 1.2, 1.3, 1.6, 2.1_

- [ ] 2. Dockerfile 与基础设施
  - [ ] 2.1 在主 Dockerfile 中添加桌面层
    - 新增 apt 包（在现有 core layer 之后，作为独立 RUN layer 方便缓存）:
      ```
      xvfb, openbox, tint2, xterm, pcmanfm, dbus-x11,
      libgl1-mesa-dri, mesa-utils, libglib2.0-0,
      fonts-noto-cjk, fonts-dejavu-core,
      xdg-utils, x11-utils, x11-xserver-utils
      ```
    - 安装 KasmVNC (从 GitHub releases 下载 .deb, pinned version)
    - 创建 `/opt/desktop-apps/` 目录和 `.registry.json` 初始文件
    - 创建 `/tmp/.desktop-cache/` 目录
    - 预创建默认配置: `image/etc/desktop/` 目录包含 openbox/rc.xml, tint2rc 默认配置模板
    - Image 大小增加预估: ~400-500MB (fonts + mesa 是大头)
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 5.4_
  - [ ] 2.2 添加 KasmVNC 配置模板
    - `image/etc/desktop/kasmvnc-defaults.yaml`: 默认 KasmVNC 配置
      - websocket_port: 6080
      - interface: 127.0.0.1
      - security_types: None (CF Access handles auth)
      - frame_rate: 30
      - dynamic_quality_min: 4
      - dynamic_quality_max: 9
      - webp_compression: 8
      - idle_timeout: 0
      - allow_resize: true
    - `image/etc/desktop/openbox-rc.xml`: Openbox 默认配置 (键盘快捷键、窗口行为)
    - `image/etc/desktop/tint2rc`: tint2 面板配置 (底部任务栏、系统托盘、时钟)
    - _Requirements: 2.1, 2.2, 2.3, 2.6, 2.7, 2.8, 2.10_
  - [ ] 2.3 添加 supervisord desktop group
    - 在 `image/supervisord.conf` 中新增 `[group:desktop]` 和四个 program（xvfb, openbox, tint2, kasmvnc）
    - 所有设为 `autostart=false`
    - 环境变量: DISPLAY=:1, HOME=/workspace/.desktop, XDG_* 全套
    - xvfb 使用 `%(ENV_DESKTOP_RESOLUTION)s` 支持运行时配置
    - 启动顺序: xvfb → (openbox, tint2, kasmvnc) 并行
    - _Requirements: 1.7, 4.1, 4.2_
  - [ ] 2.4 修改 Caddy 配置
    - 在 `image/etc/caddy/Caddyfile` 中 `/sync/*` handle 之前添加:
      ```
      handle /desktop/* {
          uri strip_prefix /desktop
          reverse_proxy 127.0.0.1:6080
      }
      ```
    - 确认 WebSocket 升级透传（Caddy 默认支持）
    - _Requirements: 1.8, 6.3_

- [ ] 3. CLI 命令
  - [ ] 3.1 实现 `packages/cli/src/desktop-start.ts`
    - 检查 vault 是否已挂载 → 否则报错退出
    - 检查 desktop 是否已运行 → 是则打印 URL 并退出
    - 初始化桌面配置 (`initDesktopConfig()`):
      - 创建 `/workspace/.desktop/.config/openbox/`, `/workspace/.desktop/.config/tint2/`, `/workspace/.desktop/.config/kasmvnc/`
      - 如果配置文件不存在，从 `/opt/workspace/image/etc/desktop/` 复制默认模板
      - 创建 `/tmp/.desktop-cache/`
    - `supervisorctl start desktop:*`
    - 等待 6080 端口就绪 (轮询, timeout 30s)
    - 输出: "Desktop ready. Access at: /desktop/"
    - _Requirements: 4.1, 3.1, 3.2, 3.5, 1.7_
  - [ ] 3.2 实现 `packages/cli/src/desktop-stop.ts`
    - `supervisorctl stop desktop:*`
    - 确认所有 desktop 进程已退出
    - 输出: "Desktop stopped."
    - _Requirements: 4.1_
  - [ ] 3.3 实现 `packages/cli/src/desktop-install.ts`
    - 解析 argv: 应用名称 (firefox, chromium, idea, burpsuite, wireshark)
    - 检查 Clash 代理可用（network access needed for downloads）
    - 按 APPS 注册表执行安装:
      - apt 类: `apt-get install -y <package>`
      - 下载类: curl → extract to /opt/desktop-apps/<name>/
    - 创建 .desktop 文件到 `/workspace/.desktop/.local/share/applications/`
    - 更新 `/opt/desktop-apps/.registry.json`
    - 输出: "Installed <app>. Launch from desktop menu or: <command>"
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6_
  - [ ] 3.4 注册 CLI 命令到 Dockerfile
    - 在 `image/Dockerfile` 的 symlink 循环中添加: `desktop-start`, `desktop-stop`, `desktop-install`
    - _Requirements: 4.1, 5.3_

- [ ] 4. 现有系统集成
  - [ ] 4.1 修改 lock-vault.ts — 停止桌面再卸载
    - 在 fusermount 调用之前新增:
      ```typescript
      if (await isDesktopRunning()) {
        console.log("Stopping desktop session...");
        await $`supervisorctl stop desktop:*`.quiet().nothrow();
      }
      ```
    - 新增 helper `isDesktopRunning()` in `lib/vault.ts` (检查 supervisorctl status)
    - _Requirements: 6.7, 3.4_
  - [ ] 4.2 修改 destroyCore() — Phase 1 添加桌面停止
    - 在 `packages/cli/src/lib/destroy.ts` Phase 1 (unmount) 之前:
      ```typescript
      await $`supervisorctl stop desktop:*`.quiet().nothrow();
      ```
    - _Requirements: 3.6_
  - [ ] 4.3 修改 pentest-enter.ts — X11 socket 共享
    - 在 enter script 的 mount 列表中添加:
      ```bash
      if [ -e /tmp/.X11-unix/X1 ]; then
        mkdir -p ${PENTEST_MOUNT}/tmp/.X11-unix
        mount --bind /tmp/.X11-unix ${PENTEST_MOUNT}/tmp/.X11-unix
      fi
      ```
    - 在 chroot 环境中设置 `export DISPLAY=:1`
    - _Requirements: 6.5_
  - [ ] 4.4 修改 doctor.ts — 添加桌面健康检查
    - 新增检查项:
      - Desktop Xvfb: 检查 `supervisorctl status desktop:xvfb` 是否 RUNNING (如果 group 已启动)
      - Desktop KasmVNC: 检查 6080 端口是否监听
      - Desktop Display: `DISPLAY=:1 xdpyinfo` 是否成功
    - 如果 desktop group 未启动，显示 "Desktop: not started (optional)" 而非 ✗
    - _Requirements: 4.6_

- [ ] 5. 环境变量与配置
  - [ ] 5.1 新增 constants
    - 在 `packages/core/src/constants.ts` 中添加:
      - `DESKTOP_HOME = "/workspace/.desktop"`
      - `DESKTOP_CONFIG_DIR = "/workspace/.desktop/.config"`
      - `DESKTOP_CACHE_DIR = "/tmp/.desktop-cache"`
      - `DESKTOP_APPS_DIR = "/opt/desktop-apps"`
      - `DESKTOP_APPS_REGISTRY = "/opt/desktop-apps/.registry.json"`
      - `KASMVNC_PORT = 6080`
      - `DESKTOP_DEFAULT_RESOLUTION = "1920x1080"`
    - _Requirements: 1.1, 4.5_
  - [ ] 5.2 更新 .env.example
    - 添加: `DESKTOP_RESOLUTION=1920x1080  # Optional: resolution for remote desktop (WxH)`
    - _Requirements: 4.5_

- [ ] 6. 文档与验证
  - [ ] 6.1 更新文档
    - `README.md`: 添加 "远程桌面" 章节，含启动/停止/安装应用的命令
    - `docs/runbook.md`: 添加桌面操作章节 (启动/停止/安装 GUI 应用/性能调优)
    - `docs/architecture.md`: 更新系统拓扑图，添加 KasmVNC 组件
  - [ ] 6.2 端到端验证
    - 验证清单:
      1. `unlock-vault` → `desktop-start` → 浏览器访问 `/desktop/` → 看到桌面
      2. 桌面内右键菜单可用，终端可打开
      3. `desktop-install firefox` → Firefox 出现在菜单中 → 启动可上网 (通过 Clash)
      4. `lock-vault` → 桌面被停止 → vault 正常锁定
      5. `unlock-vault` → `desktop-start` → 之前的配置恢复 (Firefox 书签等)
      6. `self-destruct --force --skip-shred` → 桌面被停止 → 正常销毁
      7. 通过 Cloudflare Tunnel 访问 → 延迟可接受 (< 120ms)
      8. `pentest` 环境内 `DISPLAY=:1 xterm` → 窗口显示在宿主桌面上

## Notes

- KasmVNC 版本选择: 使用最新 stable release，从 https://github.com/kasmtech/KasmVNC/releases 获取 .deb
- 如果 PoC 验证帧率不满足 (< 20fps)，备选方案: 降低默认分辨率到 1600x900 或调低 WebP 质量
- vault-sync 会同步 `/workspace/.desktop/` 到 GitHub — 如果用户安装了大量 IDEA 索引等文件，可能需要在 vault 的 `.gitignore` 中排除特定子目录
- 未来增强方向: 音频支持 (PulseAudio → WebSocket)、多显示器模拟、GPU passthrough (迁移到 Linux 物理机后)
