#!/usr/bin/env bash
# Tests for the guard that keeps secrets out of git: deploy/scripts/check-tracked-secrets.sh and the
# pre-commit hook built on it. They run in a throwaway repository, never in this one.
# Run: src/tests/scripts/secrets-guard.test.sh   (needs bash and git)
set -u
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
failed=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; else printf '  FAIL  %s\n        expected [%s]\n        got      [%s]\n' "$1" "$2" "$3"; failed=1; fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
git_() { git -C "$work/r" -c user.email=t@example.test -c user.name=t -c commit.gpgsign=false "$@"; }

fresh() {
  rm -rf "$work/r"
  mkdir -p "$work/r/deploy/scripts/git-hooks"
  cp "$repo/deploy/scripts/check-tracked-secrets.sh" "$work/r/deploy/scripts/"
  cp "$repo/deploy/scripts/git-hooks/pre-commit" "$work/r/deploy/scripts/git-hooks/"
  git -C "$work/r" init -q -b main
  git -C "$work/r" config core.hooksPath deploy/scripts/git-hooks
  echo hello >"$work/r/README.md"
  git_ add README.md deploy
  git_ commit -q -m init --no-verify
}
commit_rc() { # message → exit status of the commit with the hook ON
  git_ commit -q -m "$1" >"$work/out" 2>&1
  echo $?
}

echo "== files that must never be tracked"
fresh
echo "PASSWORD=whatever-it-is" >"$work/r/.env"
git_ add -f .env
check "a staged .env is refused" 1 "$(commit_rc add-env)"
grep -q '\.env' "$work/out" && r=named || r=silent
check "… and the refusal names the file" named "$r"

fresh
mkdir -p "$work/r/secrets" && echo x >"$work/r/secrets/password"
git_ add -f secrets/password
check "a file under secrets/ is refused" 1 "$(commit_rc add-secrets)"

fresh
mkdir -p "$work/r/deploy/orchestration" && echo "CF_API_TOKEN=x" >"$work/r/deploy/orchestration/secrets.env"
git_ add -f deploy/orchestration/secrets.env
check "an orchestration secrets.env is refused" 1 "$(commit_rc add-orch)"

fresh
echo "PASSWORD=" >"$work/r/.env.example"
git_ add .env.example
check "the template .env.example is fine" 0 "$(commit_rc add-example)"

echo "== a secret's VALUE, wherever it ends up"
fresh
printf 'PASSWORD=correct-horse-battery\nCLASH_SUBSCRIPTION_URL=https://sub.example.test/token-123456\nGIT_USER_NAME=Ada\n' >"$work/r/.env"
echo "note: the password is correct-horse-battery" >"$work/r/notes.txt"
git_ add notes.txt
check "a staged change that contains the password is refused" 1 "$(commit_rc leak-password)"
grep -q 'PASSWORD' "$work/out" && r=named || r=silent
check "… and says which variable, not its value" named "$r"
grep -q 'correct-horse-battery' "$work/out" && r=printed || r=hidden
check "… without printing the value" hidden "$r"

fresh
printf 'PASSWORD=correct-horse-battery\nCLASH_SUBSCRIPTION_URL=https://sub.example.test/token-123456\n' >"$work/r/.env"
echo "curl https://sub.example.test/token-123456" >"$work/r/run.sh"
git_ add run.sh
check "… and so does the subscription URL" 1 "$(commit_rc leak-sub)"

fresh
printf 'PASSWORD=correct-horse-battery\nGIT_USER_NAME=Ada-Lovelace-Long\n' >"$work/r/.env"
echo "author: Ada-Lovelace-Long" >"$work/r/authors.txt"
git_ add authors.txt
check "a variable that is not a secret is not guarded" 0 "$(commit_rc not-a-secret)"

fresh
printf 'PASSWORD=short\n' >"$work/r/.env"
echo "the word short appears here" >"$work/r/words.txt"
git_ add words.txt
check "a trivial value (under 8 characters) proves nothing and is not guarded" 0 "$(commit_rc trivial)"

fresh
printf 'PASSWORD=correct-horse-battery\n' >"$work/r/.env"
echo "an ordinary change" >>"$work/r/README.md"
git_ add README.md
check "an ordinary change goes through while a .env exists" 0 "$(commit_rc ordinary)"

echo "== the CI form: every tracked file"
fresh
ci_rc() { (cd "$work/r" && deploy/scripts/check-tracked-secrets.sh >/dev/null 2>&1; echo $?); }
check "a clean tree passes" 0 "$(ci_rc)"
echo x >"$work/r/.env"
git_ add -f .env
git_ commit -q -m sneaked --no-verify
check "a tracked .env fails it, however it got there" 1 "$(ci_rc)"

if [ "$failed" -eq 0 ]; then echo "all passed"; else echo "FAILED"; fi
exit "$failed"
