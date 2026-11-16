# Run dev-workspace on a phone WITHOUT root — via Podroid

The clean no-root path for phones that don't have the built-in Android Linux Terminal (i.e. almost
every Snapdragon phone, including the RedMagic 9 Pro / 9 Pro+ on Snapdragon 8 Gen 3). It avoids
rooting, bootloader unlock, DroidVM and the Gunyah flakiness entirely.

## Why Podroid fits this project

[Podroid](https://github.com/ExTV/Podroid) is a rootless Android app that boots a **real Alpine
Linux VM with its own kernel** (not proot/chroot) and ships **Docker/Podman/LXC** inside it. Two
backends: **QEMU/TCG** (software, the default, no permission — what the 8 Gen 3 uses) and **AVF/pKVM**
(hardware-accelerated, Pixel-class only). Crucially, Podroid's guest kernel is built with
**FUSE, netfilter, overlayfs, veth/bridge/tun and IPv6 forced on** — exactly what this workspace
needs: FUSE for the gocryptfs vault, netfilter for the fail-closed egress firewall. Inside the VM
you are root (no Android root), so a privileged container gets the caps it needs.

Trade-off on Snapdragon 8 Gen 3 (no app-AVF): the VM is **QEMU software-emulated** (no KVM/pKVM
without root), so slower than native. The saving grace is that this is **same-architecture**
arm64-on-arm64 emulation, far cheaper than the x86-on-arm case — but still below native. code-server
/ CLI runs comfortably; the full XFCE desktop is the heaviest load (compositing + video encode) and
needs a low-load preset — see **Run the full desktop GUI** below. The verified `v1.0.0` Release
provides the Kali/KasmVNC ARM64 desktop package; an optional code-server-only `kali-headless` image
can be built locally when the smaller package is useful.

## What was verified (device-free) vs. what needs the phone

- Verified on a real Linux kernel: the `dev-workspace:kali-headless` image boots as a **privileged
  container** with the egress firewall (nftables, fail-closed) loaded, the gocryptfs **vault
  mounted (FUSE)**, and **code-server + Caddy** up in ~40s. That is exactly how Podroid runs a
  container, so the image side is proven.
- Re-verified on the current Rust-runtime build (arm64, Colima VM): both `dev-workspace:kali-headless`
  and `dev-workspace-kali-kasmvnc:latest` boot with the `docker run` flags below and report healthy in
  ~40s. The unlock page takes a password only (no username), a wrong password gets 401, and
  `doctor` passes (firewall enforced, vault mounted). code-server loads after unlock, and the
  desktop image serves KasmVNC at `/vnc/` (the headless image has no `/vnc/`).
- Verified from Podroid's source: its kernel forces the configs we rely on to `=y`.
- NOT yet run on a physical Podroid install (no Android device here). The remaining step — install
  the APK, import the image, measure emulation speed — is done on the phone; steps below.

## Steps (no root)

On the Mac, download the verified ARM64 archive from the private `v1.0.0` Release:

```bash
cd ~
gh auth status
gh release download v1.0.0 \
  --repo crkmythical/dev-workspace \
  --pattern 'dev-workspace-kali-kasmvnc-linux-arm64.tar.gz' \
  --clobber
tar -tzf ~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz > /dev/null && echo OK
```

If a smaller code-server-only image is needed, build it on the dedicated Colima daemon rather than
Docker Desktop. Every option is explicit so values in `.env` cannot silently change the package:

```bash
cd ~/IdeaProjects/dev-workspace
export DOCKER_CONTEXT=colima-dev-workspace
BASE_PROFILE=kali BASE_IMAGE=kalilinux/kali-rolling WORKSPACE_VARIANT=kali-headless \
  DESKTOP_STACK=none EDITOR_SUITE=code-server DEV_RUNTIMES=none DESKTOP_APPS=none \
  PENTEST_SUITE=none BACKUP_TRANSFER=none make build \
  && docker save -o ~/sdw-image-new.tar dev-workspace:kali-headless \
  && gzip -f ~/sdw-image-new.tar \
  && tar -tzf ~/sdw-image-new.tar.gz > /dev/null \
  && mv ~/sdw-image-new.tar.gz ~/sdw-image.tar.gz
```

Use `docker save -o` chained with `&&`, not `docker save | gzip`: a failed save can otherwise leave a
tiny but superficially valid gzip file. `INSTALL-phone-zh.md` keeps the full write-to-`-new`, verify,
then rename procedure for both profiles.

On the phone:

1. Install the **Podroid APK** (https://github.com/ExTV/Podroid/releases/latest), open it, and in the
   setup wizard **enable SSH** (needed to copy the image in). In Settings set **~4 vCPUs** and
   **6–8 GB RAM** (you have 24 GB; more vCPUs is slower under TCG). Tap **Start VM**, wait for **Ready!**.
2. Copy the image into the VM **over the network** — the QEMU backend has no Downloads-folder share
   (that is AVF-only), so a file manager won't reach the VM. In the Podroid terminal run
   `podroid-phone-ip` for the phone's LAN IP, then from your Mac on the same Wi-Fi (password `podroid`):
   ```bash
   scp -P 9922 ~/dev-workspace-kali-kasmvnc-linux-arm64.tar.gz root@<phone-ip>:/root/
   # Optional locally built headless package: scp -P 9922 ~/sdw-image.tar.gz root@<phone-ip>:/root/
   ```
   (Also copy `docker-compose.yml` + `.env` if you prefer `docker compose up -d` over `docker run`.)
3. In the Podroid terminal (you are root in the VM):
   ```bash
   docker load < dev-workspace-kali-kasmvnc-linux-arm64.tar.gz
   # simplest: one container
   docker run -d --name dev-workspace --privileged --device /dev/fuse \
     -e PASSWORD=<your-password> -e CLASH_SUBSCRIPTION_URL=<your-sub> \
     -e DESKTOP_RESOLUTION=1280x720 \
     -p 8080:8080 dev-workspace-kali-kasmvnc:latest
   # or, with the compose file + .env present:  docker compose up -d
   ```
   For the optional locally built headless package, load `sdw-image.tar.gz`, use
   `dev-workspace:kali-headless`, and omit `DESKTOP_RESOLUTION`.
4. Expose the UI to the Android browser:
   ```bash
   podroid-forward add 8080 8080 tcp
   ```
   Open `http://localhost:8080/` in the phone browser. The unlock page has a single password field
   (no username): enter `PASSWORD`.

## Run the full desktop GUI (not just code-server)

code-server is already a browser IDE and covers most GUI work at the lightest load. For the actual
XFCE desktop (GUI pentest tools, a real window manager), use the verified Release image
`dev-workspace-kali-kasmvnc:latest` from `dev-workspace-kali-kasmvnc-linux-arm64.tar.gz`. The optional
`dev-workspace:kali-headless` local image is code-server only.

Reality check for the 9 Pro+ (no root): the desktop streams out of a software-emulated VM. Expect
**usable-but-not-smooth** — fine for editing, terminals and light browsing; sluggish for video,
animations or dragging big windows. 24 GB RAM does not fix this; the ceiling is single-core
translation speed. Whether it is good enough can only be judged on the device.

Keep the load down:

- Use **KasmVNC** (the default `DESKTOP_STACK=kasmvnc`, CPU tile encoding). Do **not** use Selkies —
  its H.264 software encode is much heavier under emulation.
- Lower `DESKTOP_RESOLUTION` from `1920x1080` to **`1280x720`** (roughly halves compositing + encode).
- Give the Podroid VM about **4 vCPUs** and 4–8 GB RAM in Settings. More vCPUs is *not* better here:
  Podroid's own tuning notes measured 8 vCPUs slower than 4 under TCG.

```bash
docker load < dev-workspace-kali-kasmvnc-linux-arm64.tar.gz
docker run -d --name dev-workspace --privileged --device /dev/fuse \
  -e PASSWORD=<your-password> -e CLASH_SUBSCRIPTION_URL=<your-sub> \
  -e DESKTOP_RESOLUTION=1280x720 \
  -p 8080:8080 dev-workspace-kali-kasmvnc:latest
podroid-forward add 8080 8080 tcp
```

Open `http://localhost:8080/` in the phone browser and unlock with `PASSWORD` (password only, no
username), then go to **`/vnc/`** for the KasmVNC desktop. code-server stays available at the same gate.

## Notes

- The egress firewall auto-detects: it uses nftables if the VM kernel's nft works, otherwise falls
  back to iptables-legacy (both are in the image; see `infra/src/egress.rs`). Podroid's kernel has
  netfilter, so one of them will load — or the boot fails closed, never leaky.
- Inside the VM you're root, so `--privileged --device /dev/fuse` is allowed; that is what lets the
  vault (FUSE) and the firewall (NET_ADMIN) work. This is the VM's root, not Android root.
- Podroid also has an AVF backend; if you later use a Pixel-class phone it runs hardware-accelerated
  after a one-time `pm grant`, much faster — same steps otherwise.
