#!/usr/bin/env bash
# The real sync client (from core) against the real service, through Caddy, inside the container.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

step "sync protocol, end to end"
# The fixture imports core by relative path, so it is copied to the same path in the container.
dx mkdir -p /opt/workspace/tests/smoke/fixtures
docker cp "$SMOKE_DIR/fixtures/sync_e2e.ts" "$WS:/opt/workspace/tests/smoke/fixtures/sync_e2e.ts" >/dev/null
# shellcheck disable=SC2016 # $PASSWORD is the container's, not ours
OUT="$(dx sh -c 'cd /opt/workspace && bun run tests/smoke/fixtures/sync_e2e.ts http://127.0.0.1:8080 "$PASSWORD"' 2>&1)"
printf '%s\n' "$OUT" | sed 's/^/  | /'
expect_match "every protocol check passed" '^ALL SYNC CHECKS PASSED' "$OUT"
expect_no_match "no protocol check failed" '^FAIL ' "$OUT"
dx rm -rf /opt/workspace/tests

step "what the upload left behind"
expect_eq "the committed file holds exactly the uploaded bytes" "hello from the smoke run" \
  "$(dx cat /workspace/shared/smoke-hello.txt 2>/dev/null)"
SUM="$(dx sh -c 'cd /workspace/shared && sha256sum smoke-hello.txt' | cut -d' ' -f1)"
expect_eq "its checksum is the one the client sealed" \
  "$(printf 'hello from the smoke run\n' | shasum -a 256 | cut -d' ' -f1)" "$SUM"

finish
