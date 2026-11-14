#!/bin/sh
# smoke-native.sh — prove the image's stack runs NATIVELY (chroot, no container runtime) on a real
# Linux kernel, the way it will inside an Android AVF VM. DEVICE-FREE: it uses a throwaway Docker
# container with its OWN network namespace as a stand-in for the VM, so the fail-closed egress
# firewall and the chroot boot cannot affect the host or the live workspace.
#
#   IMAGE=dev-workspace:headless sh deploy/android-avf/smoke-native.sh
#
# Checks, in order:
#   1. the image exports to a flat rootfs;
#   2. `sdw` executes under chroot and resolves its libraries (the native-exec mechanism);
#   3. run-workspace.sh mounts the binds and `sdw entrypoint` boots far enough to load the egress
#      firewall and bring Caddy's /auth/login + code-server (:8082) up.
# A throwaway PASSWORD is used and a dummy CLASH_SUBSCRIPTION_URL (resolves to 0 nodes, so Clash
# starts and egress is reported "degraded" — non-fatal), so no real secrets or proxy nodes needed.
#
# Artifacts go in via `docker cp` (not `-v`): with a remote daemon (Colima) a bind mount of a Mac
# temp path is invisible to the VM, but `docker cp` streams through the daemon and always works.
set -eu

IMAGE="${IMAGE:-dev-workspace:headless}"
export DOCKER_CONTEXT="${DOCKER_CONTEXT:-colima-dev-workspace}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CNAME="sdw-native-smoke-$$"
TMP="$(mktemp -d)"
# Reuse a prebuilt rootfs tar across runs with SDW_SMOKE_TAR=/path (skips the big re-export).
TAR="${SDW_SMOKE_TAR:-$TMP/rootfs.tar}"
DRIVER="$TMP/driver.sh"
cleanup() { docker rm -f "$CNAME" >/dev/null 2>&1 || true; rm -rf "$TMP"; }
trap cleanup EXIT

if [ -s "$TAR" ]; then
  echo "==> [1/4] reusing rootfs tar $TAR ($(du -h "$TAR" | cut -f1))"
else
  echo "==> [1/4] exporting $IMAGE to a flat rootfs"
  cid="$(docker create "$IMAGE")"
  docker export "$cid" > "$TAR"
  docker rm "$cid" >/dev/null
  echo "    rootfs: $(du -h "$TAR" | cut -f1)"
fi

cat > "$DRIVER" <<'DRIVER_EOF'
#!/bin/sh
set -eu
apt-get update -qq >/dev/null 2>&1
apt-get install -y -qq iptables nftables netcat-openbsd curl >/dev/null 2>&1
mkdir -p /opt/sdw/rootfs
tar -C /opt/sdw/rootfs -xf /rootfs.tar
install -m 0755 /run-workspace.sh /opt/sdw/run-workspace.sh

echo "==> [2/4] sdw executes under chroot (native-exec mechanism)"
mount -t proc proc /opt/sdw/rootfs/proc
chroot /opt/sdw/rootfs /usr/local/bin/sdw doctor >/tmp/doctor.out 2>&1 || true
umount /opt/sdw/rootfs/proc || true
if grep -qiE 'workspace|vault|egress|firewall|doctor' /tmp/doctor.out; then
  echo "    PASS: sdw ran chrooted (doctor produced a report)"
else
  echo "    FAIL: sdw did not run chrooted:"; sed 's/^/      /' /tmp/doctor.out; exit 1
fi

echo "==> [3/4] native boot via run-workspace.sh (PASSWORD set, no subscription)"
SDW_ROOTFS=/opt/sdw/rootfs; SDW_STATE=/var/lib/sdw-workspace
# A dummy (non-empty) subscription satisfies the preflight required-env check; it resolves to 0
# nodes, so egress is reported "degraded" (non-fatal) — fine for proving the boot path.
PASSWORD=smoke-test-password; EGRESS_ENFORCE=1
CLASH_SUBSCRIPTION_URL=https://example.invalid/subscription
export SDW_ROOTFS SDW_STATE PASSWORD EGRESS_ENFORCE CLASH_SUBSCRIPTION_URL
/opt/sdw/run-workspace.sh start >/tmp/boot.out 2>&1 &
boot_pid=$!

echo "==> [4/4] waiting for services (up to 150s)"
ok_fw=0; ok_cs=0; ok_caddy=0; i=0
while [ "$i" -lt 150 ]; do
  i=$((i + 1)); sleep 1
  if [ "$ok_fw" = 0 ]; then
    if nft list table inet sdw_egress >/dev/null 2>&1 \
       || iptables-legacy -S OUTPUT 2>/dev/null | grep -q -- '-P OUTPUT DROP'; then
      ok_fw=1; echo "    PASS: egress firewall loaded (${i}s)"
    fi
  fi
  if [ "$ok_cs" = 0 ] && nc -z 127.0.0.1 8082 2>/dev/null; then
    ok_cs=1; echo "    PASS: code-server listening on 8082 (${i}s)"
  fi
  if [ "$ok_caddy" = 0 ] && curl -fsS -o /dev/null http://127.0.0.1:8080/auth/login 2>/dev/null; then
    ok_caddy=1; echo "    PASS: Caddy /auth/login responds on 8080 (${i}s)"
  fi
  [ "$ok_fw" = 1 ] && [ "$ok_cs" = 1 ] && [ "$ok_caddy" = 1 ] && break
done

echo "--- boot log (tail) ---"; tail -n 30 /tmp/boot.out 2>/dev/null || true
kill "$boot_pid" 2>/dev/null || true
/opt/sdw/run-workspace.sh stop >/dev/null 2>&1 || true

fail=0
[ "$ok_fw" = 1 ]    || { echo "FAIL: egress firewall did not load"; fail=1; }
[ "$ok_cs" = 1 ]    || { echo "FAIL: code-server did not listen"; fail=1; }
[ "$ok_caddy" = 1 ] || { echo "FAIL: Caddy /auth/login did not respond"; fail=1; }
[ "$fail" = 0 ] && echo "==> NATIVE SMOKE PASSED" || echo "==> NATIVE SMOKE FAILED"
exit "$fail"
DRIVER_EOF

echo "==> creating the isolated test container (own netns)"
docker create --name "$CNAME" \
  --cap-add=NET_ADMIN --cap-add=SYS_ADMIN \
  --security-opt apparmor=unconfined --security-opt seccomp=unconfined \
  --device /dev/fuse \
  debian:trixie-slim sleep 900 >/dev/null
docker cp "$TAR" "$CNAME:/rootfs.tar"
docker cp "$DRIVER" "$CNAME:/driver.sh"
docker cp "$HERE/run-workspace.sh" "$CNAME:/run-workspace.sh"
docker start "$CNAME" >/dev/null

echo "==> running the native boot"
rc=0
docker exec "$CNAME" sh /driver.sh || rc=$?
exit "$rc"
