FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=en_US.UTF-8
ENV TZ=Asia/Shanghai

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
    && sed -i '/en_US.UTF-8/s/^# //g' /etc/locale.gen \
    && locale-gen \
    && rm -rf /var/lib/apt/lists/*

# Install Bun
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"

# Install Caddy
RUN curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=$(dpkg --print-architecture)" -o /usr/bin/caddy \
    && chmod +x /usr/bin/caddy

# Install mise (runtime version manager)
RUN curl -fsSL https://mise.jdx.dev/install.sh | bash
ENV PATH="/root/.local/bin:${PATH}"

# Pre-install default tool versions via mise
COPY image/.mise.toml /root/.mise.toml
ENV MISE_DATA_DIR=/root/.local/share/mise
RUN mise trust /root/.mise.toml && \
    mise install java@zulu-17 && \
    mise use --global java@zulu-17 && \
    mise reshim
ENV PATH="/root/.local/share/mise/shims:/root/.local/bin:${PATH}"
ENV JAVA_HOME="/root/.local/share/mise/installs/java/zulu-17"

# Install Clash (mihomo) — pinned version
ARG CLASH_VERSION=1.18.0
RUN ARCH=$(dpkg --print-architecture) && \
    curl -fsSL "https://github.com/MetaCubeX/mihomo/releases/download/v${CLASH_VERSION}/mihomo-linux-${ARCH}-v${CLASH_VERSION}.gz" \
    | gunzip > /usr/bin/clash && chmod +x /usr/bin/clash

# Install code-server — pinned version
ARG CODE_SERVER_VERSION=4.96.4
RUN curl -fsSL "https://code-server.dev/install.sh" | sh -s -- --version=${CODE_SERVER_VERSION}

# Filesystem prep
RUN mkdir -p /workspace /vault/cipher /etc/supervisor/conf.d /var/run/vault-sync \
    /etc/caddy /etc/clash /opt/workspace \
    /pentest/cipher /pentest/rootfs

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
    for cmd in init-pentest unlock-pentest lock-pentest pentest-bootstrap; do \
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
