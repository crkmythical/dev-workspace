# Requirements Document

## Introduction

为 dev-workspace 容器添加完整的 Linux 桌面环境，通过浏览器访问。支持运行任意 GUI 应用（JetBrains IDE、Burp Suite、Firefox、Wireshark 等），用于重度远程开发和渗透测试场景。使用 KasmVNC 作为传输层，提供低延迟、自适应帧率的远程桌面体验。

技术选型依据：
- **KasmVNC** 而非 noVNC+TigerVNC：WebSocket native 协议、WebP 自适应编码、30-60fps、帧差分传输、剪贴板/文件双向同步
- **CPU 软渲染 (llvmpipe)** 而非 GPU：Docker Desktop on macOS 无 GPU passthrough，但多核 CPU 下 GUI 应用（IDE、工具）性能完全够用
- **轻量窗口管理器 (openbox)** 而非完整 DE：减少资源消耗，仅提供窗口管理基础能力

## Glossary

- **KasmVNC**: 基于 TigerVNC 深度定制的 VNC 服务器，专为浏览器访问优化，支持 WebSocket 原生协议和 WebP/JPEG 自适应编码
- **Xvfb**: X Virtual Framebuffer，无硬件显示的虚拟 X Server，提供内存中的帧缓冲
- **Openbox**: 轻量级 X11 窗口管理器（~2MB 内存），提供窗口拖拽、最大化、任务切换等基础功能
- **llvmpipe**: Mesa 的 CPU 软件 OpenGL 渲染器，无需 GPU 即可运行需要 OpenGL 的应用
- **Desktop_Home**: GUI 应用的 HOME 目录 `/workspace/.desktop/`，位于 gocryptfs 加密层内，锁定后所有配置变为密文

## Requirements

### Requirement 1: 桌面环境基础设施

**User Story:** 作为开发者，我希望容器内有一个可用的 Linux 桌面环境，这样我可以运行任何需要 GUI 的应用程序。

#### Acceptance Criteria

1. THE container SHALL include a virtual X server (Xvfb) running on display `:1` with default resolution 1920x1080x24
2. THE container SHALL include Openbox window manager providing basic window management (move, resize, maximize, minimize, virtual desktops)
3. THE container SHALL include KasmVNC server binding to `127.0.0.1:6080` (WebSocket), connecting to the Xvfb display
4. THE container SHALL include essential desktop utilities: a terminal emulator (xterm or xfce4-terminal), a file manager (pcmanfm or thunar), and a taskbar/panel (tint2)
5. THE container SHALL include CJK font support (fonts-noto-cjk) to correctly render Chinese/Japanese/Korean text in GUI applications
6. THE container SHALL include mesa-utils and libgl1-mesa-dri for CPU-based OpenGL rendering (llvmpipe), enabling applications that require OpenGL (e.g., some JetBrains IDEs)
7. THE desktop environment SHALL start automatically via supervisord after vault unlock (GUI applications need access to `/workspace/.desktop/` for configs)
8. THE desktop environment SHALL be accessible via Caddy reverse proxy at path `/desktop/*`, sharing the same Cloudflare Tunnel as code-server and sync-service

### Requirement 2: KasmVNC 配置与优化

**User Story:** 作为开发者，我希望远程桌面有流畅的操作体验，即使通过 Cloudflare Tunnel 访问也能舒适地进行重度操作（写代码、用工具）。

#### Acceptance Criteria

1. KasmVNC SHALL use WebP encoding with dynamic quality adjustment (high quality on static content, lower on motion) to balance clarity and bandwidth
2. KasmVNC SHALL support dynamic resolution — automatically matching the client browser window size without requiring manual configuration
3. KasmVNC SHALL enable frame differencing — only transmit changed screen regions to minimize bandwidth usage
4. KasmVNC SHALL support bidirectional clipboard synchronization between browser and remote desktop
5. KasmVNC SHALL support file upload/download through the browser interface (drag-drop or UI button)
6. KasmVNC SHALL be configured for maximum frame rate of 30fps (balance between fluidity and CPU load on Mac Mini)
7. KasmVNC SHALL use `-websocketPort 6080` and NOT expose a traditional VNC port (5900) — browser-only access
8. KasmVNC SHALL be configured with idle timeout of 0 (never disconnect idle sessions — desktop should persist)
9. KasmVNC SHALL support multi-user view (optional) — allowing one additional read-only viewer for pair programming or demonstration
10. THE KasmVNC web interface SHALL be served without its own authentication (authentication is handled by Cloudflare Access at the tunnel layer)

### Requirement 3: 安全与加密集成

**User Story:** 作为开发者，我希望桌面环境的所有应用配置和数据都在 gocryptfs 加密保护下，锁定 vault 后磁盘上不留任何 GUI 相关的明文。

#### Acceptance Criteria

1. GUI applications SHALL use `/workspace/.desktop/` as their HOME directory (via XDG environment variables), storing all configs, caches, and data within the encrypted vault
2. THE environment SHALL set `XDG_CONFIG_HOME=/workspace/.desktop/.config`, `XDG_DATA_HOME=/workspace/.desktop/.local/share`, `XDG_STATE_HOME=/workspace/.desktop/.local/state`
3. THE environment SHALL set `XDG_CACHE_HOME=/tmp/.desktop-cache` — caches go to tmpfs (not encrypted, but ephemeral; disappears on container stop)
4. WHEN vault is locked, THE desktop session SHALL be terminated gracefully (kill X clients, stop KasmVNC) — preventing GUI applications from writing errors to non-encrypted locations
5. WHEN vault is unlocked, THE desktop environment SHALL automatically (re)start if it was previously running (detect prior desktop session state)
6. THE `self-destruct` command SHALL also terminate the desktop session as part of Phase 1 (before unmounting FUSE)
7. THE KasmVNC configuration file (containing any session settings) SHALL reside within `/workspace/.desktop/.config/kasmvnc/` so it's encrypted at rest

### Requirement 4: 资源管理与性能

**User Story:** 作为开发者，我希望桌面环境不会过度消耗资源，并且可以按需启用/停用。

#### Acceptance Criteria

1. THE desktop environment SHALL be optional — a CLI command `desktop-start` SHALL start it, `desktop-stop` SHALL stop it
2. WHEN the desktop is not running, IT SHALL consume zero CPU and near-zero memory (no Xvfb, no VNC process)
3. THE desktop environment idle state (openbox + tint2, no applications) SHALL consume < 250MB RAM
4. THE container's `mem_reservation` recommendation SHALL be updated from 8g to 10g in documentation (accounting for desktop + one heavy GUI app like IDEA)
5. THE desktop environment SHALL support configurable resolution via environment variable `DESKTOP_RESOLUTION` (default: `1920x1080`), allowing users to lower resolution for better performance on slow connections
6. THE `doctor` command SHALL include a desktop health check: X server running, KasmVNC listening, display accessible

### Requirement 5: 应用安装与管理

**User Story:** 作为开发者，我希望可以方便地在桌面环境中安装和使用 GUI 应用。

#### Acceptance Criteria

1. THE Dockerfile SHALL NOT pre-install heavy GUI applications (IDEA, Burp Suite, etc.) — these SHALL be installed by the user after deployment to keep the base image lean
2. THE container SHALL include a minimal application launcher/menu (right-click desktop or panel menu) showing installed applications
3. THE container SHALL include a `desktop-install` helper script that simplifies installation of common GUI applications with one command (e.g., `desktop-install idea`, `desktop-install burpsuite`, `desktop-install firefox`)
4. Applications installed via `desktop-install` SHALL store their binaries in `/opt/desktop-apps/` (not in the vault, to avoid inflating git-synced ciphertext) and their config/data in `/workspace/.desktop/`
5. THE `desktop-install` script SHALL support at minimum: `firefox`, `chromium`, `idea` (IntelliJ IDEA Community), `burpsuite`, `wireshark`
6. WHEN Clash proxy is available, ALL desktop applications SHALL inherit proxy settings via environment variables (http_proxy/https_proxy) — no per-app configuration needed

### Requirement 6: 与现有系统集成

**User Story:** 作为开发者，我希望桌面环境与现有的 code-server、sync-service、pentest 环境无缝集成。

#### Acceptance Criteria

1. THE desktop environment SHALL share the same `/workspace` mount with code-server — files edited in GUI apps are immediately visible in the Web IDE and vice versa
2. THE desktop environment SHALL share the same Clash proxy — all GUI application traffic routes through the encrypted egress
3. THE Caddy reverse proxy SHALL route `/desktop/*` to KasmVNC's web interface (port 6080), coexisting with existing routes (`/` → code-server, `/sync/*` → sync-service)
4. THE desktop environment SHALL be accessible from the same Cloudflare Tunnel domain — no additional DNS or tunnel configuration required
5. THE pentest chroot environment (`pentest-enter`) SHALL optionally support running GUI applications by sharing the host X display (via `DISPLAY=:1` and X11 socket passthrough into chroot)
6. THE vault-sync mechanism SHALL NOT be affected by desktop usage — `/workspace/.desktop/` should be included in vault sync unless explicitly excluded by user in `.gitignore`
7. WHEN `lock-vault` is executed, desktop services SHALL be stopped BEFORE the FUSE unmount (preventing I/O errors from GUI apps accessing disappeared files)
