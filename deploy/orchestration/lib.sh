#!/usr/bin/env bash
# Shared by the orchestration scripts. Source it; do not run it.
#
# What it exists for: secrets.env and instances/<id>.env hold a Cloudflare API token, a tunnel token
# and a service-token secret. The scripts used to `set -a; . file; set +a`, which
#   * RUNS the file (anything in it that is not NAME=value is executed), and
#   * EXPORTS every secret into the environment of every program started afterwards (curl,
#     python3, docker compose, ...),
# and passed the tokens on command lines, where any process of the same user can read them with ps.
# Here a file is only READ, line by line, into shell variables that stay in this shell; the API
# token goes to curl through a private config file, and the tunnel token to cloudflared through the
# environment of that one process.
# Written for the bash 3.2 macOS ships.

# load_env FILE: set NAME=value for each such line; skip blanks and comments; strip one pair of
# surrounding quotes. Nothing is executed or exported.
load_env() {
  local file="$1" line key value
  [ -r "$file" ] || { echo "cannot read $file" >&2; return 1; }
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    case "$line" in '' | '#'*) continue ;; esac
    line="${line#export }"
    key="${line%%=*}"
    [ "$key" != "$line" ] || continue
    case "$key" in '' | *[!A-Za-z0-9_]* | [0-9]*) continue ;; esac
    value="${line#*=}"
    case "$value" in
      \"*\") value="${value#\"}"; value="${value%\"}" ;;
      \'*\') value="${value#\'}"; value="${value%\'}" ;;
    esac
    printf -v "$key" '%s' "$value"
  done <"$file"
}

# A private (0600) curl config holding the Authorization header. Call cf_curl_init ONCE, in the
# main shell and not inside $(...) (a subshell would lose the path and leak a file with the token),
# after CF_API_TOKEN is loaded; then pass `-K "$CF_CURL_CONFIG"` to curl. cf_cleanup deletes it.
# The token is thus never an argument of a process.
CF_CURL_CONFIG=""
cf_curl_init() {
  local dir="${ORCH_TMPDIR:-${TMPDIR:-/tmp}}"
  [ -n "${CF_API_TOKEN:-}" ] || { echo "CF_API_TOKEN is not set" >&2; return 1; }
  cf_cleanup
  CF_CURL_CONFIG="$(umask 077; mktemp "$dir/cf-curl.XXXXXX")" || return 1
  printf 'header = "Authorization: Bearer %s"\n' "$CF_API_TOKEN" >"$CF_CURL_CONFIG"
}
cf_cleanup() {
  [ -z "$CF_CURL_CONFIG" ] || rm -f "$CF_CURL_CONFIG"
  CF_CURL_CONFIG=""
}
