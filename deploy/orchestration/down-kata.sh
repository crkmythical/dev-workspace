#!/usr/bin/env bash
# Tear down a Kata workspace instance started by up-kata.sh: remove the CRI pod (and its container),
# and, unless --keep-volumes, the per-instance host-path volumes (vault ciphertext included).
#
#   down-kata.sh <id> [--keep-volumes]
#
# Runs on the container host (bare-metal Linux, or the Colima VM via `colima ssh`).
set -euo pipefail

id="${1:?usage: down-kata.sh <id> [--keep-volumes]}"
keep_volumes=0
[ "${2:-}" = "--keep-volumes" ] && keep_volumes=1
pod_name="sdw-ws-${id}"
vol_root="/var/lib/sdw-kata/${id}"

command -v crictl >/dev/null || { echo "down-kata: crictl not found" >&2; exit 1; }

removed=0
for pod in $(crictl pods -q --name "$pod_name" 2>/dev/null); do
  crictl rmp -f "$pod" >/dev/null 2>&1 && removed=1
done
[ "$removed" = 1 ] && echo ">> removed pod ${pod_name}" || echo ">> no running pod ${pod_name}"

if [ "$keep_volumes" = 1 ]; then
  echo ">> kept volumes at ${vol_root}"
elif [ -d "$vol_root" ]; then
  # The vault ciphertext lives here; removing it is the point of a teardown (matches compose down -v).
  rm -rf "$vol_root"
  echo ">> removed volumes ${vol_root}"
fi
