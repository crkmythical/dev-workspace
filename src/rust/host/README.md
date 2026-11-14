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
sdw-host services      Linux systemd user-unit rendering/install/removal
sdw-host secrets-init  atomic file-secret initialization
```

Repository `.sh` entrypoints are compatibility launchers. They build the current release binary
incrementally when Cargo is available, then `exec` it; `SDW_HOST_BIN` selects a packaged binary.

## Design constraints

- Dependencies are limited to serde/serde_json and zeroize. HTTP remains the installed `curl`,
  Docker remains the Docker CLI, and service managers remain native host tools.
- Commands use `std::process::Command`; untrusted values are never shell-interpolated.
- `SDW_DOCKER_CONTEXT` optionally locks every Docker mutation and watcher to one daemon.
- Cloudflare state is versioned JSON, atomically written mode 0600 after every successful action.
- Failed rollback/teardown keeps state and returns non-zero.
- API credentials use temporary private curl config files; bodies use stdin; connector tokens use
  only the connector environment. Long-lived owned secret strings are zeroized on drop.
- Saved connector PIDs are checked against the process command before a process-group signal.
- Checkout deletion requires successful Docker verification, canonical path checks and the Compose
  project marker.

## Verification

```bash
make test-host
cargo clippy --locked -p sdw-host -- -D warnings
```

The mock lifecycle does not contact Cloudflare or a real Docker daemon. Real account mutation is
intentionally not part of automated validation.
