# BASE_IMAGE is a global ARG: it must be declared before the first FROM so the final
# `FROM ${BASE_IMAGE}` (after the rust-builder stage) can still see it.
ARG BASE_IMAGE=kalilinux/kali-rolling

# wasm-bindgen CLI must exactly match the crate selected in Cargo.lock. Build it in a cacheable
# stage whose only project input is the lockfile; no second hand-maintained version pin exists.
FROM rust:1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS wasm-bindgen-cli
RUN apk add --no-cache build-base
COPY rust/Cargo.lock /tmp/Cargo.lock
RUN VERSION="$(awk '$0 == "name = \"wasm-bindgen\"" { found=1; next } found && /^version = / { gsub(/[\" ]/, "", $3); print $3; exit }' /tmp/Cargo.lock)" \
    && test -n "$VERSION" \
    && cargo install wasm-bindgen-cli --version "$VERSION" --locked

# --- Rust build stage: Linux runtime + browser WebAssembly --------------------------------------
# One toolchain builds both deliverables:
#   * `sdw`, a static musl binary for the container runtime; and
#   * `sdw-web`, the browser sync UI compiled to wasm32-unknown-unknown.
# wasm-bindgen emits only the standards-required JavaScript loader (`--no-typescript`); all browser
# behavior and the sync protocol live in Rust. Versions are locked by Cargo.lock, and the CLI is
# pinned to the exact wasm-bindgen crate version so their wire formats cannot drift.
# Digest-pinned builder image (bump the digest deliberately).
FROM rust:1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS rust-builder
RUN apk add --no-cache musl-dev \
    && rustup target add wasm32-unknown-unknown
COPY --from=wasm-bindgen-cli /usr/local/cargo/bin/wasm-bindgen /usr/local/cargo/bin/wasm-bindgen
WORKDIR /build
COPY rust/ ./rust/
RUN cd rust \
    && cargo build --release --locked -p sdw-cli \
    && cp target/release/sdw /sdw \
    && cargo build --profile wasm --locked -p sdw-web --target wasm32-unknown-unknown \
    && wasm-bindgen target/wasm32-unknown-unknown/wasm/sdw_web.wasm \
         --target web --no-typescript --out-dir /web-dist/pkg \
    && cp -a web/static/. /web-dist/

# --- Clash (mihomo): ONE pinned, verified download, shared by the workspace and the client -------
# A static Go binary, so it runs on any base. image/tools/mihomo.env holds the version, URLs and
# sha256; fetch-tool refuses any other bytes. It reuses the rust-builder's digest-pinned image
# (already pulled for that stage) instead of introducing another base to pin.
FROM rust:1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS clash-bin
# Optional GitHub download mirror for fetch-tool (documented on the final stage's ARG of the same
# name and in tools/fetch.sh). Build-time only; empty = GitHub direct.
ARG SDW_GH_MIRROR=
RUN apk add --no-cache curl
COPY --chmod=0755 image/tools/fetch.sh /usr/local/lib/sdw/fetch-tool
COPY image/tools/mihomo.env /usr/local/lib/sdw/tools/mihomo.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool mihomo /tmp/clash.gz; \
    gunzip < /tmp/clash.gz > /clash; \
    chmod 0755 /clash; \
    rm -f /tmp/clash.gz; \
    /clash -v

# --- Standalone Clash client (deploy/client): `docker build --target clash-client` ----------------
# The workspace's Clash and the workspace's `sdw` binary and nothing else. `sdw proxy-only` renders
# the same template through the same code and then runs Clash, so there is no second hand-written
# config to drift. Not part of the workspace image: the last stage of this file is.
FROM debian:trixie-slim AS clash-client
ARG APT_MIRROR=mirrors.aliyun.com
ENV DEBIAN_FRONTEND=noninteractive
RUN rm -f /etc/apt/sources.list.d/* && \
    { echo "deb http://${APT_MIRROR}/debian trixie main"; \
      echo "deb http://${APT_MIRROR}/debian trixie-updates main"; \
      echo "deb http://${APT_MIRROR}/debian-security trixie-security main"; \
    } > /etc/apt/sources.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl libcap2-bin && \
    rm -rf /var/lib/apt/lists/*
COPY --from=clash-bin /clash /usr/bin/clash
RUN useradd --system --no-create-home --shell /usr/sbin/nologin clash \
    && setcap 'cap_net_bind_service=+ep' /usr/bin/clash \
    && install -d -o clash -g clash /etc/clash
COPY image/share/clash/config.yaml.template /usr/share/sdw/clash/config.yaml.template
COPY --from=rust-builder /sdw /usr/local/bin/sdw
EXPOSE 7890 7891
# Healthy = Clash accepts connections AND the subscription loaded nodes. Run as the clash user: the
# rendered config (which holds the controller secret) is readable by nobody else.
HEALTHCHECK --interval=30s --timeout=15s --start-period=90s --retries=3 \
  CMD ["setpriv", "--reuid=clash", "--regid=clash", "--clear-groups", "/usr/local/bin/sdw", "proxy-only", "--check"]
ENTRYPOINT ["/usr/local/bin/sdw", "proxy-only"]

# Base image toggle.
#   Default: kalilinux/kali-rolling — the production base, full pentest toolchain
#   via apt. Kali tracks Debian testing, which is what the Selkies and KasmVNC builds this image
#   installs target (a recent libva in particular: with an older one, bookworm's 2.17, Selkies' capture
#   library fails to load and the desktop shows "Waiting for stream...").
#   Alternative: build with `--build-arg BASE_IMAGE=debian:trixie-slim` for a
#   leaner non-pentest base. Do NOT use bookworm without vendoring a newer libva.
FROM ${BASE_IMAGE}

# Desktop streaming stack selector (build-time):
#   kasmvnc (default) — KasmVNC only (/vnc/): an ~8 MB .deb, builds in seconds
#   selkies           — Selkies only (/desktop/): H.264/WebCodecs, a ~160 MB Python venv
#   both              — Selkies + KasmVNC in parallel (published as the `-full` image)
#   none              — headless: no desktop at all (code-server only; the lean phone/AVF profile)
# Recorded in /etc/sdw-desktop-stack; parsed by the Rust desktop domain at runtime.
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

# apt hygiene (size only, no package/feature removed): a dpkg path-exclude config so NO apt layer
# below writes package docs, man/info pages, or other languages' UI translations (kept: en + zh_CN
# message catalogs, and each package's copyright for license compliance). Set BEFORE any install so
# every layer is smaller — a later `rm` cannot shrink bytes already committed to earlier layers.
# The rm/find then clears what the base image already shipped.
RUN printf '%s\n' \
      'path-exclude /usr/share/doc/*' \
      'path-include /usr/share/doc/*/copyright' \
      'path-exclude /usr/share/man/*' \
      'path-exclude /usr/share/groff/*' \
      'path-exclude /usr/share/info/*' \
      'path-exclude /usr/share/lintian/*' \
      'path-exclude /usr/share/linda/*' \
      'path-exclude /usr/share/locale/*' \
      'path-include /usr/share/locale/en*' \
      'path-include /usr/share/locale/zh_CN*' \
      'path-include /usr/share/locale/locale.alias' \
      > /etc/dpkg/dpkg.cfg.d/01-sdw-slim \
    && rm -rf /usr/share/man /usr/share/groff /usr/share/info /usr/share/doc /usr/share/lintian /usr/share/linda \
    && find /usr/share/locale -mindepth 1 -maxdepth 1 ! -name 'en*' ! -name 'zh_CN*' ! -name 'locale.alias' -exec rm -rf {} +

# Core layer: system packages
RUN apt-get update && apt-get install -y --no-install-recommends \
    tini \
    supervisor \
    gocryptfs \
    fuse3 \
    procps \
    psmisc \
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
    iptables \
    libcap2-bin \
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen \
    && locale-gen \
    && apt-get autoremove -y \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# No JavaScript runtime is needed by the workspace application itself: the CLI, entrypoint,
# daemons and browser UI are Rust (`sdw` + WebAssembly). Node remains installed only as a user
# development runtime, not as part of this project's runtime or build chain.

# Desktop layer — COMMON deps (both stacks): X server, XFCE, fonts, mesa, dbus.
# XFCE is the session for BOTH stacks (selkies and kasmvnc). This eliminates the
# openbox+tint2 dependency (tint2 is not in Kali repos) and gives a consistent
# desktop experience regardless of the streaming backend.
# DESKTOP_STACK=none (the headless / phone-AVF profile) skips this whole layer: no X server, no
# XFCE, no CJK fonts — the ~500 MB a phone VM does not want and that a stock-AVF guest kernel cannot
# render well anyway (software GL, no SysV IPC). Only code-server and the services run.
ARG DESKTOP_STACK
RUN if [ "$DESKTOP_STACK" = "none" ]; then \
      echo "Skipping desktop-common install (DESKTOP_STACK=none)"; \
    else \
      apt-get update && apt-get install -y --no-install-recommends \
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
        && apt-get autoremove -y \
        && apt-get clean \
        && rm -rf /var/lib/apt/lists/*; \
    fi

# Optional GitHub download mirror for the pinned third-party tools below (selkies, kasmvnc, caddy,
# restic, croc, mise, code-server). Build-time only: an unreferenced ARG, so it is injected into
# each RUN's environment for fetch-tool to read but is NOT written to the image or its history, and
# it sits AFTER the apt layers so adding it never invalidates their cache. When set, fetch-tool
# tries ${SDW_GH_MIRROR}<github-url> before GitHub; the pinned sha256 is still verified, so a mirror
# can only serve the exact pinned bytes. Pass it when GitHub is throttled or blocked, e.g. a caching
# reverse proxy: docker compose build --build-arg SDW_GH_MIRROR=https://ghfast.top/  (compose wires
# it from .env). Empty (default) keeps the unchanged GitHub-direct download path.
ARG SDW_GH_MIRROR=

# fetch-tool: the one way a third-party download enters this image. It reads the pin (version, URL,
# sha256) from /usr/local/lib/sdw/tools/<name>.env and refuses any other bytes.
COPY --chmod=0755 image/tools/fetch.sh /usr/local/lib/sdw/fetch-tool

# --- Selkies desktop streaming (H.264/WebCodecs) — installed for both|selkies ---
# Selkies 2.x is a Python server that serves its own web client and its WebSocket on one port
# (behind the Caddy gate, under /desktop/). It is installed like every other third-party piece:
#   * the release wheel, from image/tools/selkies.env (fetch-tool checks its sha256), and
#   * its Python dependencies from image/tools/selkies-requirements.txt, which holds the sha256 of
#     every file allowed for each pinned version. pip --require-hashes refuses anything else, and
#     --only-binary refuses a source build, so no compiler is in the image.
# pixelflux and pcmflux (the native screen and audio capture libraries) are among those: the wheel
# names the exact versions it works with and the lock follows it. Nothing here moves by itself:
# deploy/scripts/update-tools.sh edits the pin and regenerates the lock together.
# (Selkies 1.x, the version this image used to pin, was deleted from PyPI and GitHub by its
# authors; it cannot be built any more.) The libraries below are the runtime dependencies the
# project lists for its own Debian package; the wheels bundle their codecs.
COPY image/tools/selkies.env image/tools/selkies-requirements.txt /usr/local/lib/sdw/tools/
ARG DESKTOP_STACK
RUN if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "selkies" ]; then \
      set -e; \
      apt-get update && apt-get install -y --no-install-recommends \
        libpulse0 pulseaudio xclip xdotool xcvt \
        libxcb1 libxkbcommon0 libx11-xcb1 libva2 libva-drm2 libva-x11-2 libdrm2 libgbm1 libegl1 \
        libwayland-server0 libglib2.0-0 libpixman-1-0 libxcb-render0 libxcb-shm0 libxcb-dri3-0 \
        libxfixes3 libxext6 libice6 libsm6 python3-venv \
      && . /usr/local/lib/sdw/tools/selkies.env \
      && /usr/local/lib/sdw/fetch-tool selkies "/tmp/selkies-${VERSION}-py3-none-any.whl" \
      && /usr/bin/python3 -m venv /opt/selkies-venv \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --require-hashes --only-binary=:all: \
           -r /usr/local/lib/sdw/tools/selkies-requirements.txt \
      && /opt/selkies-venv/bin/pip install --no-cache-dir --no-deps "/tmp/selkies-${VERSION}-py3-none-any.whl" \
      && rm -f /tmp/selkies-*.whl \
      && /opt/selkies-venv/bin/python -c "import selkies.__main__, pixelflux, pcmflux" \
      && /opt/selkies-venv/bin/python -m selkies --version \
      && rm -rf /var/lib/apt/lists/*; \
    else \
      echo "Skipping selkies install (DESKTOP_STACK=$DESKTOP_STACK)"; \
    fi

# --- KasmVNC desktop streaming (integrated X + WebSocket VNC) — both|kasmvnc ---
# Native trixie .deb. Version, URL and sha256 are pinned in image/tools/kasmvnc.env and checked by
# fetch-tool; deploy/scripts/update-tools.sh is the only thing that edits them. Auth is handled by
# the ingress gate (the Perl packages are KasmVNC's runtime requirements).
COPY image/tools/kasmvnc.env /usr/local/lib/sdw/tools/kasmvnc.env
RUN if [ "$DESKTOP_STACK" = "both" ] || [ "$DESKTOP_STACK" = "kasmvnc" ]; then \
      set -e; \
      apt-get update; \
      /usr/local/lib/sdw/fetch-tool kasmvnc /tmp/kasmvnc.deb; \
      apt-get install -y --no-install-recommends /tmp/kasmvnc.deb \
        libdatetime-perl libhash-merge-simple-perl libscalar-list-utils-perl \
        libyaml-tiny-perl libtry-tiny-perl libswitch-perl; \
      rm -f /tmp/kasmvnc.deb && rm -rf /var/lib/apt/lists/*; \
    else \
      echo "Skipping kasmvnc install (DESKTOP_STACK=$DESKTOP_STACK)"; \
    fi

# Install Caddy (image/tools/caddy.env: sha256 from GitHub's release metadata AND the sha512 from
# Caddy's own checksums file, which must both match). The standard release build ships the default
# modules (reverse_proxy, file_server, forward_auth).
COPY image/tools/caddy.env /usr/local/lib/sdw/tools/caddy.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool caddy /tmp/caddy.tar.gz; \
    tar -xzf /tmp/caddy.tar.gz -C /tmp caddy; \
    install -m 0755 /tmp/caddy /usr/bin/caddy; \
    rm -f /tmp/caddy.tar.gz /tmp/caddy; \
    caddy version

# Install restic (realtime backup to S3): one static binary, no runtime deps. Downloaded to a file and
# verified BEFORE it is decompressed: a pipe straight into an executable cannot be checked.
COPY image/tools/restic.env /usr/local/lib/sdw/tools/restic.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool restic /tmp/restic.bz2; \
    bunzip2 < /tmp/restic.bz2 > /usr/local/bin/restic; \
    rm -f /tmp/restic.bz2; \
    chmod +x /usr/local/bin/restic; \
    restic version

# Install croc (peer-to-peer, end-to-end encrypted file transfer for `push`/`pull`): a single static
# Go binary, pinned and verified like restic.
COPY image/tools/croc.env /usr/local/lib/sdw/tools/croc.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool croc /tmp/croc.tar.gz; \
    tar -xzf /tmp/croc.tar.gz -C /usr/local/bin croc; \
    rm -f /tmp/croc.tar.gz; \
    croc --version

# Install mise (runtime version manager): the pinned release binary, verified. The installer script
# (mise.jdx.dev/install.sh) is not used any more; running whatever that URL returns was the one
# unverified step left in the image.
COPY image/tools/mise.env /usr/local/lib/sdw/tools/mise.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool mise /tmp/mise.tar.gz; \
    tar -xzf /tmp/mise.tar.gz -C /tmp mise/bin/mise; \
    install -m 0755 /tmp/mise/bin/mise /usr/local/bin/mise; \
    rm -rf /tmp/mise /tmp/mise.tar.gz; \
    mise --version
# The runtimes are a system install under /opt/mise: root-owned and read-only to the unprivileged
# workload user, which installs its own tools into its home (MISE_DATA_DIR there) and sees these
# through MISE_SYSTEM_DATA_DIR. For root (the build, the operator) the data dir is the same place.
ENV MISE_DATA_DIR=/opt/mise
ENV MISE_SYSTEM_DATA_DIR=/opt/mise

# Language runtimes are pinned once in image/.mise.toml. The install commands below name only the
# tools, so changing a version never requires editing this Dockerfile too. mise verifies downloads.
COPY image/.mise.toml /etc/mise/config.toml

# Java layer — online download via mise. Trim, in-layer (so the layer actually shrinks), what a JDK
# does not need to compile or run code: the source bundle for IDE navigation (src.zip), demos,
# man pages, samples and license texts. jmods (needed only for `jlink` custom runtimes) is kept.
# Then drop mise's download-archive cache — the runtime is already extracted under /opt/mise.
RUN MISE_HTTP_TIMEOUT=120 mise install java && mise reshim \
    && JH="$(mise where java)" \
    && rm -rf "$JH/lib/src.zip" "$JH/demo" "$JH/man" "$JH/sample" "$JH/legal" \
    && rm -rf /root/.cache/mise

# Node, Python and pnpm are independent cache/failure domains. One flaky upstream must not discard
# another tool's successful 60 MB download and force all three to start again. The longer timeout
# is local to each build step and does not enter the final runtime environment.
RUN MISE_HTTP_TIMEOUT=120 mise install node && mise reshim \
    && rm -rf /root/.cache/mise
RUN MISE_HTTP_TIMEOUT=120 mise install python && mise reshim \
    && rm -rf /root/.cache/mise
RUN MISE_HTTP_TIMEOUT=120 mise install pnpm && mise reshim \
    && rm -rf /root/.cache/mise

# One stable path for the JDK whatever its version: the install directory is named after it.
RUN ln -sfn "$(mise where java)" /opt/java
ENV PATH="/opt/mise/shims:${PATH}"
ENV JAVA_HOME="/opt/java"

# Clash (mihomo): the pinned, verified binary from the clash-bin stage above (image/tools/mihomo.env).
COPY --from=clash-bin /clash /usr/bin/clash
RUN /usr/bin/clash -v

# Clash always runs as its OWN uid (never root), started by `sdw entrypoint` and respawned by
# `sdw clash-watcher`. Under the fail-closed egress firewall (on by default) that uid is also the
# one the nftables OUTPUT filter lets out directly. Clash still binds the DNS port 53, which a
# non-root process cannot do without a capability, so grant the binary cap_net_bind_service.
# nftables + libcap2-bin are installed in the core apt layer; the filter is loaded at boot by
# `sdw entrypoint` (the enforce-egress step) unless EGRESS_ENFORCE=0.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin clash \
    && setcap 'cap_net_bind_service=+ep' /usr/bin/clash

# Caddy is the only program facing the network and needs no privilege, so supervisord runs it as
# its own user (src/image/supervisord.conf). It keeps its state, which it must be able to write,
# under /var/lib/caddy.
RUN useradd --system --no-create-home --home-dir /var/lib/caddy --shell /usr/sbin/nologin caddy \
    && install -d -o caddy -g caddy -m 0700 /var/lib/caddy

# The audit trail (JSON lines: logins, self-destruct calls, egress changes) is written under
# /var/log/sdw; docker-compose.yml mounts a named volume there so it outlives the container.
# The desktop programs' logs (rotated by supervisord, see etc/supervisor/desktop-*.conf) go under
# /var/log/desktop, which must exist before supervisord starts them.
RUN install -d -m 0700 /var/log/sdw && install -d -m 0755 /var/log/desktop

# Install code-server: the pinned .deb, verified (image/tools/code-server.env). The official install.sh
# is not run: it fetches the same package, but unpinned and unchecked.
COPY image/tools/code-server.env /usr/local/lib/sdw/tools/code-server.env
RUN set -eux; \
    /usr/local/lib/sdw/fetch-tool code-server /tmp/code-server.deb; \
    apt-get update; \
    apt-get install -y --no-install-recommends /tmp/code-server.deb; \
    rm -f /tmp/code-server.deb; \
    rm -rf /var/lib/apt/lists/*; \
    code-server --version

# Filesystem prep
RUN mkdir -p /workspace /vault/cipher /etc/supervisor/conf.d /var/run/vault-sync \
    /etc/caddy /etc/clash /opt/workspace \
    /pentest/cipher /pentest/rootfs \
    /opt/desktop-apps /tmp/.desktop-cache

# Desktop apps registry
RUN echo '[]' > /opt/desktop-apps/.registry.json

# Ensure proxy env is available in all shells (interactive terminals via code-server). A no_proxy
# the shell already has wins over the default: the launcher (`sdw workload`) adds the networks of
# EGRESS_ALLOW_CIDRS to it, and a login shell must not undo that.
RUN echo 'export http_proxy=http://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export https_proxy=http://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export all_proxy=socks5h://127.0.0.1:7890' >> /etc/profile.d/proxy.sh && \
    echo 'export no_proxy="${no_proxy:-localhost,127.0.0.0/8,172.16.0.0/12,10.0.0.0/8,169.254.0.0/16}"' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTP_PROXY=$http_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export HTTPS_PROXY=$https_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export ALL_PROXY=$all_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'export NO_PROXY=$no_proxy' >> /etc/profile.d/proxy.sh && \
    echo 'eval "$(mise activate bash)"' >> /etc/profile.d/proxy.sh && \
    echo '# noproxy: run a command without the proxy variables, so it connects directly. With the egress firewall on (the default) that connection is dropped; EGRESS_ENFORCE=0 turns the firewall off.' >> /etc/profile.d/proxy.sh && \
    echo 'noproxy() { env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY "$@"; }' >> /etc/profile.d/proxy.sh && \
    chmod +x /etc/profile.d/proxy.sh && \
    echo 'source /etc/profile.d/proxy.sh' >> /root/.bashrc && \
    echo '[ -r /etc/profile.d/proxy.sh ] && . /etc/profile.d/proxy.sh' >> /etc/bash.bashrc

# Copy baked configs
COPY image/supervisord.conf /etc/supervisor/supervisord.conf
# The Clash template lives outside the /etc/clash volume on purpose: a named volume is filled from
# the image only when it is created, so a template inside it would never be updated again.
COPY image/share/clash/config.yaml.template /usr/share/sdw/clash/config.yaml.template
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

# XFCE launcher (waits for X server before starting the session; both stacks)
RUN cp /opt/workspace/image/etc/desktop/start-xfce.sh /usr/local/bin/start-xfce.sh && \
    cp /opt/workspace/image/etc/desktop/start-audio.sh /usr/local/bin/start-audio.sh && \
    cp /opt/workspace/image/etc/desktop/start-selkies.sh /usr/local/bin/start-selkies.sh && \
    chmod +x /usr/local/bin/start-xfce.sh /usr/local/bin/start-audio.sh /usr/local/bin/start-selkies.sh

# glycin (GTK4's image loader, via librsvg) sandboxes each loader with bwrap/user-namespaces.
# Where unprivileged userns is unavailable (colima/Lima, locked-down Docker/CI) bwrap fails and GTK
# turns a failed SVG icon load into a FATAL assertion -> the XFCE session aborts (exit 134) in a
# crash loop -> black screen. Shim bwrap to run glycin loaders directly ONLY when a user namespace
# cannot be created; the real bwrap is kept (bwrap.real) and used on capable hosts and for every
# non-glycin caller. See image/etc/desktop/glycin-bwrap-shim.sh.
RUN if [ -e /usr/bin/bwrap ]; then \
      [ -e /usr/bin/bwrap.real ] || mv /usr/bin/bwrap /usr/bin/bwrap.real; \
      cp /opt/workspace/image/etc/desktop/glycin-bwrap-shim.sh /usr/bin/bwrap; \
      chmod 755 /usr/bin/bwrap; \
    fi

# No per-session ssh-agent / gpg-agent in the desktops (system-wide xfce4-session default, below
# any per-user setting). xfce4-session starts an ssh-agent that detaches into a session of its own
# and listens on a socket in the session's HOME, which is inside the vault. A listening unix socket
# pins the mount without being an open file (fuser does not see it), so the agent outlived the
# "stopped" desktop, one more per start, and lock-vault could never unmount cleanly (measured: 3
# leftover agents, the unmount refused for 20 s with no process holding a file). The property goes
# INTO the package's own file: it also holds the Failsafe session (window manager, panel, desktop).
RUN f=/etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-session.xml && \
    { [ -f "$f" ] || exit 0; } && \
    sed -i 's#</channel>#  <property name="startup" type="empty">\n    <property name="ssh-agent" type="empty">\n      <property name="enabled" type="bool" value="false"/>\n    </property>\n    <property name="gpg-agent" type="empty">\n      <property name="enabled" type="bool" value="false"/>\n    </property>\n  </property>\n</channel>#' "$f" && \
    grep -q 'name="ssh-agent"' "$f" && grep -q 'FailsafeSessionName' "$f"

# Pins for the applications `desktop-install` fetches on demand (IDEA, Burp Suite): the version, URL
# and sha256 it must match, checked by the same fetch-tool the image build used.
COPY image/tools/idea.env image/tools/burpsuite.env /usr/local/lib/sdw/tools/

# The Rust command surface + entrypoint + daemons: one multi-call binary. `sdw install` symlinks
# every bin command (BIN_COMMANDS in rust/cli/src/dispatch.rs) to it (argv[0] dispatch).
COPY --from=rust-builder /sdw /usr/local/bin/sdw
RUN /usr/local/bin/sdw install /usr/local/bin

# The workload user. Everything the user runs (code-server and its terminals, the desktops and their
# applications, build tools, whatever a package's install script does) runs as `dev`, not root; the
# services that hold the secrets and guard the perimeter keep their own identities. HOME is inside
# the vault (/workspace/home), created at unlock. See core/src/workspace/workload.rs.
RUN groupadd --gid 1000 dev \
    && useradd --uid 1000 --gid 1000 --home-dir /workspace/home --no-create-home --shell /bin/bash dev

# The bridge from the workload to root: `doas` runs ONLY the entries of /usr/local/libexec/sdw-priv
# (created by `sdw install` above, one per command that needs root), with a clean environment. The
# rules are generated from the same list the entries are, so they cannot disagree.
RUN apt-get update \
    && apt-get install -y --no-install-recommends opendoas \
    && rm -rf /var/lib/apt/lists/* \
    && /usr/local/bin/sdw doas-rules > /etc/doas.conf \
    && chmod 0600 /etc/doas.conf \
    && install -d -m 0700 /var/lib/sdw /run/sdw \
    && chmod 0700 /vault/cipher /pentest/cipher

# `root`: a one-word root shell for the workload user. Works ONLY when WORKLOAD_ROOT=1 (the boot
# then adds the matching doas rule; see entrypoint apply_workload_root). Otherwise doas denies it
# and the dev/root confinement (the default) holds. Host-side alternative: `docker exec -u root`.
RUN printf '#!/bin/sh\nexec doas /bin/bash "$@"\n' > /usr/local/bin/root \
    && chmod 0755 /usr/local/bin/root

# No setuid or setgid file but doas: a local-privilege-escalation bug in su, mount, passwd and the
# rest is then not reachable from the workload (it has no capabilities either).
RUN find / -xdev -type f \( -perm -4000 -o -perm -2000 \) ! -path /usr/bin/doas -exec chmod a-s {} +

# The sync API daemon is part of the sdw binary now: supervisord runs `sdw sync-service`. It serves
# /sync/api/* and reads SYNC_ENABLED / REMOTE_DESTRUCT_ENABLED from env.

# The browser sync UI is Rust/WebAssembly. wasm-bindgen generated the loader in the build stage;
# no TypeScript, Bun, Node, Vite or frontend package manager is part of the application build.
COPY --from=rust-builder /web-dist /opt/sdw-web

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

# Liveness, app-level: the Caddy ingress must actually serve the auth gate's login endpoint
# (proves Caddy AND the always-on sync-service that backs the gate are up — /auth/login is the
# un-gated allowlisted route, so a 200 means the whole front path works), and code-server must be
# accepting TCP. Deliberately does NOT fail on a locked vault or degraded egress: those are
# reported, not fatal, and the workspace is meant to run in those states. The generous start-period
# covers Clash bootstrap + vault unlock before supervisord brings the services up.
HEALTHCHECK --interval=30s --timeout=10s --start-period=180s --retries=3 \
  CMD curl -fsS -o /dev/null http://127.0.0.1:8080/auth/login && nc -z 127.0.0.1 8082 || exit 1

ENTRYPOINT ["tini", "--", "/usr/local/bin/sdw", "entrypoint"]
