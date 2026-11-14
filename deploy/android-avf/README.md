# Run dev-workspace on an Android phone (AVF VM, native)

The workspace is a Docker image, and a phone has no Docker. But a capable Android phone can boot a
**real Linux VM** through the Android Virtualization Framework (AVF / pKVM), and inside that VM the
stack runs **natively** — no container runtime needed, because the VM itself is the isolation
boundary. This fits the threat model exactly: **Android is the untrusted host, the AVF VM is the
enclave.** The encrypted vault, the fail-closed egress firewall and everything else run as they do
in Docker.

This is the *headless* profile (`DESKTOP_STACK=none`): code-server + the services, no XFCE desktop
(a phone VM renders software-GL only and the stock AVF kernel lacks SysV IPC, so the desktop is both
heavy and broken there — use the browser code-server instead).

## Requirements

- A phone whose SoC supports AVF/pKVM: Pixel 6 or newer, or a recent Snapdragon flagship
  (8 Gen 3 / 8 Elite / 8 Elite Gen 5+). Android 16+ ships the built-in **Linux Terminal**
  (Settings > Developer options > Linux development environment). Non-Pixel devices without that
  menu can use a third-party AVF launcher (e.g. Droid-VM) that boots the same kind of VM.
- 8 GB+ RAM so the VM can be given a few GB (Terminal: Settings > Advanced > Memory size).
- A build machine with Docker to produce the rootfs (a phone does not build the image).

## 1. Build and export the rootfs (on the build machine)

```bash
# lean headless image
SDW_GH_MIRROR=https://ghfast.top/ DESKTOP_STACK=none WORKSPACE_VARIANT=headless \
  docker compose build workspace

# flatten it to a rootfs tarball
cid=$(docker create dev-workspace:headless)
docker export "$cid" | gzip > sdw-rootfs.tar.gz
docker rm "$cid"
```

Copy `sdw-rootfs.tar.gz`, this directory's scripts, and your `.env` into the VM (the AVF Terminal
exposes a shared folder; or use `scp` over the VM's forwarded SSH).

## 2. Install inside the VM

```bash
sudo sh install.sh --rootfs sdw-rootfs.tar.gz --env .env
sudo systemctl enable --now sdw-workspace
journalctl -u sdw-workspace -f          # watch the boot
```

`install.sh` extracts the rootfs to `/opt/sdw/rootfs`, installs `run-workspace.sh` and the
`sdw-workspace` systemd unit, and writes the env to `/etc/sdw/sdw.env` (root-only). `run-workspace.sh`
bind-mounts the persistent state and `/dev` into the rootfs and execs `sdw entrypoint` under
`chroot` — the same BOOT_PLAN as the container (vault unlock, egress firewall, Caddy, Clash,
supervisord).

## 3. Reach it from the phone browser

Caddy serves the UI on `127.0.0.1:8080` inside the VM. Forward that port to Android (AVF Terminal:
Settings > Port forwarding, add `8080`) and open `http://localhost:8080/` in the phone's browser.
Log in with `PASSWORD`.

## How the egress firewall works here

The fail-closed firewall tries nftables first and falls back to **iptables-legacy** automatically
(`infra/src/egress.rs`), because the stock AVF guest kernel ships netfilter but a non-functional
nftables. The policy is identical either way: default-DROP on OUTPUT, only loopback, established
replies, the Clash uid and `EGRESS_ALLOW_CIDRS` may leave. If a kernel offers **neither** a working
nftables nor an iptables `owner` match, the boot **fails closed** (it refuses to start rather than
run with egress open) — it never silently leaks.

## Verify the native path without a phone

`smoke-native.sh` boots the stack natively inside an isolated throwaway container (its own network
namespace, so it cannot touch the host) and checks the firewall, Caddy and code-server come up:

```bash
IMAGE=dev-workspace:headless sh deploy/android-avf/smoke-native.sh
```

## Honest status / limitations

- The firewall backend (nftables → iptables-legacy fallback) is unit-tested and verified on a real
  Linux kernel (owner-match drop/accept proven with packet counters).
- `smoke-native.sh` verifies the native chroot boot on a real kernel, device-free.
- **Not yet verified on a physical AVF device.** The open question is only whether a given stock AVF
  guest kernel includes the iptables `owner` match (`xt_owner`); the code auto-detects and fails
  closed if it is missing, so the worst case is "refuses to boot", never "boots leaky". A guest
  kernel can also be swapped for a Docker/netfilter-complete one without root (see the project docs)
  if you prefer the nftables path.
- Resource use: give the VM enough RAM; the headless image still carries the language runtimes
  (JDK/Node/Python) on disk, which AVF storage-ballooning handles but which you can drop with a
  leaner build if needed.
