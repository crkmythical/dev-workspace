#!/usr/bin/env bash
# Reverse up.sh: stop and remove the instance's container AND its volumes (use-and-destroy), then
# tear down its Cloudflare edge (connector, Access app, service token, DNS, tunnel).
#
#   down.sh <id>
set -u
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
id="${1:?usage: down.sh <id>}"

# Container + volumes first (the vault is ephemeral for these instances; -v wipes it on purpose).
WORKSPACE_CONTAINER_NAME="ws-$id" docker compose -p "ws-$id" \
  -f "$root/docker-compose.yml" -f "$root/docker-compose.apparmor.yml" down -v 2>&1 | tail -5 || true

# Then the edge.
bash "$here/teardown.sh" "$id"
