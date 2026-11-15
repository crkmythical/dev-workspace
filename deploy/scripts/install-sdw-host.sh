#!/usr/bin/env bash
# Bootstrap the prebuilt Rust host tool before Cargo is available.
set -euo pipefail

REPOSITORY="${SDW_RELEASE_REPOSITORY:-}"
VERSION="latest"
INSTALL_DIR="${HOME}/.local/bin"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPOSITORY="${2:?--repo needs OWNER/REPO}"; shift 2 ;;
    --version) VERSION="${2:?--version needs a tag such as v1.2.3}"; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:?--install-dir needs a directory}"; shift 2 ;;
    -h | --help)
      echo "usage: install-sdw-host.sh [--repo OWNER/REPO] [--version TAG|latest] [--install-dir DIR]"
      exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [[ -z "$REPOSITORY" ]] && command -v git >/dev/null 2>&1; then
  origin="$(git config --get remote.origin.url 2>/dev/null || true)"
  case "$origin" in
    https://github.com/*) REPOSITORY="${origin#https://github.com/}" ;;
    git@github.com:*) REPOSITORY="${origin#git@github.com:}" ;;
    ssh://git@github.com/*) REPOSITORY="${origin#ssh://git@github.com/}" ;;
  esac
  REPOSITORY="${REPOSITORY%.git}"
  REPOSITORY="${REPOSITORY%/}"
fi
if [[ ! "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  echo "cannot determine a GitHub OWNER/REPO; pass --repo or set SDW_RELEASE_REPOSITORY" >&2
  exit 2
fi
if [[ "$VERSION" != latest && ! "$VERSION" =~ ^v[A-Za-z0-9._-]+$ ]]; then
  echo "invalid release version '$VERSION'" >&2
  exit 2
fi

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  *) echo "unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=amd64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

archive="sdw-host-${os}-${arch}.tar.gz"
if [[ "$VERSION" == latest ]]; then
  base="https://github.com/${REPOSITORY}/releases/latest/download"
else
  base="https://github.com/${REPOSITORY}/releases/download/${VERSION}"
fi
work="$(mktemp -d "${TMPDIR:-/tmp}/sdw-host-install.XXXXXX")"
candidate=
cleanup() {
  if [[ -n "$candidate" ]]; then rm -f "$candidate"; fi
  rm -r "$work"
}
trap cleanup EXIT

curl -fL --proto '=https' --tlsv1.2 --retry 4 --retry-delay 2 \
  -o "$work/$archive" "$base/$archive"
curl -fL --proto '=https' --tlsv1.2 --retry 4 --retry-delay 2 \
  -o "$work/$archive.sha256" "$base/$archive.sha256"
(
  cd "$work"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$archive.sha256"
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$archive.sha256"
  else
    echo "sha256sum or shasum is required" >&2
    exit 1
  fi
  entries="$(tar -tzf "$archive" | sed 's#^\./##')"
  [[ "$entries" == sdw-host ]] || { echo "unexpected archive contents: $entries" >&2; exit 1; }
  tar -xzf "$archive"
)
mkdir -p "$INSTALL_DIR"
candidate="$(mktemp "$INSTALL_DIR/.sdw-host.XXXXXX")"
install -m 0755 "$work/sdw-host" "$candidate"
"$candidate" --help >/dev/null
mv -f "$candidate" "$INSTALL_DIR/sdw-host"
candidate=
printf 'Installed %s/%s sdw-host from %s (%s) at %s\n' \
  "$os" "$arch" "$REPOSITORY" "$VERSION" "$INSTALL_DIR/sdw-host"
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add $INSTALL_DIR to PATH." ;;
esac
