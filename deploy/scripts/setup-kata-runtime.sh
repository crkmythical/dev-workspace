#!/usr/bin/env bash
# Set up the Kata Containers runtime + containerd CRI on a nested-virt-capable Linux host, so the
# ephemeral workspace instances can run inside a microVM (hardware-isolated) instead of a plain
# container. See deploy/docs/TODO.md #2 and deploy/orchestration/up-kata.sh.
#
# WHERE THIS RUNS: on the Linux host (or VM) that runs the containers — NOT inside the image. On a
# Mac this is the Colima VM, reached with `colima ssh`; the VM must have /dev/kvm (nested
# virtualization). On Apple Silicon that needs an M3+ chip and `colima start <profile> -z`
# (nestedVirtualization). On a bare-metal Linux host with KVM it runs directly.
#
#   sudo deploy/scripts/setup-kata-runtime.sh            install + configure (idempotent)
#   sudo deploy/scripts/setup-kata-runtime.sh --verify   only check the setup, change nothing
#
# Idempotent: re-running re-applies the config and leaves a working runtime. Needs: curl, tar, zstd
# (apt-get installs it), a running containerd whose CRI plugin is enabled, and outbound network
# (set https_proxy for a restricted network).
set -euo pipefail

# --- pins (bump together; sha256 is checked) ----------------------------------------------------
KATA_VERSION="4.3.1"
KATA_SHA256="2c5bf7cde334ad5307d482f45cceda496f91c9b6b96f5278c47f4f4eac1bbae6"
KATA_URL="https://github.com/kata-containers/kata-containers/releases/download/${KATA_VERSION}/kata-static-${KATA_VERSION}-arm64.tar.zst"
CRICTL_VERSION="v1.37.0"
CRICTL_SHA256="c9dfdf9ebc353a0027c88622206a5e90ee6f0e78cfabf1a9ef4dc575298fd555"
CRICTL_URL="https://github.com/kubernetes-sigs/cri-tools/releases/download/${CRICTL_VERSION}/crictl-${CRICTL_VERSION}-linux-arm64.tar.gz"
CNI_VERSION="v1.9.1"
CNI_SHA256="56171987d3947707c3563db2f4001bccaf50fd63468611b9f3cbecb1375ee7ec"
CNI_URL="https://github.com/containernetworking/plugins/releases/download/${CNI_VERSION}/cni-plugins-linux-arm64-${CNI_VERSION}.tgz"

KATA_DIR=/opt/kata
KATA_CONF=/opt/kata/share/defaults/kata-containers/runtime-rs/configuration-qemu-runtime-rs.toml
CTR_CONF=/etc/containerd/config.toml

# --with-clh (optional) builds a Cloud Hypervisor (Rust VMM) shim from source and registers it as a
# second `kata-clh` handler alongside the default QEMU `kata`. These are its paths.
KATA_SRC_DIR=/opt/kata-src
CLH_SHIM=/usr/local/bin/containerd-shim-kata-clh-v2
CLH_CONF=/opt/kata/share/defaults/kata-containers/runtime-rs/configuration-clh.toml

say() { printf '>> %s\n' "$*"; }
die() { printf 'setup-kata: %s\n' "$*" >&2; exit 1; }

require_kvm() {
  [ -e /dev/kvm ] || die "/dev/kvm is missing — this host has no (nested) virtualization. On Colima: recreate the VM with 'colima start <profile> --nested-virtualization' (needs an M3+ Mac)."
}

verify() {
  local ok=1
  [ -x "$KATA_DIR/runtime-rs/bin/containerd-shim-kata-v2" ] || { echo "  MISSING kata shim"; ok=0; }
  command -v crictl >/dev/null || { echo "  MISSING crictl"; ok=0; }
  [ -x /opt/cni/bin/bridge ] && [ -f /etc/cni/net.d/10-sdw-kata.conflist ] && echo "  CNI: installed" || { echo "  CNI: MISSING"; ok=0; }
  [ -e /dev/kvm ] && echo "  /dev/kvm: present" || { echo "  /dev/kvm: MISSING"; ok=0; }
  if crictl info 2>/dev/null | grep -q '"kata"'; then echo "  containerd CRI: kata runtime registered"; else echo "  containerd CRI: kata NOT registered"; ok=0; fi
  grep -q '^cpu_features = ""' "$KATA_CONF" 2>/dev/null && echo "  pmu workaround: applied" || { echo "  pmu workaround: NOT applied"; ok=0; }
  grep -q '^default_vcpus = 4' "$KATA_CONF" 2>/dev/null && echo "  stability tuning: applied" || { echo "  stability tuning: NOT applied"; ok=0; }
  # clh is optional (built with --with-clh); report it but do not fail the base verify on its absence.
  if [ -x "$CLH_SHIM" ]; then
    crictl info 2>/dev/null | grep -q '"kata-clh"' && echo "  clh (optional): kata-clh registered" || echo "  clh (optional): shim present but handler NOT registered"
  else
    echo "  clh (optional): not installed (run with --with-clh to add it)"
  fi
  [ "$ok" = 1 ] && say "kata runtime looks good" || die "kata runtime is not fully set up"
}

# --- optional: Cloud Hypervisor (Rust VMM) shim, built from source (`--with-clh`) ----------------
# The official kata-static build ships a runtime-rs shim compiled WITHOUT the `cloud-hypervisor`
# cargo feature — its Makefile only wires qemu/dragonball/openvmm — so the stock shim answers
# "Can not find plugin for hypervisor clh". We rebuild runtime-rs at the pinned tag WITH that feature
# and install it as a SEPARATE `kata-clh` handler (own runtime_type -> own binary), leaving the QEMU
# `kata` handler and its official shim untouched. Verified on arm64 (Apple M5 + Colima nested virt):
# `crictl runp --runtime kata-clh` boots a clh microVM (guest kernel 6.18.56), and the gocryptfs
# vault + nftables egress firewall run inside it exactly as under QEMU.
setup_clh() {
  say "clh: building the cloud-hypervisor-enabled runtime-rs shim from source (tag ${KATA_VERSION})"
  [ -x /opt/kata/bin/cloud-hypervisor ] || die "clh: /opt/kata/bin/cloud-hypervisor missing — run base setup first (it ships in kata-static)"

  say "clh: installing build dependencies"
  apt-get update -qq
  apt-get install -y -qq git make gcc g++ pkg-config libseccomp-dev clang libclang-dev protobuf-compiler curl ca-certificates

  # Rust toolchain via rustup (installs under $HOME/.cargo; $HOME is root's here).
  if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    say "clh: installing rust (rustup)"
    curl -fL --proto '=https' --tlsv1.2 https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
  export CARGO_NET_GIT_FETCH_WITH_CLI=true   # cargo fetches crates.io git deps via the git CLI (honours https_proxy)
  command -v cargo >/dev/null || die "clh: cargo not on PATH after rustup"

  if [ ! -d "$KATA_SRC_DIR/.git" ]; then
    say "clh: cloning kata-containers ${KATA_VERSION} into ${KATA_SRC_DIR}"
    git clone --depth 1 --branch "$KATA_VERSION" https://github.com/kata-containers/kata-containers "$KATA_SRC_DIR"
  fi

  # `make` first: it generates crates/shim/src/config.rs (from config.rs.in). Its default build TRIPLE
  # is musl and that final link may fail on a host without the musl target — harmless here, because
  # config.rs is produced before it, and we then build the real binary for the host target below WITH
  # the clh feature (which the Makefile never passes). `-p runtime-rs` is the package that emits the
  # containerd-shim-kata-v2 binary; output lands in the cargo workspace target dir (one level up).
  say "clh: compiling runtime-rs with --features cloud-hypervisor (several minutes)"
  (
    cd "$KATA_SRC_DIR/src/runtime-rs"
    make >/tmp/clh-make.log 2>&1 || true
    [ -f crates/shim/src/config.rs ] || { echo "config.rs not generated; see /tmp/clh-make.log" >&2; exit 1; }
    cargo build --release -p runtime-rs --features cloud-hypervisor
  ) || die "clh: build failed (see cargo output above and /tmp/clh-make.log)"
  local built="$KATA_SRC_DIR/target/release/containerd-shim-kata-v2"
  [ -x "$built" ] || die "clh: build produced no shim binary at $built"
  strings "$built" 2>/dev/null | grep -q 'CloudHypervisorInner' || die "clh: built shim lacks clh code — feature did not take"

  say "clh: installing shim -> ${CLH_SHIM}"
  install -m 0755 "$built" "$CLH_SHIM"

  say "clh: writing ${CLH_CONF}"
  cat > "$CLH_CONF" <<'CLHCONF'
[hypervisor.clh]
path = "/opt/kata/bin/cloud-hypervisor"
kernel = "/opt/kata/share/kata-containers/vmlinux.container"
image = "/opt/kata/share/kata-containers/kata-containers.img"
rootfs_type = "ext4"
vm_rootfs_driver = "virtio-blk-pci"
firmware = ""
enable_annotations = []
valid_hypervisor_paths = ["/opt/kata/bin/cloud-hypervisor"]
valid_ctlpaths = []
kernel_params = ""
default_vcpus = 4
overhead_vcpus = 0
default_maxvcpus = 0
disable_nested_virtualization = true
default_memory = 4096
overhead_memory = 0
shared_fs = "virtio-fs"
virtio_fs_daemon = "/opt/kata/libexec/virtiofsd"
virtio_fs_cache_size = 0
virtio_fs_queue_size = 1024
virtio_fs_extra_args = ["--thread-pool-size=1", "-o", "announce_submounts"]
virtio_fs_cache = "auto"
default_bridges = 0
reclaim_guest_freed_memory = false
block_device_driver = "virtio-blk-pci"
block_device_cache_direct = false
disk_rate_limiter_bw_max_rate = 0
disk_rate_limiter_bw_one_time_burst = 0
disk_rate_limiter_ops_max_rate = 0
disk_rate_limiter_ops_one_time_burst = 0
queue_size = 128
num_queues = 1
network_queues = 0
enable_mem_prealloc = false
enable_hugepages = false
rootless = false
disable_seccomp = false
enable_debug = true
disable_nesting_checks = false
guest_hook_path = ""
enable_guest_swap = false
guest_swap_path = "/run/kata-containers/swap"
guest_swap_size_percent = 100
guest_swap_create_threshold_secs = 60
[agent.kata]
enable_debug = true
enable_tracing = false
kernel_modules = []
debug_console_enabled = true
dial_timeout_ms = 100
reconnect_timeout_ms = 60000
create_container_timeout = 30
[runtime]
enable_debug = true
internetworking_model = "tcfilter"
name = "virt_container"
hypervisor_name = "clh"
agent_name = "kata"
disable_guest_seccomp = true
enable_vcpus_pinning = false
sandbox_cgroup_only = true
static_sandbox_resource_mgmt = true
sandbox_bind_mounts = []
vfio_mode = "guest-kernel"
experimental = []
enable_pprof = false
CLHCONF

  # A dedicated handler: runtime_type io.containerd.kata-clh.v2 -> containerd execs
  # containerd-shim-kata-clh-v2 (the from-source binary). The QEMU `kata` handler is left as-is.
  if ! grep -q 'runtimes.kata-clh' "$CTR_CONF" 2>/dev/null; then
    say "clh: registering the kata-clh containerd runtime handler"
    cat >> "$CTR_CONF" <<EOF

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.kata-clh]
  runtime_type = 'io.containerd.kata-clh.v2'
  privileged_without_host_devices = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.kata-clh.options]
    ConfigPath = '${CLH_CONF}'
EOF
    systemctl restart containerd
    sleep 3
  fi
  crictl info 2>/dev/null | grep -q '"kata-clh"' || die "clh: kata-clh not registered after containerd restart"
  say "clh: done — launch a workspace in a clh microVM with deploy/orchestration/up-kata-clh.sh"
}

WITH_CLH=0
for a in "$@"; do
  case "$a" in
    --verify)   verify; exit 0 ;;
    --with-clh) WITH_CLH=1 ;;
    *)          die "unknown argument: $a (use --verify, or --with-clh to also build the Cloud Hypervisor shim)" ;;
  esac
done
[ "$(id -u)" = 0 ] || die "run as root (sudo)"
require_kvm

# --- 1) Kata static (pinned, checksum-verified) -------------------------------------------------
if [ -x "$KATA_DIR/runtime-rs/bin/containerd-shim-kata-v2" ]; then
  say "kata already present at $KATA_DIR (skipping download)"
else
  command -v zstd >/dev/null || { say "installing zstd"; apt-get update -qq && apt-get install -y -qq zstd; }
  say "downloading kata ${KATA_VERSION}"
  tmp="$(mktemp)"
  curl -fL --retry 3 --retry-delay 3 -o "$tmp" "$KATA_URL"
  echo "${KATA_SHA256}  ${tmp}" | sha256sum -c -
  say "extracting to /opt/kata"
  tar -I zstd -xf "$tmp" -C /
  rm -f "$tmp"
fi
ln -sf /opt/kata/runtime-rs/bin/containerd-shim-kata-v2 /usr/local/bin/containerd-shim-kata-v2

# --- 2) kata config: QEMU default + fixes + stability tuning ------------------------------------
# The default-config symlink in the tarball is broken (points at a non-existent configuration-qemu.toml);
# point it at the runtime-rs QEMU config the shim actually uses.
ln -sf runtime-rs/configuration-qemu-runtime-rs.toml /opt/kata/share/defaults/kata-containers/configuration.toml
# pmu=off: QEMU on nested-KVM arm64 rejects `-cpu host,pmu=off` ("Property host-arm-cpu.pmu not found")
# and exits, so the agent never comes up. Clearing cpu_features drops the unsupported flag.
sed -i 's/^cpu_features = .*/cpu_features = ""/' "$KATA_CONF"
# Stability in nested virt: the guest boots slower, so the agent takes ~10-13s to answer on vsock.
# More vCPUs boots it faster; a patient dial window (100ms x 600 over 60s) rides out the flaky
# initial vsock connect (ECONNRESET until the agent listens). Measured 10/10 runp with these.
sed -i 's/^default_vcpus = .*/default_vcpus = 4/' "$KATA_CONF"
sed -i 's/^dial_timeout_ms = .*/dial_timeout_ms = 100/' "$KATA_CONF"
sed -i 's/^reconnect_timeout_ms = .*/reconnect_timeout_ms = 60000/' "$KATA_CONF"

# --- 3) crictl (pinned) -------------------------------------------------------------------------
if command -v crictl >/dev/null && crictl --version 2>/dev/null | grep -q "$CRICTL_VERSION"; then
  say "crictl ${CRICTL_VERSION} already installed"
else
  say "downloading crictl ${CRICTL_VERSION}"
  tmp="$(mktemp)"
  curl -fL --retry 3 -o "$tmp" "$CRICTL_URL"
  echo "${CRICTL_SHA256}  ${tmp}" | sha256sum -c -
  tar -C /usr/local/bin -xzf "$tmp" && rm -f "$tmp"
fi
cat > /etc/crictl.yaml <<'EOF'
runtime-endpoint: unix:///run/containerd/containerd.sock
image-endpoint: unix:///run/containerd/containerd.sock
timeout: 120
EOF

# --- 3b) CNI (bridge + portmap): the pod gets its own netns, so the workspace's egress firewall is
# isolated from the host (not loaded into the VM's network), and a host port can be mapped to the
# gate. host-network would avoid CNI but would run the firewall against the host netns. ------------
if [ ! -x /opt/cni/bin/bridge ]; then
  say "downloading CNI plugins ${CNI_VERSION}"
  mkdir -p /opt/cni/bin
  tmp="$(mktemp)"
  curl -fL --retry 3 -o "$tmp" "$CNI_URL"
  echo "${CNI_SHA256}  ${tmp}" | sha256sum -c -
  tar -C /opt/cni/bin -xzf "$tmp" && rm -f "$tmp"
fi
mkdir -p /etc/cni/net.d
cat > /etc/cni/net.d/10-sdw-kata.conflist <<'EOF'
{
  "cniVersion": "1.0.0",
  "name": "sdw-kata",
  "plugins": [
    { "type": "bridge", "bridge": "cni-sdw0", "isGateway": true, "ipMasq": true,
      "ipam": { "type": "host-local", "ranges": [[{"subnet":"10.88.0.0/16"}]], "routes": [{"dst":"0.0.0.0/0"}] } },
    { "type": "portmap", "capabilities": { "portMappings": true } }
  ]
}
EOF

# --- 4) containerd CRI: register the kata runtime handler ---------------------------------------
# Regenerate a full default config (the shipped one may disable CRI with a stale v1 plugin name)
# and append the kata handler. privileged_without_host_devices keeps a privileged pod from hot-
# plugging the host's devices into the guest (which breaks the agent); the workload gets the
# devices it needs (/dev/fuse via mknod) from CAP_MKNOD instead.
if ! grep -q 'runtimes.kata' "$CTR_CONF" 2>/dev/null; then
  say "writing containerd config with a kata runtime handler"
  cp -n "$CTR_CONF" "${CTR_CONF}.pre-kata" 2>/dev/null || true
  containerd config default > "$CTR_CONF"
  cat >> "$CTR_CONF" <<EOF

[plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.kata]
  runtime_type = 'io.containerd.kata.v2'
  privileged_without_host_devices = true
  [plugins.'io.containerd.cri.v1.runtime'.containerd.runtimes.kata.options]
    ConfigPath = '/opt/kata/share/defaults/kata-containers/configuration.toml'
EOF
  systemctl restart containerd
  sleep 3
fi
systemctl is-active --quiet containerd || die "containerd is not running after restart"

verify
say "done. Launch a workspace in a microVM with deploy/orchestration/up-kata.sh"

if [ "$WITH_CLH" = 1 ]; then
  setup_clh
  say "clh ready too. QEMU: up-kata.sh   |   Cloud Hypervisor: up-kata-clh.sh"
fi
