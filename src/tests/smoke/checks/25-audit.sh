#!/usr/bin/env bash
# The audit trail (who logged in or tried to, when, with what result) and the least-privilege
# wiring around the gate: which user Caddy and Clash run as, what Caddy can see, and the login
# throttle that keeps the master password (also the vault passphrase) from being an online
# guessing oracle.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

# audit_count <event> <outcome> [substring]: how many audit lines match.
audit_count() {
  dx audit-log --json 1000 | grep -F "\"event\":\"$1\"" | grep -F "\"outcome\":\"$2\"" |
    grep -c -F -- "${3:-}" || true
}

step "who the services run as"
expect_eq "caddy runs as the caddy user" "caddy" "$(dx ps -o user= -C caddy | head -1 | tr -d ' ')"
expect_eq "clash runs as the clash user" "clash" "$(dx ps -o user= -C clash | head -1 | tr -d ' ')"
expect_eq "… in the clash group, with no supplementary groups" "clash" "$(dx ps -o group= -C clash | head -1 | tr -d ' ')"
expect_eq "clash owns its directory" "clash" "$(dx stat -c %U /etc/clash)"
# Read as caddy itself: root without CAP_SYS_PTRACE cannot read another uid's /proc/<pid>/environ
# (which is part of the point), and a read that fails would make "holds none" pass for nothing.
# The $(…) is meant to run inside the container, hence the single quotes.
# shellcheck disable=SC2016
CADDY_ENV="$(dx sh -c 'setpriv --reuid=caddy --regid=caddy --clear-groups cat /proc/$(pgrep -x caddy | head -1)/environ' | tr '\0' '\n')"
expect_match "caddy's environment is the minimal one it was started with" '^HOME=/var/lib/caddy$' "$CADDY_ENV"
expect_no_match "… and holds none of the container's secrets" \
  '^(PASSWORD|RESTIC_PASSWORD|AWS_SECRET_ACCESS_KEY|AWS_ACCESS_KEY_ID|CLASH_SUBSCRIPTION_URL|VAULT_GIT_REPO|DESKTOP_BCRYPT_HASH)=' "$CADDY_ENV"
if hosted_runner; then
  skip "root cannot read another user's /proc/<pid>/environ without CAP_SYS_PTRACE (a restricted CI runner grants the capability; verified locally)"
else
  # shellcheck disable=SC2016
  expect_false "root cannot read another user's /proc/<pid>/environ without CAP_SYS_PTRACE" \
    dx sh -c 'cat /proc/$(pgrep -x caddy | head -1)/environ'
fi
expect_false "caddy cannot read the unlocked vault (FUSE admits only the mounting user)" \
  dx setpriv --reuid=caddy --regid=caddy --clear-groups ls /workspace
expect_false "… nor the container's secrets on disk" \
  dx setpriv --reuid=caddy --regid=caddy --clear-groups ls /run/sdw
expect_match "the egress monitor runs" '^egress-monitor +RUNNING' "$(dx supervisorctl status 2>&1)"

step "a wrong password, then the right one"
# The address comes from a header the caller can set (it reaches the origin directly here), which
# is exactly why the log says "claimed".
WRONG_CODE="$(http_code -A 'smoke-audit/1' -H 'CF-Connecting-IP: 203.0.113.77' -X POST \
  --data-urlencode 'password=definitely-not-it-xyz' "$BASE/auth/login")"
expect_eq "the wrong password is refused" "401" "$WRONG_CODE"
GATE="$(gate_login)"
expect_match "the right password gets a session" '^sdw_session=' "$GATE"
expect_true "the wrong password is in the audit log, with the claimed address" \
  test "$(audit_count auth.login failure 203.0.113.77)" -ge 1
expect_true "… and the User-Agent" test "$(audit_count auth.login failure smoke-audit/1)" -ge 1
expect_true "the successful login is recorded" test "$(audit_count auth.login success)" -ge 1

step "what the log keeps and what it does not"
LOG="$(dx cat /var/log/sdw/audit.jsonl)"
expect_no_match "the wrong password is not in the log" 'definitely-not-it-xyz' "$LOG"
expect_no_match "the master password is not in the log" "$SMOKE_PASSWORD" "$LOG"
expect_eq "every line is a JSON object with a time" "0" \
  "$(printf '%s\n' "$LOG" | grep -c -v -E '^\{.*"ts":"[0-9]{4}-[0-9]{2}-[0-9]{2}T.*Z".*\}$' || true)"
expect_eq "the log is private to root (0600)" "600" "$(dx stat -c %a /var/log/sdw/audit.jsonl)"
expect_eq "… in a private directory (0700)" "700" "$(dx stat -c %a /var/log/sdw)"
expect_match "each event is also on the container's stdout" '"event":"auth.login"' "$(docker logs "$WS" 2>&1)"
expect_true "the boot recorded how egress looked" test "$(audit_count egress.change changed '"source":"boot"')" -ge 1
expect_match "audit-log prints it for a person" 'auth\.login +failure +ip=203\.0\.113\.77' "$(dx audit-log 50)"
expect_match "doctor reports the audit trail" 'Audit trail — 24h: [0-9]+ logins' "$(dx doctor 2>&1)"

step "the login throttle"
# 10 attempts a minute per claimed address; the 11th is refused before any password check.
THROTTLE_IP='198.51.100.99'
codes=""
for i in 1 2 3 4 5 6 7 8 9 10 11; do
  codes="$codes $(http_code -H "CF-Connecting-IP: $THROTTLE_IP" -X POST --data-urlencode "password=guess-$i" "$BASE/auth/login")"
done
expect_eq "ten wrong passwords get 401, the eleventh 429" " 401 401 401 401 401 401 401 401 401 401 429" "$codes"
RIGHT_AFTER="$(http_code -H "CF-Connecting-IP: $THROTTLE_IP" -X POST --data-urlencode "password=${SMOKE_PASSWORD}" "$BASE/auth/login")"
expect_eq "even the right password waits out the window (429, no cookie)" "429" "$RIGHT_AFTER"
expect_true "the throttle is in the audit log" test "$(audit_count auth.login throttled "$THROTTLE_IP")" -ge 1
expect_true "another address is not affected" test "$(http_code -H 'CF-Connecting-IP: 198.51.100.100' -X POST --data-urlencode "password=${SMOKE_PASSWORD}" "$BASE/auth/login")" = 303

step "self-destruct calls are recorded too"
code="$(http_code -A 'smoke-audit/2' -H 'CF-Connecting-IP: 203.0.113.88' -X POST -H 'content-type: application/json' \
  -d '{"passphrase":"wrong-destruct-pass"}' "$BASE/sync/api/destruct")"
expect_eq "a wrong passphrase is still a 404" "404" "$code"
expect_true "… and is in the log as a rejected attempt" \
  test "$(audit_count destruct.attempt rejected 203.0.113.88)" -ge 1
expect_no_match "the wrong passphrase is not in the log" 'wrong-destruct-pass' "$(dx cat /var/log/sdw/audit.jsonl)"
finish
