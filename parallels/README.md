# Parallels VM Proxy Variant

## Overview

An alternative to the Docker-based client container for environments where Docker is unavailable or undesirable. Runs Clash inside a Parallels Desktop Ubuntu 22.04 VM with shared networking, exposing proxy ports to the macOS host.

## Trade-offs vs Client Container

| Aspect | Client Container | Parallels VM |
|--------|-----------------|--------------|
| Startup time | ~2s | ~30s (VM boot) |
| Resource overhead | Minimal (~50MB) | Higher (~512MB+ RAM) |
| Isolation | Process-level (Docker) | Full VM isolation |
| Persistence | Volume-backed | Full disk |
| Requires Docker | Yes | No |
| Requires Parallels | No | Yes (licensed) |
| Network config | Docker port mapping | Shared/bridged networking |
| Auto-start | Docker restart policy | Parallels auto-start |

## When to Use

- Docker Desktop is unavailable or restricted by policy
- You need full VM-level isolation (e.g., running untrusted proxy configs)
- You want the proxy to survive Docker Desktop restarts/updates
- You need to run additional services alongside Clash in the VM

## Setup

1. Create a new Ubuntu 22.04 VM in Parallels Desktop
2. Configure shared networking (default) — the VM gets an IP on the host's subnet
3. Copy and run the provisioning script:

```bash
scp provision.sh user@<vm-ip>:/tmp/
ssh user@<vm-ip> 'sudo bash /tmp/provision.sh'
```

4. Note the VM's IP address:

```bash
ssh user@<vm-ip> 'hostname -I'
```

5. On macOS, set proxy environment:

```bash
export http_proxy=http://<vm-ip>:7890
export https_proxy=http://<vm-ip>:7890
export all_proxy=socks5://<vm-ip>:7891
```

## Configuration

Edit `/etc/clash/config.yaml` inside the VM to add your subscription URL or custom proxy rules, then restart:

```bash
sudo systemctl restart clash
```

## Auto-Start

Configure Parallels to start the VM on login:
- Parallels Desktop → VM Settings → Startup and Shutdown → Start automatically

The systemd service ensures Clash starts with the VM.
