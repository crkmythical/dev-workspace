# Base image toggle.
#   Default: kalilinux/kali-rolling — the production base, full pentest toolchain
#   via apt. Kali tracks Debian testing, so it satisfies the Selkies desktop's
#   libva >=2.20 requirement (pixelflux's prebuilt screen_capture_module.so links
#   a libavutil needing symbol vaMapBuffer2; bookworm's libva 2.17 is too old and
#   the .so fails to load → "Waiting for stream...").
#   Alternative: build with `--build-arg BASE_IMAGE=debian:trixie-slim` for a
#   leaner non-pentest base (also ships libva 2.22, matches the LSIO debiantrixie
#   reference). Do NOT use bookworm without vendoring a newer libva.
ARG BASE_IMAGE=kalilinux/kali-rolling
FROM ${BASE_IMAGE}

# Desktop streaming stack selector (build-time):
#   both    (default) — install AND run selkies (/desktop/) + kasmvnc (/vnc/) in parallel
#   selkies           — selkies only (fallback)
#   kasmvnc           — kasmvnc only (fallback)
# Recorded in /etc/sdw-desktop-stack; read by desktop.ts SSOT at runtime.
ARG DESKTOP_STACK=both

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=en_US.UTF-8
ENV TZ=Asia/Shanghai

# Apt sources: Kali keeps its official repos (deb822 /etc/apt/sources.list.d/
# kali.sources — do NOT delete it). Debian switches to Aliyun mirrors for speed.
RUN . /etc/os-release && \
    if [ "$ID" != "kali" ]; then \
      rm -f /etc/apt/sources.list.d/*; \
      CN="${VERSION_CODENAME:-trixie}"; \
      { echo "deb http://mirrors.aliyun.com/debian ${CN} main contrib non-free non-free-firmware"; \
        echo "deb http://mirrors.aliyun.com/debian ${CN}-updates main contrib non-free non-free-firmware"; \
        echo "deb http://mirrors.aliyun.com/debian-security ${CN}-security main contrib non-free non-free-firmware"; \
      } > /etc/apt/sources.list; \
    fi

# Core layer: system packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    tini \
    supervisor \
    gocryptfs \
    fuse3 \
    procps \
    socat \
    netcat-openbsd \
    unzip \
    git \
    git-lfs \
    openssh-client \
    ca-certificates \
    cron \
    curl \
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

# Install Bun
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"

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
ARG DESKTOP_STACK
RUN if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "selkies" ]; then \
      set -e; \
      apt-get update && apt-get install -y --no-install-recommends \
        libpulse0 pulseaudio xclip xdotool \
        libva2 libva-drm2 libva-x11-2 libgbm1 libdrm2 libjpeg62-turbo \
        python3-dev python3-venv gcc pkg-config libxkbcommon-dev git libsrtp2-dev \
      && python3 -m venv /opt/selkies-venv \
      && /opt/selkies-venv/bin/pip install --no-cache-dir setuptools wheel \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --no-deps \
         "git+https://github.com/selkies-project/selkies.git@${SELKIES_GIT_REF}" \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --no-deps pixelflux pcmflux \
      && /opt/selkies-venv/bin/pip install --no-cache-dir \
         nvidia-cuda-nvrtc numpy \
         "python-xlib @ https://github.com/selkies-project/python-xlib/archive/master.zip" \
         aiofiles aiohttp aioice av cffi cryptography distro google-crc32c \
         msgpack pasimple Pillow prometheus_client psutil pulsectl \
         pyee pylibsrtp pyopenssl pynput watchdog websockets xkbcommon \
         GPUtil typing_extensions \
      && /opt/selkies-venv/bin/python -c "from selkies.__main__ import main" \
      && /opt/selkies-venv/bin/python -c "import pixelflux, pcmflux" \
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

# Install Caddy
RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=$(dpkg --print-architecture)" -o /usr/bin/caddy \
    && chmod +x /usr/bin/caddy

# Install mise (runtime version manager) — retry on network failure
RUN for i in 1 2 3; do curl -fsSL https://mise.jdx.dev/install.sh | bash && break || sleep 5; done
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

# Install code-server (latest)
RUN curl -fsSL "https://code-server.dev/install.sh" | sh

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

# Desktop default configs
COPY image/etc/desktop/ /opt/workspace/image/etc/desktop/

# Selkies static web client (served by Caddy)
COPY image/selkies-web/ /usr/share/selkies/web/

# XFCE launcher (waits for X server before starting the session; both stacks)
RUN cp /opt/workspace/image/etc/desktop/start-xfce.sh /usr/local/bin/start-xfce.sh && \
    cp /opt/workspace/image/etc/desktop/start-audio.sh /usr/local/bin/start-audio.sh && \
    chmod +x /usr/local/bin/start-xfce.sh /usr/local/bin/start-audio.sh

# Copy monorepo packages (workspace code)
COPY packages/ /opt/workspace/packages/
COPY package.json bunfig.toml /opt/workspace/
WORKDIR /opt/workspace
RUN bun install --frozen-lockfile 2>/dev/null || bun install

# Build SPA
RUN cd packages/spa && bunx vite build

# Symlink CLI commands onto PATH
RUN for cmd in init-vault unlock-vault lock-vault vault-sync doctor setup; do \
      printf '#!/bin/bash\nexec bun run /opt/workspace/packages/cli/src/%s.ts "$@"\n' "$cmd" > /usr/local/bin/$cmd; \
      chmod +x /usr/local/bin/$cmd; \
    done && \
    for cmd in init-pentest unlock-pentest lock-pentest pentest-bootstrap self-destruct desktop-start desktop-stop desktop-install; do \
      printf '#!/bin/bash\nexec bun run /opt/workspace/packages/cli/src/%s.ts "$@"\n' "$cmd" > /usr/local/bin/$cmd; \
      chmod +x /usr/local/bin/$cmd; \
    done && \
    printf '#!/bin/bash\nexec bun run /opt/workspace/packages/cli/src/pentest-enter.ts "$@"\n' > /usr/local/bin/pentest && \
    chmod +x /usr/local/bin/pentest

# Copy entrypoint (minimal shell → exec bun)
COPY scripts/entrypoint.sh /usr/local/lib/workspace/entrypoint.sh
RUN chmod +x /usr/local/lib/workspace/entrypoint.sh

# Environment defaults (proxy points to Clash)
ENV http_proxy=http://127.0.0.1:7890 \
    https_proxy=http://127.0.0.1:7890 \
    all_proxy=socks5://127.0.0.1:7890 \
    no_proxy=localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16

ENTRYPOINT ["tini", "--", "/usr/local/lib/workspace/entrypoint.sh"]
