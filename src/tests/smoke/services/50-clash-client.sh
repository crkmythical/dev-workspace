#!/usr/bin/env bash
# The standalone Clash client (deploy/client): the clash-client stage of image/Dockerfile, started by
# its own compose file, next to the fake upstream and without the workspace. It runs the same
# `sdw proxy-only` as the real thing, so this proves what the hand-written config it replaces never
# did: the subscription is used, traffic goes through a node, and the controller is not open.
# Set BUILD_PROXY (and APT_MIRROR) in the environment when the build needs them, as for the image.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

client() {
  step "standalone client: deploy/client"
  local client="${SMOKE_PROJECT}-client" override cip
  override="$(mktemp "${TMPDIR:-/tmp}/sdw-smoke-client.XXXXXX")"
  cat >"$override" <<EOF
services:
  clash-client:
    container_name: ${client}
    ports: !override []
networks:
  default:
    name: ${SMOKE_PROJECT}_default
    external: true
EOF
  cc() {
    CLASH_SUBSCRIPTION_URL="http://${SMOKE_UPSTREAM_IP}:8000/${SMOKE_SUB}" \
      docker compose -p "$SMOKE_PROJECT" -f "$ROOT/deploy/client/docker-compose.yml" -f "$override" "$@"
  }
  # The instance's network and the fake upstream; the workspace itself is not needed.
  compose up -d upstream >/dev/null 2>&1
  if ! cc up -d --build >/dev/null 2>&1; then
    fail "the client built and started" "$(cc up -d --build 2>&1 | tail -15)"
    cc down -v >/dev/null 2>&1
    compose down -v --remove-orphans >/dev/null 2>&1
    rm -f "$override"
    return
  fi
  if wait_until 180 container_healthy "$client"; then
    pass "the client is healthy: Clash accepts connections and the subscription loaded nodes"
  else
    fail "the client is healthy" "$(docker logs --tail 15 "$client" 2>&1)"
  fi

  # The container's init (PID 1, root) runs it; the process called clash must be the clash user's.
  # (An empty answer on either side of the comparison, such as from a container that is
  # restarting, is not a pass: the clash user's id is checked to be a number first.)
  CLASH_UID="$(docker exec "$client" id -u clash 2>/dev/null)"
  # shellcheck disable=SC2016 # expanded in the container
  RUNNING_AS="$(docker exec "$client" sh -c 'for p in /proc/[0-9]*; do awk "/^Name:/ {n=\$2} /^Uid:/ {u=\$2} END {if (n == \"clash\") print u}" $p/status; done' 2>/dev/null)"
  expect_match "the container knows the clash user" '^[0-9]+$' "$CLASH_UID"
  expect_eq "the process called clash runs as that user" "$CLASH_UID" "$RUNNING_AS"
  expect_eq "its config is readable by that user alone" "600 clash:clash" \
    "$(docker exec "$client" stat -c '%a %U:%G' /etc/clash/config.yaml)"
  expect_eq "the controller refuses a request without the secret" 401 \
    "$(docker exec "$client" curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:9090/configs)"

  cip="$(docker inspect -f "{{(index .NetworkSettings.Networks \"${SMOKE_PROJECT}_default\").IPAddress}}" "$client")"
  reach() { docker exec "$UP" python3 -c "import socket; socket.create_connection(('$cip', $1), 3)"; }
  expect_true "the proxy port is reachable from another container (it listens on the interface)" reach 7890
  expect_true "so is the SOCKS port" reach 7891
  expect_false "the controller port is not (it is loopback only)" reach 9090

  # The old config sent everything DIRECT. The fake node logs every CONNECT it relays.
  docker exec "$client" curl -s -m 30 -o /dev/null -x http://127.0.0.1:7890 https://www.google.com
  expect_match "a request through the client was relayed by the subscription's node" \
    'CONNECT www.google.com' "$(docker logs "$UP" 2>&1)"

  cc down -v >/dev/null 2>&1
  compose down -v --remove-orphans >/dev/null 2>&1
  rm -f "$override"
}

instance "${SMOKE_PROJECT}-cc" "$((SMOKE_PORT + 7))" 172.30.84 sub.yaml src/tests/smoke/empty.env client

finish
