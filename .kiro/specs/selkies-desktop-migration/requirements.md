# Requirements Document

## Introduction

This document specifies the requirements for replacing the KasmVNC-based remote desktop in the single-container secure dev workspace with the 2026 **Selkies** streaming stack (pure WebSocket + browser-side WebCodecs H.264, **not** WebRTC). The requirements are derived from the approved design document (`design.md`) and stay consistent with its six change points (CP1-CP6), its empirical PoC findings (P1-P9), and its enumerated open verification items.

The core value is a materially more fluid browser-based remote desktop while preserving the existing security model end-to-end: zero inbound ports, all traffic over Cloudflare Tunnel as HTTP/WS only, a single container, and a gocryptfs FUSE-backed home. The migration is deliberately surgical: Selkies attaches to an **external** Xvfb via `DISPLAY`, which lets the existing XFCE session, the `/workspace/.desktop` home, and `start-xfce.sh` remain unchanged.

Two user-confirmed program decisions frame these requirements:
1. KasmVNC remains available as a build-arg-gated fallback (`DESKTOP_STACK=selkies|kasmvnc`); it is not deleted outright until the open verification items close.
2. Implementation is performed on the git branch `feat/selkies-desktop-migration` (already created); this is a process constraint and not a runtime requirement.

Each requirement carries a **Traceability** line mapping it back to the design's change points and PoC findings.

## Glossary

- **Workspace**: The single secure dev container that hosts the desktop, code-server, and sync services.
- **Selkies_Stream**: The `desktop-selkies` supervisord program that runs the new-arch Selkies (module `selkies`, v0.0.0) as a **WebSocket-only** server (it does NOT serve the static web client — hitting its port with a browser returns HTTP 426), capturing the X framebuffer (via pixelflux) and streaming H.264 over WebSocket on port 6080 (`--port=6080`, chosen to avoid the code-server 8082 collision since Selkies' own default is 8082).
- **Static_Client**: The ~5.9 MB Selkies web client (HTML/JS/WASM) that is NOT part of the pip package and must be provisioned separately to `DESKTOP_WEB_ROOT` (`/usr/share/selkies/web`); it is served by Caddy, not by Selkies.
- **Front_Layer**: The component responsible for serving Static_Client, reverse-proxying the WebSocket, applying basic_auth, and mounting under `/desktop/`. Decided to be Caddy (no NGINX, no Hono).
- **Xvfb_Server**: The `desktop-xvfb` supervisord program that runs a standalone headless X server on display `:1`.
- **XFCE_Session**: The `desktop-xfce` supervisord program that runs the XFCE desktop session via `start-xfce.sh` on display `:1`.
- **Desktop_Control**: The single-source-of-truth control module `packages/cli/src/lib/desktop.ts` that defines the desktop process group, start/stop order, and readiness probes.
- **Caddy_Proxy**: The Caddy reverse proxy on `:8080` that performs `basic_auth` and routes requests to internal services.
- **Cloudflare_Tunnel**: The outbound tunnel that carries all external traffic to the Workspace as HTTP/WS only, with zero inbound ports.
- **Build_System**: The Docker image build defined by `image/Dockerfile`.
- **Entrypoint**: The container entrypoint logic `packages/cli/src/entrypoint-main.ts` that injects environment configuration.
- **Doctor**: The health-check command `packages/cli/src/doctor.ts`.
- **Desktop_Installer**: The GUI-app install command `packages/cli/src/desktop-install.ts`.
- **Vault_Lifecycle**: The callers `lock-vault`, `destroy`, and `desktop-stop` that depend on `stopDesktop()` before a gocryptfs unmount.
- **Core_Package**: The pure (no-IO) package `packages/core`, including `constants.ts`.
- **DESKTOP_BCRYPT_HASH**: The bcrypt hash injected at startup, used by Caddy basic_auth for both code-server and the desktop (shared password).
- **DESKTOP_STREAM_PORT**: The Selkies WebSocket listen port **6080** (Selkies' own default is 8082, which collides with code-server, so it runs on 6080).
- **DESKTOP_WEB_ROOT**: The container path `/usr/share/selkies/web` from which Caddy serves Static_Client.
- **CODE_SERVER_PORT**: The code-server port **8082** (which is why Selkies cannot use its own 8082 default).
- **DESKTOP_DISPLAY**: The shared X display `:1`.
- **DESKTOP_STACK**: The build argument selecting the desktop stack (`selkies` default, or `kasmvnc` fallback).
- **Adaptive_Resolution**: The behavior where the Selkies_Stream resizes the X display to match the client browser window. The client sends `r,<width>x<height>,<displayId>` over the WebSocket on window resize; the server's `reconfigure_displays()` synthesizes an xrandr modeline (via `cvt`) and applies it. Gated by `SELKIES_IS_MANUAL_RESOLUTION_MODE`.
- **Selkies_Canvas**: The Xvfb `-screen` size for the selkies display. It is a HARD framebuffer ceiling that cannot grow at runtime; the client can only resize to sizes within it. Decoupled from DESKTOP_RESOLUTION and fixed at 3840x2160 (4K).
- **cvt**: The CVT-timing modeline generator (apt package `xcvt`) that Selkies' `reconfigure_displays()` invokes to create xrandr modes for non-preset resolutions. Without it (or `gtf`), adaptive resize aborts.

## Requirements

### Requirement 1: Browser-Based H.264 Desktop Streaming

**User Story:** As a developer using the secure workspace, I want a browser-accessible remote desktop with materially better fluidity than KasmVNC, so that I can work in a GUI environment smoothly over the network.

#### Acceptance Criteria

1. WHEN a developer opens the desktop in a browser, THE Front_Layer SHALL serve the Static_Client and THE Selkies_Stream SHALL deliver the desktop as an H.264 video stream decoded by browser WebCodecs over a WebSocket connection.
2. THE Selkies_Stream SHALL transport all desktop traffic using HTTP and WebSocket protocols only, without WebRTC, STUN, TURN, or UDP.
3. THE Selkies_Stream SHALL be a WebSocket-only server that does NOT serve the Static_Client; a direct browser request to its port SHALL return HTTP 426.
4. WHILE the desktop renders continuous motion at 1280x720 resolution and 30 frames per second, THE Selkies_Stream SHALL sustain an H.264 stream of approximately 29.5 frames per second.
5. WHILE the desktop screen content is static, THE Selkies_Stream SHALL reduce H.264 encoding output to near-zero frames.

**Traceability:** CP2, CP3, CP5; P2, P3, P7, P10.

### Requirement 2: Zero Inbound Ports and Local-Only Binding

**User Story:** As a security-conscious operator, I want the new desktop stack to preserve the zero-inbound-port posture, so that the container's network attack surface remains unchanged from the KasmVNC design.

#### Acceptance Criteria

1. THE Selkies_Stream SHALL bind its WebSocket listener to address 127.0.0.1 on DESKTOP_STREAM_PORT (6080).
2. THE Workspace SHALL expose zero inbound network ports for desktop access.
3. THE Xvfb_Server SHALL run with the `-nolisten tcp` option so the X server is reachable only through its local socket.
4. WHERE remote desktop access is requested, THE Cloudflare_Tunnel SHALL carry desktop traffic as HTTP and WebSocket only, with no STUN, TURN, or UDP.

**Traceability:** CP2, CP3, CP5; P2.

### Requirement 3: Authenticated Reverse-Proxy Access

**User Story:** As an operator, I want the desktop to require the same authentication as code-server, so that access control stays consistent and simple.

#### Acceptance Criteria

1. WHEN a request for `/desktop/*` arrives with no credentials, THE Caddy_Proxy SHALL respond with HTTP status 401.
2. WHEN a request for `/desktop/*` arrives with credentials that do not match DESKTOP_BCRYPT_HASH, THE Caddy_Proxy SHALL respond with HTTP status 401.
3. WHEN a request for `/desktop/*` arrives with credentials that match DESKTOP_BCRYPT_HASH, THE Caddy_Proxy SHALL respond with HTTP status 200.
4. WHEN a WebSocket upgrade request arrives at `/desktop/websockets` with credentials that match DESKTOP_BCRYPT_HASH, THE Caddy_Proxy SHALL complete the upgrade with HTTP status 101.
5. THE Caddy_Proxy SHALL authenticate desktop access using the DESKTOP_BCRYPT_HASH that is shared with code-server.

**Traceability:** CP5; P7, P8.

### Requirement 4: Caddy Desktop Routing (Static Client + WebSocket Reverse-Proxy)

**User Story:** As a maintainer, I want Caddy to serve the static client directly and reverse-proxy only the WebSocket, so that the WS-only Selkies server works without an extra NGINX and without the `/websockify` hack.

#### Acceptance Criteria

1. THE Caddy_Proxy SHALL serve the Static_Client from DESKTOP_WEB_ROOT for requests under `/desktop/*` that are not the WebSocket path.
2. THE Caddy_Proxy SHALL match the WebSocket path `/desktop/websockets` and reverse-proxy it to the Selkies_Stream on DESKTOP_STREAM_PORT (6080), evaluating this match BEFORE the static-file handler so the WebSocket is not served as a file.
3. WHEN handling the WebSocket, THE Caddy_Proxy SHALL strip the `/desktop` prefix before forwarding so Selkies sees `/websockets`.
4. WHEN a request for `/desktop` arrives without a trailing slash, THE Caddy_Proxy SHALL redirect permanently to `/desktop/`.
5. WHEN forwarding the desktop WebSocket, THE Caddy_Proxy SHALL upgrade the connection natively without a dedicated `/websockify` root-path route.
6. THE Front_Layer SHALL be Caddy only; THE Workspace SHALL NOT add an NGINX process for desktop serving.

**Traceability:** CP5; P8, P10, P14.

### Requirement 5: On-Demand Desktop Lifecycle

**User Story:** As a developer, I want the desktop to start only when requested and stop cleanly, so that the idle container footprint stays minimal and the dependency chain comes up in order.

#### Acceptance Criteria

1. THE Desktop_Control SHALL configure desktop-xvfb, desktop-selkies, and desktop-xfce with autostart disabled.
2. WHEN a desktop start is requested, THE Desktop_Control SHALL start the programs in the order Xvfb_Server, then Selkies_Stream, then XFCE_Session.
3. WHEN a desktop stop is requested, THE Desktop_Control SHALL stop the programs in the order XFCE_Session, then Selkies_Stream, then Xvfb_Server.
4. THE Desktop_Control SHALL define the stop order as the exact reverse of the start order.
5. IF a desktop start is requested while `/workspace` is not mounted, THEN THE Desktop_Control SHALL exit with a non-zero status.
6. WHEN the desktop programs have been started, THE Desktop_Control SHALL poll DESKTOP_STREAM_PORT for up to 30 seconds and report the desktop ready when the port accepts a connection.
7. IF DESKTOP_STREAM_PORT does not accept a connection within 30 seconds, THEN THE Desktop_Control SHALL report that the desktop stream is not ready.

**Traceability:** CP3, CP4; P4.

### Requirement 6: gocryptfs FUSE-Unmount Safety

**User Story:** As an operator unlocking or destroying the vault, I want every desktop process stopped before the gocryptfs unmount, so that `fusermount -u` never fails with EBUSY.

#### Acceptance Criteria

1. WHEN `/workspace` is about to be unmounted, THE Desktop_Control SHALL stop all desktop programs before the unmount proceeds.
2. WHEN stopping the desktop, THE Desktop_Control SHALL stop desktop-xfce before desktop-selkies and desktop-xvfb so the primary HOME-file-descriptor holder is released first.
3. WHEN `stopDesktop()` is invoked, THE Desktop_Control SHALL issue the stop over all three desktop programs even when some are already stopped.
4. THE Selkies_Stream SHALL write no files to `/workspace/.desktop` by default, taking its configuration from environment variables, so the EBUSY risk surface does not grow.
5. WHEN lock-vault, destroy, or desktop-stop runs, THE Vault_Lifecycle SHALL confirm through `stopDesktop()` that no desktop process holds `/workspace/.desktop` file descriptors before the unmount.

**Traceability:** CP4; design invariants 6 and 7.

### Requirement 7: Preservation of the Existing XFCE Session and Home

**User Story:** As a developer, I want my existing XFCE session, home directory, fonts, and installed GUI apps preserved, so that the migration does not disrupt my environment.

#### Acceptance Criteria

1. THE XFCE_Session SHALL continue to run via `start-xfce.sh` without modification.
2. THE Xvfb_Server, Selkies_Stream, and XFCE_Session SHALL all attach to X display DESKTOP_DISPLAY (`:1`).
3. THE XFCE_Session SHALL use `/workspace/.desktop` as the HOME directory on the gocryptfs mount.
4. WHERE Chinese localization is configured, THE Selkies_Stream SHALL accept `LC_ALL` set to `zh_CN.UTF-8` and the image SHALL provide the `fonts-noto-cjk` Chinese fonts.
5. THE Desktop_Installer SHALL continue to install GUI applications into the XFCE session and its apps registry without modification.

**Traceability:** CP2, CP3, CP6; P4, P9.

### Requirement 8: Standalone Xvfb X Server

**User Story:** As a maintainer, I want a standalone Xvfb X server in place of the integrated Xkasmvnc server, so that Selkies and XFCE attach to an external display managed as its own program.

#### Acceptance Criteria

1. THE Xvfb_Server SHALL run as a standalone X server for display `:1` using the resolution from `DESKTOP_RESOLUTION` at 24-bit color depth.
2. THE Xvfb_Server SHALL enable the COMPOSITE, DAMAGE, RANDR, and GLX extensions.
3. WHEN `DESKTOP_RESOLUTION` is unset at container startup, THE Entrypoint SHALL set it to `DESKTOP_DEFAULT_RESOLUTION` (1920x1080) before supervisord expands it.

**Traceability:** CP2, CP3; P4.

### Requirement 9: Correct Package Sourcing and Build-Time Architecture Verification

**User Story:** As a build engineer, I want the new-arch Selkies installed from the correct sources with a build-time check, so that the wrong WebRTC/GStreamer arch can never ship.

#### Acceptance Criteria

1. THE Build_System SHALL install the new-arch `selkies` module from a pinned GitHub commit reference rather than from PyPI.
2. THE Build_System SHALL install ONLY the git `selkies` package and SHALL allow it to resolve `pixelflux` (1.6.3) and `pcmflux` (1.0.8) transitively; THE Build_System SHALL NOT list `pixelflux`/`pcmflux` explicitly in the same pip invocation (doing so triggers a hash-mismatch failure).
3. WHEN building the image, THE Build_System SHALL execute checks that import `from selkies.__main__ import main` AND `import pixelflux, pcmflux`.
4. IF either import check fails during the build, THEN THE Build_System SHALL fail the build.
5. THE Build_System SHALL install the verified HARD runtime dependencies — `libpulse0`, `xclip`, `xdotool`, `x11-xserver-utils` (xrandr), `libva2`, `libva-drm2`, `libgbm1`, `libdrm2`, `libjpeg62-turbo` — and the `xvfb` X server; missing any of `libpulse0`/`xclip`/`xdotool`/xrandr SHALL be treated as a defect because Selkies aborts at startup without them.
6. WHEN the Selkies install layer completes, THE Build_System SHALL purge the build-only dependencies `python3-dev`, `gcc`, `pkg-config`, `libxkbcommon-dev`, and `git` within the same layer.

**Traceability:** CP2; P5, P6, P12.

### Requirement 10: Reproducible, Auditable, Self-Installed Dependencies

**User Story:** As a security auditor, I want every desktop dependency named and pinned in our own Dockerfile, so that the build does not trust an external prebuilt image and remains reproducible.

#### Acceptance Criteria

1. THE Build_System SHALL install Selkies into a dedicated virtualenv at `/opt/selkies-venv` using the in-image Python 3.12.
2. THE Build_System SHALL pin the `selkies` commit reference to an explicit value; `pixelflux` and `pcmflux` versions are pinned transitively by that selkies reference.
3. THE Build_System SHALL source all desktop dependencies from named, pinned references declared in the project Dockerfile rather than copying binaries from an external prebuilt image.

**Traceability:** CP2; design "Self-Install vs Multi-Stage COPY" decision (Option A); P5, P6, P12.

### Requirement 9a: Static Client Provisioning

**User Story:** As a maintainer, I want the Selkies web client provisioned into the image and served by Caddy, so that the browser actually receives the client (the pip package does not include it).

#### Acceptance Criteria

1. THE Build_System SHALL provision the Static_Client (the ~5.9 MB web client) into DESKTOP_WEB_ROOT (`/usr/share/selkies/web`), because the pip-installed `selkies` package does NOT contain the web client.
2. THE Static_Client SHALL be sourced at the SAME selkies reference as the Selkies_Stream server, to avoid client/server protocol drift.
3. THE Build_System SHALL vendor the Static_Client as a reviewable artifact in the repository (e.g. `image/selkies-web/`) rather than fetching it from an external image at runtime.
4. WHEN the desktop is running, THE Front_Layer SHALL serve the Static_Client index and assets under `/desktop/` (verified: `/desktop/` → 200 HTML, `/desktop/assets/*` → 200).

**Traceability:** CP2, CP5; P10, P11.

### Requirement 9b: Stream Port Collision Avoidance

**User Story:** As a maintainer, I want Selkies to run on a port that does not collide with code-server, so that both bind successfully in the single container.

#### Acceptance Criteria

1. THE Selkies_Stream SHALL listen on DESKTOP_STREAM_PORT (6080) via an explicit `--port=6080`, NOT on Selkies' own default 8082.
2. THE Selkies_Stream port SHALL differ from CODE_SERVER_PORT (8082) so the two processes do not collide.
3. THE Selkies_Stream `--control-port` (default 8083) SHALL bind loopback only and SHALL NOT be reverse-proxied by the Caddy_Proxy.

**Traceability:** CP1, CP3, CP5; P9.

### Requirement 11: [REMOVED — KasmVNC fallback not in this branch]

> KasmVNC lives on `feat/dual-vault-pentest-env`. No build-arg gate in this branch.

### Requirement 12: Doctor Health Checks for the New Components

**User Story:** As an operator, I want `doctor` to report the new desktop components correctly, so that I can diagnose desktop health after the migration.

#### Acceptance Criteria

1. WHEN the Xvfb_Server is RUNNING, THE Doctor SHALL report the component "Desktop X server (Xvfb)" with status ok.
2. WHEN DESKTOP_STREAM_PORT (6080) is listening, THE Doctor SHALL report the component "Desktop stream (Selkies)" with status ok.
3. IF DESKTOP_STREAM_PORT (6080) is not responding, THEN THE Doctor SHALL report the component "Desktop stream (Selkies)" with status fail.
4. THE Doctor SHALL retain the `DISPLAY=:1 xdpyinfo` X-server check and the xfwm4/xfce4-panel fake-alive detection for the XFCE session.
5. WHEN the desktop is running, THE Doctor SHALL verify the Static_Client is actually served (e.g. `/desktop/` returns 200 with client HTML), because a port-only probe would pass even when the WS-only Selkies is up but the Static_Client is missing — which presents to the user as HTTP 426 or a white screen.

**Traceability:** CP4, CP6; P10.

### Requirement 13: Architectural Invariants

**User Story:** As a maintainer, I want the locked architectural invariants preserved through the migration, so that the codebase stays clean, layered, and free of dead code.

#### Acceptance Criteria

1. THE Core_Package SHALL contain only pure constants and logic with no IO for the desktop changes.
2. THE Desktop_Control SHALL import desktop constants from the Core_Package without introducing any upward dependency from core to server or cli.
3. THE Desktop_Control SHALL remain the single source of truth that defines the desktop process group, start order, and stop order for all dependent callers.
4. THE Entrypoint SHALL remain the single environment-configuration surface for `DESKTOP_RESOLUTION` and `DESKTOP_BCRYPT_HASH`.
5. THE Core_Package SHALL NOT define `KASMVNC_PORT` (KasmVNC lives on a separate branch; no dead constants).
6. WHEN the desktop topology is changed in the future, THE Desktop_Control SHALL keep the stop order as the exact reverse of the start order so the FUSE-safety invariant holds.

**Traceability:** CP1, CP4; design invariants 1-8 and SSOT pattern.

### Requirement 14: Performance Thresholds

**User Story:** As an operator, I want the desktop to meet the measured PoC performance thresholds, so that resource usage stays within the container's budget.

#### Acceptance Criteria

1. WHILE streaming 1280x720 at 30 frames per second under continuous motion, THE Selkies_Stream SHALL consume approximately 0.18 of one CPU core for encoding.
2. WHILE the desktop screen is static, THE Selkies_Stream SHALL consume near-zero CPU for encoding.
3. WHILE the container is idle with the desktop running, THE Workspace SHALL consume approximately 2 to 3 percent CPU and approximately 830 to 940 MB of RAM.
4. WHILE streaming through the authenticated Caddy_Proxy at the test resolution, THE Selkies_Stream SHALL sustain approximately 29.5 frames per second at approximately 9.6 Mbit per second.

**Traceability:** P3, P7; design "Performance Considerations".

### Requirement 15: Open Verification Items Gating Fallback Removal

**User Story:** As a release manager, I want each open verification item validated before the KasmVNC fallback is removed, so that the workspace never ships an unverified migration.

#### Acceptance Criteria

1. WHEN the image is built on the `kalilinux/kali-rolling` base, THE Build_System SHALL complete the Selkies virtualenv build successfully. (OPEN — verified on debian:bookworm only.)
2. WHEN the Selkies_Stream runs on the real XFCE desktop and is opened in a real browser through the authenticated Caddy_Proxy, THE Selkies_Stream SHALL deliver flowing H.264 frames (the headless PoC confirmed WS connect + pipeline start but did not count frames on an empty desktop). (OPEN — must verify in the built image.)
3. WHEN the Static_Client loads under the `/desktop/` subpath through the authenticated Caddy_Proxy, THE Front_Layer SHALL resolve all static assets (JS, WASM, icons) without HTTP 404 responses. (Verified in the no-NGINX PoC at `/desktop/assets/*` → 200; reconfirm in the built image.)
4. WHERE audio streaming is enabled, THE Selkies_Stream SHALL route audio through a pulseaudio null-sink that remains loopback-only and opens no network port, and audio SHALL default to off.
5. THE Build_System SHALL have its build-time cost for compiling `av`, `cryptography`, and `xkbcommon` measured, with a recorded decision on whether to vendor prebuilt wheels.
6. THE Selkies_Stream invocation SHALL be reconciled against the pinned commit's `--help` output so the exact `--mode`, `--port`, and `--addr` flags are locked. (Verified: `--mode=websockets --port=6080 --addr=127.0.0.1`; reconfirm against the pinned ref.)
7. KasmVNC is NOT in this branch (lives on `feat/dual-vault-pentest-env`). No fallback-removal gate applies; the Selkies implementation is the sole desktop stack.

**Traceability:** design "Open Verification Items" and "Rollback / Transition Consideration"; CP2, CP5; P8, P10, P11, P14.

### Requirement 16: Adaptive Resolution (Auto-Fit Browser Window)

**User Story:** As a developer, I want the Selkies desktop to automatically resize to match my browser window (like KasmVNC's dynamic resolution), so that the desktop fills the viewport without manual configuration.

#### Acceptance Criteria

1. THE Selkies_Stream SHALL run with `SELKIES_IS_MANUAL_RESOLUTION_MODE="false"` so that client-driven resize requests are honored rather than ignored.
2. WHEN the client sends a resize request (`r,<width>x<height>,<displayId>`) for a resolution not already present in the X mode list, THE Selkies_Stream SHALL synthesize an xrandr modeline using `cvt` and apply it.
3. THE Build_System SHALL install the `cvt` binary (apt package `xcvt`) in the selkies stack, because `cvt` and `gtf` are otherwise both absent (`x11-xserver-utils` provides `xrandr` but NOT `cvt`); without either, `reconfigure_displays()` aborts the resize with `FATAL: Could not create extended mode`.
4. THE Xvfb_Server for the selkies display SHALL use a FIXED virtual canvas of `3840x2160` (Selkies_Canvas), decoupled from DESKTOP_RESOLUTION.
5. WHEN a client requests a resolution within the Selkies_Canvas, THE Selkies_Stream SHALL resize successfully without crashing.
6. IF a client requests a resolution exceeding the Selkies_Canvas, THEN pixelflux's MIT-SHM capture SHALL read past the framebuffer and crash Selkies with `X Error BadMatch (X_ShmGetImage)`; therefore THE Selkies_Canvas SHALL be sized to cover all expected client windows (including HiDPI/`devicePixelRatio` physical-pixel requests).
7. THE DESKTOP_RESOLUTION variable SHALL continue to drive the kasmvnc `Xkasmvnc -geometry` (kasmvnc has its own dynamic-resolution engine and is unaffected by the Selkies_Canvas change).
8. WHEN the selkies desktop is opened in a real browser and the window is resized, THE Selkies_Stream SHALL track the window resolution (verified: live follow in Chrome, X `current` matching the window to within the /8 width alignment) while remaining RUNNING.

**Traceability:** design Addendum (2026-06-01) #2; CP2 (Dockerfile xcvt), CP3 (supervisord Xvfb canvas + selkies env); selkies source `selkies.py::on_resize_handler` / `reconfigure_displays` / `display_utils.generate_xrandr_gtf_modeline`.
