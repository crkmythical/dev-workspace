#!/bin/sh
# fetch-tool NAME DEST — download the pinned release of a third-party tool and refuse anything that
# does not match its pinned checksum.
#
# NAME is a file <tools dir>/NAME.env written by deploy/scripts/update-tools.sh: the version and,
# per architecture, the URL and the sha256 (and for some tools the sha512 the project publishes).
# Nothing about a tool's version or hash lives anywhere else, and no installer script is ever piped
# into a shell: the bytes that get installed are exactly the bytes that were pinned.
#
# POSIX sh (dash on the Debian-based images, which is what runs it).
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: fetch-tool NAME DEST" >&2
  exit 2
fi
name="$1"
dest="$2"
dir="${SDW_TOOLS_DIR:-/usr/local/lib/sdw/tools}"
envfile="$dir/$name.env"
[ -r "$envfile" ] || { echo "fetch-tool: no pin for '$name' ($envfile)" >&2; exit 1; }

# shellcheck disable=SC1090 # the pin file is data, one VAR=value per line
. "$envfile"

machine="$(dpkg --print-architecture 2>/dev/null || uname -m)"
case "$machine" in
  amd64 | x86_64) arch=amd64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "fetch-tool: unsupported architecture '$machine'" >&2; exit 1 ;;
esac

eval "url=\${URL_${arch}:-}"
eval "sha256=\${SHA256_${arch}:-}"
eval "sha512=\${SHA512_${arch}:-}"
if [ -z "$url" ] || [ -z "$sha256" ]; then
  echo "fetch-tool: $name has no pinned URL and sha256 for $arch" >&2
  exit 1
fi

echo "fetch-tool: $name ${VERSION:-?} ($arch)"

# Ordered list of sources to try. The pinned sha256 (and sha512 when published) is checked after
# every download, so a mirror can only serve the exact pinned bytes or be rejected: a mirror is a
# transport convenience here, never a trust anchor. Order:
#   1. $SDW_GH_MIRROR in front of a GitHub URL, when that env var is set — GitHub release assets are
#      the ones that get throttled or blocked, and such a mirror is a caching reverse proxy for them
#      (e.g. SDW_GH_MIRROR=https://ghfast.top/ -> https://ghfast.top/https://github.com/owner/...).
#   2. the pinned URL itself (the default path when no mirror is set is unchanged).
#   3. URL_MIRROR_<arch> from the pin file, if the project pins a second origin for this tool.
set --
case "${SDW_GH_MIRROR:-}" in
  "") ;;
  *) case "$url" in
       https://github.com/* | https://raw.githubusercontent.com/* | https://*.githubusercontent.com/*)
         set -- "$@" "${SDW_GH_MIRROR%/}/$url" ;;
     esac ;;
esac
set -- "$@" "$url"
eval "url_mirror=\${URL_MIRROR_${arch}:-}"
if [ -n "$url_mirror" ]; then
  set -- "$@" "$url_mirror"
fi

ok=
for src in "$@"; do
  echo "fetch-tool: GET $src"
  if ! curl -fsSL --connect-timeout 30 --retry 3 --retry-delay 3 --retry-all-errors -o "$dest" "$src"; then
    echo "fetch-tool: download failed from $src, trying next" >&2
    continue
  fi
  if ! echo "$sha256  $dest" | sha256sum -c - >/dev/null 2>&1; then
    echo "fetch-tool: sha256 mismatch from $src, rejected" >&2
    continue
  fi
  if [ -n "$sha512" ] && ! echo "$sha512  $dest" | sha512sum -c - >/dev/null 2>&1; then
    echo "fetch-tool: sha512 mismatch from $src, rejected" >&2
    continue
  fi
  ok=1
  break
done

if [ -z "$ok" ]; then
  echo "fetch-tool: every source failed for $name" >&2
  rm -f "$dest"
  exit 1
fi
echo "fetch-tool: $name verified from $src"
