# Linux host services (systemd user units)

Unit policy and rendering live in the cross-platform Rust `sdw-host` binary. This directory keeps a
four-line compatibility launcher:

```bash
deploy/scripts/setup-cloudflared.sh --hostname workspace.example.com
deploy/linux-systemd/install.sh --cloudflared
deploy/linux-systemd/install.sh --host-watcher
deploy/linux-systemd/install.sh --uninstall
```

Add `--dry-run` to print units without writing or starting them. The Cloudflare unit reads the
per-tunnel config `~/.cloudflared/sdw-<TUNNEL_NAME>.yml`; the watcher unit invokes the same
`sdw-host` executable directly and records the current PATH and Docker context.

The launcher incrementally builds the release host binary when Cargo exists. For a stable service
path, run `make install-host` and set `SDW_HOST_BIN=~/.local/bin/sdw-host` when installing.

User services stop with the final login session unless lingering is enabled. The installer calls
`loginctl enable-linger`; if policy rejects it, it prints the corresponding sudo command.

The former Shell renderer was validated on Ubuntu 24.04 with stand-ins. The new Rust renderer and
lifecycle are covered by `src/tests/scripts/host-orchestration.test.sh`, including quoting,
Cloudflare config selection, typed rollback and Docker-context rejection. A real Linux systemd +
real Cloudflare run has not yet been repeated after this migration.
