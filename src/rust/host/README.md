# sdw-host

Cross-platform Rust control plane for operations that happen outside the workspace container.

```text
sdw-host destroy       project-scoped cleanup and optional checkout deletion
sdw-host watch         self-destruct marker watcher and LaunchAgent installation
sdw-host setup-tunnel  named Cloudflare tunnel configuration
sdw-host provision     ephemeral Cloudflare Tunnel + DNS + Access resources
sdw-host teardown      retryable deletion from persisted typed state
sdw-host up/down       edge lifecycle plus per-instance Compose lifecycle
sdw-host run-tunnel    foreground connector from private instance state
sdw-host purge-history  verified bundle-first removal of .env from every Git ref
sdw-host update-tools   release/checksum resolution and transactional image-tool pin updates
sdw-host services      Linux systemd user-unit rendering/install/removal
sdw-host secrets-init  atomic file-secret initialization
```

Repository `.sh` entrypoints are compatibility launchers. They build the current release binary
incrementally when Cargo is available, then `exec` it; `SDW_HOST_BIN` selects a packaged binary.

## Design constraints

- Dependencies are limited to serde/serde_json, sha2 and zeroize. HTTP remains the installed `curl`,
  Docker remains the Docker CLI, and service managers remain native host tools.
- Commands use `std::process::Command`; untrusted values are never shell-interpolated.
- `SDW_DOCKER_CONTEXT` optionally locks every Docker mutation and watcher to one daemon.
- Cloudflare state is versioned JSON, atomically written mode 0600 after every successful action.
- Failed rollback/teardown keeps state and returns non-zero.
- API credentials use temporary private curl config files; bodies use stdin; connector tokens use
  only the connector environment. Long-lived owned secret strings are zeroized on drop.
- Saved connector PIDs are checked against the process command before a process-group signal.
- Tool pin updates are planned completely before any pin is written; failed plans leave every file
  unchanged, and write failures roll previously changed files back.
- History purge requires git-filter-repo, verifies a full bundle before rewriting, verifies all refs
  after rewriting, and only then expires reflogs/objects.
- Checkout deletion requires successful Docker verification, canonical path checks and the Compose
  project marker.

## Verification

```bash
make test-host
cargo clippy --locked -p sdw-host -- -D warnings
```

`deploy/scripts/install-sdw-host.sh` installs release archives on macOS/Linux amd64/arm64 after
checking the adjacent SHA-256 file. Tag builds publish exactly nine assets (four archives, four
adjacent checksums and `SHA256SUMS`) only after image smoke, manifest and host build jobs have
passed.

The mock lifecycle does not contact Cloudflare or a real Docker daemon. Real account mutation is
intentionally not part of automated validation.
