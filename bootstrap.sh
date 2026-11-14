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
cd "$SCRIPT_DIR"

RESTORE_DIR=""
cleanup() {
  if [[ -n "$RESTORE_DIR" && -d "$RESTORE_DIR" ]]; then
    chmod -R u+w "$RESTORE_DIR" 2>/dev/null || true
    rm -r "$RESTORE_DIR"
  fi
}
trap cleanup EXIT

# 4. Handle vault repo. Migration fetches the ciphertext now; it is put into the vault volume in
# step 7, before the container first starts (a start with an empty volume would create a NEW vault).
if [[ "$MODE" == "migrate" ]]; then
  [[ -z "$VAULT_REPO" ]] && read -rp "Vault repo URL: " VAULT_REPO
  mkdir -p "$SCRIPT_DIR/.poc-tmp"
  RESTORE_DIR="$(mktemp -d "$SCRIPT_DIR/.poc-tmp/vault-import.XXXXXX")"
  git clone "$VAULT_REPO" "$RESTORE_DIR"
elif [[ "$MODE" == "init" ]]; then
  echo "Fresh mode: the entrypoint will initialize and unlock the vault on first start"
fi

# 5. Create .env from example if not exists
if [[ ! -f .env ]]; then
  install -m 600 .env.example .env
  echo "Created private .env from .env.example — please edit with your values"
  "${EDITOR:-vi}" .env
fi
chmod 600 .env

# 6. Image acquisition
# (stderr stays visible: with it hidden a failed build, e.g. a missing buildx plugin or an apt
# mirror problem, left no trace of its cause.)
if ! docker compose build; then
  if [[ -f workspace.tar ]]; then
    echo "Build failed, importing from workspace.tar..."
    docker load -i workspace.tar
  else
    echo "ERROR: docker compose build failed and no workspace.tar found"; exit 1
  fi
fi

# 7. Setup scripts
echo "Running host setup..."
bash deploy/scripts/setup-docker.sh || true
bash deploy/scripts/setup-cloudflared.sh || true

# 7.5. Migration: put the fetched ciphertext into the (still empty) vault volume. A throwaway
# container of the workspace image mounts the same volumes as the real one; it refuses to touch a
# volume that already holds a vault. The host copy is removed afterwards: it is not used again.
if [[ "$MODE" == "migrate" ]]; then
  echo "Restoring the vault ciphertext into the vault volume..."
  docker compose run --rm --no-deps --entrypoint bash \
    -v "$RESTORE_DIR:/restore:ro" workspace -c '
      set -e
      if [ -e /vault/cipher/gocryptfs.conf ]; then
        echo "The vault volume already holds a vault; not overwriting it." >&2
        exit 1
      fi
      [ -e /restore/gocryptfs.conf ] || { echo "The cloned repository has no gocryptfs.conf: not a vault." >&2; exit 1; }
      cp -a /restore/. /vault/cipher/
      chmod 0700 /vault/cipher
      echo "Restored $(ls /vault/cipher | wc -l) entries."
    ' || { echo "ERROR: could not restore the vault; nothing was started." >&2; exit 1; }
  cleanup
  RESTORE_DIR=""
fi

# 8. Start
docker compose up -d

# 8.5. Offer host-watcher installation
read -rp "Install host-watcher (auto-cleanup on self-destruct)? [y/N]: " watcher
if [[ "${watcher:-}" =~ ^[Yy]$ ]]; then
  bash deploy/scripts/host-watcher.sh --install
fi

# 9. Instructions
echo ""
echo "=== Workspace started ==="
echo "Run: docker compose exec workspace doctor"
