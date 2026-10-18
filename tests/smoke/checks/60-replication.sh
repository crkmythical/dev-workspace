#!/usr/bin/env bash
# Replication of the vault's ciphertext to a git remote (the cron job, vault-sync), run the way
# cron runs it: no container environment, absolute paths. The remote is a bare repository
# inside the container; smoke.env points VAULT_GIT_REPO at it.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

REMOTE=/srv/smoke-remote.git
# The real cron job (every 30 minutes) would race the manual runs below: pause it.
dx supervisorctl stop vault-sync-cron >/dev/null
trap 'dx supervisorctl start vault-sync-cron >/dev/null 2>&1' EXIT
job() { dx env -i HOME=/root PATH=/usr/bin:/bin /root/.bun/bin/bun run /opt/workspace/packages/cli/src/vault-sync.ts; }
remote_commits() { dx git -C "$REMOTE" rev-list --count main 2>/dev/null || echo 0; }
remote_head() { dx git -C "$REMOTE" rev-parse main 2>/dev/null; }
local_head() { dx git -C /vault/cipher rev-parse HEAD 2>/dev/null; }
sync_failures() { dx cat /var/run/vault-sync/consecutive-failures 2>/dev/null || echo 0; }

step "set-up by the start-up"
expect_match "the cron job is installed" 'vault-sync\.ts' "$(dx cat /etc/cron.d/vault-sync)"
expect_eq "the cipher directory has the configured origin" "$REMOTE" "$(dx git -C /vault/cipher remote get-url origin 2>&1)"
expect_true "it is ignoring what must never be committed" dx test -s /vault/cipher/.gitignore
dx git init -q --bare -b main "$REMOTE"

step "the first run"
OUT="$(job 2>&1)"
CODE=$?
expect_eq "succeeds" 0 "$CODE"
expect_eq "and is silent" "" "$OUT"
expect_true "a commit reached the remote" test "$(remote_commits)" -ge 1
expect_eq "and the remote is exactly where the vault is" "$(local_head)" "$(remote_head)"
expect_eq "authored with the fallback identity" Vault "$(dx git -C "$REMOTE" log -1 --format=%an main)"
FILES="$(dx git -C "$REMOTE" ls-tree -r --name-only main 2>&1)"
expect_match "it holds gocryptfs.conf (needed to restore)" 'gocryptfs\.conf' "$FILES"
expect_no_match "but no plaintext file name" 'smoke-marker|smoke-hello|backup-test|shared' "$FILES"
dx git -C "$REMOTE" grep -l persisted-by-the-smoke-test main >/dev/null 2>&1
expect_eq "nor plaintext content (git grep finds nothing)" 1 "$?"
expect_match "doctor reports the last sync" 'Vault sync — last:' "$(dx doctor 2>&1 || true)"

step "a quiet cycle"
job >/dev/null 2>&1
BEFORE="$(remote_commits)"
OUT="$(job 2>&1)"
expect_eq "does nothing and says nothing" "" "$OUT"
expect_eq "no empty commit" "$BEFORE" "$(remote_commits)"

step "a change"
dx sh -c 'echo replication-change > /workspace/replication-test.txt && sync'
BEFORE="$(remote_commits)"
job >/dev/null 2>&1
expect_eq "is committed and pushed" "$((BEFORE + 1))" "$(remote_commits)"
expect_eq "so the remote is where the vault is" "$(local_head)" "$(remote_head)"

step "the remote is unreachable"
dx mv "$REMOTE" "$REMOTE.away"
dx sh -c 'echo during-outage > /workspace/replication-outage.txt && sync'
OUT="$(job 2>&1)"
CODE=$?
expect_eq "the job fails (exit 1)" 1 "$CODE"
expect_match "and says why" 'vault-sync failed: push failed' "$OUT"
job >/dev/null 2>&1
job >/dev/null 2>&1
expect_eq "three failures are counted" 3 "$(sync_failures)"
expect_true "and a note is left in the vault" dx test -f /workspace/.notifications/sync-failure.md
expect_match "which points to doctor" 'doctor' "$(dx cat /workspace/.notifications/sync-failure.md 2>&1)"

step "the remote is back"
dx mv "$REMOTE.away" "$REMOTE"
BEFORE="$(remote_commits)"
job >/dev/null 2>&1
CODE=$?
expect_eq "the next run succeeds, pushing the commit that was waiting" 0 "$CODE"
expect_true "it arrived (the remote has more commits than before)" test "$(remote_commits)" -gt "$BEFORE"
expect_eq "and the remote is exactly where the vault is" "$(local_head)" "$(remote_head)"
expect_eq "the failure counter is cleared" 0 "$(sync_failures)"
expect_false "and so is the note" dx test -e /workspace/.notifications/sync-failure.md
FILES="$(dx git -C "$REMOTE" ls-tree -r --name-only main 2>&1)"
expect_no_match "still no plaintext file name on the remote" 'replication-|smoke-marker' "$FILES"

finish
