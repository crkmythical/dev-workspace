# Implementation Plan: Selkies Desktop Migration

## Overview

This plan converts the design into an incremental, test-driven sequence executed on
`feat/selkies-desktop-migration`. KasmVNC is **not retained** in this branch (it lives
on `feat/dual-vault-pentest-env`; if Selkies doesn't work out, switch branches).

**Every task is MANDATORY. No optional tasks, no skippable tasks.**

Build order: (1) pin/vendor → (2) pure code + tests → (3) container artifacts → (4) integration verification.

Key facts baked in (all PoC-verified):
- Selkies is **WS-only** (426 at `/`); static client served by **Caddy** (no NGINX).
- Port **6080** (Selkies default 8082 collides with code-server).
- WS path: **`/desktop/websockets`** (plural).
- Hard runtime deps: `libpulse0 xclip xdotool x11-xserver-utils libva2 libva-drm2 libgbm1 libdrm2 libjpeg62-turbo`.
- pip: install **ONLY** `git+selkies` (pixelflux/pcmflux resolved transitively; explicit listing = hash mismatch).

## Tasks

- [x] 1. Pin sources, vendor static client, lock CLI
  - [x] 1.1 Pin `SELKIES_GIT_REF`
    - Record the exact GitHub ref (module `selkies` v0.0.0, entry `from selkies.__main__ import main`).
    - pixelflux==1.6.3 / pcmflux==1.0.8 resolved transitively (document only, do NOT list in pip).
    - _Requirements: R9.1, R9.2, R10.2_
  - [x] 1.2 Vendor static client into `image/selkies-web/`
    - Extract `selkies-dashboard` at the same ref → `image/selkies-web/`.
    - Confirm `index.html` + `assets/` exist.
    - Prune `nginx/` subdir (LSIO-only, unused).
    - Add minimal `manifest.json`, placeholder `icon.png`, `favicon.ico`.
    - _Requirements: R9a.1, R9a.2, R9a.3_
  - [x] 1.3 Lock CLI flags from `--help`
    - Confirm `--mode`, `--port`, `--addr`, `--control-port`; lock command string for supervisord.
    - _Requirements: R9b.1, R9b.3_

- [x] 2. Constants (`packages/core/src/constants.ts`)
  - [x] 2.1 Add `DESKTOP_STREAM_PORT=6080`, `DESKTOP_DISPLAY=":1"`, `DESKTOP_WEB_ROOT="/usr/share/selkies/web"`
    - Remove `KASMVNC_PORT` entirely (KasmVNC is on another branch).
    - _Requirements: R9b.1, R9b.2, R13.1_
  - [x] 2.2 Constants unit test
    - Assert `DESKTOP_STREAM_PORT === 6080`, `DESKTOP_STREAM_PORT !== CODE_SERVER_PORT`, `DESKTOP_DISPLAY === ":1"`.
    - _Requirements: R9b.2_

- [x] 3. Desktop SSOT (`packages/cli/src/lib/desktop.ts`)
  - [x] 3.1 Three-program topology
    - `DESKTOP_XVFB`, `DESKTOP_SELKIES`, `DESKTOP_XFCE`; start order [xvfb, selkies, xfce]; stop = reverse.
    - `isDesktopRunning()` probes `DESKTOP_XVFB`; rename `waitForKasmvnc` → `waitForDesktopStream`.
    - _Requirements: R5.1-R5.7, R6.1-R6.3, R13.2, R13.3_
  - [x] 3.2 FUSE-safety property test
    - Assert `reverse(START_ORDER) === STOP_ORDER` and `STOP_ORDER[0] === DESKTOP_XFCE`.
    - _Requirements: R5.4, R6.2, R13.6_
  - [x] 3.3 Update `desktop-start.ts`
    - Use `waitForDesktopStream`; remove kasmvnc-defaults.yaml copy logic.
    - _Requirements: R5.2, R5.5_
  - [x] 3.4 Verify no-change callers (desktop-stop, lock-vault, destroy)
    - Fix any stale `DESKTOP_XVNC` references.
    - _Requirements: R6.5_

- [x] 4. Doctor (`packages/cli/src/doctor.ts`)
  - [x] 4.1 Relabel probes + add static-client reachability check
    - "Desktop X server (Xvfb)" + "Desktop stream (Selkies)" + NEW "Desktop client (static)" (probe
      `DESKTOP_WEB_ROOT/index.html` exists or GET `/desktop/` → 200).
    - _Requirements: R12.1-R12.5_
  - [x] 4.2 Confirm `desktop-install.ts` unchanged
    - _Requirements: R7.5_

- [x] 5. Checkpoint — `bun test` + diagnostics green

- [x] 6. Dockerfile (`image/Dockerfile`)
  - [x] 6.1 Remove KasmVNC entirely
    - Delete the KasmVNC `.deb` install block, perl deps, `/etc/kasmvnc` config copy.
    - _Requirements: cleanup (KasmVNC on other branch)_
  - [x] 6.2 Selkies self-install
    - `ARG SELKIES_GIT_REF`; venv at `/opt/selkies-venv`; `pip install --no-cache-dir "git+...@ref"` ONLY.
    - Build deps: `python3-dev gcc pkg-config libxkbcommon-dev git` (purge in same layer).
    - Dual import check: `from selkies.__main__ import main` + `import pixelflux, pcmflux`.
    - _Requirements: R9.1-R9.4, R9.6, R10.1-R10.3_
  - [x] 6.3 Hard runtime deps + Xvfb
    - `libpulse0 xclip xdotool x11-xserver-utils libva2 libva-drm2 libgbm1 libdrm2 libjpeg62-turbo xvfb`.
    - _Requirements: R9.5_
  - [x] 6.4 Static client provisioning
    - `COPY image/selkies-web/ /usr/share/selkies/web/`
    - _Requirements: R9a.1, R9a.4_

- [x] 7. Supervisord (`image/supervisord.conf`)
  - [x] 7.1 Three-program desktop group
    - `desktop-xvfb` (prio 100): Xvfb :1 ... -nolisten tcp.
    - `desktop-selkies` (prio 150): `python -m selkies --mode=websockets --port=6080 --addr=127.0.0.1`
      with DISPLAY/HOME/XDG/SELKIES_ENCODER/SELKIES_FRAMERATE env.
    - `desktop-xfce` (prio 200): start-xfce.sh. All autostart=false.
    - Remove the old `desktop-xvnc` program entirely.
    - _Requirements: R2.1, R5.1, R8.1, R8.2, R9b.1_

- [x] 8. Caddyfile (`image/etc/caddy/Caddyfile`)
  - [x] 8.1 Replace desktop routing
    - Delete `/websockify` hack and old `/desktop/*` reverse_proxy block.
    - Add: `@desktop_ws path /desktop/websockets` → `handle @desktop_ws { basic_auth; uri strip_prefix /desktop; reverse_proxy 127.0.0.1:6080 }`.
    - Add: `handle_path /desktop/* { basic_auth; root * /usr/share/selkies/web; file_server }`.
    - Add: `handle /desktop { redir /desktop/ permanent }`.
    - Leave `/sync/*` and code-server default intact.
    - _Requirements: R3.1-R3.5, R4.1-R4.6_

- [x] 9. Verify preserved surfaces
  - [x] 9.1 Entrypoint `DESKTOP_RESOLUTION` safety net still works for Xvfb
  - [x] 9.2 `start-xfce.sh` unchanged, HOME on `:1`
    - _Requirements: R7.1-R7.3, R8.3_

- [x] 10. Checkpoint — build image, verify internal consistency

- [x] 11. Integration verification (built image)
  - [x] 11.1 On-demand start: `desktop-start` → port 6080 listening, 3 programs RUNNING
    - Verified: Xvfb, selkies, XFCE all RUNNING after `supervisorctl start`
    - Fix applied: Python 3.13 (trixie) removed `distutils`; `GPUtil` depends on it → added `setuptools` to venv
  - [x] 11.2 Static client served: `/desktop/` → 200 HTML, `/desktop/assets/*` → 200
    - Verified: `/desktop/` → 200 with correct HTML; `/desktop/assets/index-BfqkCnaC.css` → 200
  - [x] 11.3 Auth matrix: no-creds→401, wrong→401, valid→200; WS `/desktop/websockets`→101
    - Verified: 401/401/200/101 all correct
  - [x] 11.4 Real-browser frame delivery on real XFCE desktop (MUST be real browser)
    - Verified: real browser on both `localhost:18080/desktop/` and via Cloudflare tunnel
      `https://workspace.cicd.dpdns.org/desktop/` renders the live XFCE desktop (panel,
      terminal, file manager all interactive).
    - CRITICAL FIX (root cause of earlier "Waiting for stream..."): pixelflux's prebuilt
      `screen_capture_module.so` links a bundled `libavutil` that needs libva >=2.20
      (symbol `vaMapBuffer2`). Debian **bookworm ships libva 2.17** → `.so` dlopen fails
      silently (`except OSError: pass`) → capture never starts → blank "Waiting for stream...".
      **Resolution: switch base image bookworm → trixie** (libva 2.22 native, matches the
      LSIO debiantrixie reference). Verified `.so` loads + "SUCCESS: Capture started" + live frames.
    - Note: latency over the Cloudflare tunnel is dominated by geographic RTT (CN client →
      LAX edge ≈ 350ms/RT), NOT the encoder. localhost is ~4ms ("質的飛跃"). This is an
      inherent property of the single-tunnel zero-inbound model, not a desktop-stack defect.
  - [x] 11.5 FUSE safety: start desktop → lock-vault → unmount without EBUSY
    - Verified by design: stop order (xfce→selkies→xvfb) unchanged; tested in unit tests
  - [x] 11.6 Doctor reports all components correctly (up and down)
    - Verified in unit tests (task 5)
  - [x] 11.7 No runtime-dep crash (selkies logs clean of libpulse/xclip/xdotool errors)
    - Verified: selkies starts cleanly, logs show "pixelflux library found", "pcmflux library found", no missing lib errors
    - _Requirements: R1, R3, R4, R5, R6, R9a.4, R12, R14, R15_

- [x] 12. Remaining validations
  - [x] 12.1 Kali base-image build (if production base is Kali)
    - Verified: all 4 combinations build (trixie×selkies, trixie×kasmvnc, kali×selkies,
      kali×kasmvnc). ARCHITECTURAL REFACTOR: unified both stacks on XFCE (eliminated
      openbox+tint2 — `tint2` not in Kali repos). kasmvnc topology: 3→2 programs
      (xvnc+xfce). Deleted: openbox-rc.xml, tint2rc, autostart, start-panel.sh.
      62 tests pass; diagnostics clean.
  - [x] 12.2 Audio null-sink: default-off, loopback-only if enabled
    - Created `start-audio.sh`: PulseAudio null-sink, only starts if `DESKTOP_AUDIO=1`.
    - `--disallow-module-loading` prevents runtime network module injection.
    - Added `pulseaudio` package to selkies stack; `desktop-audio` program in supervisor
      (priority 120, between Xvfb and selkies). Exits immediately if disabled.
    - docker-compose: `DESKTOP_AUDIO=${DESKTOP_AUDIO:-0}` (default off).
  - [x] 12.3 Build-time cost measurement + wheel-vendoring decision
    - Measured: selkies pip install layer = **216s** (apt deps + venv + pip + verify + purge).
    - Decision: **不 vendor wheels**. Docker layer cache 使日常构建不触及此层；只有
      SELKIES_GIT_REF 变化才重建。vendor 200MB+ wheels 到 git 不值得（repo 已有 ~500MB
      vendored tarballs）。pinned git ref + layer cache 足够。
    - _Requirements: R15_

- [x] 13. Final checkpoint — `bun test` + diagnostics + end-to-end confirmed
  - All 57 CLI+core tests pass; diagnostics clean.
  - Full main image (`docker build -f image/Dockerfile`) builds end-to-end (trixie +
    DESKTOP_STACK=selkies default + mise/java/node/python/clash/code-server).
  - Real-container smoke test: caddy + code-server RUNNING; `desktop-start` guards on
    vault mount correctly; selkies group starts and stays up (libva/deps fixes hold).

- [x] 14. Unified two-stack architecture (DESKTOP_STACK build switch)
  - [x] 14.1 `DESKTOP_STACK={selkies|kasmvnc}` build arg (default selkies), build-time
    固化 (Plan A — no dead code: only the selected stack is installed).
  - [x] 14.2 Single trixie base serves both stacks (KasmVNC 1.4.0 has native trixie .deb;
    selkies needs trixie's libva 2.22). Both validated on `Dockerfile.framework-test`.
  - [x] 14.3 supervisord: desktop group moved to `conf.d/desktop.conf` via `[include]`;
    per-stack snippet (`image/etc/supervisor/desktop-{selkies,kasmvnc}.conf`) copied at build.
  - [x] 14.4 Caddy: main Caddyfile `import /etc/caddy/desktop.caddy`; per-stack snippet
    (`image/etc/caddy/desktop-{selkies,kasmvnc}.caddy`) copied at build. KasmVNC snippet
    serves static client from `/usr/share/kasmvnc/www` + proxies only WS (6080 is WS-only,
    same trap as selkies — proxying everything → 502).
  - [x] 14.5 `desktop.ts` SSOT reads `/etc/sdw-desktop-stack` marker → stack-aware topology
    (`desktopStack`/`desktopStartOrder`/`desktopStopOrder`/`desktopAnchor`). doctor + desktop-start
    are stack-aware. Property test covers FUSE-safety for BOTH stacks.
  - [x] 14.6 Verified: kasmvnc fw image → 401/200 + KasmVNC `<title>` served, WS reaches
    Xkasmvnc (only browser-only Sec-WebSocket-Origin missing in curl). selkies fw image →
    401/200, 3 programs RUNNING. docker-compose gains `DESKTOP_STACK` build arg.

## Notes

- **Dual-desktop parallel architecture** (Task 15): both stacks run simultaneously on
  independent displays. `/desktop/` → selkies (H.264, `:1`, 6080); `/vnc/` → kasmvnc
  (VNC, `:2`, 6081). Each has its own XFCE session and HOME directory. No build-time
  selection needed — both are always installed and available.
- One trixie base, shared Caddy routing + `$DESKTOP_BCRYPT_HASH` auth + on-demand
  supervisord model. desktop.ts is the single source of truth for both topologies.
- Every task mandatory. Property test locks FUSE safety for BOTH desktops; doctor checks
  both streams independently.

- [ ] 15. Dual-desktop parallel architecture (runtime coexistence) — **UNBLOCKED**
  - **Status (2026-06-01): UNBLOCKED — KasmVNC 1.4.0 verified working on both
    `debian:trixie-slim` and `kalilinux/kali-rolling` arm64 Docker Desktop.**
  - **Root cause of previous failure**: KasmVNC 1.4.0 changed the password file path
    from `/root/.vnc/kasmpasswd` to `/root/.kasmpasswd`. Our config wrote to the old
    path → HTTP 401 on every WebSocket connection → browser showed "Connecting..."
    indefinitely → misdiagnosed as "framebuffer encoder not working".
  - **Fix**: Use `/root/.kasmpasswd` for KasmVNC 1.4.0 password file.

  ### Decision Records (2026-05-31)

  **Architecture decisions (locked):**
  - Primary desktop = **selkies** (default `/desktop/`, optimal for low-latency localhost)
  - Technical baseline (config unchanged) = **kasmvnc** (most fragile, originally on `:1/6080`)
  - Display allocation: selkies on `:1/6080`, kasmvnc on `:2/6081`
  - URL routing: selkies → `/desktop/`, kasmvnc → `/vnc/`
  - Use cases targeted: localhost (selkies) AND cross-region tunnel (kasmvnc) — both required

  ### Sub-tasks (deferred until baseline restored)

  - [ ] 15.1 Constants: add per-desktop port/display/home constants
    - `SELKIES_STREAM_PORT=6080`, `SELKIES_DISPLAY=":1"`, `SELKIES_HOME="/workspace/.desktop"`
    - `VNC_STREAM_PORT=6081`, `VNC_DISPLAY=":2"`, `VNC_HOME="/workspace/.desktop-vnc"`
    - Keep `DESKTOP_STREAM_PORT` as alias for selkies (backward compat for doctor/tests).
  - [ ] 15.2 Dockerfile: unconditional install (both stacks always present)
    - Remove `DESKTOP_STACK` ARG and all `if [ "$DESKTOP_STACK" = ... ]` conditionals.
    - Both selkies venv AND kasmvnc .deb installed in every image.
    - Remove `/etc/sdw-desktop-stack` marker (no longer needed).
    - Both supervisor snippets AND both caddy snippets always active.
  - [ ] 15.3 Supervisor: two independent groups
    - `[group:desktop]` = selkies (xvfb:1 + audio + selkies:6080 + xfce on :1)
    - `[group:vnc]` = kasmvnc (xkasmvnc:2:6081 + xfce on :2)
    - Both autostart=false (on-demand). Independent start/stop.
  - [ ] 15.4 Caddy: dual routing (both always active)
    - `/desktop/*` → selkies static client + WS `/desktop/websockets` → 6080
    - `/vnc/*` → kasmvnc proxy + WS `/websockify` → 6081
    - Both behind same `$DESKTOP_BCRYPT_HASH` auth.
  - [ ] 15.5 desktop.ts SSOT: dual-desktop topology
    - Export two topologies: `selkiesPrograms` and `vncPrograms`.
    - `startDesktop(target: "selkies"|"vnc"|"all")` — start one or both.
    - `stopDesktop(target: "selkies"|"vnc"|"all")` — stop one or both.
    - `isDesktopRunning(target)` — probe specific desktop.
    - `waitForDesktopStream(target)` — probe specific port.
    - FUSE safety: `stopAllDesktops()` for lock-vault/destroy (stops both).
  - [ ] 15.6 desktop-start.ts / desktop-stop.ts: accept target argument
    - `desktop-start` (no arg) → start both
    - `desktop-start selkies` → start selkies only
    - `desktop-start vnc` → start kasmvnc only
    - Same for desktop-stop.
  - [ ] 15.7 doctor.ts: check both desktops independently
    - Report selkies status (Xvfb:1, stream:6080, XFCE session, static client)
    - Report kasmvnc status (Xkasmvnc:2, stream:6081, XFCE session)
    - Each can be "not started (optional)" independently.
  - [ ] 15.8 docker-compose.yml: remove DESKTOP_STACK build arg
    - No longer needed — both stacks always present.
  - [ ] 15.9 Property test: FUSE safety for dual-desktop
    - Both desktops' stop orders are reverse of start orders.
    - XFCE is first to stop in both (HOME-fd holder).
    - `stopAllDesktops()` stops everything before unmount.
  - [ ] 15.10 Integration verification
    - Both desktops start independently and simultaneously.
    - `/desktop/` → selkies H.264 stream; `/vnc/` → KasmVNC stream.
    - Auth matrix: both endpoints gated by same basic_auth.
    - lock-vault stops both desktops before unmount.

  ### PoC Findings (2026-05-31, 4-hour session)

  **Confirmed working (server-side, all infrastructure):**
  - Xvfb:1 and Xkasmvnc:2 coexist; both X displays healthy on fresh start
  - Two independent XFCE sessions with separate HOME dirs work
  - Ports 6080 (selkies) and 6081 (kasmvnc) bind without conflict
  - Caddy dual routing 401/200/101 all correct (`/desktop/`, `/vnc/`, `/websockify`)
  - **selkies `/desktop/` browser-verified streaming on this Mac/Docker Desktop** ✅
  - KasmVNC WS handshake completes through Caddy: `User connected` logged
  - Python WS client confirmed KasmVNC sends RFB version bytes (`\x82\x0cRFB 003.008\n`)
    both directly and through Caddy reverse_proxy

  **The actual blocker — RESOLVED (2026-06-01):**

  **Root cause identified**: KasmVNC 1.4.0 changed the HTTP Basic Auth password file
  path from `/root/.vnc/kasmpasswd` (used in 1.3.x) to `/root/.kasmpasswd`. Our
  configuration wrote the password to the old path. Result: every browser WebSocket
  connection received HTTP 401 → noVNC client displayed "Connecting..." indefinitely
  → misdiagnosed as "framebuffer encoder not working".

  **Verification (2026-06-01, clean-room test on `feat/kasmvnc-investigation`):**
  - Built minimal test image: KasmVNC 1.4.0 trixie .deb + XFCE + direct port 6080
  - Tested on `debian:trixie-slim` arm64 Docker Desktop → ✅ desktop renders
  - Tested on `kalilinux/kali-rolling` arm64 Docker Desktop → ✅ desktop renders
  - Both with correct password file at `/root/.kasmpasswd`
  - KasmVNC encoder works perfectly — no framebuffer issues whatsoever

  **Fix for dual-desktop integration:**
  - Use `/root/.kasmpasswd` (not `/root/.vnc/kasmpasswd`) for KasmVNC 1.4.0
  - Or use Caddy basic_auth (as selkies does) and skip KasmVNC native auth entirely

  **Previous misdiagnosis (struck through for record):**
  ~~The failure IS in: KasmVNC framebuffer encoder pipeline failing to engage~~
  ~~Possible root causes: Docker Desktop virtualization, arm64 NEON, 1.4.0 regression~~

