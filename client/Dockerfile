FROM debian:bookworm-slim

LABEL maintainer="dev-workspace" \
      description="Lightweight Clash proxy client container"

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    wget \
    && rm -rf /var/lib/apt/lists/*

# Install Clash (mihomo)
ARG CLASH_VERSION=1.18.0
RUN ARCH=$(dpkg --print-architecture) && \
    curl -fsSL "https://github.com/MetaCubeX/mihomo/releases/download/v${CLASH_VERSION}/mihomo-linux-${ARCH}-v${CLASH_VERSION}.gz" \
    | gunzip > /usr/local/bin/clash && \
    chmod +x /usr/local/bin/clash

# Config directory
RUN mkdir -p /etc/clash
COPY clash-config-template.yaml /etc/clash/config.yaml

EXPOSE 7890 7891

ENTRYPOINT ["/usr/local/bin/clash", "-d", "/etc/clash"]
