#!/usr/bin/env bash
# Run the cloudflared connector for an instance using the token in instances/<id>.env.
# The token stays in that gitignored file; cloudflared gets it in its environment (TUNNEL_TOKEN),
# never on a command line where `ps` would show it.
# shellcheck disable=SC1091  # lib.sh is resolved at runtime
set -eu
id="${1:?usage: run-tunnel.sh <instance-id>}"
d="$(cd "$(dirname "$0")" && pwd)"
. "$d/lib.sh"
load_env "$d/instances/$id.env"
TUNNEL_TOKEN="$TUNNEL_TOKEN" exec cloudflared tunnel --no-autoupdate run
