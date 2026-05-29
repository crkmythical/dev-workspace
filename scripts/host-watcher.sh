#!/usr/bin/env bash
# host-watcher — Monitors container for self-destruct completion and triggers
# host-side cleanup (destroy.sh --force --paranoid).
#
# Usage:
#   host-watcher.sh              # daemon mode (blocks, watches docker events)
#   host-watcher.sh --once       # single check then exit
#   host-watcher.sh --install    # install macOS LaunchAgent
#   host-watcher.sh --uninstall  # remove macOS LaunchAgent
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
CONTAINER="dev-workspace"
PLIST_NAME="com.dev-workspace.host-watcher"
PLIST_PATH="$HOME/Library/LaunchAgents/${PLIST_NAME}.plist"

command -v docker &>/dev/null || { echo "docker not found"; exit 1; }

check_destruct() {
  # Find the destruct-state volume (handles any docker compose project name)
  local vol
  vol=$(docker volume ls --filter "name=destruct-state" --format '{{.Name}}' | head -1)
  if [ -z "$vol" ]; then
    return 1
  fi
  docker run --rm -v "${vol}:/s" alpine test -f /s/completed 2>/dev/null
}

do_cleanup() {
  echo "[$(date -Iseconds)] Self-destruct confirmed. Running host cleanup..."
  bash "${SCRIPT_DIR}/destroy.sh" --force --paranoid
}

daemon_mode() {
  echo "[$(date -Iseconds)] host-watcher started (container: $CONTAINER)"
  docker events --filter "container=${CONTAINER}" --filter "event=die" | while read -r _line; do
    if check_destruct; then
      do_cleanup
      break
    fi
  done
}

once_mode() {
  if check_destruct; then
    do_cleanup
  else
    echo "No self-destruct marker found."
  fi
}

install_agent() {
  mkdir -p "$HOME/Library/LaunchAgents"
  cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${PLIST_NAME}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${SCRIPT_DIR}/host-watcher.sh</string>
  </array>
  <key>WorkingDirectory</key>
  <string>${PROJECT_DIR}</string>
  <key>KeepAlive</key>
  <true/>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/dev-workspace-watcher.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/dev-workspace-watcher.log</string>
</dict>
</plist>
EOF
  launchctl load "$PLIST_PATH"
  echo "Installed and loaded: $PLIST_PATH"
}

uninstall_agent() {
  launchctl unload "$PLIST_PATH" 2>/dev/null || true
  rm -f "$PLIST_PATH"
  echo "Uninstalled: $PLIST_PATH"
}

case "${1:-}" in
  --once)     once_mode ;;
  --install)  install_agent ;;
  --uninstall) uninstall_agent ;;
  *)          daemon_mode ;;
esac
