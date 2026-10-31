#!/usr/bin/env bash
# restic against an S3 endpoint, with the production wiring: the start-up creates the repository,
# the watcher writes snapshots, doctor reads the state. The endpoint is `rclone serve s3` (small, and
# S3-compatible enough for restic); R2 and AWS themselves cannot be reached from a test.
# What it shows beyond fixtures/local backups: restic speaks S3 here, an endpoint given by IP address
# bypasses Clash (no_proxy covers the private ranges) while one given by NAME goes through it.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
# shellcheck source=../stack.sh
. "$SMOKE_DIR/stack.sh"

s3_backup() {
  step "S3 backup: start-up and watcher"
  local s3="${SMOKE_PROJECT}-s3srv"
  compose up -d --no-start >/dev/null 2>&1
  docker rm -f "$s3" >/dev/null 2>&1
  docker run -d --name "$s3" --network "${SMOKE_PROJECT}_default" --ip 172.30.82.30 \
    --entrypoint sh rclone/rclone:latest \
    -c 'mkdir -p /data/smoke-bucket && exec rclone serve s3 /data --addr :9000 --auth-key smokekey,smokesecret' >/dev/null
  sleep 3
  if ! stack_up; then
    fail "the workspace did not become healthy" "$(docker logs --tail 15 "$WS" 2>&1)"
    docker rm -f "$s3" >/dev/null 2>&1
    return
  fi
  local log
  log="$(docker logs "$WS" 2>&1)"
  expect_match "the start-up created the repository in the bucket" 'Backup repo initialized' "$log"
  expect_no_match "without a backup warning" 'WARNING: Backup repo' "$log"
  # shellcheck disable=SC2329 # called by wait_until
  have_snapshot() { [ "$(snapshot_count)" -ge 1 ]; }
  expect_true "the watcher's baseline snapshot reaches the bucket" wait_until 150 have_snapshot
  expect_true "the repository files are in the bucket" docker exec "$s3" test -f /data/smoke-bucket/config
  expect_match "doctor reports the backup healthy" 'Backup \(S3/restic\) — last:' "$(dx doctor 2>&1 || true)"

  step "S3 backup: how the traffic is routed"
  expect_no_match "an endpoint given by IP address does not touch Clash" 'CONNECT 172\.30\.82\.30' "$(docker logs "$UP" 2>&1)"
  local by_name
  by_name="$(dx env RESTIC_REPOSITORY="s3:http://${s3}:9000/smoke-bucket" restic snapshots --no-lock 2>&1)"
  expect_match "an endpoint given by name works" 'snapshots?$|ID +Time' "$by_name"
  expect_match "and goes through Clash (the node saw the CONNECT)" "CONNECT ${s3}:9000" "$(docker logs "$UP" 2>&1)"

  step "S3 backup: restore and the failure modes"
  dx rm -rf /tmp/smoke-s3-restore
  dx restic restore latest --no-lock --target /tmp/smoke-s3-restore >/dev/null 2>&1
  expect_true "a restore from the bucket brings the workspace back" dx test -d /tmp/smoke-s3-restore/workspace
  dx rm -rf /tmp/smoke-s3-restore
  local out code
  out="$(docker exec -e RESTIC_PASSWORD=not-the-password "$WS" backup-init 2>&1)"
  code=$?
  expect_eq "backup-init with the wrong password fails (exit 1)" 1 "$code"
  expect_match "and says the password does not open it, rather than initializing" 'does not open it' "$out"
  out="$(dx backup-init 2>&1)"
  expect_match "backup-init on the existing repository is a no-op" 'already initialized' "$out"
  docker stop "$s3" >/dev/null
  out="$(dx backup-init 2>&1)"
  code=$?
  expect_eq "with the endpoint down it fails (exit 1)" 1 "$code"
  expect_match "and does not try to initialize a repository" 'Could not reach the repository' "$out"
  docker start "$s3" >/dev/null

  docker rm -f "$s3" >/dev/null 2>&1
  stack_down
}

instance "${SMOKE_PROJECT}-s3" "$((SMOKE_PORT + 5))" 172.30.82 sub.yaml src/tests/smoke/s3.env s3_backup

finish
