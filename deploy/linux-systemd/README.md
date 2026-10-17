# Linux host services (systemd)

The macOS host uses LaunchAgents, written by `scripts/setup-cloudflared.sh` and
`scripts/host-watcher.sh --install`. On Linux the same two services run as systemd
**user** units (no root needed for the units themselves):

```bash
bash scripts/setup-cloudflared.sh --hostname workspace.example.com   # tunnel + DNS, once
bash deploy/linux-systemd/install.sh --cloudflared                   # keep the tunnel up
bash deploy/linux-systemd/install.sh --host-watcher                  # clean up after a self-destruct
bash deploy/linux-systemd/install.sh --uninstall                     # remove both
```

`--dry-run` prints the units instead of installing them. The tunnel name comes from
`TUNNEL_NAME` (environment or `.env`, default `dev-workspace`).

User units stop when the last session ends unless lingering is on; the installer tries
`loginctl enable-linger` and prints the `sudo` command if that is refused.

**Not verified on a real Linux host.** These were written without one to test on; the
unit text is covered by a test, but starting them under systemd is not. The docker
daemon must be reachable by your user (`docker` group) for the host watcher.
