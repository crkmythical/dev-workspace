#!/usr/bin/env bash
# One-time setup of a DEDICATED Colima VM to build and run the workspace on a Mac.
#
#   deploy/scripts/setup-colima-build-vm.sh [--profile NAME] [--cpu N] [--memory GiB] [--disk GiB]
#                                           [--proxy URL]
#
# Why a VM of its own: the default Colima VM is 2 CPU / 2 GiB, too small to compile the Rust binary
# with LTO or to run the desktop, and it usually hosts other containers. A second profile leaves
# that one untouched; the workspace's privileged container (SYS_ADMIN, seccomp:unconfined) also
# ends up behind a VM boundary of its own. Defaults: profile dev-workspace, 6 CPU, 12 GiB, 80 GiB.
#
# --proxy URL  an HTTP proxy as the VM reaches it (on Colima the host is 192.168.5.2, so a proxy
#              on the Mac's 127.0.0.1:1080 must listen on an address the VM can reach). Without
#              it, Docker Hub and GitHub are often unreachable from mainland China. It is set on
#              the Docker daemon (image pulls); for the build steps use BUILD_PROXY in .env.
#
# An existing profile of that name is never recreated or resized. Written for bash 3.2 (macOS).
set -euo pipefail

PROFILE="dev-workspace"
CPU=6
MEMORY=12
DISK=80
PROXY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --profile) PROFILE="${2:?}"; shift 2 ;;
    --cpu) CPU="${2:?}"; shift 2 ;;
    --memory) MEMORY="${2:?}"; shift 2 ;;
    --disk) DISK="${2:?}"; shift 2 ;;
    --proxy) PROXY="${2:?}"; shift 2 ;;
    -h | --help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1 ($2)" >&2; exit 1; }; }
need colima "brew install colima"
need docker "brew install docker"
# Without the buildx plugin, `docker compose build` silently falls back to the classic builder,
# which cannot build this Dockerfile (COPY --chmod, build caches).
if ! docker buildx version >/dev/null 2>&1; then
  cat >&2 <<'EOF'
missing: the docker buildx plugin. Install it and make Docker find it:
  brew install docker-buildx
  ln -sfn /opt/homebrew/lib/docker/cli-plugins/docker-buildx ~/.docker/cli-plugins/docker-buildx
EOF
  exit 1
fi

if colima list 2>/dev/null | awk -v p="$PROFILE" 'NR>1 && $1==p {found=1} END {exit !found}'; then
  state="$(colima list 2>/dev/null | awk -v p="$PROFILE" 'NR>1 && $1==p {print $2}')"
  echo "Colima profile '$PROFILE' exists ($state); leaving its size alone."
  [ "$state" = "Running" ] || colima start --profile "$PROFILE"
else
  echo "Creating Colima profile '$PROFILE' (${CPU} CPU, ${MEMORY} GiB, ${DISK} GiB disk) ..."
  colima start --profile "$PROFILE" --cpu "$CPU" --memory "$MEMORY" --disk "$DISK" \
    --vm-type vz --mount-type virtiofs --runtime docker --activate=false
fi

if [ -n "$PROXY" ]; then
  echo "Pointing the Docker daemon in '$PROFILE' at $PROXY ..."
  colima ssh --profile "$PROFILE" -- env PROXY="$PROXY" bash -s <<'EOF'
set -eu
code=$(curl -sS -o /dev/null --max-time 15 -x "$PROXY" -w '%{http_code}' https://registry-1.docker.io/v2/ || true)
[ "$code" = 401 ] || { echo "the proxy does not reach Docker Hub from inside the VM (HTTP $code)" >&2; exit 1; }
sudo mkdir -p /etc/systemd/system/docker.service.d
sudo tee /etc/systemd/system/docker.service.d/http-proxy.conf >/dev/null <<UNIT
[Service]
Environment="HTTP_PROXY=$PROXY"
Environment="HTTPS_PROXY=$PROXY"
Environment="NO_PROXY=localhost,127.0.0.0/8,192.168.0.0/16,172.16.0.0/12,10.0.0.0/8,mirrors.aliyun.com,mirrors.ustc.edu.cn"
UNIT
sudo systemctl daemon-reload
sudo systemctl restart docker
sleep 3
sudo systemctl is-active docker
EOF
fi

cat <<EOF

Done. Use this VM for the workspace in the shell where you run docker / make:

  export DOCKER_CONTEXT=colima-$PROFILE
  docker compose build        # or: make build
  docker compose up -d

and, in .env, for a network where GitHub / apt are slow (see .env.example, "IMAGE BUILD"):

  KALI_MIRROR=mirrors.aliyun.com
  BUILD_PROXY=${PROXY:-http://192.168.5.2:1080}
  COMPOSE_FILE=docker-compose.yml:docker-compose.apparmor.yml   # the VM is Ubuntu with AppArmor
EOF
