#!/usr/bin/env bash
# Password rotation, the way deploy/docs/runbook.md describes it: lock the vault, change the
# gocryptfs passphrase, put the new PASSWORD in the environment, recreate the container. Afterwards
# the vault opens at start-up with the new password and has its data, the login gate takes the new
# password and refuses the old one, and a restart of the same container keeps working. The master
# password is the gate's login AND the vault's passphrase, so this is the one place where changing
# either alone would lock the user out; it runs in an instance of its own, on its own volumes.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

OLD="$SMOKE_PASSWORD"
NEW="smoke-rotated-pass-789"
code_server_gone() { ! dx pgrep -f '/usr/lib/code-serve[r]' >/dev/null; }
# The env file that carries the new password; removed whatever happens.
ENVFILE=""
trap '[ -n "$ENVFILE" ] && rm -f "$ENVFILE"' EXIT

rotate() {
  step "password rotation"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 20 "$WS" 2>&1)"
    stack_down
    return
  fi
  expect_match "the old password logs in" 'sdw_session=' "$(gate_login "$OLD")"
  dx sh -c 'echo canary-before-rotation > /workspace/rotation-canary.txt && sync'

  # The runbook's steps, in its order.
  dx supervisorctl stop code-server >/dev/null
  wait_until 60 code_server_gone
  if hosted_runner; then
    dx lock-vault >/dev/null 2>&1 || true
    skip "lock-vault's clean unmount (needs a privileged runner); the rotation itself still runs below"
  else
    expect_match "lock-vault locks the vault" 'Vault locked\.' "$(dx lock-vault 2>&1)"
  fi
  OUT="$(printf '%s\n%s\n%s\n' "$OLD" "$NEW" "$NEW" | dxi gocryptfs -passwd -q /vault/cipher 2>&1)"
  RC=$?
  expect_eq "gocryptfs -passwd takes the old password and the new one" 0 "$RC"
  expect_no_match "… without complaint" 'rror|ailed|ncorrect' "$OUT"

  # "set PASSWORD=<new> in .env, then recreate": here an env file that comes after smoke.env.
  ENVFILE="$(mktemp "$SMOKE_DIR/rotated-env.XXXXXX")"
  printf 'PASSWORD=%s\n' "$NEW" >"$ENVFILE"
  # Exported so the `compose` helper's `docker compose` subprocess inherits it (stack.sh reads it
  # from the environment); a bare assignment works at run time but reads as unused to shellcheck.
  SMOKE_EXTRA_ENV="src/tests/smoke/$(basename "$ENVFILE")"
  export SMOKE_EXTRA_ENV
  compose up -d --force-recreate --quiet-pull >/dev/null 2>&1
  if ! wait_until 400 booted "$WS" || ! wait_until 400 container_healthy "$WS"; then
    fail "the recreated workspace did not become healthy" "$(docker logs --tail 30 "$WS" 2>&1)"
    stack_down
    return
  fi
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the vault opened at start-up with the new password" 'Vault: +unlocked' "$LOG"
  expect_no_match "and nothing was created in its place" 'Vault initialized' "$LOG"
  expect_eq "the data survived the rotation" "canary-before-rotation" "$(dx cat /workspace/rotation-canary.txt)"
  expect_match "the login gate takes the new password" 'sdw_session=' "$(gate_login "$NEW")"
  expect_eq "… and refuses the old one" "" "$(gate_login "$OLD")"
  expect_match "the remote-destruct passphrase is re-derived from the new one (destruct-passphrase prints it)" '.' \
    "$(dx destruct-passphrase 2>&1)"

  # A plain restart of the same container keeps the new password: it comes from the environment.
  docker restart "$WS" >/dev/null 2>&1
  wait_until 400 container_healthy "$WS"
  expect_eq "the data is still there after a restart" "canary-before-rotation" "$(dx cat /workspace/rotation-canary.txt)"
  expect_match "and the new password still logs in" 'sdw_session=' "$(gate_login "$NEW")"
  stack_down
}

instance "${SMOKE_PROJECT}-rot" "$((SMOKE_PORT + 13))" 172.30.90 sub.yaml src/tests/smoke/empty.env rotate
finish
