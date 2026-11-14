#!/usr/bin/env bash
# Launch a workspace instance inside a Kata microVM (hardware isolation) via containerd CRI, instead
# of a plain Docker container. Experimental; the stable, default path stays docker compose (up.sh).
#
#   up-kata.sh <id> [host_port] [env_file]
#
# Each instance is one CRI pod (pause + the workspace container) on the kata runtime, with its own
# network namespace (CNI bridge) so the workspace's egress nftables firewall is confined to the
# microVM, a host port mapped to the gate, and per-instance host-path volumes. Reverse: down-kata.sh.
#
# WHERE THIS RUNS: on the container host — a bare-metal Linux box with KVM, or the Colima VM on a
# Mac (reach it with `colima ssh --profile <profile> -- sudo bash .../up-kata.sh ...`, having copied
# the repo in, OR run the one-liner this prints). It needs the kata runtime set up first:
#   deploy/scripts/setup-kata-runtime.sh
# and the workspace image available to Docker on the same host (it is imported into containerd's
# k8s.io namespace here if missing).
#
# KNOWN LIMITS in nested virtualization (Colima/vz on Apple Silicon): the agent's first vsock
# connect is flaky, so pod start is retried; and the heavy GUI (code-server, desktops) is sluggish
# under the doubled virtualization. The security-critical parts (vault/gocryptfs, egress nftables,
# the gate + login) work. On bare-metal Linux none of these limits apply.
set -euo pipefail

id="${1:?usage: up-kata.sh <id> [host_port] [env_file]}"
host_port="${2:-}"
env_file="${3:-}"
image_docker="${SDW_KATA_IMAGE:-dev-workspace:kali}"       # Docker image tag on this host
image_ref="docker.io/library/${image_docker}"             # how containerd/CRI names it
pod_name="sdw-ws-${id}"
vol_root="/var/lib/sdw-kata/${id}"
# Which containerd runtime handler to launch under. Default is the QEMU-backed `kata`; up-kata-clh.sh
# sets this to `kata-clh` to use the Cloud Hypervisor (Rust VMM) shim instead. Both are set up by
# deploy/scripts/setup-kata-runtime.sh (clh needs its --with-clh step).
runtime="${SDW_KATA_RUNTIME:-kata}"

die() { printf 'up-kata: %s\n' "$*" >&2; exit 1; }
command -v crictl >/dev/null || die "crictl not found — run deploy/scripts/setup-kata-runtime.sh first"
crictl info 2>/dev/null | grep -q "\"${runtime}\"" || die "containerd has no '${runtime}' runtime — run setup-kata-runtime.sh${runtime:+ (for kata-clh: --with-clh)}"
[ -e /dev/kvm ] || die "/dev/kvm missing — no (nested) virtualization on this host"

# A free loopback port on this host for the gate, unless given.
if [ -z "$host_port" ]; then
  for p in $(seq 18090 18189); do
    if ! ss -ltn 2>/dev/null | grep -q ":${p} "; then host_port="$p"; break; fi
  done
  [ -n "$host_port" ] || die "no free port in 18090-18189"
fi

# The image must be in containerd's k8s.io namespace (CRI). Import from Docker if missing.
if ! crictl images 2>/dev/null | grep -q "dev-workspace"; then
  command -v docker >/dev/null || die "image $image_ref not in containerd and docker not available to import it"
  echo ">> importing $image_docker into containerd (k8s.io namespace)"
  docker save "$image_docker" | ctr -n k8s.io images import -
fi

mkdir -p "$vol_root"/{vault,cache,clash,destruct,audit} /var/log/sdw-kata
workdir="$(mktemp -d)"; trap 'rm -rf "$workdir"' EXIT

# --- pod sandbox: own netns (CNI), map host_port -> the gate on 8080 ---------------------------
cat > "$workdir/pod.json" <<EOF
{ "metadata": { "name": "${pod_name}", "namespace": "default", "uid": "${pod_name}-uid", "attempt": 1 },
  "log_directory": "/var/log/sdw-kata",
  "port_mappings": [ { "protocol": 0, "container_port": 8080, "host_port": ${host_port} } ],
  "linux": {} }
EOF

# --- container: full workspace, the caps compose grants, /dev/fuse, env from the file -----------
# Build the env array from env_file (KEY=VALUE lines) + required defaults, JSON-escaped by python.
python3 - "$env_file" "$image_ref" "$vol_root" > "$workdir/ctr.json" <<'PY'
import json, sys, os
env_file, image_ref, vol_root = sys.argv[1], sys.argv[2], sys.argv[3]
envs = {}
if env_file and os.path.isfile(env_file):
    for line in open(env_file):
        line = line.strip()
        if not line or line.startswith('#') or '=' not in line:
            continue
        k, v = line.split('=', 1)
        envs[k.strip()] = v.strip().strip('"').strip("'")
# A plain local launch still needs these to pass the entrypoint preflight.
envs.setdefault('PASSWORD', 'changeme-kata')
envs.setdefault('CLASH_SUBSCRIPTION_URL', 'https://example.com/none')
envs.setdefault('DESKTOP_AUDIO', '0')
caps = ["SYS_ADMIN","NET_ADMIN","CHOWN","DAC_OVERRIDE","FOWNER","FSETID","SETUID","SETGID","KILL","NET_BIND_SERVICE","MKNOD"]
spec = {
  "metadata": {"name": "workspace"},
  "image": {"image": image_ref},
  "log_path": "workspace.log",
  "envs": [{"key": k, "value": v} for k, v in envs.items()],
  "mounts": [
    {"host_path": f"{vol_root}/vault",    "container_path": "/vault/cipher"},
    {"host_path": f"{vol_root}/cache",    "container_path": "/pentest/cipher"},
    {"host_path": f"{vol_root}/clash",    "container_path": "/etc/clash"},
    {"host_path": f"{vol_root}/destruct", "container_path": "/var/run/self-destruct"},
    {"host_path": f"{vol_root}/audit",    "container_path": "/var/log/sdw"},
  ],
  "devices": [{"host_path": "/dev/fuse", "container_path": "/dev/fuse", "permissions": "rwm"}],
  "linux": {"security_context": {
    "capabilities": {"add_capabilities": caps},
    "seccomp": {"profile_type": 1},
  }},
}
json.dump(spec, sys.stdout)
PY

echo ">> starting ${runtime} pod ${pod_name} (host port ${host_port})"
# nested-virt vsock first-connect is flaky; retry runp until the sandbox is Ready.
pod=""
for i in $(seq 1 8); do
  crictl runp --runtime "$runtime" "$workdir/pod.json" >/dev/null 2>&1 || true
  pod="$(crictl pods -q --name "$pod_name" --state Ready 2>/dev/null | head -1)"
  [ -n "$pod" ] && break
  echo "   runp retry $i…"
done
[ -n "$pod" ] || die "pod sandbox did not come up after retries (nested-virt vsock); try again"
ctr_id="$(crictl create "$pod" "$workdir/ctr.json" "$workdir/pod.json" 2>&1 | tail -1)"
crictl start "$ctr_id" >/dev/null
echo ">> workspace container started; waiting for the gate…"
up=0
for i in $(seq 1 48); do
  if curl -s --noproxy '*' -o /dev/null --max-time 3 "http://127.0.0.1:${host_port}/auth/login" 2>/dev/null; then
    code="$(curl -s --noproxy '*' -o /dev/null -w '%{http_code}' --max-time 3 "http://127.0.0.1:${host_port}/auth/login" 2>/dev/null)"
    [ "$code" = "200" ] && { up=1; break; }
  fi
  sleep 5
done
echo
if [ "$up" = 1 ]; then
  echo "== ${id} up in a Kata microVM (${runtime}): http://127.0.0.1:${host_port}/  (pod ${pod_name})"
  echo "   login with PASSWORD; vault + egress firewall run inside the microVM"
  echo "   from a Mac host, tunnel it:  colima ssh --profile <profile> -- -N -L ${host_port}:127.0.0.1:${host_port}"
else
  echo "!! ${id}: pod is up but the gate did not answer 200 in time (nested-virt can be slow)."
  echo "   check:  crictl logs ${ctr_id}   /   curl --noproxy '*' http://127.0.0.1:${host_port}/auth/login"
fi
echo "   down (pod + volumes): down-kata.sh ${id}"
