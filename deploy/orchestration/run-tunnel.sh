#!/usr/bin/env bash
# Run the cloudflared connector for an instance using the token in instances/<id>.env.
# The token stays in that gitignored file and is never passed on this script's command line.
# shellcheck disable=SC1090,SC1091  # instances/<id>.env is resolved at runtime
set -eu
id="${1:?usage: run-tunnel.sh <instance-id>}"
d="$(cd "$(dirname "$0")" && pwd)"
set -a; . "$d/instances/$id.env"; set +a
exec cloudflared tunnel --no-autoupdate run --token "$TUNNEL_TOKEN"
