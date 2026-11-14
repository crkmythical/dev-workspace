#!/usr/bin/env bash
# Tests for deploy/orchestration/lib.sh: a secrets file is read, never run, and never exported.
# Run: src/tests/scripts/orchestration-lib.test.sh   (needs only bash)
set -u
here="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../../deploy/orchestration/lib.sh
. "$here/../../../deploy/orchestration/lib.sh"
failed=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; else printf '  FAIL  %s\n        expected [%s]\n        got      [%s]\n' "$1" "$2" "$3"; failed=1; fi
}
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
marker="$dir/pwned"
{
  printf '# a comment\n\n'
  printf 'PLAIN=value\n'
  printf 'SPACED=has some spaces\n'
  printf 'DQ="double quoted"\n'
  printf "SQ='single quoted'\n"
  printf 'EQUALS=a=b=c\n'
  printf 'export EXPORTED=yes\n'
  printf 'CRLF=line\r\n'
  printf 'EVIL=$(touch %s)\n' "$marker"
  printf 'BACKTICK=`touch %s`\n' "$marker"
  printf 'bad-key=ignored\n'
  printf '9LEADING=ignored\n'
  printf 'NOEQUALS\n'
  printf 'CF_API_TOKEN=tok-123\n'
  printf 'LAST=no trailing newline'
} >"$dir/secrets.env"

load_env "$dir/secrets.env"
check "a plain value" value "${PLAIN-}"
check "spaces are kept" "has some spaces" "${SPACED-}"
check "double quotes are stripped" "double quoted" "${DQ-}"
check "single quotes are stripped" "single quoted" "${SQ-}"
check "only the first equals sign splits" "a=b=c" "${EQUALS-}"
check "a leading export is allowed" yes "${EXPORTED-}"
check "CRLF endings are tolerated" line "${CRLF-}"
check "a command substitution is kept as text, not run" "\$(touch $marker)" "${EVIL-}"
check "a backtick too" "\`touch $marker\`" "${BACKTICK-}"
check "nothing was executed" no "$([ -e "$marker" ] && echo yes || echo no)"
check "invalid names are skipped" 0 "$(compgen -A variable | grep -c -E '^(9LEADING|bad-key|NOEQUALS)$')"
check "a last line without a newline is read" "no trailing newline" "${LAST-}"

# Nothing is exported: a child process does not see the secret.
check "the token is not in a child's environment" "" "$(env | grep -c '^CF_API_TOKEN=' | sed 's/^0$//')"

# The curl config carries the header, is private, and is not on any command line.
export ORCH_TMPDIR="$dir"
cf_curl_init
cfg="$CF_CURL_CONFIG"
check "the curl config holds the header" 'header = "Authorization: Bearer tok-123"' "$(cat "$cfg")"
mode="$(stat -c %a "$cfg" 2>/dev/null || stat -f %Lp "$cfg")"
check "… and is private to the user" 600 "$mode"
cf_curl_init
check "initialising again replaces it rather than leaking a second" 1 "$(find "$dir" -name 'cf-curl.*' | wc -l | tr -d ' ')"
cf_cleanup
check "it is removed after use" 0 "$(find "$dir" -name 'cf-curl.*' | wc -l | tr -d ' ')"

check "an unreadable file is an error" 1 "$(load_env "$dir/missing" 2>/dev/null; echo $?)"
printf '\n%s\n' "$([ "$failed" = 0 ] && echo "all passed" || echo "FAILED")"
exit "$failed"
