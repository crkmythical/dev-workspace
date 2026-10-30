#!/usr/bin/env bash
# croc file transfer (push / pull): a self-hosted relay and an end-to-end encrypted round-trip into
# the vault. Beyond the unit test (which pins the argv, including the default --socks5 through Clash),
# this proves the pinned croc binary runs, push and pull speak to a real relay, and the bytes arrive
# intact. The relay here is on the LAN, reached with --direct; a remote relay through Clash is the
# default and is argv-tested, not boot-tested (Clash cannot reach a private test relay).
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

# push/pull with the proxy env cleared, so --direct genuinely reaches the LAN relay whether or not
# croc would otherwise honour the container's all_proxy.
direct_exec() {
  docker exec -e all_proxy= -e ALL_PROXY= -e http_proxy= -e https_proxy= -e HTTP_PROXY= -e HTTPS_PROXY= "$@"
}

croc_transfer() {
  step "croc: a self-hosted relay and an end-to-end round-trip"
  local relay="${SMOKE_PROJECT}-crocrelay"
  compose up -d --no-start >/dev/null 2>&1
  docker rm -f "$relay" >/dev/null 2>&1
  # The workspace image carries the pinned croc; reuse it as the relay. CROC_PASS is its secret.
  docker run -d --name "$relay" --network "${SMOKE_PROJECT}_default" --ip 172.30.84.30 \
    -e CROC_PASS=smoke-croc-pass --entrypoint /usr/local/bin/croc "$SMOKE_IMAGE" relay >/dev/null
  sleep 3
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    docker rm -f "$relay" >/dev/null 2>&1
    return
  fi

  dx sh -c 'echo croc-smoke-payload-42 > /workspace/croc-send.txt && mkdir -p /workspace/croc-recv'
  # `push` waits for a receiver, so background it (logging where we can read it), then `pull`.
  direct_exec -d "$WS" sh -c 'push --direct --code smoke-croc-code-2468 /workspace/croc-send.txt > /tmp/push.log 2>&1'
  sleep 3
  direct_exec "$WS" pull --direct --out /workspace/croc-recv smoke-croc-code-2468 >/dev/null 2>&1 || true
  # shellcheck disable=SC2329 # called by wait_until
  received() { dx test -f /workspace/croc-recv/croc-send.txt; }
  expect_true "the file arrives through the relay" wait_until 40 received
  expect_eq "its bytes are intact end to end" "croc-smoke-payload-42" \
    "$(dx sh -c 'cat /workspace/croc-recv/croc-send.txt 2>/dev/null' | tr -d '\r')"
  expect_no_match "the relay only ever saw ciphertext (not the payload)" 'croc-smoke-payload-42' \
    "$(docker logs "$relay" 2>&1)"
  if ! received; then
    echo "  -- push log:"
    dx sh -c 'cat /tmp/push.log 2>/dev/null' | sed 's/^/     /'
  fi

  step "croc: push is off without a relay"
  local out
  out="$(docker exec -e CROC_RELAY= "$WS" push /workspace/croc-send.txt 2>&1 || true)"
  expect_match "push without CROC_RELAY says the feature is off" 'File transfer is off' "$out"

  docker rm -f "$relay" >/dev/null 2>&1
  stack_down
}

instance "${SMOKE_PROJECT}-croc" "$((SMOKE_PORT + 7))" 172.30.84 sub.yaml tests/smoke/croc.env croc_transfer

finish
