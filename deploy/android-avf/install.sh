#!/bin/sh
# install.sh — provision the dev-workspace stack inside an Android AVF (pKVM) Terminal VM, or any
# plain Debian VM, to run NATIVELY (no Docker; the VM is the isolation boundary). Run as root INSIDE
# the VM.
#
# You bring a rootfs tarball exported on a build machine (where Docker is available):
#
#   # on the build machine, from the repo:
#   SDW_GH_MIRROR=https://ghfast.top/ DESKTOP_STACK=none WORKSPACE_VARIANT=headless \
#     docker compose build workspace
#   cid=$(docker create dev-workspace:headless)
#   docker export "$cid" | gzip > sdw-rootfs.tar.gz
#   docker rm "$cid"
#   # transfer sdw-rootfs.tar.gz into the VM (AVF Terminal shared folder, or scp)
#
#   # then, inside the VM:
#   sudo sh install.sh --rootfs sdw-rootfs.tar.gz --env sdw.env
#   sudo systemctl enable --now sdw-workspace
#
# --env is your .env (at least PASSWORD and CLASH_SUBSCRIPTION_URL); a template is written if omitted.
set -eu

ROOTFS_DIR=/opt/sdw/rootfs
RUN_DIR=/opt/sdw
ENV_FILE=/etc/sdw/sdw.env
UNIT=/etc/systemd/system/sdw-workspace.service
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"

rootfs_tar=""
env_src=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --rootfs) rootfs_tar="$2"; shift 2 ;;
    --env) env_src="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "install: unknown option '$1'" >&2; exit 2 ;;
  esac
done

[ "$(id -u)" -eq 0 ] || { echo "install: run as root (inside the VM)" >&2; exit 1; }
[ -n "$rootfs_tar" ] || { echo "install: --rootfs <tarball> is required (see --help)" >&2; exit 2; }
[ -r "$rootfs_tar" ] || { echo "install: cannot read '$rootfs_tar'" >&2; exit 1; }
command -v tini >/dev/null 2>&1 || true   # tini is inside the rootfs; nothing to check on the host

echo "==> extracting rootfs into $ROOTFS_DIR"
mkdir -p "$ROOTFS_DIR"
# A docker-export tar is a flat rootfs. gzip or plain are both handled by tar -a on decompress.
case "$rootfs_tar" in
  *.gz|*.tgz) gzip -dc "$rootfs_tar" | tar -C "$ROOTFS_DIR" -xf - ;;
  *)          tar -C "$ROOTFS_DIR" -xf "$rootfs_tar" ;;
esac
[ -x "$ROOTFS_DIR/usr/local/bin/sdw" ] || {
  echo "install: the tarball has no /usr/local/bin/sdw — is it a docker export of the image?" >&2
  exit 1
}

echo "==> installing run-workspace.sh -> $RUN_DIR"
mkdir -p "$RUN_DIR"
install -m 0755 "$SRC_DIR/run-workspace.sh" "$RUN_DIR/run-workspace.sh"

echo "==> env file -> $ENV_FILE"
mkdir -p "$(dirname "$ENV_FILE")"
if [ -n "$env_src" ]; then
  [ -r "$env_src" ] || { echo "install: cannot read env file '$env_src'" >&2; exit 1; }
  install -m 0600 "$env_src" "$ENV_FILE"
elif [ ! -e "$ENV_FILE" ]; then
  umask 077
  cat > "$ENV_FILE" <<'EOF'
# dev-workspace environment (native VM). Fill in at least these two, then:
#   systemctl restart sdw-workspace
PASSWORD=
CLASH_SUBSCRIPTION_URL=
# EGRESS_ENFORCE=1 by default (fail-closed firewall; nft or iptables-legacy, auto-detected).
EOF
  echo "    wrote a template; edit $ENV_FILE (PASSWORD, CLASH_SUBSCRIPTION_URL) before starting."
fi
chmod 0600 "$ENV_FILE"

echo "==> systemd unit -> $UNIT"
install -m 0644 "$SRC_DIR/sdw-workspace.service" "$UNIT"
systemctl daemon-reload

cat <<EOF

Installed. Next:
  1. Edit $ENV_FILE (PASSWORD, CLASH_SUBSCRIPTION_URL).
  2. sudo systemctl enable --now sdw-workspace
  3. Watch it:   journalctl -u sdw-workspace -f
  4. The web UI is Caddy on 127.0.0.1:8080 inside the VM. To reach it from the Android browser,
     forward the port (AVF Terminal: Settings > Port forwarding, add 8080), then open
     http://localhost:8080/  on the phone. Log in with PASSWORD.

Stop/remove:
  sudo systemctl disable --now sdw-workspace
  sudo /opt/sdw/run-workspace.sh stop   # unmount the rootfs binds
EOF
