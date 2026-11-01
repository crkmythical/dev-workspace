#!/usr/bin/env bash
# The other CPU architecture, through qemu-user, without building the whole image there.
#
# On an arm64 host this answers "does the amd64 half of the Dockerfile hold up?"; on an amd64 host,
# the reverse (pass linux/arm64). The architecture-specific part of image/Dockerfile is the
# download-and-verify blocks (Bun and its sha256, Caddy and its sha512, restic, Clash), and the
# JavaScript the repository ships. This script lifts exactly those RUN blocks out of the real
# Dockerfile (nothing is retyped) and builds them for the other platform: the pinned checksums must
# match the downloads, the binaries must be the right architecture (ELF machine type), and Bun must
# start. It then tries the repository's own packages and unit tests on that platform, which only
# works where Bun can run under qemu-user (here it did not). Go binaries (Caddy) can fail to start
# under qemu-user too; that is reported, not fatal. The rest of the Dockerfile is hundreds of
# megabytes of architecture-neutral apt and mise installs that would take hours under emulation.
# For the real thing, run src/tests/smoke/run.sh on a host of that architecture (a CI runner).
#
#   src/tests/smoke/manual/emulated.sh [linux/amd64]
#
# Needs Docker with binfmt/qemu registered for the platform (Docker Desktop and Colima have it).
set -eu

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PLATFORM="${1:-linux/amd64}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

python3 - "$ROOT/image/Dockerfile" "$TMP/Dockerfile" <<'PY'
import sys

src, out = sys.argv[1], sys.argv[2]
text = open(src).read()


def block(start_marker, end_marker):
    i = text.index(start_marker)
    return text[i : text.index(end_marker, i)]


parts = [
    "FROM debian:trixie-slim\n"
    "ENV DEBIAN_FRONTEND=noninteractive\n"
    "RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates unzip bzip2 git \\\n"
    "    && rm -rf /var/lib/apt/lists/*\n",
    block("ARG BUN_VERSION", 'ENV PATH="/root/.bun/bin:${PATH}"'),
    'ENV PATH="/root/.bun/bin:${PATH}"\nRUN bun --version\n',
    # Go binaries can fail to START under qemu-user (their regexp initialisers panic with
    # "expression too large"): that is the emulator, not the download. The checksum in the block
    # is what proves the download; the ELF machine type below proves it is the right architecture.
    block("ARG CADDY_VERSION", "# Install restic").replace(
        "\\\n    caddy version",
        "\\\n    (caddy version || echo 'caddy: does not start under qemu-user; checksum verified above')",
    ),
    block("ARG RESTIC_VERSION", "# Install mise"),
    block("ARG CLASH_VERSION", "# Install code-server"),
    "RUN uname -m && dpkg --print-architecture && bun --version\n"
    "RUN (restic version || echo 'restic: does not start under qemu-user'); (clash -v || echo 'clash: does not start under qemu-user')\n"
    "RUN set -eu; want=3e00; [ \"$(dpkg --print-architecture)\" = arm64 ] && want=b700; \\\n"
    "    for b in /root/.bun/bin/bun /usr/bin/caddy /usr/local/bin/restic /usr/bin/clash; do \\\n"
    "      got=\"$(od -An -tx1 -j18 -N2 \"$b\" | tr -d ' \\n')\"; echo \"$b e_machine=$got\"; \\\n"
    "      [ \"$got\" = \"$want\" ] || { echo \"$b is not the architecture of this platform\" >&2; exit 1; }; \\\n"
    "    done\n"
    # Bun's x64 build wants AVX2, which qemu-user's default CPU model lacks (it crashes with a
    # segmentation fault as soon as it runs JavaScript). qemu-user reads QEMU_CPU from the
    # environment of the emulated process.
    "ENV QEMU_CPU=max\n"
    "COPY package.json bunfig.toml bun.lock /opt/workspace/\n"
    "COPY packages/ /opt/workspace/packages/\n"
    "WORKDIR /opt/workspace\n"
    # Best effort: Bun often cannot run its JavaScript engine under qemu-user (measured here: "Out of
    # memory" inside libarchive during bun install, a segmentation fault without QEMU_CPU). Where it
    # works, the repository's own tests run on the other architecture.
    "RUN (bun install --frozen-lockfile && bun test packages/core) || "
    "echo 'NOTE: Bun cannot run the repository under qemu-user; the checks above are what emulation can show'\n",
]
open(out, "w").write("\n".join(parts))
PY

echo "== building for $PLATFORM from the real Dockerfile's blocks"
docker build --platform "$PLATFORM" --progress plain -f "$TMP/Dockerfile" \
  -t "dev-workspace:emulated-check" "$ROOT"
echo "== done: $PLATFORM downloads verified against their pinned checksums, binaries are the right architecture"
