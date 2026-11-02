#!/usr/bin/env bash
# Provision one ephemeral, Access-protected workspace instance on Cloudflare.
#
#   provision.sh <id> [origin_port]        # default origin port 18091
#
# Creates (all via the Cloudflare API, no browser): a remotely-managed Tunnel, a one-level DNS
# hostname (ws-<id>.<zone> — one level so Universal SSL covers it), tunnel ingress to a local
# origin, a service token, an Access self-hosted application, and TWO policies on it:
#   - decision=non_identity, include service_token  (machine / CF-Access-Client-* headers)
#   - decision=allow,        include email           (human / browser email-OTP login)
# Then starts a cloudflared connector for the tunnel. State (incl. the tunnel token and the
# service-token secret) is written to instances/<id>.env (chmod 600, gitignored).
#
# Reverse everything with teardown.sh <id>.
# shellcheck disable=SC1090,SC1091  # secrets.env and instances/<id>.env are resolved at runtime
set -u
here="$(cd "$(dirname "$0")" && pwd)"
id="${1:?usage: provision.sh <id> [origin_port]}"
port="${2:-18091}"
set -a; . "$here/secrets.env"; set +a
for v in CF_API_TOKEN CF_ACCOUNT_ID CF_ZONE_ID CF_ZONE_NAME WS_SUBDOMAIN ALLOW_EMAIL; do
  eval "[ -n \"\${$v:-}\" ]" || { echo "missing $v in secrets.env" >&2; exit 1; }
done
case "$ALLOW_EMAIL" in *@*.*) : ;; *) echo "ALLOW_EMAIL is not a full address: '$ALLOW_EMAIL'" >&2; exit 1 ;; esac

API="https://api.cloudflare.com/client/v4"
AUTH="Authorization: Bearer $CF_API_TOKEN"
FQDN="${WS_SUBDOMAIN}-${id}.${CF_ZONE_NAME}"
mkdir -p "$here/instances"
STATE="$here/instances/$id.env"
[ -e "$STATE" ] && { echo "instance '$id' already exists: $STATE (teardown first)" >&2; exit 1; }
: > "$STATE"; chmod 600 "$STATE"
{ echo "FQDN=$FQDN"; echo "ORIGIN_PORT=$port"; } >> "$STATE"

# Roll back everything created so far if we exit before finishing (so a partial run never leaks).
DONE=0
rollback() { local rc=$?; if [ "$rc" -ne 0 ] && [ "$DONE" -ne 1 ]; then
  echo "provision failed (rc=$rc) — rolling back $id" >&2; bash "$here/teardown.sh" "$id" >/dev/null 2>&1 || true; fi; }
trap rollback EXIT

# post/put/delete helper: args = METHOD PATH [JSON]; on API error prints message and exits 1.
cf() {
  local method="$1" path="$2" data="${3:-}" out
  if [ -n "$data" ]; then
    out="$(curl -s --retry 4 --retry-delay 2 --max-time 30 -X "$method" -H "$AUTH" -H "Content-Type: application/json" --data "$data" "$API$path")"
  else
    out="$(curl -s --retry 4 --retry-delay 2 --max-time 30 -X "$method" -H "$AUTH" "$API$path")"
  fi
  [ -n "$out" ] || { echo "  empty response from $method $path (network?)" >&2; return 1; }
  printf '%s' "$out"
}
# extract: reads JSON on stdin, writes KEY=<result.FIELD> lines to state, prints a short note.
save() { # save "<label>" "<state_key>:<result_path>" ...   (JSON on stdin)
  local label="$1"; shift
  local json; json="$(cat)"
  JSON="$json" python3 -c '
import os,sys,json
state=sys.argv[1]; label=sys.argv[2]; pairs=sys.argv[3:]
try:
    d=json.loads(os.environ["JSON"])
except Exception:
    sys.stderr.write("  %s FAILED: non-JSON/empty response\n"%label); sys.exit(1)
if not d.get("success"):
    sys.stderr.write("  %s FAILED: %s\n"%(label,[e.get("message") for e in d.get("errors",[])])); sys.exit(1)
r=d["result"]
with open(state,"a") as f:
    for p in pairs:
        key,path=p.split(":",1)
        cur=r
        for k in path.split("."): cur=cur[k]
        f.write("%s=%s\n"%(key,cur))
print("  %s ok"%label)
' "$STATE" "$label" "$@"
}

echo "== provisioning $FQDN (origin 127.0.0.1:$port)"
cf POST "/accounts/$CF_ACCOUNT_ID/cfd_tunnel" \
  "{\"name\":\"ws-$id-$(date +%s)\",\"config_src\":\"cloudflare\"}" \
  | save "tunnel" "TUNNEL_ID:id" "TUNNEL_TOKEN:token" || exit 1
set -a; . "$STATE"; set +a

cf POST "/zones/$CF_ZONE_ID/dns_records" \
  "{\"type\":\"CNAME\",\"name\":\"$FQDN\",\"content\":\"$TUNNEL_ID.cfargotunnel.com\",\"proxied\":true}" \
  | save "dns" "DNS_RECORD_ID:id" || exit 1

cf PUT "/accounts/$CF_ACCOUNT_ID/cfd_tunnel/$TUNNEL_ID/configurations" \
  "{\"config\":{\"ingress\":[{\"hostname\":\"$FQDN\",\"service\":\"http://127.0.0.1:$port\"},{\"service\":\"http_status:404\"}]}}" \
  | save "ingress" || exit 1

cf POST "/accounts/$CF_ACCOUNT_ID/access/service_tokens" "{\"name\":\"ws-$id\"}" \
  | save "service-token" "ST_ID:id" "ST_CLIENT_ID:client_id" "ST_CLIENT_SECRET:client_secret" || exit 1
set -a; . "$STATE"; set +a

cf POST "/accounts/$CF_ACCOUNT_ID/access/apps" \
  "{\"name\":\"ws-$id\",\"domain\":\"$FQDN\",\"type\":\"self_hosted\",\"session_duration\":\"30m\"}" \
  | save "access-app" "APP_ID:id" "APP_AUD:aud" || exit 1
set -a; . "$STATE"; set +a

cf POST "/accounts/$CF_ACCOUNT_ID/access/apps/$APP_ID/policies" \
  "{\"name\":\"machine\",\"decision\":\"non_identity\",\"include\":[{\"service_token\":{\"token_id\":\"$ST_ID\"}}]}" \
  | save "policy-machine" "POLICY_SVC_ID:id" || exit 1

cf POST "/accounts/$CF_ACCOUNT_ID/access/apps/$APP_ID/policies" \
  "{\"name\":\"human\",\"decision\":\"allow\",\"include\":[{\"email\":{\"email\":\"$ALLOW_EMAIL\"}}]}" \
  | save "policy-human" "POLICY_HUMAN_ID:id" || exit 1

# Start the connector (token read from state, never on this script's visible command line).
nohup cloudflared tunnel --no-autoupdate run --token "$TUNNEL_TOKEN" \
  >"$here/instances/$id.cloudflared.log" 2>&1 &
echo "CFD_PID=$!" >> "$STATE"
DONE=1

echo
echo "== $id ready: https://$FQDN/"
echo "   human : browser -> Access login -> email OTP to $ALLOW_EMAIL"
echo "   machine: CF-Access-Client-Id=$ST_CLIENT_ID  (secret in $STATE, not printed)"
echo "   connector log: instances/$id.cloudflared.log   state: $STATE"
echo "   point it at the real workspace by running that container on 127.0.0.1:$port"
echo "   single-sign-on (skip the workspace's own login behind Access): set on the container"
echo "     ACCESS_AUD=$APP_AUD"
echo "     ACCESS_TEAM_DOMAIN=<your-team>.cloudflareaccess.com   (the host Access redirects to)"
