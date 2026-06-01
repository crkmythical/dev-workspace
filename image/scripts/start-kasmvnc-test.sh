#!/bin/bash
set -e

echo "=== KasmVNC Test Container ==="
echo "KasmVNC binary: $(which Xkasmvnc)"
echo "Architecture: $(dpkg --print-architecture)"
echo "Base: $(grep PRETTY_NAME /etc/os-release | cut -d= -f2)"
echo ""

# Print version
Xkasmvnc -version 2>&1 || true
echo ""

# Start Xkasmvnc directly
echo "Starting Xkasmvnc on :1 port 6080..."
Xkasmvnc :1 \
  -geometry 1920x1080 \
  -depth 24 \
  -websocketPort 6080 \
  -interface 0.0.0.0 \
  -publicIP 127.0.0.1 \
  -RectThreads 0 \
  -FrameRate 30 \
  -httpd /usr/share/kasmvnc/www \
  +extension COMPOSITE \
  +extension DAMAGE \
  +extension RANDR \
  +extension GLX \
  -nolisten tcp \
  -SecurityTypes None \
  &

XPID=$!
echo "Xkasmvnc PID: $XPID"

# Wait for X server
echo "Waiting for X server..."
for i in $(seq 1 30); do
  if xdpyinfo -display :1 >/dev/null 2>&1; then
    echo "X server ready after ${i}s"
    break
  fi
  sleep 1
done

if ! xdpyinfo -display :1 >/dev/null 2>&1; then
  echo "ERROR: X server failed to start"
  ls -la /tmp/.X11-unix/ 2>/dev/null || true
  cat /tmp/.X1-lock 2>/dev/null || true
  exit 1
fi

# Start XFCE
echo "Starting XFCE on :1..."
export DISPLAY=:1
export HOME=/root
export XDG_CONFIG_HOME=/root/.config
export XDG_DATA_HOME=/root/.local/share
export XDG_CACHE_HOME=/root/.cache
dbus-run-session -- startxfce4 &

sleep 5
echo ""
echo "========================================="
echo "  READY — http://localhost:6080"
echo "  No auth required (SecurityTypes None)"
echo "========================================="
echo ""
echo "X11 extensions:"
xdpyinfo -display :1 2>/dev/null | grep -E "DAMAGE|COMPOSITE|MIT-SHM|RANDR" || true
echo ""
echo "Xkasmvnc threads:"
ls /proc/$XPID/task/ 2>/dev/null | wc -l || true
echo ""
echo "Listening ports:"
ss -tlnp | grep 6080 || true
echo ""

# Keep alive and tail logs
wait $XPID
