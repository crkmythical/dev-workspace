#!/usr/bin/env bash
# End-to-end mocks for the Rust Cloudflare host orchestrator. No real account, network or Docker is
# touched: curl, cloudflared and docker are fakes on PATH, while state/rollback/PID handling is real.
set -u

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
host="$repo/src/rust/target/debug/sdw-host"
cargo build --quiet --locked --manifest-path "$repo/src/rust/Cargo.toml" -p sdw-host || exit 1
failed=0
check() {
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; else
    printf '  FAIL  %s\n        expected [%s]\n        got      [%s]\n' "$1" "$2" "$3"; failed=1
  fi
}
has() {
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then printf '  PASS  %s\n' "$1"; else
    printf '  FAIL  %s\n        no /%s/ in: %s\n' "$1" "$3" "$2"; failed=1
  fi
}
hasnt() {
  if printf '%s\n' "$2" | grep -Fq -- "$3"; then
    printf '  FAIL  %s\n        leaked [%s]\n' "$1" "$3"; failed=1
  else printf '  PASS  %s\n' "$1"; fi
}

work="$(mktemp -d)"
pids="$work/pids"
cleanup() {
  if [ -f "$pids" ]; then
    while IFS= read -r pid; do kill -TERM -- "-$pid" 2>/dev/null || true; done <"$pids"
  fi
  rm -r "$work"
}
trap cleanup EXIT
bin="$work/bin"
project="$work/project"
state="$work/state"
mkdir -p "$bin" "$project/deploy/orchestration" "$state" "$work/tmp" "$work/home/.cloudflared"
: >"$work/home/.cloudflared/named-tunnel-id.json"
chmod 600 "$work/home/.cloudflared/named-tunnel-id.json"
: >"$project/docker-compose.yml"
: >"$project/docker-compose.apparmor.yml"

TOKEN='cf-test-token-never-print-1234567890'
cat >"$project/deploy/orchestration/secrets.env" <<EOF
CF_API_TOKEN=$TOKEN
CF_ACCOUNT_ID=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
CF_ZONE_ID=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
CF_ZONE_NAME=example.test
WS_SUBDOMAIN=ws
ALLOW_EMAIL=user@example.test
CF_TEAM_DOMAIN=team.cloudflareaccess.test
EOF
chmod 600 "$project/deploy/orchestration/secrets.env"

cat >"$bin/curl" <<'EOF'
#!/usr/bin/env bash
root="$(cd "$(dirname "$0")/.." && pwd)"
method=GET
url=
prev=
for arg in "$@"; do
  if [ "$prev" = -X ]; then method="$arg"; fi
  case "$arg" in https://*) url="$arg" ;; esac
  prev="$arg"
done
body="$(cat 2>/dev/null || true)"
printf '%s %s body=%s env_cf=%s\n' "$method" "$url" "$body" "${CF_API_TOKEN:-none}" >>"$root/state/calls.log"
if [ -f "$root/state/fail-dns" ] && [ "$method" = POST ] && printf '%s' "$url" | grep -q '/dns_records$'; then
  printf '%s\n' '{"success":false,"errors":[{"message":"injected dns failure"}]}'
  exit 0
fi
case "$method $url" in
  *'/cfd_tunnel/'*'/connections') result='{}' ;;
  DELETE*) result='{}' ;;
  POST*'/cfd_tunnel') result='{"id":"tunnel-1","token":"tunnel-secret-1"}' ;;
  POST*'/dns_records') result='{"id":"dns-1"}' ;;
  PUT*'/configurations') result='{}' ;;
  POST*'/access/service_tokens') result='{"id":"st-1","client_id":"client-1","client_secret":"client-secret-1"}' ;;
  POST*'/policies')
    if printf '%s' "$body" | grep -q 'machine'; then result='{"id":"policy-machine-1"}'; else result='{"id":"policy-human-1"}'; fi ;;
  POST*'/access/apps') result='{"id":"app-1","aud":"aud-1"}' ;;
  *) printf '%s\n' '{"success":false,"errors":[{"message":"unexpected fake request"}]}'; exit 0 ;;
esac
printf '{"success":true,"result":%s}\n' "$result"
EOF

cat >"$bin/cloudflared" <<'EOF'
#!/usr/bin/env bash
root="$(cd "$(dirname "$0")/.." && pwd)"
case "$*" in
  'tunnel list --output json')
    printf '%s\n' '[{"id":"named-tunnel-id","name":"named"}]'; exit 0 ;;
  'tunnel list') exit 0 ;;
  tunnel\ delete\ *) echo 'tunnel delete' >>"$root/state/calls.log"; exit 0 ;;
  'tunnel route dns named named.example.test')
    echo 'route dns' >>"$root/state/calls.log"; exit 0 ;;
esac
printf 'cloudflared token_present=%s args=%s\n' "$([ -n "${TUNNEL_TOKEN:-}" ] && echo yes || echo no)" "$*" >>"$root/state/calls.log"
echo "$$" >>"$root/pids"
trap 'exit 0' TERM INT HUP
while :; do sleep 1; done
EOF

cat >"$bin/launchctl" <<'EOF'
#!/usr/bin/env bash
printf 'launchctl %s\n' "$*" >>"$(cd "$(dirname "$0")/.." && pwd)/state/calls.log"
exit 0
EOF

cat >"$bin/docker" <<'EOF'
#!/usr/bin/env bash
printf 'docker %s aud=%s team=%s port=%s name=%s\n' "$*" "${ACCESS_AUD:-}" "${ACCESS_TEAM_DOMAIN:-}" "${TUNNEL_HOST_PORT:-}" "${WORKSPACE_CONTAINER_NAME:-}" >>"$FAKE_STATE/calls.log"
if [ "$*" = 'context show' ]; then echo "${FAKE_DOCKER_CONTEXT:-mock-context}"; exit 0; fi
if [ "$1" = inspect ] && [ -f "$FAKE_STATE/marker" ]; then
  printf '%s\n' '[{"Mounts":[{"Destination":"/var/run/self-destruct","Name":"marker-volume"}],"Config":{"Image":"fake:image"}}]'; exit 0
fi
if [ "$1" = run ] && [ -f "$FAKE_STATE/marker" ]; then exit 0; fi
if [ "$1 $2" = 'compose config' ]; then printf '%s\n' '{"name":"watchproj"}'; exit 0; fi
if [ "$1 $2" = 'image ls' ] || [ "$1 $2" = 'volume ls' ] || [ "$1 $2" = 'ps -aq' ]; then exit 0; fi
if [ "$1" = info ]; then exit 0; fi
if [ "${FAKE_DOCKER_UP_FAIL:-}" = 1 ]; then case "$*" in *' up -d') exit 1 ;; esac; fi
exit 0
EOF
chmod +x "$bin"/*
: >"$state/calls.log"
: >"$pids"

run_host() {
  OUT="$(env PATH="$bin:$PATH" HOME="$work/home" CF_API_TOKEN=exported-token-must-not-reach-curl ORCH_TMPDIR="$work/tmp" FAKE_STATE="$state" FAKE_PIDS="$pids" "$host" "$@" 2>&1)"
  rc=$?
}
mode_of() { stat -f %Lp "$1" 2>/dev/null || stat -c %a "$1"; }

echo '== Rust secret initialization'
cat >"$project/config.env" <<'EOF'
PASSWORD=password-super-secret-test
CLASH_SUBSCRIPTION_URL=https://subscription.example.test/secret-token
EOF
chmod 600 "$project/config.env"
run_host secrets-init --project-dir "$project" --env-file config.env
check 'secrets-init exits 0' 0 "$rc"
check 'password file is private' 600 "$(mode_of "$project/secrets/password")"
check 'subscription file is private' 600 "$(mode_of "$project/secrets/clash_subscription_url")"
check 'password bytes are exact' password-super-secret-test "$(cat "$project/secrets/password")"
hasnt 'secret values are not printed' "$OUT" 'password-super-secret-test'
hasnt 'subscription values are not printed' "$OUT" 'secret-token'
printf 'PASSWORD=changed-password\nCLASH_SUBSCRIPTION_URL=https://changed.example.test\n' >"$project/config.env"
run_host secrets-init --project-dir "$project" --env-file config.env
check 'existing secret is retained without --force' password-super-secret-test "$(cat "$project/secrets/password")"
printf 'PASSWORD=only-password\n' >"$project/config.env"
run_host secrets-init --project-dir "$project" --env-file config.env --force
check 'missing required input fails before changing files' 1 "$rc"
check 'failed validation did not clobber existing secret' password-super-secret-test "$(cat "$project/secrets/password")"

echo '== named tunnel setup uses scoped typed configuration'
run_host setup-tunnel --project-dir "$project" --name named --hostname named.example.test --port 19005
check 'named tunnel setup exits 0' 0 "$rc"
tunnel_config="$work/home/.cloudflared/sdw-named.yml"
check 'a tunnel-specific config is written' yes "$([ -f "$tunnel_config" ] && echo yes || echo no)"
check 'the tunnel config is private' 600 "$(mode_of "$tunnel_config")"
has 'the config names the requested hostname' "$(cat "$tunnel_config")" 'named\.example\.test'
has 'the config points at the requested loopback port' "$(cat "$tunnel_config")" 'localhost:19005'
check 'the generic Cloudflare config is not overwritten' no "$([ -e "$work/home/.cloudflared/config.yml" ] && echo yes || echo no)"
printf 'TUNNEL_NAME=named\n' >"$project/.env"
run_host services --project-dir "$project" --cloudflared --host-watcher --dry-run
check 'typed systemd rendering exits 0' 0 "$rc"
has 'systemd tunnel unit uses the scoped config' "$OUT" 'sdw-named\.yml'
has 'systemd watcher invokes sdw-host directly' "$OUT" 'sdw-host.*watch --project-dir'
rm -f "$project/.env"

echo '== typed provision state and secret transport'
run_host provision --project-dir "$project" demo 19000
check 'provision exits 0' 0 "$rc"
state_file="$project/deploy/orchestration/instances/demo.json"
check 'state is written as JSON' yes "$([ -f "$state_file" ] && echo yes || echo no)"
check 'state is private' 600 "$(mode_of "$state_file")"
has 'state records every created resource' "$(cat "$state_file")" '"tunnel_id": "tunnel-1"'
has 'state keeps the service secret for the caller' "$(cat "$state_file")" '"service_client_secret": "client-secret-1"'
hasnt 'API token never appears in output' "$OUT" "$TOKEN"
hasnt 'API token never appears in child arguments/logs' "$(cat "$state/calls.log")" "$TOKEN"
hasnt 'an exported API token is stripped from curl too' "$(cat "$state/calls.log")" 'exported-token-must-not-reach-curl'
has 'connector gets a token without putting it in argv' "$(cat "$state/calls.log")" 'cloudflared token_present=yes args=tunnel --no-autoupdate run'

run_host teardown --project-dir "$project" demo
check 'teardown exits 0' 0 "$rc"
check 'successful teardown removes state' no "$([ -e "$state_file" ] && echo yes || echo no)"
has 'teardown deletes edge objects' "$(cat "$state/calls.log")" 'DELETE .*access/apps/app-1'

echo '== rollback is driven by persisted partial state'
: >"$state/calls.log"
: >"$state/fail-dns"
OUT="$(env PATH="$bin:$PATH" HOME="$work/home" ORCH_TMPDIR="$work/tmp" FAKE_STATE="$state" FAKE_PIDS="$pids" "$host" provision --project-dir "$project" rollback 19001 2>&1)"
rc=$?
rm -f "$state/fail-dns"
check 'an injected API failure exits non-zero' 1 "$rc"
check 'complete rollback removes partial state' no "$([ -e "$project/deploy/orchestration/instances/rollback.json" ] && echo yes || echo no)"
has 'rollback deletes the tunnel it had already created' "$(cat "$state/calls.log")" 'DELETE .*cfd_tunnel/tunnel-1'

echo '== up rolls edge back when compose fails'
: >"$state/calls.log"
OUT="$(env PATH="$bin:$PATH" ORCH_TMPDIR="$work/tmp" FAKE_STATE="$state" FAKE_PIDS="$pids" FAKE_DOCKER_UP_FAIL=1 "$host" up --project-dir "$project" failup 19002 2>&1)"
rc=$?
check 'failed compose makes up fail' 1 "$rc"
check 'failed up leaves no state' no "$([ -e "$project/deploy/orchestration/instances/failup.json" ] && echo yes || echo no)"
has 'failed up asks Docker to remove local volumes' "$(cat "$state/calls.log")" 'docker compose .* down -v'

echo '== boundary validation'
run_host watch --project-dir "$project" --once
check 'watch --once is harmless without a marker' 0 "$rc"
has 'watch reports the absent marker' "$OUT" 'No self-destruct marker found'
run_host provision --project-dir "$project" '../escape' 19003
check 'path-like instance IDs are rejected' 1 "$rc"
printf 'SDW_DOCKER_CONTEXT=expected-context\n' >"$project/.env"
OUT="$(env PATH="$bin:$PATH" HOME="$work/home" ORCH_TMPDIR="$work/tmp" FAKE_STATE="$state" FAKE_PIDS="$pids" FAKE_DOCKER_CONTEXT=wrong-context "$host" down --project-dir "$project" contextcheck 2>&1)"
rc=$?
check 'a mismatched Docker context fails closed' 1 "$rc"
has 'the context error names both values' "$OUT" "wrong-context.*expected-context"
rm -f "$project/.env"
chmod 644 "$project/deploy/orchestration/secrets.env"
run_host provision --project-dir "$project" insecure 19004
check 'a group-readable secrets file is rejected' 1 "$rc"
has 'the refusal names its unsafe mode' "$OUT" 'mode 644'

echo '== watcher confirms its own marker and delegates typed destruction'
watch_project="$work/watch-project"
mkdir -p "$watch_project"
: >"$watch_project/docker-compose.yml"
: >"$state/marker"
run_host watch --project-dir "$watch_project" --once
check 'watcher cleanup exits 0' 0 "$rc"
check 'watcher cleanup removes the confirmed project' no "$([ -e "$watch_project" ] && echo yes || echo no)"
has 'watcher reports the confirmed transition' "$OUT" 'Self-destruct confirmed'
rm -f "$state/marker"

if [ "$failed" -eq 0 ]; then echo 'all passed'; else echo 'FAILED'; fi
exit "$failed"
