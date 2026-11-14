#!/usr/bin/env bash
# The vault: what is on disk, lock, unlock, a forced lock, and surviving a container re-creation.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

watcher_running() { dx supervisorctl status backup-watcher 2>/dev/null | grep -q RUNNING; }
wm_and_panel() { dx sh -c 'pgrep -x xfwm4 >/dev/null && pgrep -x xfce4-panel >/dev/null'; }
desktop_running() { dx supervisorctl status vnc:desktop-xfce-vnc 2>/dev/null | grep -q RUNNING; }
code_server_up() { [ "$(http_code "$BASE/")" = 302 ]; }
# The bracket keeps pgrep from matching the command line of the shell that runs it.
code_server_gone() { ! dx pgrep -f '/usr/lib/code-serve[r]' >/dev/null; }
# Asked of the sync-service itself from inside the container (its loopback port, no gate in between):
# the question is what the service says about the vault, and this is polled, so it must not log in
# (a login is rate limited) on every poll.
sync_state() { dx curl -s --max-time 30 http://127.0.0.1:8081/sync/api/doctor | sed -E 's/.*"state":"([a-z-]+)".*/\1/'; }

step "what is on disk"
dx sh -c 'echo "persisted-by-the-smoke-test" > /workspace/smoke-marker.txt && sync'
expect_eq "the file is readable through the mount" "persisted-by-the-smoke-test" "$(dx cat /workspace/smoke-marker.txt)"
expect_eq "the ciphertext directory does not contain the plaintext" 0 \
  "$(dx sh -c 'grep -rl persisted-by-the-smoke-test /vault/cipher 2>/dev/null | wc -l' | tr -d ' ')"
expect_eq "nor the file name" 0 "$(dx sh -c 'ls -A /vault/cipher | grep -c smoke-marker' | tr -d ' ')"
expect_true "the key file is in the ciphertext directory (what self-destruct shreds)" dx test -f /vault/cipher/gocryptfs.conf

step "lock: refused while something holds the mount"
# A process whose working directory is inside the mount makes a graceful unmount fail (EBUSY): the
# documented case in which lock-vault asks whether to force it. (code-server is one such holder
# once it has opened something there; an idle one holds nothing, so it is not relied on here.)
docker exec -d "$WS" sh -c 'cd /workspace && exec sleep 599'
sleep 2
OUT="$(printf 'n\n' | dxi lock-vault 2>&1)"
expect_match "lock-vault reports the busy mount" 'Could not unmount cleanly' "$OUT"
expect_match "declining the force aborts" 'Aborted' "$OUT"
expect_true "the vault is still mounted" dx mountpoint -q /workspace
expect_true "the backup watcher the attempt stopped runs again" wait_until 40 watcher_running
expect_match "and the user is told it was started again" 'Started again: .*backup watcher' "$OUT"
case "$(dx cat /etc/sdw-desktop-stack)" in
  kasmvnc | both)
    expect_true "so does the desktop that was running" wait_until 60 desktop_running
    expect_true "with its window manager and panel (a complete session, with no ssh-agent default in the way)" \
      wait_until 120 wm_and_panel
    ;;
esac

dx pkill -f 'sleep 599' || true
sleep 1
step "lock: clean, when nothing holds the mount"
# A listening unix socket inside the vault pins the mount without being an open file, so fuser
# cannot see it. The desktop session used to start an ssh-agent whose socket sat in the vault's
# HOME, survived the desktop being stopped, and made every graceful lock fail.
expect_eq "the running desktop has bound no unix socket inside the vault" 0 \
  "$(dx sh -c "grep -c ' /workspace' /proc/net/unix || true" | tr -d ' ')"
# Stopping code-server returns once its main process is gone; wait for its helpers (extension host,
# file watcher) too, so that nothing but the desktop and the backup watcher is left to hold the vault.
dx supervisorctl stop code-server >/dev/null
expect_true "(set-up) code-server and its helper processes are gone" wait_until 60 code_server_gone
OUT="$(dx lock-vault 2>&1)"
expect_match "lock-vault locks cleanly" 'Vault locked\.' "$OUT"
expect_match "and says the plaintext is gone" 'Plaintext data is no longer accessible' "$OUT"
expect_eq "the desktop session left nothing behind (helpers die with their group)" 0 \
  "$(dx sh -c 'ps -eo comm | grep -cE "at-spi-bus-laun|xfconfd|xfsettingsd|Xkasmvnc"' | tr -d ' ')"
expect_false "/workspace is no longer a mount" dx mountpoint -q /workspace
# What the image keeps in the bare mount point: the instructions for unlocking, nothing else.
expect_eq "only the unlock instructions are left there" README-UNLOCK.md "$(dx ls -A /workspace)"
expect_match "the backup watcher was stopped first (it holds the mount)" '^backup-watcher +STOPPED' "$(dx supervisorctl status 2>&1)"
expect_eq "the sync service no longer has its key" vault-locked "$(sync_state)"
expect_match "doctor says so" 'Vault mount — not mounted' "$(dx doctor 2>&1 || true)"
OUT="$(dx lock-vault 2>&1)"
expect_match "locking a locked vault is a no-op" 'not currently mounted' "$OUT"

step "unlock"
OUT="$(printf 'wrong-passphrase-xyz\n' | dxi unlock-vault 2>&1)"
expect_match "a wrong passphrase is refused" 'Failed to unlock vault' "$OUT"
expect_false "and nothing is mounted" dx mountpoint -q /workspace
OUT="$(printf '%s\n' "$SMOKE_PASSWORD" | dxi unlock-vault 2>&1)"
expect_match "the right passphrase unlocks" 'Vault unlocked\. Workspace available' "$OUT"
expect_true "/workspace is a mount again" dx mountpoint -q /workspace
expect_eq "the data is back" "persisted-by-the-smoke-test" "$(dx cat /workspace/smoke-marker.txt)"
expect_eq "the sync service has its key again" ready "$(sync_state)"
expect_true "the backup watcher is started again" wait_until 40 watcher_running
OUT="$(printf '%s\n' "$SMOKE_PASSWORD" | dxi unlock-vault 2>&1)"
expect_match "unlocking an unlocked vault is a no-op" 'already unlocked' "$OUT"
dx supervisorctl start code-server >/dev/null
expect_true "code-server runs again, on the unlocked vault" wait_until 40 code_server_up

step "forced lock"
# A process whose working directory is inside the mount makes a graceful unmount fail (EBUSY).
docker exec -d "$WS" sh -c 'cd /workspace && exec sleep 600'
sleep 2
OUT="$(printf 'y\n' | dxi lock-vault 2>&1)"
expect_match "the busy mount is reported" 'Could not unmount cleanly' "$OUT"
expect_match "the force is confirmed" 'Vault force-locked\.' "$OUT"
expect_match "the user is told that open files stay usable" 'can keep using them' "$OUT"
expect_false "yet /workspace is no longer a mount" dx mountpoint -q /workspace
dx pkill -f 'sleep 600' || true
sleep 2
OUT="$(printf '%s\n' "$SMOKE_PASSWORD" | dxi unlock-vault 2>&1)"
expect_match "the vault unlocks again afterwards" 'Vault unlocked\. Workspace available' "$OUT"
expect_eq "with its data" "persisted-by-the-smoke-test" "$(dx cat /workspace/smoke-marker.txt)"

step "re-creating the container (new image layer, same volumes)"
compose up -d --force-recreate --no-deps workspace >/dev/null 2>&1
if wait_until 400 booted "$WS" && wait_until 400 container_healthy "$WS"; then
  LOG="$(docker logs "$WS" 2>&1)"
  expect_match "the vault was unlocked at start-up, with the same password" 'Vault unlocked\.' "$LOG"
  expect_no_match "and not created again" 'Vault initialized' "$LOG"
  expect_eq "the data survived" "persisted-by-the-smoke-test" "$(dx cat /workspace/smoke-marker.txt)"
  expect_eq "the sync key is back" ready "$(sync_state)"
  expect_no_match "the destruction passphrase is announced only once" 'Destruction passphrase \(save this\)' "$LOG"
  expect_true "the backup watcher is running" wait_until 60 watcher_running
else
  fail "the re-created container did not come back healthy" "$(docker logs --tail 20 "$WS" 2>&1)"
fi

finish
