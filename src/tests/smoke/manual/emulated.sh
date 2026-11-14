#!/usr/bin/env bash
# Build the architecture-sensitive Rust and mihomo stages of the REAL Dockerfile under qemu-user.
# This is deliberately manual: a native CI runner is the authoritative multi-arch check, while a
# full desktop image takes hours under emulation.
#
#   src/tests/smoke/manual/emulated.sh [linux/amd64]
#
# Needs Docker with binfmt/qemu registered for the requested platform.
set -eu

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PLATFORM="${1:-linux/amd64}"
ARCH_TAG="${PLATFORM#linux/}"
RUST_IMAGE="dev-workspace:rust-builder-${ARCH_TAG}"
CLASH_IMAGE="dev-workspace:clash-bin-${ARCH_TAG}"

cleanup() {
  docker image rm "$RUST_IMAGE" "$CLASH_IMAGE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "== Rust runtime + Rust/WASM browser build for $PLATFORM"
docker build --platform "$PLATFORM" --progress plain --target rust-builder \
  -t "$RUST_IMAGE" -f "$ROOT/image/Dockerfile" "$ROOT"
docker run --rm --platform "$PLATFORM" --entrypoint /bin/sh "$RUST_IMAGE" -c '
  set -eu
  /sdw env-example >/dev/null
  test -s /web-dist/pkg/sdw_web_bg.wasm
  test -s /web-dist/pkg/sdw_web.js
  test ! -e /web-dist/pkg/sdw_web.d.ts
'

echo "== Pinned mihomo binary for $PLATFORM"
docker build --platform "$PLATFORM" --progress plain --target clash-bin \
  -t "$CLASH_IMAGE" -f "$ROOT/image/Dockerfile" "$ROOT"
docker run --rm --platform "$PLATFORM" --entrypoint /clash "$CLASH_IMAGE" -v

echo "== done: $PLATFORM Rust, WASM and mihomo stages built from the real Dockerfile"
