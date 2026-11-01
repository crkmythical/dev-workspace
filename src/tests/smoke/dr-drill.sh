#!/usr/bin/env bash
# Disaster-recovery drill (#4): prove that a TOTAL loss of the workspace's volumes is recoverable
# from the off-host restic backup. Unlike services/10-s3-backup.sh (which restores within one
# container's life), this destroys the container AND every volume, recreates a fresh stack, and
# restores from the backup that survived off-host — the real DR path.
#
#   first life → write a canary into the vault → back it up to S3
#   TOTAL LOSS → compose down -v (container + vault/cipher/clash/destruct volumes + network gone)
#   RECOVER    → fresh stack → restic restore → the canary is back, byte-for-byte
#
# The S3 repo lives on a STANDALONE docker volume so it survives `compose down -v` (that is the
# point: the backup is off the lost host). Run it directly (not auto-discovered by run.sh, because
# it is heavy and destructive):
#
#   SMOKE_IMAGE=dev-workspace:<img> SMOKE_PASSWORD=... bash src/tests/smoke/dr-drill.sh
set -uo pipefail
. "$(cd "$(dirname "$0")" && pwd)/lib.sh"
# shellcheck source=stack.sh
. "$SMOKE_DIR/stack.sh"

S3_VOL="dr-drill-s3"      # standalone backup store — NOT a compose volume, so `down -v` spares it
S3_SRV="dr-drill-s3srv"
S3_IP="172.30.82.30"      # must match RESTIC_REPOSITORY in s3.env

# (re)create the rclone S3 server on the current instance network, data on the standalone volume.
start_s3() {
  docker rm -f "$S3_SRV" >/dev/null 2>&1
  docker run -d --name "$S3_SRV" --network "${SMOKE_PROJECT}_default" --ip "$S3_IP" \
    -v "$S3_VOL:/data" --entrypoint sh rclone/rclone:latest \
    -c 'mkdir -p /data/smoke-bucket && exec rclone serve s3 /data --addr :9000 --auth-key smokekey,smokesecret' >/dev/null
  sleep 3
}

cleanup() {
  stack_down
  docker rm -f "$S3_SRV" >/dev/null 2>&1
  docker volume rm "$S3_VOL" >/dev/null 2>&1
}

drill() {
  docker volume create "$S3_VOL" >/dev/null

  step "DR drill — first life: write a canary, back it up off-host"
  compose up -d --no-start >/dev/null 2>&1
  start_s3
  if ! stack_up; then
    fail "the workspace became healthy" "$(docker logs --tail 15 "$WS" 2>&1)"; cleanup; return
  fi
  local canary="CANARY-$(date +%s)-${RANDOM}"
  dx sh -c "printf '%s\n' '$canary' > /workspace/dr-canary.txt"
  expect_true "canary written into the unlocked vault" dx test -f /workspace/dr-canary.txt
  # The watcher takes a BASELINE snapshot at boot, so snapshot_count>=1 can be true before the
  # canary is captured. Wait for a snapshot that actually CONTAINS the canary (restic ls latest),
  # or we would back up and restore the pre-canary baseline.
  # shellcheck disable=SC2329 # called by wait_until
  canary_in_backup() { dx restic ls latest 2>/dev/null | grep -q 'dr-canary\.txt'; }
  expect_true "a snapshot CONTAINING the canary reached the off-host S3 repo" wait_until 300 canary_in_backup
  # Capture THIS snapshot's id. Real DR must restore the pre-loss snapshot by id: after recreate,
  # the fresh container's own watcher writes a new EMPTY baseline to the same repo, so "latest"
  # would no longer be the snapshot that holds the canary.
  local snap
  snap="$(dx restic snapshots --no-lock --json 2>/dev/null | grep -o '"short_id":"[^"]*"' | tail -1 | cut -d'"' -f4)"
  expect_true "captured the pre-loss snapshot id ($snap)" test -n "$snap"

  step "DR drill — TOTAL LOSS: destroy the container and every workspace volume"
  stack_down
  docker rm -f "$S3_SRV" >/dev/null 2>&1
  expect_false "the workspace container is gone" docker inspect "$WS"
  expect_false "the vault-data volume is gone" docker volume inspect "${SMOKE_PROJECT}_vault-data"
  expect_true "the OFF-HOST backup volume survived the loss" docker volume inspect "$S3_VOL"

  step "DR drill — RECOVER: fresh stack, restore from the surviving backup"
  compose up -d --no-start >/dev/null 2>&1
  start_s3
  if ! stack_up; then
    fail "the fresh workspace became healthy" "$(docker logs --tail 15 "$WS" 2>&1)"; cleanup; return
  fi
  expect_false "the canary is gone from the fresh (empty) vault" dx test -f /workspace/dr-canary.txt
  local out
  out="$(dx restic restore "$snap" --target / 2>&1)"
  expect_match "restic restored the pre-loss snapshot by id" 'Summary|[Rr]estored|finished' "$out"
  expect_true "the canary file is back in the vault" dx test -f /workspace/dr-canary.txt
  expect_eq "and byte-for-byte identical to what was lost" "$canary" \
    "$(dx sh -c 'cat /workspace/dr-canary.txt 2>/dev/null' | tr -d '\r\n')"

  cleanup
}

echo "#### dr-drill.sh (image: ${SMOKE_IMAGE:-dev-workspace:smoke})"
instance "${SMOKE_PROJECT}-dr" "$((SMOKE_PORT + 6))" 172.30.82 sub.yaml src/tests/smoke/s3.env drill
finish
