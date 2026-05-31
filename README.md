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
# Desktop: http://localhost:18080/desktop/
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

- **Remote Desktop** — Selkies H.264 streaming (1920x1080, 30fps)
- **Code Server** — VS Code in browser
- **Encrypted Vault** — gocryptfs auto-init/unlock
- **Proxy** — Clash global mode (all traffic through proxy)
- **Audio** — PulseAudio null-sink for desktop audio streaming
- **Base** — Kali Linux (pentest tools available via apt)

## Commands (inside container)

```bash
doctor          # Health check
noproxy <cmd>   # Run command bypassing proxy (direct network)
lock-vault      # Lock the encrypted workspace
unlock-vault    # Unlock with password
desktop-stop    # Stop the desktop session
desktop-start   # Start the desktop session
```

## Build Options

```bash
# Kali base (default)
docker compose build

# Debian trixie base
docker compose build --build-arg BASE_IMAGE=debian:trixie-slim
```

## Remote Access (Cloudflare Tunnel)

Expose the workspace to the internet via Cloudflare Tunnel (zero inbound ports):

```bash
# One-time setup (run on host machine, not inside container)
./scripts/setup-cloudflared.sh

# Access from anywhere:
# https://workspace.cicd.dpdns.org
# https://workspace.cicd.dpdns.org/desktop/
```

The tunnel auto-starts on macOS login via LaunchAgent. No port forwarding needed.

## Environment Variables (.env)

| Variable | Default | Description |
|----------|---------|-------------|
| PASSWORD | (auto-generated) | Unified password for code-server, desktop, and vault |
| CLASH_SUBSCRIPTION_URL | (required) | Clash proxy subscription URL |
| DESKTOP_RESOLUTION | 1920x1080 | Desktop resolution |
| DESKTOP_AUDIO | 1 | Enable PulseAudio (0 to disable) |
| TUNNEL_HOST_PORT | 18080 | Host port mapping |
