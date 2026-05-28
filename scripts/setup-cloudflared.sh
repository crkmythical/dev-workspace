#!/usr/bin/env bash
# Cloudflare Tunnel — fully automated setup with sensible defaults
# Usage:
#   ./setup-cloudflared.sh                    # Use defaults (dev-workspace, workspace.cicd.dpdns.org)
#   ./setup-cloudflared.sh --name myname      # Custom tunnel name
#   ./setup-cloudflared.sh --hostname x.y.z   # Custom hostname
set -euo pipefail

# Defaults (change these for your setup)
TUNNEL_NAME="${TUNNEL_NAME:-dev-workspace}"
TUNNEL_HOSTNAME="${TUNNEL_HOSTNAME:-workspace.cicd.dpdns.org}"
TUNNEL_PORT="${TUNNEL_HOST_PORT:-18080}"
CLOUDFLARED_DIR="$HOME/.cloudflared"

# Parse args
while [[ $# -gt 0 ]]; do
  case $1 in
    --name) TUNNEL_NAME="$2"; shift 2 ;;
    --hostname) TUNNEL_HOSTNAME="$2"; shift 2 ;;
    --port) TUNNEL_PORT="$2"; shift 2 ;;
    *) shift ;;
  esac
done

echo "=== Cloudflare Tunnel Setup ==="
echo "  Tunnel:   $TUNNEL_NAME"
echo "  Hostname: $TUNNEL_HOSTNAME"
echo "  Target:   http://localhost:$TUNNEL_PORT"
echo ""

# 1. Check cloudflared
if ! command -v cloudflared &>/dev/null; then
  echo "Installing cloudflared..."
  if [[ "$(uname)" == "Darwin" ]]; then
    brew install cloudflared 2>/dev/null || {
      curl -fsSL https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz | tar xz -C /usr/local/bin/
    }
  else
    curl -fsSL https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64 -o /usr/local/bin/cloudflared
    chmod +x /usr/local/bin/cloudflared
  fi
fi
echo "✓ cloudflared $(cloudflared --version 2>&1 | head -1)"

# 2. Login (idempotent)
if ! cloudflared tunnel list &>/dev/null 2>&1; then
  echo ""
  echo "Opening browser for Cloudflare login..."
  cloudflared tunnel login
fi
echo "✓ Authenticated"

# 3. Create tunnel (idempotent)
if cloudflared tunnel info "$TUNNEL_NAME" &>/dev/null 2>&1; then
  echo "✓ Tunnel '$TUNNEL_NAME' exists"
else
  cloudflared tunnel create "$TUNNEL_NAME"
  echo "✓ Tunnel '$TUNNEL_NAME' created"
fi

# 4. Get tunnel ID and write config
TUNNEL_ID=$(cloudflared tunnel info "$TUNNEL_NAME" 2>&1 | grep -oE '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' | head -1)
CREDS_FILE="$CLOUDFLARED_DIR/${TUNNEL_ID}.json"

cat > "$CLOUDFLARED_DIR/config.yml" << EOF
tunnel: ${TUNNEL_ID}
credentials-file: ${CREDS_FILE}

ingress:
  - hostname: ${TUNNEL_HOSTNAME}
    service: http://localhost:${TUNNEL_PORT}
  - service: http_status:404
EOF
echo "✓ Config written: $CLOUDFLARED_DIR/config.yml"

# 5. Route DNS (idempotent)
cloudflared tunnel route dns "$TUNNEL_NAME" "$TUNNEL_HOSTNAME" 2>/dev/null && \
  echo "✓ DNS: $TUNNEL_HOSTNAME → tunnel" || \
  echo "✓ DNS route already exists"

# 6. Install as LaunchAgent (no sudo needed for user-level agent)
if [[ "$(uname)" == "Darwin" ]]; then
  PLIST_PATH="$HOME/Library/LaunchAgents/com.cloudflare.tunnel.plist"
  if [[ -f "$PLIST_PATH" ]]; then
    echo "✓ LaunchAgent already installed"
  else
    mkdir -p "$HOME/Library/LaunchAgents"
    cat > "$PLIST_PATH" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.cloudflare.tunnel</string>
  <key>ProgramArguments</key>
  <array>
    <string>$(which cloudflared)</string>
    <string>tunnel</string>
    <string>run</string>
    <string>${TUNNEL_NAME}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/cloudflared.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/cloudflared.err</string>
</dict>
</plist>
PLIST
    launchctl load "$PLIST_PATH" 2>/dev/null
    echo "✓ LaunchAgent installed (auto-start on login, no sudo)"
  fi
fi

echo ""
echo "=== Done ==="
echo "  Access: https://$TUNNEL_HOSTNAME"
echo ""
echo "  To start manually: cloudflared tunnel run $TUNNEL_NAME"
echo "  To stop: cloudflared tunnel cleanup $TUNNEL_NAME"
