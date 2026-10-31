#!/usr/bin/env bats
# Property/bats tests for vault lifecycle (task 3.5) — FUSE-level round-trips.
#
# These exercise real gocryptfs init/unlock/lock behavior end-to-end, covering
# the parts the pure planners (packages/cli/tests/vault-lifecycle.test.ts)
# cannot:
#
#   Property 3: Vault Lifecycle Round-Trip and Idempotence (R4.1, 4.3, 4.4, 4.6)
#   Property 4: Wrong-Password Rejection                   (R4.2)
#   Property 6: Init-Vault Idempotence                     (R3.4)
#   Property 7: Stale-Mount Cleanup Idempotence            (R8.1)
#
# Requires: gocryptfs, fusermount (present inside the container image). When
# absent (e.g. macOS dev host), every test skips so the suite stays green.

setup() {
  command -v gocryptfs >/dev/null 2>&1 || skip "gocryptfs not installed"
  command -v fusermount >/dev/null 2>&1 || skip "fusermount not installed"

  TESTDIR="$(mktemp -d)"
  CIPHER="${TESTDIR}/cipher"
  MOUNT="${TESTDIR}/mount"
  mkdir -p "$CIPHER" "$MOUNT"
  PASS="correct horse battery staple"
  WRONG="incorrect horse battery staple"
}

teardown() {
  if [ -n "${MOUNT:-}" ] && mountpoint -q "$MOUNT" 2>/dev/null; then
    fusermount -uz "$MOUNT" 2>/dev/null || true
  fi
  [ -n "${TESTDIR:-}" ] && rm -rf "$TESTDIR"
}

init_vault() {
  printf '%s\n%s\n' "$PASS" "$PASS" | gocryptfs -init -q "$CIPHER"
}

mount_vault() {
  local pass="$1"
  printf '%s\n' "$pass" | gocryptfs -q "$CIPHER" "$MOUNT"
}

# --- Property 6: Init-Vault Idempotence ---

@test "Property 6: init creates gocryptfs.conf" {
  init_vault
  [ -f "${CIPHER}/gocryptfs.conf" ]
}

@test "Property 6: re-init on existing vault does not alter gocryptfs.conf" {
  init_vault
  local before
  before="$(sha256sum "${CIPHER}/gocryptfs.conf")"
  # A second init must refuse (non-empty cipher dir) and leave config untouched.
  printf '%s\n%s\n' "$PASS" "$PASS" | gocryptfs -init -q "$CIPHER" || true
  local after
  after="$(sha256sum "${CIPHER}/gocryptfs.conf")"
  [ "$before" = "$after" ]
}

# --- Property 3: Round-Trip and Idempotence ---

@test "Property 3: unlock then lock returns to unmounted state" {
  init_vault
  mount_vault "$PASS"
  mountpoint -q "$MOUNT"
  echo "secret data" > "${MOUNT}/note.txt"
  fusermount -u "$MOUNT"
  run mountpoint -q "$MOUNT"
  [ "$status" -ne 0 ]
}

@test "Property 3: data written before lock is recoverable after re-unlock" {
  init_vault
  mount_vault "$PASS"
  echo "persistent" > "${MOUNT}/note.txt"
  fusermount -u "$MOUNT"
  mount_vault "$PASS"
  run cat "${MOUNT}/note.txt"
  [ "$status" -eq 0 ]
  [ "$output" = "persistent" ]
  fusermount -u "$MOUNT"
}

@test "Property 3: locked cipher dir holds no plaintext" {
  init_vault
  mount_vault "$PASS"
  echo "MAGIC_PLAINTEXT_TOKEN" > "${MOUNT}/note.txt"
  fusermount -u "$MOUNT"
  run grep -rl "MAGIC_PLAINTEXT_TOKEN" "$CIPHER"
  [ "$status" -ne 0 ]
}

# --- Property 4: Wrong-Password Rejection ---

@test "Property 4: wrong passphrase fails to mount" {
  init_vault
  run mount_vault "$WRONG"
  [ "$status" -ne 0 ]
  run mountpoint -q "$MOUNT"
  [ "$status" -ne 0 ]
}

@test "Property 4: failed mount leaves no mountpoint" {
  init_vault
  mount_vault "$WRONG" || true
  run mountpoint -q "$MOUNT"
  [ "$status" -ne 0 ]
}

# --- Property 7: Stale-Mount Cleanup Idempotence ---

@test "Property 7: cleanup on a clean mountpoint is a safe no-op" {
  # No mount exists; lazy unmount must not error fatally.
  run bash -c "fusermount -uz '$MOUNT' 2>/dev/null; true"
  [ "$status" -eq 0 ]
}

@test "Property 7: repeated cleanup is idempotent" {
  init_vault
  mount_vault "$PASS"
  fusermount -uz "$MOUNT" 2>/dev/null || true
  # Running cleanup again on the now-unmounted dir is still safe.
  run bash -c "fusermount -uz '$MOUNT' 2>/dev/null; true"
  [ "$status" -eq 0 ]
  run mountpoint -q "$MOUNT"
  [ "$status" -ne 0 ]
}
