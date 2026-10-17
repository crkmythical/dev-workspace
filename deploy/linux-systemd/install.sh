#!/usr/bin/env bash
# Install the host-side services as systemd USER units (Linux). The macOS equivalents are
# the LaunchAgents written by scripts/setup-cloudflared.sh and scripts/host-watcher.sh.
#
#   install.sh --cloudflared     keep the Cloudflare Tunnel running
#                                (run scripts/setup-cloudflared.sh --hostname ... first)
#   install.sh --host-watcher    watch for a self-destruct and clean up this host
#   install.sh --uninstall       remove both units
#   --dry-run                    print the units instead of installing them
#
# Checked under real systemd (Ubuntu 24.04 VM, stand-in docker/cloudflared), not on real
# hardware or against a real tunnel; see README.md. Read the units (--dry-run) before relying on them.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
# shellcheck source=../../scripts/lib/env.sh
source "$PROJECT_DIR/scripts/lib/env.sh"

UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
CLOUDFLARED_UNIT="cloudflared-dev-workspace.service"
WATCHER_UNIT="dev-workspace-host-watcher.service"

DO_CLOUDFLARED=false
DO_WATCHER=false
DO_UNINSTALL=false
DRY_RUN=false
for arg in "$@"; do
  case "$arg" in
    --cloudflared) DO_CLOUDFLARED=true ;;
    --host-watcher) DO_WATCHER=true ;;
    --uninstall) DO_UNINSTALL=true ;;
    --dry-run) DRY_RUN=true ;;
    *) echo "Unknown option: $arg" >&2; exit 2 ;;
  esac
done

# shellcheck disable=SC2329  # called by name, through install_unit
render_cloudflared_unit() {
  local bin tunnel
  bin="$(command -v cloudflared || echo /usr/local/bin/cloudflared)"
  tunnel="$(env_value TUNNEL_NAME dev-workspace)"
  cat <<UNIT
[Unit]
Description=Cloudflare Tunnel for dev-workspace
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=${bin} tunnel run ${tunnel}
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
UNIT
}

# shellcheck disable=SC2329  # called by name, through install_unit
render_watcher_unit() {
  cat <<UNIT
[Unit]
Description=dev-workspace host watcher (cleans up this host after a self-destruct)
After=network-online.target

[Service]
WorkingDirectory=${PROJECT_DIR}
ExecStart=/bin/bash ${PROJECT_DIR}/scripts/host-watcher.sh
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
UNIT
}

install_unit() {
  local name="$1" render="$2"
  if [[ "$DRY_RUN" == "true" ]]; then
    echo "# ${UNIT_DIR}/${name}"
    "$render"
    echo
    return 0
  fi
  mkdir -p "$UNIT_DIR"
  "$render" > "$UNIT_DIR/$name"
  systemctl --user daemon-reload
  systemctl --user enable --now "$name"
  echo "✓ ${name} installed and started"
}

if [[ "$DO_UNINSTALL" == "true" ]]; then
  for name in "$CLOUDFLARED_UNIT" "$WATCHER_UNIT"; do
    if [[ "$DRY_RUN" == "true" ]]; then
      echo "would remove ${UNIT_DIR}/${name}"
      continue
    fi
    systemctl --user disable --now "$name" 2>/dev/null || true
    rm -f "$UNIT_DIR/$name"
  done
  [[ "$DRY_RUN" == "true" ]] || systemctl --user daemon-reload
  exit 0
fi

if [[ "$DO_CLOUDFLARED" != "true" && "$DO_WATCHER" != "true" ]]; then
  echo "Nothing to do: pass --cloudflared and/or --host-watcher (or --uninstall)." >&2
  exit 2
fi

[[ "$DO_CLOUDFLARED" == "true" ]] && install_unit "$CLOUDFLARED_UNIT" render_cloudflared_unit
[[ "$DO_WATCHER" == "true" ]] && install_unit "$WATCHER_UNIT" render_watcher_unit

if [[ "$DRY_RUN" != "true" ]]; then
  # User services only run while a session exists unless lingering is enabled.
  loginctl enable-linger "$USER" 2>/dev/null \
    || echo "Hint: run 'sudo loginctl enable-linger $USER' so the services start at boot without a login."
fi
exit 0
