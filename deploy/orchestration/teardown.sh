#!/usr/bin/env bash
# Destroy one instance created by provision.sh — use-and-destroy.
#
#   teardown.sh <id>
#
# Stops the connector and deletes the Access application (and its policies), the service token,
# the DNS record, and the Tunnel. Leaves nothing at the edge. Does not touch the origin/container
# (stop that separately, e.g. docker compose -p ws-<id> down -v).
# shellcheck disable=SC1091  # lib.sh is resolved at runtime
set -u
here="$(cd "$(dirname "$0")" && pwd)"
. "$here/lib.sh"
id="${1:?usage: teardown.sh <id>}"
STATE="$here/instances/$id.env"
[ -e "$STATE" ] || { echo "no such instance: $STATE" >&2; exit 1; }
load_env "$here/secrets.env" || exit 1
load_env "$STATE" || exit 1
cf_curl_init || exit 1
trap cf_cleanup EXIT
API="https://api.cloudflare.com/client/v4"

del() { # del "<label>" "<METHOD path>"
  local label="$1" path="$2" out
  out="$(curl -s --retry 4 --retry-delay 2 --max-time 30 -X DELETE -K "$CF_CURL_CONFIG" "$API$path")"
  if printf '%s' "$out" | python3 -c 'import sys,json; sys.exit(0 if json.load(sys.stdin).get("success") else 1)' 2>/dev/null; then
    echo "  deleted $label"
  else
    echo "  WARN could not delete $label (may already be gone)"
  fi
}

# 1) stop the connector first (a tunnel with live connections cannot be deleted)
if [ -n "${CFD_PID:-}" ] && kill -0 "$CFD_PID" 2>/dev/null; then
  kill "$CFD_PID" 2>/dev/null && echo "  stopped cloudflared (pid $CFD_PID)"
  sleep 2
fi

# 2) Access application (deleting the app removes its policies)
[ -n "${APP_ID:-}" ]   && del "access app"   "/accounts/$CF_ACCOUNT_ID/access/apps/$APP_ID"
# 3) service token
[ -n "${ST_ID:-}" ]    && del "service token" "/accounts/$CF_ACCOUNT_ID/access/service_tokens/$ST_ID"
# 4) DNS record
[ -n "${DNS_RECORD_ID:-}" ] && del "dns record" "/zones/$CF_ZONE_ID/dns_records/$DNS_RECORD_ID"
# 5) tunnel (cleanup connections first, then delete)
if [ -n "${TUNNEL_ID:-}" ]; then
  # A tunnel cannot be deleted while connections are registered; drain them, then retry.
  curl -s --retry 4 --retry-delay 2 --max-time 30 -X DELETE -K "$CF_CURL_CONFIG" "$API/accounts/$CF_ACCOUNT_ID/cfd_tunnel/$TUNNEL_ID/connections" >/dev/null
  tdok=
  for _ in 1 2 3 4 5; do
    out="$(curl -s --retry 3 --retry-delay 2 --max-time 30 -X DELETE -K "$CF_CURL_CONFIG" "$API/accounts/$CF_ACCOUNT_ID/cfd_tunnel/$TUNNEL_ID")"
    if printf '%s' "$out" | python3 -c 'import sys,json; sys.exit(0 if json.load(sys.stdin).get("success") else 1)' 2>/dev/null; then tdok=1; break; fi
    sleep 3
  done
  [ -n "$tdok" ] && echo "  deleted tunnel" || echo "  WARN tunnel delete failed (connections still draining — rerun teardown)"
fi

rm -f "$here/instances/$id.cloudflared.log"
rm -f "$STATE"
echo "== $id torn down (state removed)"
