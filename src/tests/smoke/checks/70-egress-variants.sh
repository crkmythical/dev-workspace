#!/usr/bin/env bash
# Egress is best effort: when it fails the workspace says so loudly and carries on. Three ways
# for it to fail, each in an instance of its own beside the main one (own project, port, subnet).
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

# --- 1. the subscription lists no nodes -------------------------------------------------------
empty_provider() {
  step "egress: the subscription has no nodes"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the banner says DEGRADED" 'Egress: +DEGRADED' "$LOG"
  expect_match "and why" 'subscription loaded no nodes' "$LOG"
  expect_match "the vault was unlocked all the same" 'Vault: +unlocked' "$LOG"
  expect_match "the status file records zero nodes" '"status":"degraded".*"providerNodes":0' "$(dx cat /var/run/egress-status 2>&1)"
  expect_match "a note is left in the vault" 'Egress DEGRADED' "$(dx cat /workspace/.notifications/egress-degraded.md 2>&1)"
  DOCTOR="$(dx doctor 2>&1 || true)"
  expect_match "doctor warns about it" '⚠ Egress \(Clash proxy\)' "$DOCTOR"
  expect_code "code-server is up" 302 "$BASE/"
  stack_down
}

# --- 2. Clash cannot start ----------------------------------------------------------------------
broken_clash() {
  step "egress: Clash cannot start"
  # The image's template is replaced by one the renderer refuses, so no config is rendered. Clash
  # must then NOT start: left without a config.yaml it writes a default and listens on 7890 as a
  # plain DIRECT proxy, which would pass the probe while no subscription is in use.
  # shellcheck disable=SC2034 # read by compose() in stack.sh
  SMOKE_EXTRA_COMPOSE="$SMOKE_DIR/fixtures/broken-clash.compose.yml"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    unset SMOKE_EXTRA_COMPOSE
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the render failure is reported" 'render-clash-config.*the Clash template has no' "$LOG"
  expect_match "Clash is not started without a rendered config" 'not starting Clash' "$LOG"
  expect_match "the banner says DOWN" 'Egress: +DOWN' "$LOG"
  expect_match "and that nothing is listening" 'nothing is listening on the proxy port' "$LOG"
  expect_false "no Clash process is running" dx pgrep -x clash
  expect_false "and nothing listens on the proxy port" dx nc -z 127.0.0.1 7890
  expect_false "no default config was left behind for a later start" dx test -e /etc/clash/config.yaml
  expect_match "the vault was unlocked all the same" 'Vault: +unlocked' "$LOG"
  expect_match "the status file says down" '"status":"down"' "$(dx cat /var/run/egress-status 2>&1)"
  DOCTOR="$(dx doctor 2>&1 || true)"
  expect_match "doctor fails the egress row" '✗ Egress \(Clash proxy\) — nothing listening' "$DOCTOR"
  expect_code "code-server is up" 302 "$BASE/"
  stack_down
  unset SMOKE_EXTRA_COMPOSE
}

# --- 3. a required variable is missing: the container refuses to start --------------------------
missing_subscription() {
  step "start-up: a required variable is missing"
  OUT="$(docker run --rm -e PASSWORD=x "$SMOKE_IMAGE" 2>&1)"
  CODE=$?
  expect_eq "the container exits with status 1" 1 "$CODE"
  expect_match "and names the variable" 'Required env var CLASH_SUBSCRIPTION_URL is not set' "$OUT"
}

instance "${SMOKE_PROJECT}-e1" "$((SMOKE_PORT + 1))" 172.30.78 sub-empty.yaml src/tests/smoke/empty.env empty_provider
instance "${SMOKE_PROJECT}-e2" "$((SMOKE_PORT + 2))" 172.30.79 sub.yaml src/tests/smoke/empty.env broken_clash
SMOKE_IMAGE="${SMOKE_IMAGE:-dev-workspace:smoke}" missing_subscription

finish
