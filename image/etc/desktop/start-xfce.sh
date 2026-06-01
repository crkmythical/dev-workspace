#!/bin/bash
# XFCE launcher for the remote desktop. Used by BOTH stacks (selkies on :1,
# kasmvnc on :2). The target display + HOME are taken from the environment
# (DISPLAY, HOME, XDG_*) set by the supervisor program, defaulting to the
# selkies values for backward compatibility.
#
# Two reliability fixes baked in (both verified empirically in the container):
#
# 1. Wait for the X server before launching. Otherwise startxfce4 races the X
#    server during supervisord startup and the WM / panel fail to attach.
#
# 2. Launch via `dbus-run-session`. XFCE needs a real per-session D-Bus bus to
#    orchestrate xfwm4 + xfce4-panel + xfdesktop. Relying on
#    DBUS_SESSION_BUS_ADDRESS=autolaunch: is flaky in a container and leaves the
#    session with only xfce4-session running (black screen, no WM/panel).
set -u

export DISPLAY="${DISPLAY:-:1}"
export HOME="${HOME:-/workspace/.desktop}"
export XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
export XDG_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-/tmp/.desktop-cache}"

# Proxy settings for GUI apps (Firefox, Chromium, etc.)
export http_proxy=http://127.0.0.1:7890
export https_proxy=http://127.0.0.1:7890
export all_proxy=socks5://127.0.0.1:7890
export no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8
export HTTP_PROXY=$http_proxy
export HTTPS_PROXY=$https_proxy
export ALL_PROXY=$all_proxy
export NO_PROXY=$no_proxy

# Wait up to 30s for the X server on $DISPLAY to accept connections.
for _ in $(seq 1 60); do
  if xdpyinfo -display "$DISPLAY" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

# Set consistent DPI for both stacks. Selkies runs a 4K framebuffer with
# adaptive resize — force 96 DPI so XFCE renders at standard size regardless
# of the framebuffer dimensions. KasmVNC also benefits from explicit DPI.
xrandr --dpi 96 2>/dev/null || true

exec dbus-run-session -- /usr/bin/startxfce4
