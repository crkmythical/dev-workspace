#!/bin/bash
# migrate-clash-live.sh — 在已运行的容器内执行，热迁移 Clash 到 proxy-providers 模式
# 用法: docker exec -it dev-workspace bash < scripts/migrate-clash-live.sh
#   或: docker exec -it dev-workspace bash 然后粘贴执行
set -euo pipefail

SUB_URL="${CLASH_SUBSCRIPTION_URL:-}"
if [[ -z "$SUB_URL" ]]; then
  echo "ERROR: CLASH_SUBSCRIPTION_URL not set in environment" >&2
  exit 1
fi

echo "=== Step 1: Write config template ==="
mkdir -p /etc/clash/providers

cat > /etc/clash/config.yaml.new << 'TEMPLATE'
# Clash (mihomo) configuration — proxy-providers mode
mixed-port: 7890
socks-port: 7891
allow-lan: false
mode: global
log-level: warning
external-controller: 127.0.0.1:9090
unified-delay: true
tcp-concurrent: true

dns:
  enable: true
  listen: 0.0.0.0:53
  enhanced-mode: fake-ip
  fake-ip-range: 198.18.0.1/16
  nameserver:
    - https://1.1.1.1/dns-query
    - https://8.8.8.8/dns-query

proxy-providers:
  subscription:
    type: http
    url: "PLACEHOLDER_URL"
    interval: 3600
    proxy: DIRECT
    path: ./providers/subscription.yaml
    health-check:
      enable: true
      url: http://www.gstatic.com/generate_204
      interval: 180

proxy-groups:
  - name: auto-select
    type: url-test
    include-all: true
    exclude-type: direct
    exclude-filter: "优选域名|IPv6优选"
    url: http://www.gstatic.com/generate_204
    interval: 180
    tolerance: 100

rules:
  - MATCH,auto-select
TEMPLATE

# Inject actual subscription URL
sed -i "s|PLACEHOLDER_URL|${SUB_URL}|" /etc/clash/config.yaml.new

echo "=== Step 2: Seed provider file (extract proxies from subscription) ==="
curl -fsSL --noproxy '*' --max-time 30 "$SUB_URL" -o /tmp/sub-full.yaml 2>/dev/null
if [[ $? -eq 0 ]]; then
  # Extract proxies section
  awk '/^proxies:/{found=1} found{print} /^[a-z]/ && found && !/^proxies:/{exit}' /tmp/sub-full.yaml > /etc/clash/providers/subscription.yaml
  echo "Provider seed: $(grep -c '  - name:' /etc/clash/providers/subscription.yaml) nodes"
  rm -f /tmp/sub-full.yaml
else
  echo "WARNING: Could not seed provider (Clash will fetch on its own)"
fi

echo "=== Step 3: Swap config (atomic) ==="
mv /etc/clash/config.yaml.new /etc/clash/config.yaml

echo "=== Step 4: Reload Clash via API ==="
# PUT /configs forces Clash to re-read config from disk
HTTP_CODE=$(curl -s -o /dev/null -w '%{http_code}' -X PUT \
  -H 'Content-Type: application/json' \
  -d '{"path":"/etc/clash/config.yaml"}' \
  'http://127.0.0.1:9090/configs?force=true')

if [[ "$HTTP_CODE" == "204" || "$HTTP_CODE" == "200" ]]; then
  echo "Clash reloaded successfully."
else
  echo "WARNING: Reload returned HTTP $HTTP_CODE, falling back to process restart..."
  pkill -f '/usr/bin/clash' || true
  sleep 1
  /usr/bin/clash -d /etc/clash &
  disown
  sleep 3
fi

# Point GLOBAL to auto-select (GLOBAL defaults to DIRECT in global mode)
sleep 2
curl -s -X PUT -H 'Content-Type: application/json' \
  -d '{"name":"auto-select"}' \
  'http://127.0.0.1:9090/proxies/GLOBAL' > /dev/null
echo "GLOBAL → auto-select"

echo "=== Step 5: Verify ==="
sleep 5

# Check provider loaded
NODE_COUNT=$(curl -s http://127.0.0.1:9090/providers/proxies/subscription 2>/dev/null | grep -o '"name"' | wc -l)
echo "Provider nodes: $NODE_COUNT"

# Check auto-select group
AUTO_NOW=$(curl -s http://127.0.0.1:9090/proxies/auto-select 2>/dev/null | grep -o '"now":"[^"]*"' | head -1)
echo "Auto-select: $AUTO_NOW"

# Egress test
if curl -fsSL --proxy http://127.0.0.1:7890 --max-time 10 https://www.google.com -o /dev/null 2>/dev/null; then
  echo "✅ Egress OK — migration complete!"
else
  echo "⚠️  Egress probe failed. Nodes may still be initializing (wait 30s and retry)."
  echo "    Manual check: curl -x http://127.0.0.1:7890 https://www.google.com"
fi
