# shellcheck shell=bash
# shellcheck disable=SC2034 # ROOT, UP and BASE are used by the files that source this one
# Helpers shared by run.sh and the checks. Source it; do not run it.
# Written for the bash 3.2 that macOS ships (no associative arrays, no mapfile).

SMOKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SMOKE_DIR/../.." && pwd)"

: "${SMOKE_PROJECT:=sdw-smoke}"
: "${SMOKE_PORT:=18090}"
# PASSWORD in smoke.env: the master password of the throwaway workspace.
: "${SMOKE_PASSWORD:=smoke-master-pass-123}"
# First three octets of the main instance's network (a /24). Every instance has a network of its own,
# because Clash resolves names with its own DNS: the fake upstream needs a fixed address.
: "${SMOKE_NET:=172.30.77}"

WS="${SMOKE_PROJECT}-workspace"
UP="${SMOKE_PROJECT}-upstream"
BASE="http://127.0.0.1:${SMOKE_PORT}"

PASSED=0
FAILED=0
SKIPPED=0

step() { printf '\n== %s\n' "$1"; }
pass() {
  PASSED=$((PASSED + 1))
  printf '  PASS  %s\n' "$1"
}
fail() {
  FAILED=$((FAILED + 1))
  printf '  FAIL  %s\n' "$1"
  if [ -n "${2:-}" ]; then printf '        %s\n' "$2" | head -c 600; printf '\n'; fi
}
skip() {
  SKIPPED=$((SKIPPED + 1))
  printf '  SKIP  %s\n' "$1"
}

# Printed last by every check; run.sh adds the numbers up.
finish() {
  printf 'RESULT passed=%d failed=%d skipped=%d\n' "$PASSED" "$FAILED" "$SKIPPED"
  [ "$FAILED" -eq 0 ]
}

expect_eq() { # name expected actual
  if [ "$2" = "$3" ]; then pass "$1"; else fail "$1" "expected '$2', got '$3'"; fi
}
expect_match() { # name extended-regex text
  if printf '%s\n' "$3" | grep -Eq -- "$2"; then pass "$1"; else fail "$1" "no match for /$2/ in: $(printf '%s' "$3" | tail -c 400)"; fi
}
expect_no_match() { # name extended-regex text
  if printf '%s\n' "$3" | grep -Eq -- "$2"; then fail "$1" "unexpected /$2/ in: $(printf '%s' "$3" | tail -c 400)"; else pass "$1"; fi
}
expect_true() { # name command...
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then pass "$name"; else fail "$name" "command failed: $*"; fi
}
expect_false() { # name command...
  local name="$1"
  shift
  if "$@" >/dev/null 2>&1; then fail "$name" "command unexpectedly succeeded: $*"; else pass "$name"; fi
}

# HTTP status of a request to the published port ("000" when nothing answers).
http_code() { curl -s --noproxy '*' --max-time 30 -o /dev/null -w '%{http_code}' "$@" || true; }
expect_code() { # name expected-code curl-args...
  local name="$1" want="$2"
  shift 2
  expect_eq "$name" "$want" "$(http_code "$@")"
}

# docker exec into the main workspace (dxi: with stdin).
dx() { docker exec "$WS" "$@"; }
dxi() { docker exec -i "$WS" "$@"; }

# wait_until <seconds> <command...>: poll every 2 s until the command succeeds.
wait_until() {
  local limit="$1" waited=0
  shift
  until "$@" >/dev/null 2>&1; do
    waited=$((waited + 2))
    [ "$waited" -ge "$limit" ] && return 1
    sleep 2
  done
}

container_healthy() { [ "$(docker inspect -f '{{.State.Health.Status}}' "${1:-$WS}" 2>/dev/null)" = healthy ]; }
booted() { docker logs "${1:-$WS}" 2>&1 | grep -q 'Starting supervisord'; }

# Snapshots in the workspace's local restic repository.
snapshot_count() { dx restic snapshots --no-lock --json 2>/dev/null | grep -o '"short_id"' | wc -l | tr -d ' '; }
