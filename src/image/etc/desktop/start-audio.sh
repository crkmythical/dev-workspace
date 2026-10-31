#!/bin/bash
# PulseAudio null-sink for desktop audio streaming (selkies pcmflux capture).
# Creates sink "output" → monitor device "output.monitor" for pcmflux.
set -u

if [ "${DESKTOP_AUDIO:-0}" != "1" ]; then
  echo "Audio disabled (DESKTOP_AUDIO not set). Exiting."
  exit 0
fi

export HOME=/workspace/.desktop
export XDG_RUNTIME_DIR=/tmp/.desktop-cache

# Clean stale socket/pid from previous run
rm -rf /tmp/.desktop-cache/pulse 2>/dev/null
mkdir -p /tmp/.desktop-cache/pulse

# Wait for Xvfb
for _ in $(seq 1 30); do
  if xdpyinfo -display :1 >/dev/null 2>&1; then break; fi
  sleep 0.5
done

# Run as root in non-system mode. PULSE_LOG suppresses dbus warnings.
export PULSE_LOG=0
exec pulseaudio \
  --daemonize=no \
  --disallow-exit \
  --exit-idle-time=-1 \
  --log-level=error \
  --load="module-null-sink sink_name=output sink_properties=device.description=VirtualOutput" \
  --load="module-native-protocol-unix auth-anonymous=1 socket=/tmp/.desktop-cache/pulse/native" \
  --load="module-always-sink"
