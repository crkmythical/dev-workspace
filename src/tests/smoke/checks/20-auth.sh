#!/usr/bin/env bash
# Who gets in, through the published port, exactly as a browser or the tunnel would.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"

AUTH="user:${SMOKE_PASSWORD}"

step "code-server"
expect_code "the root redirects to the login page" 302 "$BASE/"
expect_code "the right password logs in (redirect plus session cookie)" 302 \
  -X POST --data-urlencode "password=${SMOKE_PASSWORD}" "$BASE/login"
expect_code "a wrong password stays on the login page" 200 \
  -X POST --data-urlencode "password=not-the-password" "$BASE/login"
COOKIE="$(curl -s --noproxy '*' --max-time 30 -i -X POST --data-urlencode "password=${SMOKE_PASSWORD}" "$BASE/login" | tr -d '\r' | grep -i '^set-cookie: code-server-session=' | head -c 40)"
expect_match "the session cookie is set" '^[Ss]et-[Cc]ookie: code-server-session=' "$COOKIE"

step "desktop (KasmVNC)"
expect_code "/vnc/ without credentials is refused" 401 "$BASE/vnc/"
expect_code "/vnc/ with the master password is served" 200 -u "$AUTH" "$BASE/vnc/"
expect_code "a wrong password is refused" 401 -u "user:wrong-password" "$BASE/vnc/"
UPGRADE=(-H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13'
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H 'Sec-WebSocket-Protocol: binary'
  -H "Origin: ${BASE}")
expect_code "the WebSocket upgrade without credentials is refused" 401 "${UPGRADE[@]}" "$BASE/websockify"
# curl never finishes an upgraded connection, so read the status line and stop.
WS_STATUS="$(curl -s --noproxy '*' --max-time 6 -i -N -u "$AUTH" "${UPGRADE[@]}" "$BASE/websockify" 2>/dev/null | head -1 | tr -d '\r')"
expect_match "the WebSocket upgrade with credentials switches protocols" '^HTTP/1.1 101' "$WS_STATUS"

STACK="$(dx cat /etc/sdw-desktop-stack 2>/dev/null)"
case "$STACK" in
  both | selkies)
    step "desktop (Selkies)"
    expect_code "/desktop/ without credentials is refused" 401 "$BASE/desktop/"
    expect_code "/desktop/ with the master password is served" 200 -u "$AUTH" "$BASE/desktop/"
    expect_match "Selkies stays up (not a crash loop)" '^desktop:desktop-selkies +RUNNING' "$(dx supervisorctl status 2>&1)"
    ;;
esac

step "sync service and remote self-destruct"
expect_code "the sync page is served" 200 "$BASE/sync/"
expect_match "the sync service reports ready with a key" '"state":"ready".*"sync_key_available":true' "$(curl -s --noproxy '*' --max-time 30 "$BASE/sync/api/doctor")"
expect_code "listing without proof of the key is refused" 401 "$BASE/sync/api/download"
expect_code "a wrong destruct passphrase is indistinguishable from no endpoint (404)" 404 \
  -X POST -H 'content-type: application/json' -d '{"passphrase":"nope"}' "$BASE/sync/api/destruct"
expect_code "a destruct request without a body is a 404 too" 404 -X POST "$BASE/sync/api/destruct"

finish
