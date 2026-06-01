# KasmVNC Framebuffer Investigation — Findings

**Status: RESOLVED (2026-06-01). KasmVNC 1.4.0 works on arm64 Docker Desktop.**

## Summary

The original "KasmVNC 1.4.0 framebuffer encoder broken on arm64 Docker Desktop"
diagnosis (Task 15 of selkies-desktop-migration, marked BLOCKED) was **incorrect**.
KasmVNC 1.4.0 streams correctly. The failures were a chain of **configuration**
problems, not an encoder, virtualization, multi-threading, or NEON bug.

## Verification

Clean-room test (`image/Dockerfile.kasmvnc-test` + `docker-compose.kasmvnc-test.yml`):
minimal KasmVNC 1.4.0 + XFCE, direct port 6080, no Caddy/supervisor/vault.

| Config | Result |
|--------|--------|
| KasmVNC 1.4.0 trixie .deb on `debian:trixie-slim` arm64 Docker Desktop | ✅ desktop renders in browser |
| KasmVNC 1.4.0 trixie .deb on `kalilinux/kali-rolling` arm64 Docker Desktop | ✅ desktop renders in browser |

Then verified in the full dual-desktop integration (selkies `/desktop/` +
kasmvnc `/vnc/` running in parallel behind Caddy) — both render.

## Root Cause (chain of config issues)

1. **Password file path.** KasmVNC 1.4.0 reads `$HOME/.kasmpasswd` (HOME-relative).
   Earlier configs wrote `/root/.vnc/kasmpasswd` (1.3.x layout) or `/root/.kasmpasswd`
   while HOME was elsewhere → HTTP 401 on every WS connection → browser "Connecting…"
   → misread as "encoder produces 0 framebuffer updates".
2. **`-SecurityTypes None` required.** Without it the RFB layer demands a VNC
   password → "No password configured for VNC Auth".
3. **Keep KasmVNC native HTTP Basic Auth (no `-DisableBasicAuth`).** KasmVNC must
   authenticate its own `/websockify` WebSocket upgrade.
4. **Caddy must not add basic_auth to `/vnc/`.** Browsers (Safari) don't replay
   Caddy basic-auth creds to a JS WebSocket handshake → 101 then auth-fail → stall.
5. **WS path is ROOT `/websockify`.** The noVNC client hardcodes `ws://<host>/websockify`
   (no `/vnc/` prefix). Caddy routes the bare `/websockify` to `:6081`.

## Resolution

Implemented as the dual-desktop architecture in the selkies-desktop-migration
spec (Task 15). KasmVNC config (`image/etc/supervisor/desktop-vnc.conf`):

```
Xkasmvnc :2 ... -SecurityTypes None ... -httpd /usr/share/kasmvnc/www
HOME=/workspace/.desktop-vnc   →   reads /workspace/.desktop-vnc/.kasmpasswd
```

Entrypoint writes `$HOME/.kasmpasswd` with the master password. Caddy
(`image/etc/caddy/desktop-vnc.caddy`) reverse-proxies `/vnc/*` and the root
`/websockify` to `:6081` WITHOUT its own basic_auth (KasmVNC owns auth).

## Outcome for this spec

The investigation tasks (1–4) below are **superseded** by the resolution above:
the bug did not need a code "fix" — it needed correct configuration, which now
lives in the production dual-desktop build. The test harness
(`Dockerfile.kasmvnc-test`, `docker-compose.kasmvnc-test.yml`,
`image/scripts/start-kasmvnc-test.sh`) is retained for future KasmVNC debugging.
