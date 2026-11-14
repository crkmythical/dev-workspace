#!/usr/bin/env bash
# The real sync client (from core) against the real service, through Caddy, inside the container.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

step "sync protocol, end to end"
# The browser SPA stays TypeScript, so the sync client used to prove the protocol is run with Bun
# on the HOST (the runtime image carries no Bun) against the published port, through Caddy into the
# Rust sync-service. The upload still lands in the container's vault, checked below.
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
# The master password is the throwaway workspace's PASSWORD (smoke.env). It is not read back from
# the container: a workspace that takes its secrets from files has no PASSWORD in its environment.
OUT="$(cd "$REPO_ROOT" && bun run tests/smoke/fixtures/sync_e2e.ts "http://127.0.0.1:${SMOKE_PORT:-18090}" "$SMOKE_PASSWORD" 2>&1)"
printf '%s\n' "$OUT" | sed 's/^/  | /'
expect_match "every protocol check passed" '^ALL SYNC CHECKS PASSED' "$OUT"
expect_no_match "no protocol check failed" '^FAIL ' "$OUT"

step "what the upload left behind"
expect_eq "the committed file holds exactly the uploaded bytes" "hello from the smoke run" \
  "$(dx cat /workspace/shared/smoke-hello.txt 2>/dev/null)"
SUM="$(dx sh -c 'cd /workspace/shared && sha256sum smoke-hello.txt' | cut -d' ' -f1)"
expect_eq "its checksum is the one the client sealed" \
  "$(printf 'hello from the smoke run\n' | shasum -a 256 | cut -d' ' -f1)" "$SUM"

finish
