#!/usr/bin/env bash
# Write the secrets that docker-compose.secrets.yml mounts into files, from the values in .env:
#
#   deploy/scripts/secrets-init.sh [--env-file FILE] [--force]     (or: make secrets-init)
#
#   secrets/password                 PASSWORD (generated if .env has none: a fixed file, unlike the
#                                    password the container would generate and lose on re-creation)
#   secrets/clash_subscription_url   CLASH_SUBSCRIPTION_URL
#
# Files are 0600 in a 0700 directory (git-ignored). A value is never printed. Nothing is deleted:
# once the overlay works, removing PASSWORD= and CLASH_SUBSCRIPTION_URL= from .env is yours to do.
# Written for the bash 3.2 and BSD tools macOS ships.
set -euo pipefail

cd "$(dirname "$0")/../.."
ENV_FILE=".env"
FORCE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --env-file) ENV_FILE="${2:?--env-file needs a file}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ -f "$ENV_FILE" ] || { echo "no $ENV_FILE here; copy .env.example first" >&2; exit 1; }

umask 077
mkdir -p secrets
chmod 700 secrets

# env_value VAR: the value of VAR= in the env file, one pair of surrounding quotes removed.
env_value() {
  local v
  v="$(sed -n "s/^$1=//p" "$ENV_FILE" | head -1)"
  v="${v%\"}"; v="${v#\"}"
  v="${v%\'}"; v="${v#\'}"
  printf '%s' "$v"
}

# write_secret FILE VALUE: writes unless the file exists (and --force was not given).
write_secret() {
  local file="secrets/$1" value="$2"
  if [ -e "$file" ] && [ "$FORCE" != 1 ]; then
    echo "  keep   $file (exists; --force replaces it)"
    return
  fi
  printf '%s' "$value" >"$file"
  chmod 600 "$file"
  echo "  wrote  $file"
}

echo "Writing secret files from $ENV_FILE:"

password="$(env_value PASSWORD)"
if [ -z "$password" ]; then
  # 16 random bytes as hex, the same shape the container generates when PASSWORD is empty.
  password="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
  echo "  $ENV_FILE has no PASSWORD: generated one (read it with: cat secrets/password)"
fi
write_secret password "$password"

url="$(env_value CLASH_SUBSCRIPTION_URL)"
if [ -z "$url" ]; then
  echo "  $ENV_FILE has no CLASH_SUBSCRIPTION_URL; put the URL in secrets/clash_subscription_url yourself" >&2
  : >secrets/clash_subscription_url
  chmod 600 secrets/clash_subscription_url
else
  write_secret clash_subscription_url "$url"
fi

cat <<'EOF'

Next:
  1. delete the PASSWORD= and CLASH_SUBSCRIPTION_URL= lines from .env
  2. COMPOSE_FILE=docker-compose.yml:docker-compose.secrets.yml  (in .env, after any other overlay)
  3. docker compose up -d
EOF
