#!/bin/bash
# Respawn Clash if it dies — what supervisord's [program:clash-watcher] runs.
#
# Under EGRESS_ENFORCE, Clash must run as the `clash` uid: the only uid the egress firewall lets
# out (see `sdw entrypoint enforce-egress` and the clash user/setcap in image/Dockerfile). When
# EGRESS_ENFORCE is off, Clash runs as root, exactly as before.
#
# This lives as a shipped script rather than an inline supervisord `command=` on purpose:
# supervisord's own parsing mishandles a nested-quote shell one-liner (it raised
# "No closing quotation" and the program never started). A bare `command=` path sidesteps that.
set -u

start_clash() {
  case "${EGRESS_ENFORCE:-}" in
    1 | true | TRUE | yes | YES | on | ON)
      setpriv --reuid="$(id -u clash)" --regid="$(id -g clash)" --clear-groups \
        /usr/bin/clash -d /etc/clash &
      ;;
    *)
      /usr/bin/clash -d /etc/clash &
      ;;
  esac
}

while true; do
  if ! pgrep -x clash >/dev/null 2>&1; then
    start_clash
  fi
  sleep 5
done
