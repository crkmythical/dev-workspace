# BASE_IMAGE is a global ARG: it must be declared before the first FROM so the final
# `FROM ${BASE_IMAGE}` (after the rust-builder stage) can still see it.
ARG BASE_IMAGE=kalilinux/kali-rolling

# --- Rust build stage: the sync-service daemon --------------------------------------------------
# Compiled to a static musl binary so it drops into any base (glibc or not) with no runtime deps.
# Pure-Rust crypto (no OpenSSL/ring), so musl-dev is all the toolchain it needs. --locked pins the
# committed Cargo.lock. rust:1-alpine tracks the latest stable 1.x; CI can pin it tighter.
FROM rust:1-alpine AS rust-builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY rust/ ./rust/
RUN cd rust && cargo build --release --locked && cp target/release/sync-service /sync-service

# Base image toggle.
#   Default: kalilinux/kali-rolling — the production base, full pentest toolchain
#   via apt. Kali tracks Debian testing, so it satisfies the Selkies desktop's
#   libva >=2.20 requirement (pixelflux's prebuilt screen_capture_module.so links
#   a libavutil needing symbol vaMapBuffer2; bookworm's libva 2.17 is too old and
#   the .so fails to load → "Waiting for stream...").
#   Alternative: build with `--build-arg BASE_IMAGE=debian:trixie-slim` for a
#   leaner non-pentest base (also ships libva 2.22, matches the LSIO debiantrixie
#   reference). Do NOT use bookworm without vendoring a newer libva.
FROM ${BASE_IMAGE}

# Desktop streaming stack selector (build-time):
#   kasmvnc (default) — KasmVNC only (/vnc/): an ~8 MB .deb, builds in seconds
#   selkies           — Selkies only (/desktop/): H.264/WebCodecs, ~420 MB venv, ~12 min layer
#   both              — Selkies + KasmVNC in parallel (published as the `-full` image)
# Recorded in /etc/sdw-desktop-stack; read by desktop.ts SSOT at runtime.
ARG DESKTOP_STACK=kasmvnc

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=en_US.UTF-8
ENV TZ=Asia/Shanghai

# Apt sources. Kali keeps its official repos (deb822 /etc/apt/sources.list.d/
# kali.sources — do NOT delete it); KALI_MIRROR only swaps the host inside it. The default,
# the official redirector, picks a mirror by location and can pick a very slow one (~14 KB/s
# was seen): build with e.g. --build-arg KALI_MIRROR=mirrors.tuna.tsinghua.edu.cn there.
# Debian bases switch to APT_MIRROR: Aliyun by default (fast inside mainland China, very
# slow elsewhere: build with --build-arg APT_MIRROR=deb.debian.org there).
ARG APT_MIRROR=mirrors.aliyun.com
ARG KALI_MIRROR=http.kali.org
RUN . /etc/os-release && \
    if [ "$ID" = "kali" ]; then \
      sed -i "s#http://http.kali.org/kali#http://${KALI_MIRROR}/kali#" /etc/apt/sources.list.d/kali.sources; \
    else \
      rm -f /etc/apt/sources.list.d/*; \
      CN="${VERSION_CODENAME:-trixie}"; \
      { echo "deb http://${APT_MIRROR}/debian ${CN} main contrib non-free non-free-firmware"; \
        echo "deb http://${APT_MIRROR}/debian ${CN}-updates main contrib non-free non-free-firmware"; \
        echo "deb http://${APT_MIRROR}/debian-security ${CN}-security main contrib non-free non-free-firmware"; \
      } > /etc/apt/sources.list; \
    fi

# Core layer: system packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    tini \
    supervisor \
    gocryptfs \
    fuse3 \
    procps \
    psmisc \
    socat \
    netcat-openbsd \
    unzip \
    git \
    git-lfs \
    openssh-client \
    ca-certificates \
    cron \
    curl \
    bzip2 \
    jq \
    tmux \
    bash \
    locales \
    debootstrap \
    vim \
    firefox-esr \
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen \
    && locale-gen \
    && rm -rf /var/lib/apt/lists/*

# Install Bun — pinned version + sha256 verification against the published
# SHASUMS256.txt (the GitHub release zip is the same artifact bun.sh/install
# fetches). Reproducible + tamper-evident; bump BUN_VERSION and both hashes
# together (from .../releases/download/bun-v<ver>/SHASUMS256.txt). The zip holds only
# `bun`; `bunx` is the symlink bun.sh/install would have made (the SPA build uses it).
ARG BUN_VERSION=1.4.3
ARG BUN_SHA256_AMD64=3e730528d89775f03f87f05c06787c4df4099d0a37f3f252e7b829cb23816e18
ARG BUN_SHA256_ARM64=efa9813da5ed72423bf847f916e8d2c47c0d776add972354026a75e10da9aa21
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) bun_arch=x64;     bun_sha="$BUN_SHA256_AMD64" ;; \
      arm64) bun_arch=aarch64; bun_sha="$BUN_SHA256_ARM64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/bun-linux-${bun_arch}.zip" -o /tmp/bun.zip; \
    echo "${bun_sha}  /tmp/bun.zip" | sha256sum -c -; \
    unzip -q /tmp/bun.zip -d /tmp/bun; \
    mkdir -p /root/.bun/bin; \
    install -m 0755 "/tmp/bun/bun-linux-${bun_arch}/bun" /root/.bun/bin/bun; \
    ln -sf bun /root/.bun/bin/bunx; \
    rm -rf /tmp/bun /tmp/bun.zip
ENV PATH="/root/.bun/bin:${PATH}"
RUN bun --version

# Desktop layer — COMMON deps (both stacks): X server, XFCE, fonts, mesa, dbus.
# XFCE is the session for BOTH stacks (selkies and kasmvnc). This eliminates the
# openbox+tint2 dependency (tint2 is not in Kali repos) and gives a consistent
# desktop experience regardless of the streaming backend.
RUN apt-get update && apt-get install -y --no-install-recommends \
    xvfb \
    xfce4 xfce4-terminal xfce4-whiskermenu-plugin thunar \
    dbus-x11 \
    libgl1-mesa-dri \
    mesa-utils \
    libglib2.0-0 \
    librsvg2-common \
    at-spi2-core \
    fonts-noto-cjk \
    fonts-dejavu-core \
    xdg-utils \
    x11-utils \
    x11-xserver-utils \
    && rm -rf /var/lib/apt/lists/*

# --- Selkies desktop streaming (H.264/WebCodecs) — installed for both|selkies ---
# Install from pinned GitHub ref. pixelflux pins nvidia-cuda-nvrtc with a stale
# hash, so install with --no-deps and list the full dependency tree.
ARG SELKIES_GIT_REF=0d134b6e1ffe42a579bc66363b0e7159ab22aacc
# pixelflux / pcmflux are Selkies' native capture libraries and must match the pinned ref:
# that commit imports StripeCallback (pixelflux 1.x only, gone in 2.0) and AudioChunkCallback
# (pcmflux 1.0.x only, gone in 2.0). Unpinned, pip took 2.1.0 of both and Selkies died on
# start-up with an ImportError, in an image that built fine. Bump all three together.
ARG PIXELFLUX_VERSION=1.6.4
ARG PCMFLUX_VERSION=1.0.8
ARG DESKTOP_STACK
RUN if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "selkies" ]; then \
      set -e; \
      apt-get update && apt-get install -y --no-install-recommends \
        libpulse0 pulseaudio xclip xdotool xcvt \
        libva2 libva-drm2 libva-x11-2 libgbm1 libdrm2 libjpeg62-turbo \
        python3-dev python3-venv gcc pkg-config libxkbcommon-dev git libsrtp2-dev \
      && python3 -m venv /opt/selkies-venv \
      && /opt/selkies-venv/bin/pip install --no-cache-dir setuptools wheel \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --no-deps \
         "git+https://github.com/selkies-project/selkies.git@${SELKIES_GIT_REF}" \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --no-deps \
         "pixelflux==${PIXELFLUX_VERSION}" "pcmflux==${PCMFLUX_VERSION}" \
      && /opt/selkies-venv/bin/pip install --no-cache-dir \
         nvidia-cuda-nvrtc numpy \
         "python-xlib @ https://github.com/selkies-project/python-xlib/archive/master.zip" \
         aiofiles aiohttp aioice av cffi cryptography distro google-crc32c \
         msgpack pasimple Pillow prometheus_client psutil pulsectl \
         pyee pylibsrtp pyopenssl pynput watchdog websockets xkbcommon \
         GPUtil typing_extensions \
      && /opt/selkies-venv/bin/python -c "from selkies.__main__ import main" \
      && /opt/selkies-venv/bin/python -c "import pixelflux, pcmflux" \
      && /opt/selkies-venv/bin/python -c "import selkies.selkies" \
      && apt-get purge -y python3-dev gcc pkg-config libxkbcommon-dev libsrtp2-dev \
      && apt-get autoremove -y \
      && rm -rf /var/lib/apt/lists/*; \
    else \
      echo "Skipping selkies install (DESKTOP_STACK=$DESKTOP_STACK)"; \
    fi

ENV SELKIES_VENV=/opt/selkies-venv
ENV PATH="/opt/selkies-venv/bin:${PATH}"

# --- KasmVNC desktop streaming (integrated X + WebSocket VNC) — both|kasmvnc ---
# Native trixie .deb (KasmVNC 1.4.0 has a trixie build; Kali is trixie-based and
# installs it fine). Auth handled by Caddy (we run with -DisableBasicAuth), so no
# kasmvncpasswd / /root/.kasmpasswd needed. Perl deps are KasmVNC's runtime reqs.
ARG KASMVNC_VERSION=1.4.0
RUN if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "kasmvnc" ]; then \
      set -e; \
      KASM_DISTRO="trixie"; \
      ARCH=$(dpkg --print-architecture); \
      apt-get update; \
      curl -fsSL "https://github.com/kasmtech/KasmVNC/releases/download/v${KASMVNC_VERSION}/kasmvncserver_${KASM_DISTRO}_${KASMVNC_VERSION}_${ARCH}.deb" \
        -o /tmp/kasmvnc.deb; \
      apt-get install -y --no-install-recommends /tmp/kasmvnc.deb \
        libdatetime-perl libhash-merge-simple-perl libscalar-list-utils-perl \
        libyaml-tiny-perl libtry-tiny-perl libswitch-perl; \
      rm -f /tmp/kasmvnc.deb && rm -rf /var/lib/apt/lists/*; \
    else \
      echo "Skipping kasmvnc install (DESKTOP_STACK=$DESKTOP_STACK)"; \
    fi

# Install Caddy — pinned version + sha512 verification (caddy's checksums.txt
# uses SHA-512). The standard GitHub release build ships the same default
# modules the download API returns (reverse_proxy, file_server, basic_auth).
ARG CADDY_VERSION=2.11.7
ARG CADDY_SHA512_AMD64=a7a433a1b133efc3c8d10eb0b99d52a24b5ef5c322dc77f5282182b1c0402139ab83f3a99f0c52409df77d20123fb0b523edad8a66d8f5e49136197bf61ef0e7
ARG CADDY_SHA512_ARM64=3db36ba90c7a6e8dda40ee3dd71fa08844c76b5fb08f61b31e5e78d2ed38e71c51dc7baed875e50d1ca1279196e84302967237386ae87c91ae9f2aaceada682e
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) caddy_sha="$CADDY_SHA512_AMD64" ;; \
      arm64) caddy_sha="$CADDY_SHA512_ARM64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/caddyserver/caddy/releases/download/v${CADDY_VERSION}/caddy_${CADDY_VERSION}_linux_${arch}.tar.gz" -o /tmp/caddy.tar.gz; \
    echo "${caddy_sha}  /tmp/caddy.tar.gz" | sha512sum -c -; \
    tar -xzf /tmp/caddy.tar.gz -C /tmp caddy; \
    install -m 0755 /tmp/caddy /usr/bin/caddy; \
    rm -f /tmp/caddy.tar.gz /tmp/caddy; \
    caddy version

# Install restic (realtime backup to S3) — single static binary, no runtime deps.
ARG RESTIC_VERSION=0.17.3
RUN curl -fsSL "https://github.com/restic/restic/releases/download/v${RESTIC_VERSION}/restic_${RESTIC_VERSION}_linux_$(dpkg --print-architecture).bz2" \
    | bunzip2 > /usr/local/bin/restic \
    && chmod +x /usr/local/bin/restic

# Install croc (peer-to-peer, end-to-end encrypted file transfer for `push`/`pull`) — pinned +
# sha256 against the release checksums.txt. Single static Go binary, like restic.
ARG CROC_VERSION=11.5.5
ARG CROC_SHA256_AMD64=2dee9b7ede849971532c597ef3fd29507bfc970330758cfed13657be04f34243
ARG CROC_SHA256_ARM64=c8606c6b4450e4a50b3ef25bd486ed4f9fa4a1ac6e2ecefb2bcbdbbfce1a51a9
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) croc_arch="64bit"; croc_sha="$CROC_SHA256_AMD64" ;; \
      arm64) croc_arch="ARM64"; croc_sha="$CROC_SHA256_ARM64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/schollz/croc/releases/download/v${CROC_VERSION}/croc_v${CROC_VERSION}_Linux-${croc_arch}.tar.gz" -o /tmp/croc.tar.gz; \
    echo "${croc_sha}  /tmp/croc.tar.gz" | sha256sum -c -; \
    tar -xzf /tmp/croc.tar.gz -C /usr/local/bin croc; \
    rm -f /tmp/croc.tar.gz; \
    croc --version

# Install mise (runtime version manager) — pinned via MISE_VERSION; retry on
# network failure. The installer verifies its own release checksum.
ARG MISE_VERSION=v2026.10.12
RUN for i in 1 2 3; do curl -fsSL https://mise.jdx.dev/install.sh | MISE_VERSION="${MISE_VERSION}" bash && break || sleep 5; done
ENV PATH="/root/.local/bin:${PATH}"
ENV MISE_DATA_DIR=/root/.local/share/mise

# Pre-install tool versions via mise (split layers for cache efficiency)
COPY image/.mise.toml /root/.mise.toml
RUN mise trust /root/.mise.toml

# Java layer — online download via mise
RUN mise install java@zulu-17 && mise use --global java@zulu-17 && mise reshim

# Node + Python + pnpm — online download via mise
RUN mise install node@20 python@3.12 pnpm@latest && mise use --global node@20 python@3.12 pnpm@latest && mise reshim

ENV PATH="/root/.local/share/mise/shims:/root/.local/bin:${PATH}"
ENV JAVA_HOME="/root/.local/share/mise/installs/java/zulu-17"

# Install Clash (mihomo) — pinned version
ARG CLASH_VERSION=1.18.0
RUN ARCH=$(dpkg --print-architecture) && \
    curl -fsSL "https://github.com/MetaCubeX/mihomo/releases/download/v${CLASH_VERSION}/mihomo-linux-${ARCH}-v${CLASH_VERSION}.gz" \
    | gunzip > /usr/bin/clash && chmod +x /usr/bin/clash

# Install code-server — pinned. The official installer fetches the matching
# GitHub release; --version makes rebuilds deterministic.
ARG CODE_SERVER_VERSION=4.141.0
RUN curl -fsSL "https://code-server.dev/install.sh" | sh -s -- --version "${CODE_SERVER_VERSION}"

# Filesystem prep
RUN mkdir -p /workspace /vault/cipher /etc/supervisor/conf.d /var/run/vault-sync \
    /etc/caddy /etc/clash /opt/workspace \
    /pentest/cipher /pentest/rootfs \
    /opt/desktop-apps /tmp/.desktop-cache

# Desktop apps registry
RUN echo '[]' > /opt/desktop-apps/.registry.json

# Ensure proxy env is available in all shells (interactive terminals via code-server)
RUN echo 'export http_proxy=http://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export https_proxy=http://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export all_proxy=socks5://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTP_PROXY=$http_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTPS_PROXY=$https_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export ALL_PROXY=$all_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export NO_PROXY=$no_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export PATH="/root/.local/bin:/root/.local/share/mise/shims:/root/.bun/bin:$PATH"' >> /etc/profile.d/proxy.sh && \
    echo 'eval "$(mise activate bash)"' >> /etc/profile.d/proxy.sh && \
    echo '# noproxy: run a command bypassing Clash proxy (direct network)' >> /etc/profile.d/proxy.sh && \
    echo 'noproxy() { env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY "$@"; }' >> /etc/profile.d/proxy.sh && \
    chmod +x /etc/profile.d/proxy.sh && \
    echo 'source /etc/profile.d/proxy.sh' >> /root/.bashrc

# Copy baked configs
COPY image/supervisord.conf /etc/supervisor/supervisord.conf
COPY image/etc/clash/config.yaml /etc/clash/config.yaml
COPY image/etc/clash/config.yaml.template /etc/clash/config.yaml.template
COPY image/etc/clash/rules.yaml /etc/clash/rules.yaml
COPY image/etc/caddy/Caddyfile /etc/caddy/Caddyfile
COPY image/etc/README-UNLOCK.md /workspace/README-UNLOCK.md
COPY image/etc/cron.d/vault-sync /etc/cron.d/vault-sync
COPY image/etc/cron.d/cleanup /etc/cron.d/cleanup
RUN chmod 0644 /etc/cron.d/vault-sync /etc/cron.d/cleanup

# Desktop config snippets — staged, then provisioned per DESKTOP_STACK below.
# supervisor: per-stack group conf → /etc/supervisor/conf.d/{desktop,vnc}.conf
# caddy:      per-stack routing    → /etc/caddy/desktop.d/*.caddy (glob-imported)
COPY image/etc/supervisor/desktop-selkies.conf /opt/desktop-conf/supervisor-selkies.conf
COPY image/etc/supervisor/desktop-vnc.conf /opt/desktop-conf/supervisor-vnc.conf
COPY image/etc/caddy/desktop-selkies.caddy /opt/desktop-conf/caddy-selkies.caddy
COPY image/etc/caddy/desktop-vnc.caddy /opt/desktop-conf/caddy-vnc.caddy

# Provision desktop topology + record the stack marker. "both" installs both
# supervisor groups and both Caddy snippets; single-stack installs only one.
ARG DESKTOP_STACK
RUN mkdir -p /etc/supervisor/conf.d /etc/caddy/desktop.d && \
    echo "${DESKTOP_STACK}" > /etc/sdw-desktop-stack && \
    if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "selkies" ]; then \
      cp /opt/desktop-conf/supervisor-selkies.conf /etc/supervisor/conf.d/desktop.conf; \
      cp /opt/desktop-conf/caddy-selkies.caddy /etc/caddy/desktop.d/desktop-selkies.caddy; \
    fi && \
    if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "kasmvnc" ]; then \
      cp /opt/desktop-conf/supervisor-vnc.conf /etc/supervisor/conf.d/vnc.conf; \
      cp /opt/desktop-conf/caddy-vnc.caddy /etc/caddy/desktop.d/desktop-vnc.caddy; \
    fi

# Backup watcher supervisor config (always installed; feature is opt-in via env)
COPY image/etc/supervisor/backup-watcher.conf /etc/supervisor/conf.d/backup-watcher.conf

# Desktop default configs
COPY image/etc/desktop/ /opt/workspace/image/etc/desktop/

# Selkies static web client (served by Caddy)
COPY image/selkies-web/ /usr/share/selkies/web/

# XFCE launcher (waits for X server before starting the session; both stacks)
RUN cp /opt/workspace/image/etc/desktop/start-xfce.sh /usr/local/bin/start-xfce.sh && \
    cp /opt/workspace/image/etc/desktop/start-audio.sh /usr/local/bin/start-audio.sh && \
    chmod +x /usr/local/bin/start-xfce.sh /usr/local/bin/start-audio.sh

# No per-session ssh-agent / gpg-agent in the desktops (system-wide xfce4-session default, below
# any per-user setting). xfce4-session starts an ssh-agent that detaches into a session of its own
# and listens on a socket in the session's HOME, which is inside the vault. A listening unix socket
# pins the mount without being an open file (fuser does not see it), so the agent outlived the
# "stopped" desktop, one more per start, and lock-vault could never unmount cleanly (measured: 3
# leftover agents, the unmount refused for 20 s with no process holding a file). The property goes
# INTO the package's own file: it also holds the Failsafe session (window manager, panel, desktop).
RUN f=/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-session.xml && \
    sed -i 's#</channel>#  <property name="startup" type="empty">\n    <property name="ssh-agent" type="empty">\n      <property name="enabled" type="bool" value="false"/>\n    </property>\n    <property name="gpg-agent" type="empty">\n      <property name="enabled" type="bool" value="false"/>\n    </property>\n  </property>\n</channel>#' "$f" && \
    grep -q 'name="ssh-agent"' "$f" && grep -q 'FailsafeSessionName' "$f"

# Copy monorepo packages (workspace code)
COPY packages/ /opt/workspace/packages/
COPY package.json bunfig.toml bun.lock /opt/workspace/
WORKDIR /opt/workspace
RUN bun install --frozen-lockfile 2>/dev/null || bun install

# Build SPA
RUN cd packages/spa && bunx vite build

# Put a launcher for every command in packages/cli/package.json's `bin` map on PATH.
# That map is the only list of commands (add one there, never here).
RUN bun run /opt/workspace/packages/cli/scripts/install-bins.ts /usr/local/bin

# The sync-service daemon (Rust). Replaces `bun run packages/server` — see supervisord.conf and
# docs/rust-migration.md. It serves /sync/* (the SPA from packages/spa/dist, built above, plus the
# upload/download/doctor/destruct API) and reads SYNC_ENABLED / REMOTE_DESTRUCT_ENABLED from env.
COPY --from=rust-builder /sync-service /usr/local/bin/sync-service

# Copy entrypoint (minimal shell → exec bun)
COPY scripts/entrypoint.sh /usr/local/lib/workspace/entrypoint.sh
RUN chmod +x /usr/local/lib/workspace/entrypoint.sh

# Environment defaults (proxy points to Clash)
ENV http_proxy=http://127.0.0.1:7890 \
    https_proxy=http://127.0.0.1:7890 \
    all_proxy=socks5://127.0.0.1:7890 \
    no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16

# Liveness: Caddy ingress (:8080) + code-server (:8082) both accepting TCP.
# Assumption-free port probe (nc from netcat-openbsd). The generous
# start-period covers Clash bootstrap + vault unlock before supervisord brings
# the services up. For an app-level signal, swap to:
#   CMD curl -fsS http://127.0.0.1:8080/ -o /dev/null || exit 1
HEALTHCHECK --interval=30s --timeout=10s --start-period=180s --retries=3 \
  CMD nc -z 127.0.0.1 8080 && nc -z 127.0.0.1 8082 || exit 1

ENTRYPOINT ["tini", "--", "/usr/local/lib/workspace/entrypoint.sh"]
