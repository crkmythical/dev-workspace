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
  compose up -d --no-start >/dev/null 2>&1
  # The clash-config volume holds the template the start-up renders; break it.
  docker run --rm -v "${SMOKE_PROJECT}_clash-config:/etc/clash" alpine \
    sh -c 'printf "mixed-port: [broken\n  - : yaml {\n" > /etc/clash/config.yaml.template' >/dev/null 2>&1
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the banner says DOWN" 'Egress: +DOWN' "$LOG"
  expect_match "and that nothing is listening" 'nothing is listening on the proxy port' "$LOG"
  expect_match "the vault was unlocked all the same" 'Vault: +unlocked' "$LOG"
  expect_match "the status file says down" '"status":"down"' "$(dx cat /var/run/egress-status 2>&1)"
  DOCTOR="$(dx doctor 2>&1 || true)"
  expect_match "doctor fails the egress row" '✗ Egress \(Clash proxy\) — nothing listening' "$DOCTOR"
  expect_code "code-server is up" 302 "$BASE/"
  stack_down
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
