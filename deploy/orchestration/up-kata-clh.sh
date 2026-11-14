#!/usr/bin/env bash
# Launch a workspace instance inside a Cloud Hypervisor (Rust VMM) microVM via Kata, instead of the
# default QEMU-backed `kata` runtime. Thin wrapper over up-kata.sh with SDW_KATA_RUNTIME=kata-clh —
# identical pod/container spec (caps, /dev/fuse, the five host-path volumes, seccomp, gate port map),
# only the containerd runtime handler differs.
#
#   up-kata-clh.sh <id> [host_port] [env_file]
#   down-kata.sh   <id>                         # teardown is the same (pod name is runtime-agnostic)
#
# Requires the clh runtime set up first (one-time, builds the shim from source):
#   sudo deploy/scripts/setup-kata-runtime.sh --with-clh
#
# WHY clh OVER QEMU: a lighter Rust VMM — faster boot and a smaller memory/overhead footprint. On a
# nested-virt Mac (Colima/vz) the *guest CPU* penalty (~2x, see deploy/docs/TODO.md #2) is the same
# as QEMU because it is a KVM-level nested cost, VMM-agnostic; clh does not make the heavy GUI faster
# here. Its win shows on bare-metal Linux (single-level virt). The security-critical parts
# (vault/gocryptfs, egress nftables firewall in the pod's own netns) run identically under clh —
# verified: the microVM boots the full image, gocryptfs mounts with real ciphertext on the host path,
# and the `inet sdw_egress` firewall table is enforced inside the guest.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"

die() { printf 'up-kata-clh: %s\n' "$*" >&2; exit 1; }
command -v crictl >/dev/null || die "crictl not found — run: sudo deploy/scripts/setup-kata-runtime.sh --with-clh"
crictl info 2>/dev/null | grep -q '"kata-clh"' \
  || die "containerd has no 'kata-clh' runtime — run: sudo deploy/scripts/setup-kata-runtime.sh --with-clh"
[ -x /usr/local/bin/containerd-shim-kata-clh-v2 ] \
  || die "clh shim missing (/usr/local/bin/containerd-shim-kata-clh-v2) — run setup-kata-runtime.sh --with-clh"

# Everything else (image import, volumes, pod/ctr spec, gate wait) is up-kata.sh; just swap the runtime.
exec env SDW_KATA_RUNTIME=kata-clh "$here/up-kata.sh" "$@"
