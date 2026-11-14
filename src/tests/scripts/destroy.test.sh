#!/usr/bin/env bash
# Tests for destroy.sh against a fake `docker`, `cloudflared` and `gh` on the PATH: what it removes
# (only THIS compose project's volumes), what it keeps (the checkout, unless --delete-project), and
# that its summary is true. The script is destructive, so it is never run for real here.
# Run: src/tests/scripts/destroy.test.sh   (needs only bash)
set -u
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
failed=0
check() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; else printf '  FAIL  %s\n        expected [%s]\n        got      [%s]\n' "$1" "$2" "$3"; failed=1; fi
}
has() { # name text pattern
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then printf '  PASS  %s\n' "$1"; else printf '  FAIL  %s\n        no /%s/ in: %s\n' "$1" "$3" "$2"; failed=1; fi
}
hasnt() { # name text pattern
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then printf '  FAIL  %s\n        unexpected /%s/ in: %s\n' "$1" "$3" "$2"; failed=1; else printf '  PASS  %s\n' "$1"; fi
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
bin="$work/bin"
state="$work/state"
mkdir -p "$bin" "$state"

# --- fakes ---------------------------------------------------------------------------------------
# docker: state lives in files under $FAKE_STATE; every call is logged. `compose down -v` removes
# the fakeproj volumes unless FAKE_DOWN_FAILS is set (a daemon that would not let go).
cat >"$bin/docker" <<'EOF'
#!/usr/bin/env bash
echo "docker $*" >>"$FAKE_STATE/calls.log"
[ -n "${FAKE_DOCKER_DOWN:-}" ] && [ "$1" = info ] && exit 1
case "$1 $2" in
  "compose down") [ -n "${FAKE_DOWN_FAILS:-}" ] || : >"$FAKE_STATE/vol.fakeproj"; exit 0 ;;
  "compose config") echo '{"name":"fakeproj"}'; exit 0 ;;
  "volume ls")
    for a in "$@"; do case "$a" in label=com.docker.compose.project=*) proj="${a#label=com.docker.compose.project=}" ;; esac; done
    cat "$FAKE_STATE/vol.$proj" 2>/dev/null; exit 0 ;;
  "volume rm") shift 2; for v in "$@"; do for f in "$FAKE_STATE"/vol.*; do grep -vx "$v" "$f" >"$f.new"; mv "$f.new" "$f"; done; done; exit 0 ;;
  "ps -aq") cat "$FAKE_STATE/containers" 2>/dev/null; exit 0 ;;
  "image ls") cat "$FAKE_STATE/images" 2>/dev/null; exit 0 ;;
  "image rm") : >"$FAKE_STATE/images"; exit 0 ;;
esac
exit 0
EOF
cat >"$bin/cloudflared" <<'EOF'
#!/usr/bin/env bash
echo "cloudflared $*" >>"$FAKE_STATE/calls.log"
exit "${FAKE_CLOUDFLARED_RC:-0}"
EOF
cat >"$bin/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh $*" >>"$FAKE_STATE/calls.log"
exit 1
EOF
chmod +x "$bin"/*

# A project directory shaped like the real one: the script, its helpers, a compose file.
new_project() {
  rm -rf "$work/fakeproj"
  mkdir -p "$work/fakeproj/deploy/scripts/lib"
  cp "$repo/destroy.sh" "$work/fakeproj/"
  cp "$repo/deploy/scripts/lib/env.sh" "$repo/deploy/scripts/lib/scrub.sh" "$work/fakeproj/deploy/scripts/lib/"
  : >"$work/fakeproj/docker-compose.yml"
  printf 'fakeproj_vault-data\nfakeproj_audit-data\nfakeproj_notmine\n' >"$state/vol.fakeproj"
  printf 'otherproj_vault-data\n' >"$state/vol.otherproj"
  : >"$state/containers"
  printf 'dev-workspace:kali\ndev-workspace:smoke\npython:3.12-alpine\n' >"$state/images"
  : >"$state/calls.log"
}
run_destroy() { # args...  → prints the output, sets $rc
  OUT="$(cd "$work/fakeproj" && env PATH="$bin:$PATH" FAKE_STATE="$state" HOME="$work/home" ./destroy.sh --force "$@" 2>&1)"
  rc=$?
}
mkdir -p "$work/home"

# --- 1. the ordinary run ----------------------------------------------------------------------
echo "== a clean run"
new_project
run_destroy
check "exits 0" 0 "$rc"
has "says the containers and volumes are gone" "$OUT" 'Containers \+ volumes removed'
has "… and the images" "$OUT" 'Docker images removed'
has "… and the tunnel" "$OUT" "Tunnel 'dev-workspace' deleted"
check "the volumes of ANOTHER project are untouched" "otherproj_vault-data" "$(cat "$state/vol.otherproj")"
hasnt "no volume of another project was asked to be removed" "$(cat "$state/calls.log")" 'volume rm.*otherproj'
hasnt "a volume that is not one of the workspace's was not removed either" "$(cat "$state/calls.log")" 'volume rm.*notmine'
check "the checkout is still there" yes "$([ -f "$work/fakeproj/destroy.sh" ] && echo yes || echo no)"
has "and the output says so" "$OUT" 'left in place; add --delete-project'

echo "== --delete-project"
new_project
run_destroy --delete-project
check "exits 0" 0 "$rc"
check "the checkout is gone" no "$([ -e "$work/fakeproj/destroy.sh" ] && echo yes || echo no)"

# --- 2. what is not gone is not reported as gone --------------------------------------------------
echo "== a container Docker would not remove"
new_project
echo abc123 >"$state/containers"
run_destroy --delete-project
check "exits non-zero" 1 "$rc"
has "the summary names what is left" "$OUT" '! Containers or volumes of project .fakeproj. are still there: abc123'
hasnt "and does not claim they were removed" "$OUT" 'Containers \+ volumes removed'
check "the checkout is kept so the run can be repeated, even with --delete-project" yes "$([ -f "$work/fakeproj/destroy.sh" ] && echo yes || echo no)"

echo "== volumes that survive 'down'"
# The fake 'down' empties the project's volumes; here it does not, and the backstop must finish the job.
new_project
OUT="$(cd "$work/fakeproj" && env PATH="$bin:$PATH" FAKE_STATE="$state" FAKE_DOWN_FAILS=1 HOME="$work/home" ./destroy.sh --force 2>&1)"
rc=$?
check "the backstop removes the workspace's own volumes and only those" "fakeproj_notmine" "$(cat "$state/vol.fakeproj")"
has "… of which two were the workspace's" "$(cat "$state/calls.log")" 'volume rm fakeproj_vault-data fakeproj_audit-data'
has "the leftover that is not the workspace's is reported, not hidden" "$OUT" 'fakeproj_notmine'
check "exits non-zero" 1 "$rc"

echo "== Docker not running"
new_project
OUT="$(cd "$work/fakeproj" && env PATH="$bin:$PATH" FAKE_STATE="$state" FAKE_DOCKER_DOWN=1 HOME="$work/home" ./destroy.sh --force 2>&1)"
rc=$?
check "exits non-zero" 1 "$rc"
has "it says nothing was removed" "$OUT" 'Docker is not reachable: .*NOT removed'

echo "== the tunnel and the remote repo are reported as they went"
new_project
OUT="$(cd "$work/fakeproj" && env PATH="$bin:$PATH" FAKE_STATE="$state" FAKE_CLOUDFLARED_RC=1 HOME="$work/home" ./destroy.sh --force 2>&1)"
has "a tunnel that could not be deleted is not claimed deleted" "$OUT" "Tunnel 'dev-workspace' not deleted"
new_project
printf 'VAULT_GIT_REPO=owner/vault\n' >"$work/fakeproj/.env"
run_destroy --remote
has "a remote that could not be deleted is not claimed deleted" "$OUT" '! The remote vault repo was NOT deleted'
check "exits non-zero" 1 "$rc"

if [ "$failed" -eq 0 ]; then echo "all passed"; else echo "FAILED"; fi
exit "$failed"
