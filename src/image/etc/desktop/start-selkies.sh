#!/bin/bash
# Selkies streamer (supervisor program desktop-selkies). Started as the workload user through
# `sdw workload`, after Xvfb is up. Every setting Selkies is given lives here.
#
# What is fixed (not a user setting), and why:
#   * It listens on loopback only, under /desktop. Caddy's session gate is the only way in.
#   * It has no login of its own. Selkies 2.x turns HTTP Basic auth ON by default and refuses to
#     start without a password; the session cookie the gate checks is the one login for everything
#     here, as for code-server and KasmVNC. A second prompt would only be a second place to keep
#     a copy of the master password.
#   * WebSocket transport only (dual mode off). The other transport, WebRTC, needs UDP and a
#     STUN/TURN server on the internet; neither is published or reachable behind the egress
#     firewall, so it could only fail, and its endpoints would be one more thing to guard.
#   * No sharing links, no printing (there is no CUPS here). A single-user workspace has no use
#     for either.
# What follows the container's configuration:
#   * Audio is on only when the PulseAudio sink exists (DESKTOP_AUDIO=1, see start-audio.sh).
#     Without it every connection would try to capture a device that is not there.
#   * Any other SELKIES_* variable given to the container (the encoder, the frame rate range,
#     the clipboard policy, ...) reaches Selkies as it is; the defaults below apply to what is unset.
set -u

export SELKIES_ENABLE_BASIC_AUTH=false
export SELKIES_ENABLE_DUAL_MODE=false
export SELKIES_ENABLE_SHARING=false
export SELKIES_PRINTING_ENABLED=false

case "${DESKTOP_AUDIO:-0}" in
  1 | true | yes | on) export SELKIES_AUDIO_ENABLED=true ;;
  *) export SELKIES_AUDIO_ENABLED=false ;;
esac

# The applications panel installs software into the session with proot-apps, which is not part of
# this image (applications come from `desktop-install`), so it could only show an empty list.
export SELKIES_UI_SIDEBAR_SHOW_APPS="${SELKIES_UI_SIDEBAR_SHOW_APPS:-false}"

# The client drives the size of the X screen (Xvfb is started at 4K, the ceiling).
export SELKIES_MANUAL_RESOLUTION="${SELKIES_MANUAL_RESOLUTION:-false}"
export SELKIES_ENABLE_RESIZE="${SELKIES_ENABLE_RESIZE:-true}"
export SELKIES_ENCODER="${SELKIES_ENCODER:-h264enc}"
export SELKIES_FRAMERATE="${SELKIES_FRAMERATE:-30}"

exec /opt/selkies-venv/bin/python -m selkies \
  --addr=127.0.0.1 --port=6080 --subfolder=/desktop --mode=websockets
