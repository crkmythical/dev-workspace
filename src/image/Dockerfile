# BASE_IMAGE is a global ARG: it must be declared before the first FROM so the final
# `FROM ${BASE_IMAGE}` (after the rust-builder stage) can still see it.
ARG BASE_IMAGE=kalilinux/kali-rolling
# Also global: the selkies-web stage selector below resolves `FROM selkies-web-${DESKTOP_STACK}`.
# The final stage re-declares it for its RUN blocks.
ARG DESKTOP_STACK=kasmvnc

# --- Rust build stage: the sdw binary (CLI + entrypoint + daemons + sync API) -------------------
# Compiled to a static musl binary so it drops into any base (glibc or not) with no runtime deps.
# Pure-Rust crypto (no OpenSSL/ring), so musl-dev is all the toolchain it needs. --locked pins the
# committed Cargo.lock. rust:1-alpine tracks the latest stable 1.x; CI can pin it tighter.
# Digest-pinned for reproducible builds (bump the digest to move the toolchain). Tag: rust:1-alpine.
FROM rust:1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS rust-builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY rust/ ./rust/
RUN cd rust && cargo build --release --locked \
    && cp target/release/sdw /sdw

# --- SPA build stage: the browser sync page (stays TypeScript) -----------------------------------
# Built with Bun here, at BUILD TIME ONLY, so the runtime image carries no Bun. The compiled dist
# is copied into the final image and served by the Rust sync-service.
# Digest-pinned for reproducible builds. Tag: oven/bun:1.4.3.
FROM oven/bun:1.4.3@sha256:ec06c3b6cea04192ae6770c434f668ca41d343ad19fa6472216c7b48be39c598 AS spa-builder
WORKDIR /opt/workspace
COPY package.json bunfig.toml bun.lock ./
COPY packages/ ./packages/
# Strict: the committed bun.lock governs what lands in the SPA bundle. The old
# `|| bun install` fallback (with 2>/dev/null) silently re-resolved dependencies fresh on any
# lockfile drift — defeating the lockfile. Fail loudly instead; CI pins the lockfile too.
RUN bun install --frozen-lockfile
RUN cd packages/spa && bunx vite build

# --- Selkies web client (conditional) -----------------------------------------------------------
# The static H.264 web client (~6 MB) is only served by the Selkies stack. Select a source stage by
# DESKTOP_STACK so a KasmVNC-only image does not carry it at all (a copy-then-delete would still
# bake it into a layer): `present` holds the files, the kasmvnc stage is an empty /web, and the
# final stage COPYs /web from whichever one DESKTOP_STACK selects.
FROM ${BASE_IMAGE} AS selkies-web-present
RUN mkdir -p /web
COPY image/selkies-web/ /web/
FROM ${BASE_IMAGE} AS selkies-web-kasmvnc
RUN mkdir -p /web
FROM selkies-web-present AS selkies-web-both
FROM selkies-web-present AS selkies-web-selkies
FROM selkies-web-${DESKTOP_STACK} AS selkies-web

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
    nftables \
    libcap2-bin \
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen \
    && locale-gen \
    && rm -rf /var/lib/apt/lists/*

# Bun is gone from the runtime image: the CLI, entrypoint and daemons are the Rust `sdw` binary
# (installed below) and the SPA is pre-built in the spa-builder stage. Only the pinned third-party
# binaries and the single `sdw` binary run here.

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
# sha256 cross-checked against restic's official SHA256SUMS for v0.17.3 (per-arch). Download to a
# temp file, verify, THEN decompress — a pipe straight to an executable cannot be checked.
ARG RESTIC_SHA256_AMD64=5097faeda6aa13167aae6e36efdba636637f8741fed89bbf015678334632d4d3
ARG RESTIC_SHA256_ARM64=db27b803534d301cef30577468cf61cb2e242165b8cd6d8cd6efd7001be2e557
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) restic_sha="$RESTIC_SHA256_AMD64" ;; \
      arm64) restic_sha="$RESTIC_SHA256_ARM64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/restic/restic/releases/download/v${RESTIC_VERSION}/restic_${RESTIC_VERSION}_linux_${arch}.bz2" -o /tmp/restic.bz2; \
    echo "${restic_sha}  /tmp/restic.bz2" | sha256sum -c -; \
    bunzip2 < /tmp/restic.bz2 > /usr/local/bin/restic; \
    rm -f /tmp/restic.bz2; \
    chmod +x /usr/local/bin/restic

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
# mihomo ships no separate checksum file, so these pin the exact published asset bytes (sha256 of
# the release .gz, recorded here): a later asset swap or a corrupted download no longer silently
# becomes /usr/bin/clash. Download, verify, THEN decompress.
ARG CLASH_SHA256_AMD64=f1c8638d57e0a80705854286ec9d0a26681fc18441d7d708be6b9349029554ac
ARG CLASH_SHA256_ARM64=980e2d74ae3724e6ce88f38ca3885bb756aee41359ee38f89799fa6fe6962e5e
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
      amd64) clash_sha="$CLASH_SHA256_AMD64" ;; \
      arm64) clash_sha="$CLASH_SHA256_ARM64" ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL "https://github.com/MetaCubeX/mihomo/releases/download/v${CLASH_VERSION}/mihomo-linux-${arch}-v${CLASH_VERSION}.gz" -o /tmp/clash.gz; \
    echo "${clash_sha}  /tmp/clash.gz" | sha256sum -c -; \
    gunzip < /tmp/clash.gz > /usr/bin/clash; \
    rm -f /tmp/clash.gz; \
    chmod +x /usr/bin/clash

# Fail-closed egress (opt-in via EGRESS_ENFORCE): under enforcement Clash runs as its OWN uid so
# the nftables OUTPUT filter can let only its traffic out. It still binds the DNS port 53, which a
# non-root process cannot do without a capability, so grant the binary cap_net_bind_service.
# nftables + libcap2-bin are installed in the core apt layer; the filter is applied at boot by
# `sdw entrypoint` (the enforce-egress step). When EGRESS_ENFORCE is off, Clash runs as root as
# before and none of this is engaged.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin clash \
    && setcap 'cap_net_bind_service=+ep' /usr/bin/clash

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
    echo 'export all_proxy=socks5h://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTP_PROXY=$http_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTPS_PROXY=$https_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export ALL_PROXY=$all_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export NO_PROXY=$no_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export PATH="/root/.local/bin:/root/.local/share/mise/shims:$PATH"' >> /etc/profile.d/proxy.sh && \
    echo 'eval "$(mise activate bash)"' >> /etc/profile.d/proxy.sh && \
    echo '# noproxy: run a command bypassing Clash proxy (direct network)' >> /etc/profile.d/proxy.sh && \
    echo 'noproxy() { env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY "$@"; }' >> /etc/profile.d/proxy.sh && \
    chmod +x /etc/profile.d/proxy.sh && \
    echo 'source /etc/profile.d/proxy.sh' >> /root/.bashrc

# Copy baked configs
COPY image/supervisord.conf /etc/supervisor/supervisord.conf
COPY --chmod=0755 image/clash-watcher.sh /usr/local/bin/clash-watcher
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

# Selkies static web client (served by Caddy). Empty in a KasmVNC-only image — the selkies-web
# stage selector above drops it there, and only the Selkies Caddy snippet serves this path.
COPY --from=selkies-web /web /usr/share/selkies/web

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

# The Rust command surface + entrypoint + daemons: one multi-call binary. `sdw install` symlinks
# every bin command (BIN_COMMANDS in rust/cli/src/dispatch.rs) to it (argv[0] dispatch).
COPY --from=rust-builder /sdw /usr/local/bin/sdw
RUN /usr/local/bin/sdw install /usr/local/bin

# The sync API daemon is part of the sdw binary now: supervisord runs `sdw sync-service`. It serves
# /sync/api/* and reads SYNC_ENABLED / REMOTE_DESTRUCT_ENABLED from env.

# The compiled SPA (built by the Bun spa-builder stage; the browser SPA stays TypeScript).
COPY --from=spa-builder /opt/workspace/packages/spa/dist /opt/workspace/packages/spa/dist

# No shell entrypoint: tini execs `sdw entrypoint` directly (it does the inert-mode check, the
# required-env check and the whole boot). See ENTRYPOINT below.

# Environment defaults (proxy points to Clash)
# all_proxy uses socks5h (not socks5): the 'h' makes SOCKS-honoring tools send the hostname to
# Clash to resolve, instead of resolving locally against the container resolver (1.1.1.1/8.8.8.8)
# and leaking the DNS query. http(s)_proxy already pass the hostname via CONNECT.
ENV http_proxy=http://127.0.0.1:7890 \
    https_proxy=http://127.0.0.1:7890 \
    all_proxy=socks5h://127.0.0.1:7890 \
    no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16

# Liveness: Caddy ingress (:8080) + code-server (:8082) both accepting TCP.
# Assumption-free port probe (nc from netcat-openbsd). The generous
# start-period covers Clash bootstrap + vault unlock before supervisord brings
# the services up. For an app-level signal, swap to:
#   CMD curl -fsS http://127.0.0.1:8080/ -o /dev/null || exit 1
HEALTHCHECK --interval=30s --timeout=10s --start-period=180s --retries=3 \
  CMD nc -z 127.0.0.1 8080 && nc -z 127.0.0.1 8082 || exit 1

ENTRYPOINT ["tini", "--", "/usr/local/bin/sdw", "entrypoint"]
