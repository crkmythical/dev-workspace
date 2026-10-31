#!/usr/bin/env bash
# host-watcher — Monitors container for self-destruct completion and triggers
# host-side cleanup (destroy.sh --force --paranoid).
#
# Usage:
#   host-watcher.sh              # daemon mode (blocks, watches docker events)
#   host-watcher.sh --once       # single check then exit
#   host-watcher.sh --install    # install macOS LaunchAgent (Linux: deploy/linux-systemd/install.sh)
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

# destroy.sh --paranoid removes the host services, this watcher's included (launchctl unload,
# systemctl disable --now), and the service manager then kills everything the watcher started:
# destroy.sh with it, partway through. The credentials, shell history, DNS cache and the project
# directory were never reached. Seen on macOS (launchd) and on Linux (systemd). So the cleanup runs
# OUTSIDE the watcher's service: in a new session on macOS, in a transient unit on Linux.
run_outside_service() {
  if [[ "$(uname)" == "Darwin" ]]; then
    if command -v perl &>/dev/null; then
      perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' "$@" &
      wait $!
      return
    fi
  elif command -v systemd-run &>/dev/null && systemctl --user show-environment &>/dev/null; then
    systemd-run --user --quiet --collect --pipe --wait \
      --working-directory="$PROJECT_DIR" --setenv=PATH="$PATH" -- "$@"
    return
  fi
  "$@"
}

do_cleanup() {
  echo "[$(date -Iseconds)] Self-destruct confirmed. Running host cleanup..."
  run_outside_service bash "${PROJECT_DIR}/destroy.sh" --force --paranoid
}

daemon_mode() {
  echo "[$(date -Iseconds)] host-watcher started (container: $CONTAINER)"
  # Read events through a FIFO with the producer backgrounded, so it can be stopped cleanly.
  # A plain `docker events | while read` leaves the producer blocked on the closed pipe after we
  # break out (an orphan until its next write hits SIGPIPE); killing it by PID avoids that.
  local fifo producer
  fifo="$(mktemp -u)"
  mkfifo "$fifo"
  docker events --filter "container=${CONTAINER}" --filter "event=die" > "$fifo" &
  producer=$!
  while read -r _line; do
    if check_destruct; then
      do_cleanup
      break
    fi
  done < "$fifo"
  kill "$producer" 2>/dev/null || true
  rm -f "$fifo"
}

once_mode() {
  if check_destruct; then
    do_cleanup
  else
    echo "No self-destruct marker found."
  fi
}

install_agent() {
  if [[ "$(uname)" != "Darwin" ]]; then
    echo "LaunchAgents are macOS only. On Linux: bash deploy/linux-systemd/install.sh --host-watcher" >&2
    exit 1
  fi
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
