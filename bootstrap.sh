#!/usr/bin/env bash
# One-command workspace setup/migration
set -euo pipefail

# 1. Verify prerequisites
for cmd in docker git curl; do
  command -v "$cmd" &>/dev/null || { echo "ERROR: $cmd not found"; exit 1; }
done

# 2. Mode detection
MODE="migrate"
VAULT_REPO=""
while [[ $# -gt 0 ]]; do
  case $1 in
    --init) MODE="init"; shift ;;
    --vault-repo) VAULT_REPO="$2"; shift 2 ;;
    *) echo "Unknown: $1"; exit 1 ;;
  esac
done

# 3. Clone config repo (if not already in it)
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ ! -f "$SCRIPT_DIR/docker-compose.yml" ]]; then
  echo "ERROR: Run this script from the dev-workspace project directory"; exit 1
fi

# 4. Handle vault repo
if [[ "$MODE" == "migrate" ]]; then
  [[ -z "$VAULT_REPO" ]] && read -rp "Vault repo URL: " VAULT_REPO
  git clone "$VAULT_REPO" vault-cipher
elif [[ "$MODE" == "init" ]]; then
  mkdir -p vault-cipher
  echo "Fresh mode: run 'init-vault' after container starts"
fi

# 5. Create .env from example if not exists
if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "Created .env from .env.example — please edit with your values"
  ${EDITOR:-nano} .env
fi

# 6. Image acquisition
if ! docker compose build 2>/dev/null; then
  if [[ -f workspace.tar ]]; then
    echo "Build failed, importing from workspace.tar..."
    docker load -i workspace.tar
  else
    echo "ERROR: docker compose build failed and no workspace.tar found"; exit 1
  fi
fi

# 7. Setup scripts
echo "Running host setup..."
bash scripts/setup-docker.sh || true
bash scripts/setup-cloudflared.sh || true

# 8. Start
docker compose up -d

# 8.5. Offer host-watcher installation
read -rp "Install host-watcher (auto-cleanup on self-destruct)? [y/N]: " watcher
if [[ "${watcher:-}" =~ ^[Yy]$ ]]; then
  bash scripts/host-watcher.sh --install
fi

# 9. Instructions
echo ""
echo "=== Workspace started ==="
if [[ "$MODE" == "init" ]]; then
  echo "Run: docker exec -it dev-workspace init-vault"
else
  echo "Run: docker exec -it dev-workspace unlock-vault"
fi
