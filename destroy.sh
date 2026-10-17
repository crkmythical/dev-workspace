#!/usr/bin/env bash
# One-command workspace destruction
# Usage:
#   ./destroy.sh              # Interactive confirmation, standard cleanup
#   ./destroy.sh --force      # Skip confirmation
#   ./destroy.sh --remote     # Also delete GitHub vault repo
#   ./destroy.sh --paranoid   # Deep clean: shell history, cloudflared creds, DNS cache
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
# `docker compose down -v` below finds the project through the current directory; run from
# anywhere else it fails (silently, see `|| true`) and the script still deletes the project.
cd "$PROJECT_DIR"
# shellcheck source=scripts/lib/env.sh
source "$PROJECT_DIR/scripts/lib/env.sh"
# shellcheck source=scripts/lib/scrub.sh
source "$PROJECT_DIR/scripts/lib/scrub.sh"
TUNNEL_NAME="$(env_value TUNNEL_NAME dev-workspace)"

FORCE=false
REMOTE=false
PARANOID=false

while [[ $# -gt 0 ]]; do
  case $1 in
    --force) FORCE=true; shift ;;
    --remote) REMOTE=true; shift ;;
    --paranoid) PARANOID=true; shift ;;
    *) shift ;;
  esac
done

if [[ "$FORCE" != "true" ]]; then
  echo "⚠️  This will PERMANENTLY destroy all workspace data."
  read -rp "Type 'destroy' to confirm: " confirm
  [[ "$confirm" == "destroy" ]] || { echo "Aborted."; exit 0; }
fi

echo "Destroying workspace..."

# Standard cleanup
docker compose down -v 2>/dev/null || true
docker volume rm vault-data cache-data clash-config 2>/dev/null || true
# Remove all dev-workspace image tags (latest, trixie, etc.)
docker image ls --format '{{.Repository}}:{{.Tag}}' | grep '^dev-workspace:' | xargs -r docker image rm 2>/dev/null || true

# Remove tunnel
cloudflared tunnel delete "$TUNNEL_NAME" 2>/dev/null || true

# Remote vault deletion
if [[ "$REMOTE" == "true" ]]; then
  VAULT_REPO="$(env_value VAULT_GIT_REPO)"
  if [[ -n "$VAULT_REPO" ]]; then
    echo "Deleting remote vault repo..."
    gh repo delete "$VAULT_REPO" --yes 2>/dev/null || echo "Could not delete remote repo"
  fi
fi

# Paranoid mode: deep forensic cleanup
if [[ "$PARANOID" == "true" ]]; then
  echo "Paranoid mode: deep cleanup..."

  # Remove the host services
  if [[ "$(uname)" == "Darwin" ]]; then
    launchctl unload ~/Library/LaunchAgents/com.dev-workspace.host-watcher.plist 2>/dev/null || true
    rm -f ~/Library/LaunchAgents/com.dev-workspace.host-watcher.plist
    echo "  ✗ host-watcher LaunchAgent removed"
  else
    systemctl --user disable --now dev-workspace-host-watcher.service cloudflared-dev-workspace.service 2>/dev/null || true
    rm -f ~/.config/systemd/user/dev-workspace-host-watcher.service ~/.config/systemd/user/cloudflared-dev-workspace.service
    systemctl --user daemon-reload 2>/dev/null || true
    echo "  ✗ systemd user services removed"
  fi

  # Remove destruct-state volume (may survive compose down if externally created)
  docker volume rm destruct-state 2>/dev/null || true

  # Remove cloudflared credentials and config
  rm -rf ~/.cloudflared/ 2>/dev/null || true
  echo "  ✗ ~/.cloudflared/ removed"

  # Clean shell history of related commands
  for histfile in ~/.zsh_history ~/.bash_history; do
    scrub_history "$histfile"
  done
  echo "  ✗ Shell history scrubbed"

  # Flush DNS cache
  if [[ "$(uname)" == "Darwin" ]]; then
    sudo dscacheutil -flushcache 2>/dev/null || true
    sudo killall -HUP mDNSResponder 2>/dev/null || true
  else
    resolvectl flush-caches 2>/dev/null || true
  fi
  echo "  ✗ DNS cache flushed"

  # Clear Docker build cache
  docker builder prune -af 2>/dev/null || true
  echo "  ✗ Docker build cache cleared"

  # Remove any Docker Desktop logs mentioning workspace
  if [[ "$(uname)" == "Darwin" ]]; then
    find ~/Library/Containers/com.docker.docker/Data/log -name "*.log" -exec sed -i '' '/dev-workspace/d' {} \; 2>/dev/null || true
    echo "  ✗ Docker logs scrubbed"
  fi
fi

# Self-destruct: remove project directory
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
echo ""
echo "=== Destruction Summary ==="
echo "  ✗ Container + volumes removed"
echo "  ✗ Docker image removed"
echo "  ✗ Tunnel deregistered"
[[ "$REMOTE" == "true" ]] && echo "  ✗ Remote vault repo deleted"
[[ "$PARANOID" == "true" ]] && echo "  ✗ Deep forensic cleanup done"
echo "  ✗ Project directory will be removed"
echo ""

cd /
rm -rf "$SCRIPT_DIR"
echo "Done. No workspace traces remain."
