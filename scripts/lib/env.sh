#!/usr/bin/env bash
# Shared helper for the host scripts. Source it; do not run it.
#
#   PROJECT_DIR=... source scripts/lib/env.sh
#   env_value TUNNEL_HOSTNAME [default]
#
# env_value reads NAME from the process environment, else from the project's .env, else
# prints the default. It never `source`s .env: values may hold spaces or shell characters.

env_value() {
  local name="$1" default="${2:-}" line value
  if [[ -n "${!name:-}" ]]; then
    printf '%s' "${!name}"
    return 0
  fi
  if [[ -n "${PROJECT_DIR:-}" && -f "$PROJECT_DIR/.env" ]]; then
    line=$(grep -E "^${name}=" "$PROJECT_DIR/.env" | tail -1 || true)
    if [[ -n "$line" ]]; then
      value="${line#*=}"
      value="${value%$'\r'}"                     # CRLF files
      value="${value#\"}"; value="${value%\"}"   # one pair of surrounding quotes
      value="${value#\'}"; value="${value%\'}"
      if [[ -n "$value" ]]; then
        printf '%s' "$value"
        return 0
      fi
    fi
  fi
  printf '%s' "$default"
}
