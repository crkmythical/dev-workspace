#!/usr/bin/env bash
# Remove .env from EVERY commit of this repository's history. Run it once, after .env has been
# untracked, and before the repository is pushed or shared anywhere.
#
#   deploy/scripts/purge-env-history.sh [--yes]
#
# It rewrites history, which cannot be undone in place, so it:
#   * refuses to run with uncommitted changes to tracked files,
#   * writes a full backup first (git bundle, next to the repository),
#   * does nothing without --yes (it prints what it would do).
# Rewriting does not make the leaked values safe: anyone who has a copy of the old history still has
# them. ROTATE them (checklist printed at the end) whether or not you rewrite.
# Written for the bash 3.2 and BSD tools macOS ships.
set -euo pipefail

YES=0
[ "${1:-}" = "--yes" ] && YES=1

cd "$(git rev-parse --show-toplevel)"
repo="$(basename "$PWD")"

if git ls-files --error-unmatch .env >/dev/null 2>&1; then
  echo "refusing: .env is still tracked. Run: git rm --cached .env  (and keep it in .gitignore)" >&2
  exit 1
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "refusing: there are uncommitted changes to tracked files; commit or stash them first." >&2
  exit 1
fi

count="$(git log --all --oneline -- .env | wc -l | tr -d ' ')"
echo "$count commit(s) touch .env."
if [ "$count" = 0 ]; then
  echo "nothing to purge."
  exit 0
fi

backup="../${repo}-before-purge-$(date +%Y%m%d-%H%M%S).bundle"
if [ "$YES" != 1 ]; then
  echo "dry run. With --yes this would:"
  echo "  1. write a backup bundle to $backup"
  echo "  2. rewrite all branches and tags so that .env never existed"
  echo "  3. expire the reflog and garbage-collect the old objects"
  exit 0
fi

git bundle create "$backup" --all
echo "backup written: $backup"

if command -v git-filter-repo >/dev/null 2>&1; then
  git filter-repo --path .env --invert-paths --force
else
  echo "git-filter-repo not found; using git filter-branch (slower, deprecated but available)"
  FILTER_BRANCH_SQUELCH_WARNING=1 git filter-branch --force \
    --index-filter 'git rm --cached --ignore-unmatch -q .env' \
    --prune-empty --tag-name-filter cat -- --all
fi
git reflog expire --expire=now --all
git gc --prune=now --aggressive -q

cat <<EOF

History rewritten. Now ROTATE what was in it (the old values are still in $backup and in any copy
anyone made before):
  1. PASSWORD is also the vault passphrase. For an existing vault:  gocryptfs -passwd /vault/cipher
     (inside the container), then put the new value in .env or, better, in secrets/password
     (make secrets-init).
  2. The Clash subscription URL carries a token: ask the provider to reset it, then update .env.
  3. VAULT_GIT_REPO, RESTIC_*, AWS_*, if you ever put them in .env: revoke and reissue them.
EOF
