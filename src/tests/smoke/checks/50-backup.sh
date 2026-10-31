#!/usr/bin/env bash
# The restic backup watcher against a repository inside the container: a snapshot after changes
# (debounced), what it leaves out, restore, and what happens when the repository goes away.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

REPO=/var/lib/smoke-restic/repo
watcher_running() { dx supervisorctl status backup-watcher 2>/dev/null | grep -q RUNNING; }
snapshot_grew() { [ "$(snapshot_count)" -gt "$1" ]; }
note_exists() { dx test -f /workspace/.notifications/backup-failure.md; }
note_gone() { ! note_exists; }
failures() { dx cat /var/run/backup-state/consecutive-failures 2>/dev/null || echo 0; }

step "the watcher"
expect_true "is running" watcher_running
expect_true "took a baseline snapshot at once (the first run is immediate)" wait_until 120 snapshot_grew 0
expect_match "ran retention after its first success" 'Prune completed successfully' "$(docker logs "$WS" 2>&1)"

step "a change becomes a snapshot"
N0="$(snapshot_count)"
dx sh -c 'mkdir -p /workspace/backup-test && echo first > /workspace/backup-test/first.txt'
expect_true "a snapshot follows the debounce window" wait_until 150 snapshot_grew "$N0"
expect_match "and holds the new file" '/workspace/backup-test/first\.txt' "$(dx restic ls latest --no-lock 2>&1)"

step "a burst of writes is one snapshot"
N1="$(snapshot_count)"
# shellcheck disable=SC2016 # expanded inside the container
dx sh -c 'for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do echo "burst $i" > /workspace/backup-test/burst-$i.txt; sleep 0.3; done'
expect_true "a snapshot follows" wait_until 150 snapshot_grew "$N1"
sleep 40
expect_eq "and no second one for the same burst" 1 "$(($(snapshot_count) - N1))"

step "what is left out"
# The project directory itself is a real change, so it goes first and is allowed to be saved.
N2="$(snapshot_count)"
dx sh -c 'mkdir -p /workspace/backup-test/app && echo readme > /workspace/backup-test/app/readme.txt'
expect_true "(set-up) the new project is saved" wait_until 150 snapshot_grew "$N2"
sleep 10
N2="$(snapshot_count)"
dx sh -c 'mkdir -p /workspace/backup-test/app/node_modules/pkg /workspace/backup-test/app/.git &&
  echo x > /workspace/backup-test/app/node_modules/pkg/index.js && echo y > /workspace/backup-test/app/.git/HEAD'
sleep 35
expect_eq "changes only under node_modules and .git trigger nothing" 0 "$(($(snapshot_count) - N2))"
dx sh -c 'echo real > /workspace/backup-test/app/main.txt'
expect_true "a real change in the same project does" wait_until 150 snapshot_grew "$N2"
LIST="$(dx restic ls latest --no-lock 2>&1)"
expect_match "main.txt is in the snapshot" 'backup-test/app/main\.txt' "$LIST"
expect_no_match "node_modules is not" 'node_modules' "$LIST"
expect_no_match "nor .git" 'backup-test/app/\.git' "$LIST"

step "restore"
dx rm -rf /tmp/smoke-restore
dx restic restore latest --no-lock --target /tmp/smoke-restore --include /workspace/backup-test/first.txt >/dev/null 2>&1
expect_eq "a file comes back byte for byte" first "$(dx cat /tmp/smoke-restore/workspace/backup-test/first.txt 2>/dev/null)"
dx rm -rf /tmp/smoke-restore
expect_match "restic check finds the repository healthy" 'no errors were found' "$(dx restic check --no-lock 2>&1)"

step "the repository goes away and comes back"
N3="$(snapshot_count)"
dx mv "$REPO" "$REPO.away"
dx sh -c 'echo during-outage > /workspace/backup-test/outage.txt'
expect_true "after three failed attempts a note is left in the vault" wait_until 240 note_exists
expect_match "which points to doctor" 'doctor' "$(dx cat /workspace/.notifications/backup-failure.md 2>&1)"
expect_match "doctor reports the failures" 'Backup \(S3/restic\) — [0-9]+ consecutive failures' "$(dx doctor 2>&1 || true)"
expect_true "the watcher keeps running meanwhile" watcher_running
dx mv "$REPO.away" "$REPO"
expect_true "once the repository is back a snapshot is taken" wait_until 240 snapshot_grew "$N3"
expect_true "and the note is removed: it must not outlive the problem" wait_until 30 note_gone
expect_eq "the failure counter is back to 0" 0 "$(failures)"
expect_match "the file written during the outage was saved" 'backup-test/outage\.txt' "$(dx restic ls latest --no-lock 2>&1)"

finish
