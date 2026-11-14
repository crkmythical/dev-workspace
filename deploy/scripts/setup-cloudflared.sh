#!/usr/bin/env bash
# Compatibility entry point for Rust-managed named Cloudflare Tunnel setup.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
exec "$ROOT/deploy/scripts/sdw-host.sh" setup-tunnel --project-dir "$ROOT" "$@"
