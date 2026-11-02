#!/usr/bin/env bash
# One command: provision an ephemeral, Access-protected workspace AND start the wired container,
# with the gate's Access single-sign-on already enabled — no manual AUD copying, no second login at
# the workspace gate behind Access.
#
#   up.sh <id> [origin_port]
#
# Batchable: run it for several ids; each instance gets its own tunnel, hostname, Access app +
# policies, service token, and its own container (compose project + name ws-<id>), volumes and
# loopback port. A free port is chosen automatically when not given. Reverse with: down.sh <id>.
#
# Single-sign-on wiring is automatic when CF_TEAM_DOMAIN is set in secrets.env: provision.sh records
# the team domain and the app's AUD, and this script passes them to the container as ACCESS_* so the
# gate accepts the Access session. Without CF_TEAM_DOMAIN the instance still comes up, but the
# workspace gate asks for its own login after Access.
# shellcheck disable=SC1090,SC1091  # instances/<id>.env is written by provision.sh at runtime
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
id="${1:?usage: up.sh <id> [origin_port]}"
port="${2:-}"

# A loopback port the container publishes and the tunnel's ingress points at. Pick a free one.
if [ -z "$port" ]; then
  for p in $(seq 18090 18189); do
    if ! nc -z 127.0.0.1 "$p" 2>/dev/null; then port="$p"; break; fi
  done
  [ -n "$port" ] || { echo "up: no free port in 18090-18189" >&2; exit 1; }
fi

# 1) The Cloudflare edge: tunnel, DNS, Access app + policies, service token, connector.
bash "$here/provision.sh" "$id" "$port"

# 2) Load what it recorded (FQDN, APP_AUD, TEAM_DOMAIN).
set -a; . "$here/instances/$id.env"; set +a

# 3) The container, Access-wired for single-sign-on, isolated per instance.
sso="off (set CF_TEAM_DOMAIN in secrets.env to enable)"
[ -n "${APP_AUD:-}" ] && [ -n "${TEAM_DOMAIN:-}" ] && sso="ON"
ACCESS_AUD="${APP_AUD:-}" ACCESS_TEAM_DOMAIN="${TEAM_DOMAIN:-}" \
TUNNEL_HOST_PORT="$port" WORKSPACE_CONTAINER_NAME="ws-$id" \
  docker compose -p "ws-$id" \
    -f "$root/docker-compose.yml" -f "$root/docker-compose.apparmor.yml" up -d

echo
echo "== $id up: https://${FQDN}/   (origin 127.0.0.1:${port}, container ws-${id})"
echo "   gate single-sign-on behind Access: ${sso}"
echo "   down (edge + container + volumes): $here/down.sh $id"
