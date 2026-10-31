#!/usr/bin/env bash
# Host Docker Desktop setup and macOS power management configuration
set -euo pipefail

echo "=== Docker Desktop Setup ==="

# Verify Docker Desktop is installed and running
if ! command -v docker &>/dev/null; then
  echo "✗ Docker not found. Install Docker Desktop: https://www.docker.com/products/docker-desktop/"
  exit 1
fi

if ! docker info &>/dev/null 2>&1; then
  echo "✗ Docker daemon not running. Start Docker Desktop and retry."
  exit 1
fi
echo "✓ Docker Desktop running"

# Check memory allocation
DOCKER_MEM=$(docker info --format '{{.MemTotal}}' 2>/dev/null || echo "0")
DOCKER_MEM_GB=$(( DOCKER_MEM / 1073741824 ))
if [[ "$DOCKER_MEM_GB" -lt 12 ]]; then
  echo "⚠ Docker memory: ${DOCKER_MEM_GB}GB — recommend ≥12GB"
  echo "  → Docker Desktop → Settings → Resources → Memory"
else
  echo "✓ Docker memory: ${DOCKER_MEM_GB}GB"
fi

# Check disk space
DOCKER_ROOT=$(docker info --format '{{.DockerRootDir}}' 2>/dev/null || echo "/var/lib/docker")
if command -v df &>/dev/null; then
  DISK_AVAIL=$(df -BG "$DOCKER_ROOT" 2>/dev/null | awk 'NR==2{print $4}' | tr -d 'G')
  if [[ -n "$DISK_AVAIL" ]] && [[ "$DISK_AVAIL" -lt 100 ]]; then
    echo "⚠ Available disk: ${DISK_AVAIL}GB — recommend ≥100GB"
    echo "  → Docker Desktop → Settings → Resources → Disk image size"
  else
    echo "✓ Disk space: ${DISK_AVAIL:-unknown}GB available"
  fi
fi

# Recommend disabling auto-update
echo ""
echo "Recommendation: Disable Docker Desktop auto-update to avoid surprise restarts."
echo "  → Docker Desktop → Settings → Software updates → uncheck auto-update"

# macOS power management (pmset)
if [[ "$(uname)" == "Darwin" ]]; then
  echo ""
  echo "=== macOS Power Management ==="
  echo "Configuring pmset for always-on server operation..."

  # Prevent sleep on AC power
  sudo pmset -c sleep 0 displaysleep 0 disksleep 0 2>/dev/null && \
    echo "✓ System sleep disabled on AC power" || \
    echo "⚠ Could not set pmset (need sudo)"

  # Wake on network access
  sudo pmset -c womp 1 2>/dev/null && \
    echo "✓ Wake-on-network enabled" || true

  # Auto-restart on power loss
  sudo pmset -c autorestart 1 2>/dev/null && \
    echo "✓ Auto-restart on power loss enabled" || true

  echo ""
  echo "Rationale: The workspace container must stay running for tunnel access."
  echo "These settings prevent the Mac from sleeping and auto-recover from power events."
fi

echo ""
echo "=== Docker setup complete ==="
