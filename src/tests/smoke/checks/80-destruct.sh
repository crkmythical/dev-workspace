#!/usr/bin/env bash
# Self-destruct, both ways in, on throwaway instances of their own: the HTTP endpoint (remote
# trigger) and the CLI run from INSIDE the vault. Afterwards the container must be inert and the
# keys gone. Nothing here can touch a real workspace: the instances are named, numbered and
# volume-isolated by the harness.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

inert() { docker logs "$WS" 2>&1 | grep -q 'Container is inert'; }

# What must be true of a destroyed container, however the destruction was triggered.
assert_destroyed() {
  expect_true "the container comes back in inert mode (restart policy, then the marker)" wait_until 150 inert
  expect_false "the vault key is gone" dx test -e /vault/cipher/gocryptfs.conf
  expect_false "the pentest vault key is gone" dx test -e /pentest/cipher/gocryptfs.conf
  expect_eq "nothing is left in the cipher directories, hidden entries included" 0 \
    "$(dx sh -c 'find /vault/cipher /pentest/cipher -mindepth 1 2>/dev/null | wc -l' | tr -d ' ')"
  expect_true "the completion marker is on the persistent volume" dx test -s /var/run/self-destruct/completed
  expect_false "the vault is not mounted" dx mountpoint -q /workspace
  expect_eq "nothing answers on the published port" 000 "$(http_code "$BASE/")"
  OUT="$(dx unlock-vault 2>&1 </dev/null || true)"
  expect_match "unlock-vault refuses for good" 'has been destroyed' "$OUT"
  docker restart "$WS" >/dev/null 2>&1
  sleep 8
  expect_eq "a restart stays inert" 000 "$(http_code "$BASE/")"
  expect_false "and still has no key" dx test -e /vault/cipher/gocryptfs.conf
}

# --- 1. the remote trigger (POST /sync/api/destruct) -----------------------------------------------
http_route() {
  step "remote self-destruct: the endpoint"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    return
  fi
  dx sh -c 'echo secret-data > /workspace/secret.txt && sync'
  dx init-pentest >/dev/null 2>&1
  PASS="$(dx destruct-passphrase 2>/dev/null)"
  expect_match "the passphrase can be re-derived from the vault passphrase" '^[A-Za-z0-9+/]{43}=$' "$PASS"
  expect_no_match "and is not the vault passphrase" "$SMOKE_PASSWORD" "$PASS"

  post() { # JSON body, extra curl args...
    local body="$1"
    shift
    curl -s --noproxy '*' --max-time 30 -o /dev/null -w '%{http_code}' -X POST \
      -H 'content-type: application/json' -d "$body" "$@" "$BASE/sync/api/destruct" || true
  }
  step "remote self-destruct: rate limits"
  expect_eq "a wrong passphrase looks like no endpoint at all" 404 "$(post '{"passphrase":"nope-1"}')"
  expect_eq "2nd wrong attempt" 404 "$(post '{"passphrase":"nope-2"}')"
  expect_eq "3rd wrong attempt" 404 "$(post '{"passphrase":"nope-3"}')"
  expect_eq "the 4th from the same client is refused (429)" 429 "$(post '{"passphrase":"nope-4"}')"
  expect_eq "even the right passphrase is refused while over budget" 429 "$(post "{\"passphrase\":\"$PASS\"}")"
  seen_429=0
  verified=0
  for n in 1 2 3 4 5 6 7 8 9 10 11 12; do
    code="$(post "{\"passphrase\":\"rotate-$n\"}" -H "CF-Connecting-IP: 10.9.8.$n" -H "X-Forwarded-For: 10.7.6.$n")"
    if [ "$code" = 429 ]; then seen_429=$((seen_429 + 1)); else verified=$((verified + 1)); fi
  done
  expect_true "rotating the client header cannot go on forever: the global cap answers 429" test "$seen_429" -ge 1
  expect_true "at most 10 attempts per window reach the check (here: $verified)" test "$verified" -le 10
  sleep 65 # let the fixed window pass

  step "remote self-destruct: the real thing"
  expect_eq "the vault passphrase itself is not the destruct passphrase" 404 \
    "$(post "{\"passphrase\":\"$SMOKE_PASSWORD\"}")"
  expect_true "the data is still there" dx test -s /workspace/secret.txt
  expect_eq "the right passphrase schedules it (202)" 202 "$(post "{\"passphrase\":\"$PASS\"}")"
  assert_destroyed
  stack_down
}

# --- 2. the CLI, run from inside the vault ---------------------------------------------------------
cli_route() {
  step "self-destruct: the command line"
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    return
  fi
  dx sh -c 'echo secret-data > /workspace/secret.txt && sync'
  dx init-pentest >/dev/null 2>&1
  OUT="$(printf 'no\n' | dxi self-destruct 2>&1)"
  expect_match "anything but 'destroy' aborts" 'Aborted' "$OUT"
  expect_true "and destroys nothing" dx test -s /workspace/secret.txt
  # Started from inside the mount: the process's own working directory is what keeps it busy.
  OUT="$(docker exec -w /workspace "$WS" self-destruct --force 2>&1 || true)"
  printf '%s\n' "$OUT" | sed 's/^/  | /'
  if hosted_runner; then
    skip "self-destruct's final clean unmount needs a privileged runner; the keys are still shredded (asserted below)"
  else
    expect_match "it ran to the end" 'destroyWorkspace complete' "$OUT"
  fi
  expect_match "and reported the point of no return" 'POINT OF NO RETURN' "$OUT"
  assert_destroyed
  stack_down
}

instance "${SMOKE_PROJECT}-d1" "$((SMOKE_PORT + 3))" 172.30.80 sub.yaml src/tests/smoke/empty.env http_route
instance "${SMOKE_PROJECT}-d2" "$((SMOKE_PORT + 4))" 172.30.81 sub.yaml src/tests/smoke/empty.env cli_route

finish
