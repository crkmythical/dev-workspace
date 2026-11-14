#!/usr/bin/env bash
# The container came up the way the start-up claims it did.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

step "boot and health"
expect_eq "the container is healthy" healthy "$(docker inspect -f '{{.State.Health.Status}}' "$WS")"

LOG="$(docker logs "$WS" 2>&1)"
expect_match "the banner prints the published port, not a fixed one" "Code:      http://localhost:${SMOKE_PORT}" "$LOG"
expect_match "the vault was created on first start" 'Vault initialized' "$LOG"
expect_match "the vault was unlocked" 'Vault unlocked' "$LOG"
expect_match "the banner says the vault is unlocked" 'Vault: +unlocked' "$LOG"
expect_match "the ciphertext repository was prepared" 'Vault replication: ciphertext repository created' "$LOG"
expect_match "the backup repository was initialized" 'Backup repo initialized' "$LOG"
expect_match "the remote-destruct passphrase was announced once" 'Destruction passphrase \(save this\)' "$LOG"
expect_no_match "no boot step was reported as failed" 'boot step ".*" failed' "$LOG"

step "egress"
STATUS="$(dx cat /var/run/egress-status 2>/dev/null | sed -E 's/.*"status":"([a-z]+)".*/\1/')"
if [ "$STATUS" = confirmed ]; then
  pass "egress is confirmed through the proxy"
elif [ "$STATUS" = degraded ] && docker logs "$UP" 2>&1 | grep -q 'CONNECT www.google.com:443'; then
  # The request did take the Clash path (the fake node saw it) but the probe target is not
  # reachable from this network: a property of the network, not of the workspace.
  skip "egress is degraded because www.google.com is unreachable from here (the Clash path works)"
else
  fail "egress is confirmed through the proxy" "status: $STATUS"
fi
expect_match "traffic really went through Clash to the node" 'CONNECT ' "$(docker logs "$UP" 2>&1)"

step "services"
# A program only counts as RUNNING after its startsecs (backup-watcher: 5 s), yet the container
# turns healthy the moment Caddy answers, so on a fast VM this check used to race the slow starters.
# Wait for STARTING ones to settle instead of asserting at once.
not_starting() { ! dx supervisorctl status 2>&1 | grep -q ' STARTING '; }
wait_until 30 not_starting
SUP="$(dx supervisorctl status 2>&1)"
for program in caddy code-server sync-service vault-sync-cron clash-watcher backup-watcher; do
  expect_match "$program is running" "^${program} +RUNNING" "$SUP"
done

step "image contents"
expect_true "fuser is installed (the unmount escalation needs it)" dx sh -c 'command -v fuser'
expect_true "restic is installed" dx sh -c 'command -v restic'
STACK="$(dx cat /etc/sdw-desktop-stack 2>/dev/null)"
case "$STACK" in
  kasmvnc | selkies | both) pass "the image records its desktop stack ($STACK)" ;;
  *) fail "the image records its desktop stack" "got '$STACK'" ;;
esac

step "doctor"
DOCTOR="$(dx doctor 2>&1)"
expect_no_match "doctor reports no failed check" '✗' "$DOCTOR"
expect_match "doctor sees the vault mounted" 'Vault mount — /workspace mounted' "$DOCTOR"
case "$STACK" in
  selkies | both)
    # An open port only shows that something listens; this asks Selkies itself.
    expect_match "doctor asks the Selkies server, not just its port" \
      'Desktop client \(HTTP\) — answers at /desktop/api/health' "$DOCTOR"
    ;;
esac

finish
