#!/bin/sh
# run-workspace.sh — run the dev-workspace stack NATIVELY inside a Linux VM, from the image's
# exported rootfs, WITHOUT a container runtime.
#
# Why no Docker here: on Android the workspace runs inside an AVF (pKVM) Terminal VM. That VM IS the
# isolation boundary (isolated from the untrusted Android host), so the container layer is
# redundant. And the stock AVF guest kernel lacks the PID/IPC/USER namespaces and cgroup
# controllers Docker needs (it has mount namespaces, FUSE, OverlayFS, netfilter). So the stack runs
# as plain processes under the VM's root: `sdw entrypoint` does the same BOOT_PLAN as in Docker
# (vault unlock, egress firewall, Caddy, Clash) and then supervisord. The egress firewall uses its
# iptables-legacy backend automatically where the VM's nftables is non-functional (see
# infra/src/egress.rs); everything else is identical to the container.
#
# Invoked by sdw-workspace.service (install.sh). `start` (default) mounts and boots; `stop`
# tears the mounts down. Runnable by hand for debugging. Needs root (the VM's root).
set -eu

ROOTFS="${SDW_ROOTFS:-/opt/sdw/rootfs}"
STATE="${SDW_STATE:-/var/lib/sdw-workspace}"
ACTION="${1:-start}"

# The Docker volumes, as persistent host dirs -> the paths the image expects:
#   vault-data:/vault/cipher   cache-data:/pentest/cipher   clash-config:/etc/clash
#   destruct-state:/var/run/self-destruct   audit-data:/var/log/sdw
# "name host_subdir rootfs_path", one per line.
binds() {
  cat <<'EOF'
vault-cipher     /vault/cipher
pentest-cipher   /pentest/cipher
clash-config     /etc/clash
destruct-state   /var/run/self-destruct
audit            /var/log/sdw
EOF
}

do_umount() {
  # Reverse of mount; never fail the stop on a leftover.
  for p in /var/log/sdw /var/run/self-destruct /etc/clash /pentest/cipher /vault/cipher \
           /dev/pts /dev /sys /proc; do
    mountpoint -q "$ROOTFS$p" && umount -l "$ROOTFS$p" || true
  done
}

case "$ACTION" in
  stop)
    do_umount
    exit 0
    ;;
  start) ;;
  *) echo "usage: run-workspace.sh [start|stop]" >&2; exit 2 ;;
esac

[ "$(id -u)" -eq 0 ] || { echo "run-workspace: must run as root (the VM's root)" >&2; exit 1; }
[ -x "$ROOTFS/usr/local/bin/sdw" ] || {
  echo "run-workspace: no sdw binary at $ROOTFS/usr/local/bin/sdw — run install.sh first" >&2
  exit 1
}

# Mount TARGET only if it is not already a mountpoint (idempotent across restarts). The check is on
# the TARGET, not the source: checking the source silently skipped /proc and /sys, because the
# host's own /proc and /sys are themselves mountpoints — which left /proc unmounted in the chroot
# and broke supervisord's /dev/fd/1 (-> /proc/self/fd) program-log targets.
mnt() {  # mnt TARGET MOUNT_OPTS...  (TARGET is appended as the mount destination)
  _t="$1"; shift
  mountpoint -q "$_t" 2>/dev/null || mount "$@" "$_t"
}

# Kernel filesystems. /dev is bind-mounted whole so /dev/fuse (the vault) and /dev/pts are present;
# /proc is needed by the vault, Clash and supervisord (program logs go to /dev/fd -> /proc/self/fd).
mkdir -p "$ROOTFS/proc" "$ROOTFS/sys" "$ROOTFS/dev" "$ROOTFS/dev/pts"
mnt "$ROOTFS/proc"    -t proc  proc
mnt "$ROOTFS/sys"     -t sysfs sys
mnt "$ROOTFS/dev"     --bind /dev
mnt "$ROOTFS/dev/pts" --bind /dev/pts

# Belt and suspenders for a minimal /dev lacking the std-stream symlinks (a full devtmpfs VM already
# has them); needs /proc, mounted above.
chroot "$ROOTFS" sh -c \
  'ln -sfn /proc/self/fd /dev/fd; ln -sf /proc/self/fd/0 /dev/stdin; ln -sf /proc/self/fd/1 /dev/stdout; ln -sf /proc/self/fd/2 /dev/stderr' \
  2>/dev/null || true

# Persistent state binds.
binds | while read -r name path; do
  [ -n "$name" ] || continue
  mkdir -p "$STATE/$name" "$ROOTFS$path"
  mnt "$ROOTFS$path" --bind "$STATE/$name"
done

# A resolver for the boot before Clash rewrites /etc/resolv.conf to itself.
[ -s "$ROOTFS/etc/resolv.conf" ] || echo "nameserver 1.1.1.1" > "$ROOTFS/etc/resolv.conf"

# Boot. systemd (the VM's PID 1) reaps orphans, so no tini is needed here. The env the service
# carries (PASSWORD, CLASH_SUBSCRIPTION_URL, ...) is inherited through chroot into sdw.
exec chroot "$ROOTFS" /usr/local/bin/sdw entrypoint
