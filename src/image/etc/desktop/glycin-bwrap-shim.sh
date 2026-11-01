#!/bin/bash
# bwrap shim for glycin image loaders — portable across hosts.
#
# glycin (GTK4's image loader, pulled in via librsvg) runs each loader inside a
# bubblewrap sandbox built on user namespaces. On hosts where unprivileged user
# namespaces are unavailable (colima/Lima, some locked-down Docker/CI runtimes)
# `bwrap --unshare-all` fails with "setting up uid map: Operation not permitted";
# the SVG icon loader dies, and GTK turns the failed icon load into a FATAL
# assertion -> the whole XFCE session aborts (SIGABRT / exit 134) in a crash loop
# -> black screen in the VNC/Selkies desktop.
#
# This shim intercepts bwrap. For a glycin loader invocation it FIRST checks
# whether a user namespace can be created here; only if it CANNOT does it run the
# loader directly (no sandbox). On hosts where userns works, and for every
# non-glycin caller, it falls through to the real bwrap so the sandbox is kept.
# Running a trusted system image loader unsandboxed on an otherwise-unusable host
# is an acceptable trade-off for a single-tenant workspace.
set -u

# Is this a glycin loader call? Collect the loader command (loader path + its args).
cmd=()
is_glycin=0
for a in "$@"; do
  if [ "$is_glycin" -eq 0 ]; then
    case "$a" in
      /usr/libexec/glycin-loaders/*) is_glycin=1 ;;
    esac
  fi
  [ "$is_glycin" -eq 1 ] && cmd+=("$a")
done

if [ "$is_glycin" -eq 1 ] && [ "${#cmd[@]}" -gt 0 ]; then
  if ! unshare --user --map-root-user true >/dev/null 2>&1; then
    # User namespaces unavailable → the real bwrap would fail → run loader directly.
    exec "${cmd[@]}"
  fi
fi

exec /usr/bin/bwrap.real "$@"
