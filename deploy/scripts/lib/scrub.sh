#!/usr/bin/env bash
# Shared helper for destroy.sh. Source it; do not run it.
#
# scrub_history FILE removes every line that mentions the workspace from a shell history
# file. It rewrites the file in place (owner and mode are kept) and leaves no backup copy
# of the original behind: `sed -i` differs between macOS and Linux, and `-i.bak` would
# keep exactly what is being scrubbed.

HISTORY_PATTERN='dev-workspace|vault|clash|cloudflared|gocryptfs|unlock-vault|init-vault'

scrub_history() {
  local file="$1" tmp
  [[ -f "$file" ]] || return 0
  tmp="$(mktemp)"
  # grep exits 1 when nothing is left to print, which is a fine outcome here.
  grep -v -E "$HISTORY_PATTERN" "$file" > "$tmp" || true
  cat "$tmp" > "$file"
  rm -f "$tmp"
}
