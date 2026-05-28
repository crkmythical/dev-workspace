#!/usr/bin/env bash
# One-command workspace destruction
# Usage:
#   ./destroy.sh              # Interactive confirmation, standard cleanup
#   ./destroy.sh --force      # Skip confirmation
#   ./destroy.sh --remote     # Also delete GitHub vault repo
#   ./destroy.sh --paranoid   # Deep clean: shell history, cloudflared creds, DNS cache
set -euo pipefail

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
docker volume rm vault-data clash-config 2>/dev/null || true
docker image rm dev-workspace-workspace 2>/dev/null || true
docker image rm dev-workspace 2>/dev/null || true

# Remove tunnel
cloudflared tunnel delete dev-workspace 2>/dev/null || true

# Remote vault deletion
if [[ "$REMOTE" == "true" ]]; then
  VAULT_REPO=$(grep VAULT_GIT_REPO .env 2>/dev/null | cut -d= -f2)
  if [[ -n "$VAULT_REPO" ]]; then
    echo "Deleting remote vault repo..."
    gh repo delete "$VAULT_REPO" --yes 2>/dev/null || echo "Could not delete remote repo"
  fi
fi

# Paranoid mode: deep forensic cleanup
if [[ "$PARANOID" == "true" ]]; then
  echo "Paranoid mode: deep cleanup..."

  # Remove cloudflared credentials and config
  rm -rf ~/.cloudflared/ 2>/dev/null || true
  echo "  ✗ ~/.cloudflared/ removed"

  # Clean shell history of related commands
  for histfile in ~/.zsh_history ~/.bash_history; do
    if [[ -f "$histfile" ]]; then
      sed -i '' '/dev-workspace\|vault\|clash\|cloudflared\|gocryptfs\|unlock-vault\|init-vault/d' "$histfile" 2>/dev/null || true
    fi
  done
  echo "  ✗ Shell history scrubbed"

  # Flush DNS cache
  sudo dscacheutil -flushcache 2>/dev/null || true
  sudo killall -HUP mDNSResponder 2>/dev/null || true
  echo "  ✗ DNS cache flushed"

  # Clear Docker build cache
  docker builder prune -af 2>/dev/null || true
  echo "  ✗ Docker build cache cleared"

  # Remove any Docker Desktop logs mentioning workspace
  find ~/Library/Containers/com.docker.docker/Data/log -name "*.log" -exec sed -i '' '/dev-workspace/d' {} \; 2>/dev/null || true
  echo "  ✗ Docker logs scrubbed"
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
