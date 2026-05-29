#!/bin/bash
# Minimal entrypoint — validates env then delegates to Bun TS
set -euo pipefail

# Inert mode: container was self-destructed
if [ -f /var/run/self-destruct/completed ]; then
  echo "Environment destroyed. Container is inert. Clean up: docker compose down -v"
  exec sleep infinity
fi

for v in CLASH_SUBSCRIPTION_URL; do
  if [[ -z "${!v:-}" ]]; then
    echo "ERROR: Required env var $v is not set." >&2
    exit 1
  fi
done

exec bun run /opt/workspace/packages/cli/src/entrypoint-main.ts
