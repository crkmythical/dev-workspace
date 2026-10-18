#!/usr/bin/env bash
# Expose a THROWAWAY smoke instance through a Cloudflare quick tunnel (trycloudflare.com) and ask
# what the real tunnel will be asked: does traffic get through the edge, does authentication hold,
# do WebSockets work, can the destruct rate limit be talked out of its budget with forged headers?
#
#   tests/smoke/run.sh --image dev-workspace:latest --keep --only '^10'   # leaves an instance on 18090
#   tests/smoke/manual/tunnel.sh [origin] [password]
#
# Needs cloudflared and outbound access to Cloudflare. A quick tunnel has no Access policy in front
# of it, so what it exercises is the application's own authentication; the origin is the throwaway
# instance, never a real workspace. HOME is a temporary directory so that an existing
# ~/.cloudflared (a named tunnel of yours) is never read. Run it by hand: the tunnel is the
# machine's connection to the internet, which a test suite should not depend on.
set -u

ORIGIN="${1:-http://127.0.0.1:18090}"
PASS="${2:-smoke-master-pass-123}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CLOUDFLARED="$(command -v cloudflared || true)"
[ -n "$CLOUDFLARED" ] || { echo "cloudflared is not installed" >&2; exit 2; }

TMPHOME="$(mktemp -d)"
LOG="$TMPHOME/cloudflared.log"
cleanup() {
  [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null
  rm -rf "$TMPHOME"
}
trap cleanup EXIT

HOME="$TMPHOME" "$CLOUDFLARED" tunnel --no-autoupdate --protocol http2 --url "$ORIGIN" >"$LOG" 2>&1 &
PID=$!
URL=""
for _ in $(seq 1 45); do
  URL="$(grep -o 'https://[a-z0-9-]*\.trycloudflare\.com' "$LOG" | head -1)"
  [ -n "$URL" ] && break
  sleep 2
done
[ -n "$URL" ] || { echo "no tunnel URL"; tail -15 "$LOG"; exit 1; }
echo "tunnel: $URL"

# code <curl args...>: the status, retried while the edge says the tunnel is down (530) or nothing
# answered. A quick tunnel drops and re-registers now and then, and the edge answers 530 / error
# 1033 in between.
code() {
  local c
  for _ in 1 2 3 4 5 6 7 8; do
    c="$(curl -s -o /dev/null -w '%{http_code}' --max-time 20 "$@" || true)"
    case "$c" in 530 | 000 | 502 | 503 | 504) sleep 4 ;; *) break ;; esac
  done
  printf '%s' "$c"
}

# Wait for six good answers in a row, four seconds apart.
good=0
for _ in $(seq 1 40); do
  if [ "$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 "$URL/" || true)" = 302 ]; then good=$((good + 1)); else good=0; fi
  [ "$good" -ge 6 ] && break
  sleep 4
done
echo "connector stable after $good consecutive answers"

failed=0
expect() { # name expected actual
  if [ "$2" = "$3" ]; then printf '  PASS  %s\n' "$1"; else printf '  FAIL  %s (expected %s, got %s)\n' "$1" "$2" "$3"; failed=$((failed + 1)); fi
}
expect "GET / is code-server's login redirect" 302 "$(code "$URL/")"
expect "/vnc/ without credentials is refused" 401 "$(code "$URL/vnc/")"
expect "/vnc/ with the master password is served" 200 "$(code -u "user:$PASS" "$URL/vnc/")"
expect "/vnc/ with a wrong password is refused" 401 "$(code -u "user:wrong" "$URL/vnc/")"
expect "the sync page is served" 200 "$(code "$URL/sync/")"
expect "listing without proof of the key is refused" 401 "$(code "$URL/sync/api/download")"

# WebSocket through the edge. curl must speak HTTP/1.1 for an Upgrade; over HTTP/2 the header is
# meaningless and the origin sees a plain GET. (The browser check below proves the real thing.)
ws() {
  curl -s --http1.1 --max-time 8 -i -N "$@" -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
    -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    -H 'Sec-WebSocket-Protocol: binary' -H "Origin: $URL" "$URL/websockify" 2>/dev/null | head -1 | tr -d '\r'
}
expect "the WebSocket upgrade with credentials switches protocols" "HTTP/1.1 101 Switching Protocols" "$(ws -u "user:$PASS")"
expect "and is refused without them" "HTTP/1.1 401 Unauthorized" "$(ws)"

# The destruct endpoint allows 3 wrong attempts per source address per minute and 10 in all. Behind
# the edge the source address is CF-Connecting-IP, which the edge sets itself.
post() { code -X POST -H 'content-type: application/json' "$@" "$URL/sync/api/destruct"; }
codes() { # five plain attempts, their statuses on one line
  local n out=""
  for n in 1 2 3 4 5; do out="$out $(post -d "{\"passphrase\":\"try-$n\"}")"; done
  printf '%s' "${out# }"
}
expect "destruct: plain attempts are limited after three" "404 404 404 429 429" "$(codes)"
sleep 65
rotating_xff=""
for n in 1 2 3 4 5; do rotating_xff="$rotating_xff $(post -H "X-Forwarded-For: 10.1.1.$n" -d "{\"passphrase\":\"xff-$n\"}")"; done
expect "destruct: a forged X-Forwarded-For buys no new budget" "404 404 404 429 429" "${rotating_xff# }"
sleep 65
forged_cf="$(post -H 'CF-Connecting-IP: 10.2.2.2' -d '{"passphrase":"cf"}')"
expect "destruct: a client-supplied CF-Connecting-IP never reaches the service (the edge refuses it)" 403 "$forged_cf"

echo "cloudflared lost its connection $(grep -c 'Lost connection with the edge' "$LOG") time(s) meanwhile"

if [ -d "$HERE/../browser/node_modules/playwright-core" ] && command -v node >/dev/null 2>&1; then
  echo "browser checks through the tunnel:"
  (cd "$HERE/../browser" && SMOKE_BASE="$URL" SMOKE_PASSWORD="$PASS" SMOKE_STACK="${SMOKE_STACK:-kasmvnc}" node check.mjs) || failed=$((failed + 1))
else
  echo "(browser checks skipped: run 'bun install' in tests/smoke/browser, and have node)"
fi
[ "$failed" -eq 0 ]
