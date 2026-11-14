#!/usr/bin/env bash
# One-command workspace destruction
# Usage:
#   ./destroy.sh              # Interactive confirmation, standard cleanup
#   ./destroy.sh --force      # Skip confirmation
#   ./destroy.sh --remote     # Also delete GitHub vault repo
#   ./destroy.sh --paranoid   # Deep clean: shell history, cloudflared creds, DNS cache
#   ./destroy.sh --delete-project   # Also delete this checkout (the directory holding this script)
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
# `docker compose down -v` below finds the project through the current directory; run from
# anywhere else it fails (silently, see `|| true`) and the script still deletes the project.
cd "$PROJECT_DIR"
# shellcheck source=deploy/scripts/lib/env.sh
source "$PROJECT_DIR/deploy/scripts/lib/env.sh"
# shellcheck source=deploy/scripts/lib/scrub.sh
source "$PROJECT_DIR/deploy/scripts/lib/scrub.sh"
TUNNEL_NAME="$(env_value TUNNEL_NAME dev-workspace)"

FORCE=false
REMOTE=false
PARANOID=false
DELETE_PROJECT=false

while [[ $# -gt 0 ]]; do
  case $1 in
    --force) FORCE=true; shift ;;
    --remote) REMOTE=true; shift ;;
    --paranoid) PARANOID=true; shift ;;
    --delete-project) DELETE_PROJECT=true; shift ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
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
# `down -v` removes the project's volumes; this is a backstop for any that survived it. It is
# limited to THIS compose project's volumes (their label), by name too: matching on the volume names
# alone would also delete another project's vault-data.
project="$(docker compose config --format json 2>/dev/null | jq -r '.name // empty' 2>/dev/null || true)"
[[ -n "$project" ]] || project="$(basename "$PROJECT_DIR" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9_-')"
docker volume ls -q --filter "label=com.docker.compose.project=$project" \
  | grep -E '(^|_)(vault-data|cache-data|clash-config|destruct-state|audit-data)$' \
  | xargs -r docker volume rm 2>/dev/null || true
# Remove all dev-workspace image tags (latest, trixie, etc.)
docker image ls --format '{{.Repository}}:{{.Tag}}' | grep '^dev-workspace:' | xargs -r docker image rm 2>/dev/null || true

# Remove tunnel
tunnel_deleted=false
if command -v cloudflared >/dev/null 2>&1 && cloudflared tunnel delete "$TUNNEL_NAME" >/dev/null 2>&1; then
  tunnel_deleted=true
fi

# Remote vault deletion
remote_deleted=false
if [[ "$REMOTE" == "true" ]]; then
  VAULT_REPO="$(env_value VAULT_GIT_REPO)"
  if [[ -n "$VAULT_REPO" ]]; then
    echo "Deleting remote vault repo..."
    if gh repo delete "$VAULT_REPO" --yes 2>/dev/null; then
      remote_deleted=true
    else
      echo "Could not delete remote repo"
    fi
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
# What is gone is asked of Docker, not assumed: the cleanup above ignores errors on purpose (a
# half-removed workspace should still be cleaned as far as it can), so a success line has to be
# earned here. "✗" marks something destroyed, "!" something that is still there.
problems=0
left() { echo "  ! $1"; problems=$((problems + 1)); }
if docker info >/dev/null 2>&1; then
  still="$( { docker ps -aq --filter "label=com.docker.compose.project=$project"; docker volume ls -q --filter "label=com.docker.compose.project=$project"; } | tr '\n' ' ')"
  if [[ -z "${still// /}" ]]; then
    echo "  ✗ Containers + volumes removed"
  else
    left "Containers or volumes of project '$project' are still there: $still"
  fi
  images="$(docker image ls --format '{{.Repository}}:{{.Tag}}' | grep '^dev-workspace:' | tr '\n' ' ' || true)"
  if [[ -z "${images// /}" ]]; then
    echo "  ✗ Docker images removed"
  else
    left "Docker images are still there: $images"
  fi
else
  left "Docker is not reachable: the containers, volumes and images were NOT removed"
fi
if [[ "$tunnel_deleted" == "true" ]]; then
  echo "  ✗ Tunnel '$TUNNEL_NAME' deleted"
else
  echo "  - Tunnel '$TUNNEL_NAME' not deleted (already gone, or cloudflared is not available)"
fi
if [[ "$REMOTE" == "true" ]]; then
  if [[ "$remote_deleted" == "true" ]]; then
    echo "  ✗ Remote vault repo deleted"
  else
    left "The remote vault repo was NOT deleted (no VAULT_GIT_REPO, or gh could not)"
  fi
fi
[[ "$PARANOID" == "true" ]] && echo "  ✗ Deep forensic cleanup done"
echo ""

if [[ "$problems" -gt 0 ]]; then
  echo "Not everything was removed; the project directory ($SCRIPT_DIR) is kept so this can be run again."
  exit 1
fi
if [[ "$DELETE_PROJECT" != "true" ]]; then
  echo "The project directory ($SCRIPT_DIR) was left in place; add --delete-project to remove it too."
  exit 0
fi
echo "  ✗ Project directory will be removed"
cd /
# Guard the self-delete: $SCRIPT_DIR is derived from $0, and a mis-resolved path must never turn
# this into `rm -rf /` or `rm -rf $HOME`. Refuse empty/root/home, and require the directory to
# still look like the project (its docker-compose.yml survives the volume/image cleanup above).
case "$SCRIPT_DIR" in
  "" | "/" | "$HOME" | "$HOME/")
    echo "Refusing to remove '$SCRIPT_DIR' (not a safe target)."; exit 1 ;;
esac
if [[ ! -f "$SCRIPT_DIR/docker-compose.yml" ]]; then
  echo "Refusing to remove '$SCRIPT_DIR': does not look like the project directory."; exit 1
fi
rm -rf "$SCRIPT_DIR"
echo "Done. No workspace traces remain."
