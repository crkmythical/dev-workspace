#!/usr/bin/env bash
# Cloud-init style provisioning script for Parallels Ubuntu 22.04 VM
# Installs Clash and exposes proxy to the host via shared networking.
set -euo pipefail

echo "=== Parallels VM Provisioning ==="
echo "Target: Ubuntu 22.04 guest with Clash proxy"

# Update system
apt-get update && apt-get upgrade -y

# Install dependencies
apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  wget \
  gunzip \
  systemd

# Install Clash (mihomo)
CLASH_VERSION="${CLASH_VERSION:-1.18.0}"
ARCH=$(dpkg --print-architecture)
curl -fsSL "https://github.com/MetaCubeX/mihomo/releases/download/v${CLASH_VERSION}/mihomo-linux-${ARCH}-v${CLASH_VERSION}.gz" \
  | gunzip > /usr/local/bin/clash
chmod +x /usr/local/bin/clash

# Create config directory
mkdir -p /etc/clash

# Write base config (user must supply subscription URL)
cat > /etc/clash/config.yaml <<'EOF'
port: 7890
socks-port: 7891
allow-lan: true
bind-address: "0.0.0.0"
mode: rule
log-level: info

external-controller: "0.0.0.0:9090"

dns:
  enable: true
  enhanced-mode: fake-ip
  nameserver:
    - https://dns.cloudflare.com/dns-query
    - https://dns.google/dns-query

rules:
  - DOMAIN-SUFFIX,local,DIRECT
  - IP-CIDR,127.0.0.0/8,DIRECT
  - IP-CIDR,10.0.0.0/8,DIRECT
  - IP-CIDR,192.168.0.0/16,DIRECT
  - MATCH,DIRECT
EOF

# Create systemd service
cat > /etc/systemd/system/clash.service <<'EOF'
[Unit]
Description=Clash Proxy
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/clash -d /etc/clash
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable clash
systemctl start clash

echo ""
echo "=== Provisioning Complete ==="
echo "Clash proxy running on:"
echo "  HTTP:   0.0.0.0:7890"
echo "  SOCKS5: 0.0.0.0:7891"
echo ""
echo "From macOS host, configure proxy:"
echo "  export http_proxy=http://<vm-ip>:7890"
echo "  export https_proxy=http://<vm-ip>:7890"
echo ""
echo "To add subscription, edit /etc/clash/config.yaml and restart:"
echo "  systemctl restart clash"
