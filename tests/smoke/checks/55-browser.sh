#!/usr/bin/env bash
# What only a browser can show: the sync page (WebCrypto, IndexedDB, a real upload), the emergency
# page, and the remote desktop's picture and input. A real Chrome drives them (browser/check.mjs,
# Playwright without its own browser download). Skipped when node or a Chrome is not there.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

BROWSER="$SMOKE_DIR/browser"

if ! command -v node >/dev/null 2>&1; then
  step "browser"
  skip "node is not installed (the browser checks need it)"
  finish
  exit
fi
if [ ! -d "$BROWSER/node_modules/playwright-core" ]; then
  (cd "$BROWSER" && { bun install --frozen-lockfile || npm install --no-audit --no-fund; }) >/dev/null 2>&1
fi

STACK="$(dx cat /etc/sdw-desktop-stack 2>/dev/null)"
OUT="$(cd "$BROWSER" && SMOKE_BASE="$BASE" SMOKE_PASSWORD="$SMOKE_PASSWORD" SMOKE_STACK="${STACK:-kasmvnc}" \
  SHOTS_DIR="${SMOKE_SHOTS_DIR:-$BROWSER/shots}" node check.mjs 2>&1)"

# The script speaks the same PASS / FAIL / SKIP language; count its lines with the others.
while IFS= read -r line; do
  case "$line" in
    "  PASS  "*) pass "${line#  PASS  }" ;;
    "  FAIL  "*) fail "${line#  FAIL  }" ;;
    "  SKIP  "*) skip "${line#  SKIP  }" ;;
    *) printf '%s\n' "$line" ;;
  esac
done <<<"$OUT"

if printf '%s' "$OUT" | grep -q 'sync page: a real browser uploads'; then
  step "what the browser upload left behind"
  expect_eq "the file is in the vault, byte for byte" "uploaded by a real browser" \
    "$(dx cat /workspace/shared/browser-upload.txt 2>/dev/null)"
fi

finish
