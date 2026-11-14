#!/usr/bin/env bash
# Locate or build the single Rust host tool, then replace this compatibility process with it.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [[ -n "${SDW_HOST_BIN:-}" ]]; then
  exec "$SDW_HOST_BIN" "$@"
fi
BIN="$ROOT/src/rust/target/release/sdw-host"
if command -v cargo >/dev/null 2>&1; then
  # Cargo is incremental; this is near-instant when current and prevents a stale installed binary
  # from applying destructive policy that no longer matches this checkout.
  cargo build --quiet --locked --release --manifest-path "$ROOT/src/rust/Cargo.toml" -p sdw-host
  exec "$BIN" "$@"
fi
if command -v sdw-host >/dev/null 2>&1; then
  exec sdw-host "$@"
fi
echo "sdw-host is not installed and cargo is unavailable; install a release binary first." >&2
exit 127
