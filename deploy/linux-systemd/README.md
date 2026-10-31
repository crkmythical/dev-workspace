# Linux host services (systemd)

The macOS host uses LaunchAgents, written by `deploy/scripts/setup-cloudflared.sh` and
`deploy/scripts/host-watcher.sh --install`. On Linux the same two services run as systemd
**user** units (no root needed for the units themselves):

```bash
bash deploy/scripts/setup-cloudflared.sh --hostname workspace.example.com   # tunnel + DNS, once
bash deploy/linux-systemd/install.sh --cloudflared                   # keep the tunnel up
bash deploy/linux-systemd/install.sh --host-watcher                  # clean up after a self-destruct
bash deploy/linux-systemd/install.sh --uninstall                     # remove both
```

`--dry-run` prints the units instead of installing them. The tunnel name comes from
`TUNNEL_NAME` (environment or `.env`, default `dev-workspace`).

User units stop when the last session ends unless lingering is on; the installer tries
`loginctl enable-linger` and prints the `sudo` command if that is refused.

**Verified** on Ubuntu 24.04 (systemd 255, in a VM, with a stand-in `docker` and `cloudflared`):
both units pass `systemd-analyze --user verify`, install and start with `install.sh`, the tunnel
unit comes back after `kill -9` (`Restart=always`), and a fake "container died + destruct marker"
event makes the watcher run `destroy.sh --force --paranoid` to the end (services and unit files
removed, `~/.cloudflared` removed, shell history scrubbed, project directory deleted).
**Not verified** on real hardware or with a real Cloudflare Tunnel. The docker daemon must be
reachable by your user (`docker` group) for the host watcher.

The watcher starts `destroy.sh` in a transient unit (`systemd-run --user`), not as its own
child: `destroy.sh` removes the watcher's service, and systemd kills everything a stopped
service started, so the cleanup used to be cut off right there (found by running it).
The unit text did not change, the fix is in `deploy/scripts/host-watcher.sh`: if the watcher is already
running, restart it to pick the script up (`systemctl --user restart dev-workspace-host-watcher.service`).
On macOS do the same with the LaunchAgent (`bash deploy/scripts/host-watcher.sh --uninstall`, then `--install`).
