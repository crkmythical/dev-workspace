#!/usr/bin/env bash
# Keep the third-party tools baked into the image current, pinned and verified.
#
#   deploy/scripts/update-tools.sh            move every tool to its latest release
#   deploy/scripts/update-tools.sh NAME...    only those (caddy restic croc mihomo code-server kasmvnc selkies
#                                             mise idea burpsuite)
#   deploy/scripts/update-tools.sh --check    change nothing; exit 1 if any pin is behind (for CI)
#   deploy/scripts/update-tools.sh --relock selkies
#                                             re-resolve Selkies' Python dependencies at the pinned
#                                             release (for a security update of one of them)
#
# Every tool has one file, src/image/tools/<name>.env: its version and, per architecture, the
# download URL and the sha256 it must have. The Dockerfiles do not know versions or hashes; they
# call `fetch-tool <name>` (src/image/tools/fetch.sh), which refuses a download that does not match.
# That is the single source of truth, and this script is the only thing that writes it.
#
# Where does a pinned hash come from?
#   * the sha256 GitHub computed for the uploaded asset (release metadata), and
#   * where the project publishes its own checksums (caddy, restic, croc, selkies, mise), those too: the
#     two must agree, or the script stops. A hash taken from one place only is a hash of whatever
#     was uploaded; two independent places are a check.
#   * idea: JetBrains' release feed names the archive and its .sha256 file.
#   * burpsuite: PortSwigger publishes no checksum for the Community jar, so the hash is that of
#     the bytes downloaded now (684 MB). It guards every later install against a different file.
#   * selkies: the release wheel is pinned like the others. Its Python dependencies (pixelflux and
#     pcmflux at the exact versions the wheel names, aiohttp, cryptography, ...) are not wheels of
#     the release, so they are pinned in src/image/tools/selkies-requirements.txt, with the hashes
#     of every file PyPI has for each version, which pip refuses to deviate from (--require-hashes).
#     The file is regenerated from the wheel's own metadata every time the pin moves.
# Review the diff of src/image/tools/ like any other dependency bump, then run `make smoke`.
#
# Needs curl, jq (and unzip and uv for selkies). GITHUB_TOKEN raises the API limit (60 requests/hour
# without one); UPDATE_TOOLS_PROXY=http://host:port sends the requests through a proxy.
# Written for the bash 3.2 and BSD tools macOS ships.
set -euo pipefail

cd "$(dirname "$0")/../.."
TOOLS_DIR="src/image/tools"
ALL_TOOLS="caddy restic croc mihomo code-server kasmvnc selkies mise idea burpsuite"

command -v jq >/dev/null 2>&1 || { echo "missing: jq" >&2; exit 1; }
CURL=(curl -fsSL --retry 3 --retry-delay 2 --max-time 120)
[ -n "${GITHUB_TOKEN:-}" ] && CURL+=(-H "Authorization: Bearer ${GITHUB_TOKEN}")
[ -n "${UPDATE_TOOLS_PROXY:-}" ] && CURL+=(-x "${UPDATE_TOOLS_PROXY}")

CHECK=0
RELOCK=0
SELECTED=""
for arg in "$@"; do
  case "$arg" in
    --check) CHECK=1 ;;
    --relock) RELOCK=1 ;;
    -h | --help) sed -n '2,/^set -euo pipefail$/{/^set -euo pipefail$/!p;}' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) SELECTED="$SELECTED $arg" ;;
  esac
done
[ -n "$SELECTED" ] || SELECTED="$ALL_TOOLS"

# --- per-tool facts: where to look, what the assets are called, who publishes checksums --------------
# Sets: REPO, ASSET_amd64, ASSET_arm64 (given $VERSION), SUMS_ASSET and SUMS_ALGO when the project
# publishes checksums (SUMS_ALGO is the hash they use), TAG (given $VERSION).
# shellcheck disable=SC2034 # ASSET_<arch> is read through eval "asset=\$ASSET_$arch"
describe() {
  TAG="v${VERSION}"
  SUMS_ASSET=""
  SUMS_ALGO=""
  case "$1" in
    caddy)
      REPO="caddyserver/caddy"
      ASSET_amd64="caddy_${VERSION}_linux_amd64.tar.gz"
      ASSET_arm64="caddy_${VERSION}_linux_arm64.tar.gz"
      SUMS_ASSET="caddy_${VERSION}_checksums.txt"
      SUMS_ALGO="sha512"
      ;;
    restic)
      REPO="restic/restic"
      ASSET_amd64="restic_${VERSION}_linux_amd64.bz2"
      ASSET_arm64="restic_${VERSION}_linux_arm64.bz2"
      SUMS_ASSET="SHA256SUMS"
      SUMS_ALGO="sha256"
      ;;
    croc)
      REPO="schollz/croc"
      ASSET_amd64="croc_v${VERSION}_Linux-64bit.tar.gz"
      ASSET_arm64="croc_v${VERSION}_Linux-ARM64.tar.gz"
      SUMS_ASSET="croc_v${VERSION}_checksums.txt"
      SUMS_ALGO="sha256"
      ;;
    mihomo)
      REPO="MetaCubeX/mihomo"
      ASSET_amd64="mihomo-linux-amd64-v${VERSION}.gz"
      ASSET_arm64="mihomo-linux-arm64-v${VERSION}.gz"
      ;;
    code-server)
      REPO="coder/code-server"
      ASSET_amd64="code-server_${VERSION}_amd64.deb"
      ASSET_arm64="code-server_${VERSION}_arm64.deb"
      ;;
    kasmvnc)
      REPO="kasmtech/KasmVNC"
      ASSET_amd64="kasmvncserver_debian_trixie_${VERSION}_amd64.deb"
      ASSET_arm64="kasmvncserver_debian_trixie_${VERSION}_arm64.deb"
      ;;
    selkies)
      # Tags carry no "v"; the wheel is platform-independent, so both architectures get the same
      # file, and the project lists it in SHA256SUMS.
      REPO="selkies-project/selkies"
      TAG="${VERSION}"
      ASSET_amd64="selkies-${VERSION}-py3-none-any.whl"
      ASSET_arm64="selkies-${VERSION}-py3-none-any.whl"
      SUMS_ASSET="SHA256SUMS"
      SUMS_ALGO="sha256"
      ;;
    mise)
      REPO="jdx/mise"
      ASSET_amd64="mise-v${VERSION}-linux-x64.tar.gz"
      ASSET_arm64="mise-v${VERSION}-linux-arm64.tar.gz"
      SUMS_ASSET="SHASUMS256.txt"
      SUMS_ALGO="sha256"
      ;;
    *) echo "unknown tool: $1 (known: $ALL_TOOLS)" >&2; exit 2 ;;
  esac
}

# The tool's own repository for a name, with no version needed (to ask for the latest tag).
repo_of() { VERSION=0 describe "$1"; printf '%s' "$REPO"; }

current_version() { sed -n 's/^VERSION=//p' "$TOOLS_DIR/$1.env" 2>/dev/null | head -1; }

# digest_of <release json> <asset name>: the sha256 GitHub recorded for the asset.
digest_of() { printf '%s' "$1" | jq -r --arg n "$2" '.assets[] | select(.name==$n) | .digest // empty' | sed 's/^sha256://'; }

# published_hash <algo> <sums file text> <asset name>: the hash the project lists for the asset.
published_hash() { printf '%s\n' "$2" | awk -v n="$3" '{ f=$2; sub(/^\*/, "", f); sub(/^\.\//, "", f); if (f == n) print $1 }' | head -1; }

# lock_selkies: the hash-pinned Python dependencies of the pinned Selkies wheel.
#
# The wheel names its requirements (pixelflux and pcmflux at exact versions, the rest as ranges).
# uv resolves them once, for every platform, and writes the sha256 of every file PyPI has for each
# chosen version. The image installs with `pip --require-hashes`, so it can install nothing else.
lock_selkies() {
  local version url sha dir whl
  command -v uv >/dev/null 2>&1 || { echo "selkies: uv is needed to resolve the Python dependencies (https://docs.astral.sh/uv/)" >&2; return 1; }
  command -v unzip >/dev/null 2>&1 || { echo "selkies: missing: unzip" >&2; return 1; }
  version="$(current_version selkies)"
  url="$(sed -n "s/^URL_amd64='\(.*\)'$/\1/p" "$TOOLS_DIR/selkies.env")"
  sha="$(sed -n 's/^SHA256_amd64=//p' "$TOOLS_DIR/selkies.env")"
  dir="$(mktemp -d)"
  whl="$dir/selkies-$version-py3-none-any.whl"
  "${CURL[@]}" -o "$whl" "$url" || { echo "selkies: could not download $url" >&2; return 1; }
  if [ "$(sha256_stdin <"$whl")" != "$sha" ]; then
    echo "selkies: the wheel does not match its pinned sha256; not locking" >&2
    return 1
  fi
  unzip -p "$whl" '*.dist-info/METADATA' \
    | awk '/^$/ { exit } /^Requires-Dist:/ && !/extra ==/ { sub(/^Requires-Dist: /, ""); print }' >"$dir/selkies.in"
  [ -s "$dir/selkies.in" ] || { echo "selkies: the wheel's metadata lists no requirements" >&2; return 1; }
  (
    if [ -n "${UPDATE_TOOLS_PROXY:-}" ]; then export HTTPS_PROXY="$UPDATE_TOOLS_PROXY" HTTP_PROXY="$UPDATE_TOOLS_PROXY"; fi
    # --universal: one file for amd64 and arm64 (and whichever Python the base image moves to next).
    uv pip compile --universal --python-version 3.13 --generate-hashes --no-header --no-annotate --quiet \
      "$dir/selkies.in" -o "$dir/selkies-requirements.txt"
  ) || { echo "selkies: uv could not resolve the requirements" >&2; return 1; }
  {
    echo "# Generated by deploy/scripts/update-tools.sh from the requirements of selkies $version; never by hand."
    cat "$dir/selkies-requirements.txt"
  } >"$TOOLS_DIR/selkies-requirements.txt"
  chmod 644 "$TOOLS_DIR/selkies-requirements.txt"
  rm -r "$dir"
  printf '%-12s %-12s locked %s requirements\n' selkies "$version" "$(grep -c '^[A-Za-z]' "$TOOLS_DIR/selkies-requirements.txt")"
}

update_one() {
  local name="$1" repo latest cur release sums arch asset url h_api h_pub out
  repo="$(repo_of "$name")"
  release="$("${CURL[@]}" "https://api.github.com/repos/$repo/releases/latest")"
  latest="$(printf '%s' "$release" | jq -r .tag_name | sed 's/^v//')"
  [ -n "$latest" ] && [ "$latest" != null ] || { echo "$name: could not read the latest release of $repo" >&2; return 1; }
  cur="$(current_version "$name")"
  if [ "$cur" = "$latest" ]; then
    printf '%-12s %-12s up to date\n' "$name" "$cur"
    # The Selkies dependency lock belongs to the pin: made on request, or when it is not there yet.
    if [ "$name" = selkies ] && [ "$CHECK" = 0 ] && { [ "$RELOCK" = 1 ] || [ ! -f "$TOOLS_DIR/selkies-requirements.txt" ]; }; then
      lock_selkies
      return $?
    fi
    return 0
  fi
  printf '%-12s %-12s -> %s\n' "$name" "${cur:-none}" "$latest"
  [ "$CHECK" = 1 ] && return 3

  VERSION="$latest"
  describe "$name"
  sums=""
  if [ -n "$SUMS_ASSET" ]; then
    sums="$("${CURL[@]}" "https://github.com/$REPO/releases/download/$TAG/$SUMS_ASSET")"
  fi
  out="$(mktemp)"
  {
    echo "# Generated by deploy/scripts/update-tools.sh; update by running it, never by hand."
    echo "NAME=$name"
    echo "VERSION=$VERSION"
  } >"$out"
  for arch in amd64 arm64; do
    eval "asset=\$ASSET_$arch"
    url="https://github.com/$REPO/releases/download/$TAG/$asset"
    h_api="$(digest_of "$release" "$asset")"
    [ -n "$h_api" ] || { echo "$name: no sha256 in the release metadata for $asset" >&2; rm -f "$out"; return 1; }
    if [ "$SUMS_ALGO" = sha256 ]; then
      h_pub="$(published_hash sha256 "$sums" "$asset")"
      if [ -z "$h_pub" ] || [ "$h_pub" != "$h_api" ]; then
        echo "$name: $asset: GitHub says $h_api, the project's checksums say '${h_pub:-nothing}'; not pinning" >&2
        rm -f "$out"
        return 1
      fi
    fi
    {
      echo "URL_$arch='$url'"
      echo "SHA256_$arch=$h_api"
    } >>"$out"
    if [ "$SUMS_ALGO" = sha512 ]; then
      h_pub="$(published_hash sha512 "$sums" "$asset")"
      [ -n "$h_pub" ] || { echo "$name: no sha512 for $asset in $SUMS_ASSET" >&2; rm -f "$out"; return 1; }
      echo "SHA512_$arch=$h_pub" >>"$out"
    fi
  done
  mkdir -p "$TOOLS_DIR"
  mv "$out" "$TOOLS_DIR/$name.env"
  chmod 644 "$TOOLS_DIR/$name.env"
  if [ "$name" = selkies ]; then lock_selkies; fi
}

# write_pin <name> <version> <amd64 url> <amd64 sha256> <arm64 url> <arm64 sha256>
write_pin() {
  local out
  out="$(mktemp)"
  {
    echo "# Generated by deploy/scripts/update-tools.sh; update by running it, never by hand."
    echo "NAME=$1"
    echo "VERSION=$2"
    echo "URL_amd64='$3'"
    echo "SHA256_amd64=$4"
    echo "URL_arm64='$5'"
    echo "SHA256_arm64=$6"
  } >"$out"
  mkdir -p "$TOOLS_DIR"
  mv "$out" "$TOOLS_DIR/$1.env"
  chmod 644 "$TOOLS_DIR/$1.env"
}

is_sha256() { printf '%s' "$1" | grep -Eq '^[0-9a-f]{64}$'; }

# sha256 of stdin, with whatever this machine has.
sha256_stdin() { if command -v sha256sum >/dev/null 2>&1; then sha256sum; else shasum -a 256; fi | awk '{print $1}'; }

# IntelliJ IDEA: JetBrains' release feed lists the archive and a .sha256 file for each architecture.
update_idea() {
  local feed latest cur url_amd64 url_arm64 sum_amd64 sum_arm64
  feed="$("${CURL[@]}" 'https://data.services.jetbrains.com/products/releases?code=IIC&latest=true&type=release')"
  latest="$(printf '%s' "$feed" | jq -r '.IIC[0].version // empty')"
  [ -n "$latest" ] || { echo "idea: could not read the latest release from JetBrains" >&2; return 1; }
  cur="$(current_version idea)"
  if [ "$cur" = "$latest" ]; then
    printf '%-12s %-12s up to date\n' idea "$cur"
    return 0
  fi
  printf '%-12s %-12s -> %s\n' idea "${cur:-none}" "$latest"
  [ "$CHECK" = 1 ] && return 3
  url_amd64="$(printf '%s' "$feed" | jq -r '.IIC[0].downloads.linux.link // empty')"
  url_arm64="$(printf '%s' "$feed" | jq -r '.IIC[0].downloads.linuxARM64.link // empty')"
  sum_amd64="$("${CURL[@]}" "$(printf '%s' "$feed" | jq -r '.IIC[0].downloads.linux.checksumLink // empty')" | awk '{print $1}')"
  sum_arm64="$("${CURL[@]}" "$(printf '%s' "$feed" | jq -r '.IIC[0].downloads.linuxARM64.checksumLink // empty')" | awk '{print $1}')"
  if [ -z "$url_amd64" ] || [ -z "$url_arm64" ] || ! is_sha256 "$sum_amd64" || ! is_sha256 "$sum_arm64"; then
    echo "idea: the JetBrains feed did not give a download and a sha256 for both architectures" >&2
    return 1
  fi
  write_pin idea "$latest" "$url_amd64" "$sum_amd64" "$url_arm64" "$sum_arm64"
}

# Burp Suite Community: one platform-independent jar. The unversioned link serves the latest
# Community build; the file name in its response says which version that is, and the versioned link
# serves that very build from then on.
update_burpsuite() {
  local latest cur url sum
  latest="$("${CURL[@]}" -r 0-0 -D - -o /dev/null 'https://portswigger-cdn.net/burp/releases/download?product=community&type=Jar' \
    | sed -n 's/.*burpsuite_community_v\([0-9][0-9.]*\)\.jar.*/\1/p' | head -1)"
  [ -n "$latest" ] || { echo "burpsuite: could not read the latest Community version from PortSwigger" >&2; return 1; }
  cur="$(current_version burpsuite)"
  if [ "$cur" = "$latest" ]; then
    printf '%-12s %-12s up to date\n' burpsuite "$cur"
    return 0
  fi
  printf '%-12s %-12s -> %s\n' burpsuite "${cur:-none}" "$latest"
  [ "$CHECK" = 1 ] && return 3
  url="https://portswigger.net/burp/releases/download?product=community&version=${latest}&type=Jar"
  echo "burpsuite: hashing the download (about 700 MB)..." >&2
  sum="$("${CURL[@]}" --max-time 3600 "$url" | sha256_stdin)"
  is_sha256 "$sum" || { echo "burpsuite: could not hash $url" >&2; return 1; }
  write_pin burpsuite "$latest" "$url" "$sum" "$url" "$sum"
}

update_tool() {
  case "$1" in
    idea) update_idea ;;
    burpsuite) update_burpsuite ;;
    *) update_one "$1" ;;
  esac
}

behind=0
failed=0
for tool in $SELECTED; do
  rc=0
  update_tool "$tool" || rc=$?
  case "$rc" in
    0) ;;
    3) behind=1 ;;
    *) failed=1 ;;
  esac
done

if [ "$failed" = 1 ]; then exit 1; fi
if [ "$CHECK" = 1 ] && [ "$behind" = 1 ]; then
  echo "some pinned tools are behind; run deploy/scripts/update-tools.sh" >&2
  exit 1
fi
if [ "$CHECK" = 0 ]; then
  echo "done. Review: git diff $TOOLS_DIR, then build and run make smoke."
fi
