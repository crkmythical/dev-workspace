# Secure Dev Workspace

Encrypted, cloud-ready development environment with remote desktop, code-server, and proxy.

## Quick Start

```bash
# 1. Configure (copy and edit .env)
cp .env.example .env
# Set PASSWORD (or leave empty for auto-generated)
# Set CLASH_SUBSCRIPTION_URL (required)

# 2. Build & Run
docker compose build
docker compose up -d

# 3. Access
# Code:    http://localhost:18080
# Desktop: http://localhost:18080/desktop/   (Selkies, H.264)
# VNC:     http://localhost:18080/vnc/        (KasmVNC)
# Auth:    user / <your PASSWORD>
```

## Rebuild / Reset

```bash
# Stop
docker compose down

# Rebuild (after code changes)
docker compose build
docker compose up -d

# Full reset (wipe vault data)
docker compose down
docker volume rm dev-workspace_vault-data
docker compose build --no-cache
docker compose up -d
```

## Features

- **Dual Remote Desktop** — two independent desktops run in parallel:
  - **Selkies** (`/desktop/`) — H.264/WebCodecs streaming, best for low-latency localhost.
    Auto-fits the browser window (adaptive resolution); no fixed size to configure.
  - **KasmVNC** (`/vnc/`) — VNC-over-WebSocket, robust over high-latency tunnels
- **Code Server** — VS Code in browser
- **Encrypted Vault** — gocryptfs auto-init/unlock
- **Proxy** — Clash global mode (all traffic through proxy)
- **Audio** — PulseAudio null-sink for desktop audio streaming
- **Base** — Kali Linux (pentest tools available via apt)

## Build Options

```bash
# Both desktops (default) — selkies + kasmvnc in parallel
docker compose build

# Single stack (fallback)
DESKTOP_STACK=selkies docker compose build   # selkies only
DESKTOP_STACK=kasmvnc docker compose build   # kasmvnc only

# Debian trixie base instead of Kali
docker compose build --build-arg BASE_IMAGE=debian:trixie-slim
```

## Commands (inside container)

```bash
doctor              # Health check (reports both desktops + backup)
noproxy <cmd>       # Run command bypassing proxy (direct network)
lock-vault          # Lock the encrypted workspace (stops desktops + backup first)
unlock-vault        # Unlock with password
desktop-start       # Start all installed desktops
desktop-start vnc   # Start only the KasmVNC desktop
desktop-stop        # Stop all desktops
desktop-stop selkies # Stop only the Selkies desktop
backup-init         # Initialize restic repo (auto-runs on first boot if configured)
```

## Authentication

All endpoints share the same credentials (`user` / your `PASSWORD`):

- **Code-server** and **Selkies** (`/desktop/`) are gated by Caddy (realm `restricted`).
- **KasmVNC** (`/vnc/`) uses its own native auth (realm `Websockify`).

Because KasmVNC owns its auth realm, the browser prompts for it separately from
Selkies — same username/password, just one extra prompt. This is required:
browsers do not replay Caddy's basic-auth credentials to the VNC WebSocket
handshake, so KasmVNC must authenticate the WebSocket itself.

## Remote Access (Cloudflare Tunnel)

Expose the workspace to the internet via Cloudflare Tunnel (zero inbound ports):

```bash
# One-time setup (run on host machine, not inside container)
./scripts/setup-cloudflared.sh

# Access from anywhere:
# https://workspace.cicd.dpdns.org
# https://workspace.cicd.dpdns.org/desktop/   (Selkies)
# https://workspace.cicd.dpdns.org/vnc/        (KasmVNC)
```

The tunnel auto-starts on macOS login via LaunchAgent. No port forwarding needed.

## Environment Variables (.env)

| Variable | Default | Description |
|----------|---------|-------------|
| PASSWORD | (auto-generated) | Unified password for code-server, desktop, and vault |
| CLASH_SUBSCRIPTION_URL | (required) | Clash proxy subscription URL |
| DESKTOP_RESOLUTION | 1920x1080 | KasmVNC (`/vnc/`) geometry. Selkies (`/desktop/`) ignores this — it auto-fits the browser window (adaptive). |
| DESKTOP_AUDIO | 1 | Enable PulseAudio (0 to disable) |
| TUNNEL_HOST_PORT | 18080 | Host port mapping |
| RESTIC_REPOSITORY | (empty) | S3 backup repo URL. Presence enables realtime backup. |
| RESTIC_PASSWORD | (empty) | Backup repo encryption password |
| AWS_ACCESS_KEY_ID | (empty) | S3 credentials for backup |
| AWS_SECRET_ACCESS_KEY | (empty) | S3 credentials for backup |
