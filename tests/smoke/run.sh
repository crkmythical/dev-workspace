#!/usr/bin/env bash
# Smoke test: boot the real image next to a fake upstream and check what unit tests cannot.
#
#   tests/smoke/run.sh [--image NAME] [--build] [--base IMAGE] [--variant default|full]
#                      [--port N] [--project NAME] [--net A.B.C] [--only REGEX] [--services] [--keep] [--reuse]
#
#   --image NAME     image to test (default dev-workspace:smoke)
#   --build          build it first, from image/Dockerfile (BASE_IMAGE defaults to the production
#                    base, kalilinux/kali-rolling; APT_MIRROR to deb.debian.org)
#   --base IMAGE     base image for --build
#   --variant V      with --build: default (KasmVNC) or full (Selkies and KasmVNC)
#   --port N         host port of the main instance (default 18090)
#   --project NAME   compose project name (default sdw-smoke)
#   --net A.B.C      the main instance's network, a /24 (default 172.30.77; the other instances use
#                    fixed ones, so two full runs cannot share a Docker host)
#   --only REGEX     run only the checks whose file name matches
#   --services       also run services/*.sh (extra containers: MinIO, sshd, a git server, ...)
#   --keep           leave the main instance running afterwards
#   --reuse          use the main instance if it is already up and healthy (and leave it up),
#                    instead of starting a fresh one: for re-running a single check quickly
#
# Everything lives in its own compose project, container and network, and the repository's .env
# is never read, so this is safe to run on a host that has a real workspace. See README.md.
set -u

SMOKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE="dev-workspace:smoke"
BUILD=0
BASE_IMAGE_ARG="${SMOKE_BASE_IMAGE:-kalilinux/kali-rolling}"
VARIANT=default
ONLY=""
SERVICES=0
KEEP=0
REUSE=0

usage() { sed -n '2,/^set -u/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    --build) BUILD=1; shift ;;
    --base) BASE_IMAGE_ARG="$2"; shift 2 ;;
    --variant) VARIANT="$2"; shift 2 ;;
    --port) export SMOKE_PORT="$2"; shift 2 ;;
    --project) export SMOKE_PROJECT="$2"; shift 2 ;;
    --net) export SMOKE_NET="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --services) SERVICES=1; shift ;;
    --keep) KEEP=1; shift ;;
    --reuse) REUSE=1; shift ;;
    -h | --help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
export SMOKE_IMAGE="$IMAGE"

# shellcheck source=lib.sh
. "$SMOKE_DIR/lib.sh"
# shellcheck source=stack.sh
. "$SMOKE_DIR/stack.sh"

# --- preflight --------------------------------------------------------------------------------
command -v docker >/dev/null 2>&1 || { echo "docker not found" >&2; exit 2; }
docker info >/dev/null 2>&1 || { echo "cannot reach the Docker daemon" >&2; exit 2; }
compose_version="$(docker compose version --short 2>/dev/null | sed 's/^v//')"
if ! awk -v v="${compose_version:-0.0}" 'BEGIN { split(v, p, "."); exit !(p[1] > 2 || (p[1] == 2 && p[2] >= 24)) }'; then
  echo "Docker Compose >= 2.24 is required (found '${compose_version:-none}')" >&2
  exit 2
fi

# purge_project <name>: remove whatever a compose project left (containers, volumes, networks).
purge_project() {
  local p="$1" ids
  ids="$(docker ps -aq --filter "label=com.docker.compose.project=$p")"
  # shellcheck disable=SC2086 # a newline-separated list of ids, split on purpose
  [ -n "$ids" ] && docker rm -f $ids >/dev/null 2>&1
  ids="$(docker volume ls -q --filter "label=com.docker.compose.project=$p")"
  # shellcheck disable=SC2086
  [ -n "$ids" ] && docker volume rm $ids >/dev/null 2>&1
  ids="$(docker network ls -q --filter "label=com.docker.compose.project=$p")"
  # shellcheck disable=SC2086
  [ -n "$ids" ] && docker network rm $ids >/dev/null 2>&1
  return 0
}
cleanup_all() {
  local p
  for p in $(docker ps -a --format '{{.Label "com.docker.compose.project"}}' | sort -u | grep -E "^${SMOKE_PROJECT}(-[a-z0-9]+)?$" || true); do
    purge_project "$p"
  done
}

if [ "$BUILD" -eq 1 ]; then
  echo "== building $IMAGE ($VARIANT variant, base $BASE_IMAGE_ARG)"
  export BASE_IMAGE="$BASE_IMAGE_ARG" APT_MIRROR="${APT_MIRROR:-deb.debian.org}"
  case "$VARIANT" in
    full) export DESKTOP_STACK=both ;;
    default) export DESKTOP_STACK=kasmvnc ;;
    *) echo "unknown variant: $VARIANT" >&2; exit 2 ;;
  esac
  compose build workspace || { echo "build failed" >&2; exit 2; }
fi
docker image inspect "$IMAGE" >/dev/null 2>&1 || {
  echo "image $IMAGE not found (build it, or pass --build)" >&2
  exit 2
}

# --- run --------------------------------------------------------------------------------------
if [ "$REUSE" -eq 1 ] && container_healthy "$WS"; then
  echo "== reusing the running $WS"
else
  cleanup_all
  echo "== starting $SMOKE_PROJECT on port $SMOKE_PORT ($IMAGE)"
  if ! stack_up; then
    echo "the workspace did not become healthy" >&2
    docker logs --tail 80 "$WS" 2>&1 | sed 's/^/  | /' >&2
    [ "$KEEP" -eq 0 ] && [ "$REUSE" -eq 0 ] && cleanup_all
    exit 2
  fi
fi
if [ "$KEEP" -eq 0 ] && [ "$REUSE" -eq 0 ]; then trap cleanup_all EXIT; fi

total_pass=0
total_fail=0
total_skip=0
failed_checks=""

run_check() {
  local file="$1" out line
  echo
  echo "#### $(basename "$file")"
  out="$(mktemp)"
  bash "$file" 2>&1 | tee "$out"
  line="$(grep '^RESULT ' "$out" | tail -1)"
  rm -f "$out"
  if [ -z "$line" ]; then
    total_fail=$((total_fail + 1))
    failed_checks="$failed_checks $(basename "$file")(crashed)"
    return
  fi
  total_pass=$((total_pass + $(printf '%s' "$line" | sed -E 's/.*passed=([0-9]+).*/\1/')))
  total_skip=$((total_skip + $(printf '%s' "$line" | sed -E 's/.*skipped=([0-9]+).*/\1/')))
  local f
  f="$(printf '%s' "$line" | sed -E 's/.*failed=([0-9]+).*/\1/')"
  if [ "$f" -gt 0 ]; then
    total_fail=$((total_fail + f))
    failed_checks="$failed_checks $(basename "$file")"
  fi
}

for file in "$SMOKE_DIR"/checks/[0-9][0-9]-*.sh; do
  if [ -n "$ONLY" ] && ! printf '%s' "$(basename "$file")" | grep -Eq -- "$ONLY"; then continue; fi
  run_check "$file"
done
if [ "$SERVICES" -eq 1 ]; then
  for file in "$SMOKE_DIR"/services/*.sh; do
    if [ -n "$ONLY" ] && ! printf '%s' "$(basename "$file")" | grep -Eq -- "$ONLY"; then continue; fi
    run_check "$file"
  done
fi

echo
echo "== summary: $total_pass passed, $total_fail failed, $total_skip skipped"
if [ "$total_fail" -gt 0 ]; then
  echo "   failed:$failed_checks"
  echo "-- last log lines of $WS (if it still exists):"
  docker logs --tail 40 "$WS" 2>&1 | sed 's/^/  | /'
  exit 1
fi
