# shellcheck shell=bash
# Bringing a smoke-test workspace up and down. Source lib.sh first.
#
# Every instance is a compose project of its own (container, network, volumes, published port),
# so extra instances can run beside the main one and beside a real workspace on the same host.

# Linux hosts with AppArmor forbid the FUSE mount under Docker's default profile; Docker Desktop
# has none. SMOKE_APPARMOR=yes|no|auto (default).
apparmor_file() {
  case "${SMOKE_APPARMOR:-auto}" in
    yes) printf '%s' "$ROOT/docker-compose.apparmor.yml" ;;
    no) ;;
    *)
      if docker info --format '{{.SecurityOptions}}' 2>/dev/null | grep -q apparmor; then
        printf '%s' "$ROOT/docker-compose.apparmor.yml"
      fi
      ;;
  esac
}

# compose <args...>: docker compose for the current SMOKE_* settings. SMOKE_EXTRA_COMPOSE, when set,
# is one more override file (a check that needs a mount or a variable the others do not).
compose() {
  local extra
  extra="$(apparmor_file)"
  env SMOKE_PROJECT="$SMOKE_PROJECT" SMOKE_PORT="$SMOKE_PORT" TUNNEL_HOST_PORT="$SMOKE_PORT" \
    SMOKE_SUBNET="${SMOKE_SUBNET:-${SMOKE_NET}.0/24}" SMOKE_UPSTREAM_IP="${SMOKE_UPSTREAM_IP:-${SMOKE_NET}.10}" \
    SMOKE_IMAGE="${SMOKE_IMAGE:-dev-workspace:smoke}" SMOKE_SUB="${SMOKE_SUB:-sub.yaml}" \
    SMOKE_EXTRA_ENV="${SMOKE_EXTRA_ENV:-src/tests/smoke/empty.env}" \
    docker compose --project-directory "$ROOT" -p "$SMOKE_PROJECT" \
    -f "$ROOT/docker-compose.yml" -f "$SMOKE_DIR/docker-compose.smoke.yml" \
    ${extra:+-f "$extra"} ${SMOKE_EXTRA_COMPOSE:+-f "$SMOKE_EXTRA_COMPOSE"} "$@"
}

# stack_up: start the current instance and wait until the workspace has booted and is healthy.
stack_up() {
  compose up -d --quiet-pull >/dev/null 2>&1 || compose up -d
  wait_until 400 booted "$WS" || return 1
  wait_until 400 container_healthy "$WS"
}

stack_down() { compose down -v --remove-orphans >/dev/null 2>&1; }

# instance <project> <port> <subnet-prefix> <sub-file> <extra-env-file> <command...>
# Runs <command> with the SMOKE_* settings of another instance, then puts the caller's back. It
# runs in the current shell (not a subshell) so that the pass/fail counters of the checks inside
# still add up. The instance is NOT started or stopped for you.
#   sub-file        which provider the fake upstream serves (sub.yaml, sub-empty.yaml, ...)
#   extra-env-file  extra workspace environment, relative to the repository root
instance() {
  local project="$1" port="$2" prefix="$3" sub="$4" extra="$5" rc
  local keep_project="$SMOKE_PROJECT" keep_port="$SMOKE_PORT" keep_net="$SMOKE_NET" keep_ws="$WS" keep_up="$UP" keep_base="$BASE"
  shift 5
  SMOKE_PROJECT="$project"
  SMOKE_PORT="$port"
  SMOKE_NET="$prefix"
  SMOKE_SUBNET="${prefix}.0/24"
  SMOKE_UPSTREAM_IP="${prefix}.10"
  SMOKE_SUB="$sub"
  SMOKE_EXTRA_ENV="$extra"
  WS="${project}-workspace"
  UP="${project}-upstream"
  BASE="http://127.0.0.1:${port}"
  "$@"
  rc=$?
  SMOKE_PROJECT="$keep_project"
  SMOKE_PORT="$keep_port"
  SMOKE_NET="$keep_net"
  WS="$keep_ws"
  UP="$keep_up"
  BASE="$keep_base"
  unset SMOKE_SUBNET SMOKE_UPSTREAM_IP SMOKE_SUB SMOKE_EXTRA_ENV
  return "$rc"
}
