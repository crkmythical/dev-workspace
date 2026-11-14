#!/usr/bin/env bash
# The egress path with a REAL proxy protocol instead of the fake HTTP node: a Shadowsocks server
# (the usual kind of node behind a subscription) on the instance's network, listed by the
# subscription, and Clash's url-test choosing it. The server then goes to the real internet, so
# this is also the one place where the container really goes online through Clash.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

ss_node() {
  step "Shadowsocks node behind the subscription"
  local node="${SMOKE_PROJECT}-ssnode" out
  docker_build_retry sdw-smoke-ss <<'EOF'
FROM alpine:latest
RUN apk add --no-cache shadowsocks-rust-ssserver
CMD ["ssserver", "-s", "0.0.0.0:8388", "-k", "smoke-ss-password", "-m", "chacha20-ietf-poly1305", "-vv"]
EOF
  compose up -d --no-start >/dev/null 2>&1
  docker rm -f "$node" >/dev/null 2>&1
  docker run -d --name "$node" --network "${SMOKE_PROJECT}_default" --ip "${SMOKE_NET}.32" sdw-smoke-ss >/dev/null
  export SMOKE_SS_NODE_IP="${SMOKE_NET}.32" SMOKE_SS_PASSWORD=smoke-ss-password
  sleep 2
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    docker rm -f "$node" >/dev/null 2>&1
    unset SMOKE_SS_NODE_IP SMOKE_SS_PASSWORD
    return
  fi

  # The node belongs to the subscription provider, so it is asked for there (in rule mode the top-level
  # /proxies lists only the groups).
  out="$(clash_api /providers/proxies/subscription/ss-node 2>&1)"
  expect_match "Clash loaded the node from the subscription, as a Shadowsocks proxy" '"type":"Shadowsocks"' "$out"
  out="$(clash_api /proxies/auto-select 2>&1)"
  expect_match "the url-test group selected it" '"now":"ss-node"' "$out"
  out="$(clash_api '/providers/proxies/subscription/ss-node/healthcheck?url=http://www.gstatic.com/generate_204&timeout=10000' 2>&1)"
  expect_match "Clash measured a delay through the node (the node works end to end)" '"delay":[0-9]+' "$out"

  expect_eq "a request through Clash reaches the internet (gstatic answers 204)" 204 \
    "$(dx curl -s -o /dev/null -w '%{http_code}' --max-time 30 -x http://127.0.0.1:7890 http://www.gstatic.com/generate_204)"
  expect_match "the Shadowsocks server relayed it (it logs the target)" 'gstatic' "$(docker logs "$node" 2>&1)"
  expect_no_match "the fake HTTP node was not used for it" 'CONNECT www.gstatic' "$(docker logs "$UP" 2>&1)"

  local status
  status="$(dx cat /var/run/egress-status 2>/dev/null | sed -E 's/.*"status":"([a-z]+)".*/\1/')"
  if [ "$status" = confirmed ]; then
    pass "the start-up probe confirmed egress through the node"
  else
    skip "the start-up probe is $status (https://www.google.com is not reachable from this network; the data path above works)"
  fi

  docker rm -f "$node" >/dev/null 2>&1
  stack_down
  unset SMOKE_SS_NODE_IP SMOKE_SS_PASSWORD
}

instance "${SMOKE_PROJECT}-ss" "$((SMOKE_PORT + 6))" 172.30.83 sub-ss.yaml src/tests/smoke/empty.env ss_node

finish
