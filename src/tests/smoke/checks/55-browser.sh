#!/usr/bin/env bash
# What only a browser can show: the sync page (WebCrypto, IndexedDB, a real upload), the emergency
# page, and the remote desktop's picture and input. A real Chrome drives them (browser/check.mjs,
# Playwright without its own browser download). Skipped when node or a Chrome is not there.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

BROWSER="$SMOKE_DIR/browser"

if ! command -v node >/dev/null 2>&1; then
  step "browser"
  if [ "${SMOKE_REQUIRE_BROWSER:-0}" = 1 ]; then
    fail "node is required for the release browser gate"
  else
    skip "node is not installed (the browser checks need it)"
  fi
  finish
  exit
fi
if [ ! -d "$BROWSER/node_modules/playwright-core" ]; then
  if ! (cd "$BROWSER" && npm ci --ignore-scripts --no-audit --no-fund) >/dev/null 2>&1; then
    step "browser"
    fail "could not install the integrity-locked Playwright driver with npm ci"
    finish
    exit
  fi
fi

STACK="$(dx cat /etc/sdw-desktop-stack 2>/dev/null)"
OUT="$(cd "$BROWSER" && SMOKE_BASE="$BASE" SMOKE_PASSWORD="$SMOKE_PASSWORD" SMOKE_STACK="${STACK:-kasmvnc}" \
  SHOTS_DIR="${SMOKE_SHOTS_DIR:-$BROWSER/shots}" node check.mjs 2>&1)"
BROWSER_EXIT=$?

# The script speaks the same PASS / FAIL / SKIP language; count its lines with the others.
while IFS= read -r line; do
  case "$line" in
    "  PASS  "*) pass "${line#  PASS  }" ;;
    "  FAIL  "*) fail "${line#  FAIL  }" ;;
    "  SKIP  "*) skip "${line#  SKIP  }" ;;
    *) printf '%s\n' "$line" ;;
  esac
done <<<"$OUT"

if [ "$BROWSER_EXIT" -ne 0 ] && ! printf '%s\n' "$OUT" | grep -q '^  FAIL  '; then
  fail "browser driver exited $BROWSER_EXIT without a structured failure" "$OUT"
fi

if printf '%s' "$OUT" | grep -q 'sync page: a real browser uploads'; then
  step "what the browser upload left behind"
  expect_eq "the file is in the vault, byte for byte" "uploaded by a real browser" \
    "$(dx cat /workspace/shared/browser-upload.txt 2>/dev/null)"
fi

finish
