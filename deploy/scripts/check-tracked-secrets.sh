#!/usr/bin/env bash
# Fail when a file that holds secrets is in git: .env, an orchestration secrets.env, anything under
# secrets/ (docker-compose.secrets.yml's files) or instances/ (per-instance orchestration state).
#
#   deploy/scripts/check-tracked-secrets.sh            every tracked file (CI)
#   deploy/scripts/check-tracked-secrets.sh --staged   only what is about to be committed (pre-commit)
#
# Templates (.env.example, secrets.env.example) are fine. Written for the bash 3.2 macOS ships.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
PATTERN='(^|/)(\.env|secrets\.env)$|(^|/)secrets/|(^|/)instances/'
if [ "${1:-}" = "--staged" ]; then
  files="$(git diff --cached --name-only --diff-filter=ACMR)"
  what="staged"
else
  files="$(git ls-files)"
  what="tracked"
fi
bad="$(printf '%s\n' "$files" | grep -E -- "$PATTERN" || true)"
# Files intentionally tracked in this private, single-user repo are allowlisted (one path per line,
# whole-line match). The list is absent from the throwaway test repos, so the guard stays strict there.
ALLOW_FILE="deploy/scripts/allowed-tracked-secrets.txt"
if [ -n "$bad" ] && [ -f "$ALLOW_FILE" ]; then
  bad="$(printf '%s\n' "$bad" | grep -vxF -f "$ALLOW_FILE" || true)"
fi
if [ -n "$bad" ]; then
  echo "secret files are $what:" >&2
  printf '%s\n' "$bad" | sed 's/^/  /' >&2
  echo "Untrack them (git rm --cached <file>) and keep them in .gitignore; rotate what they held." >&2
  exit 1
fi
