#!/usr/bin/env bash
# Who gets in, through the published port, exactly as a browser or the tunnel would.
#
# Every route except /auth/* and POST /sync/api/destruct sits behind the ingress auth gateway: Caddy
# asks the sync-service (forward_auth -> /auth/verify) and sends a request without a valid session
# cookie to the login page. code-server and KasmVNC have no login of their own: the gate is the only
# one, so no copy of the master password sits in their configuration.
# shellcheck source=../lib.sh
. "$(cd "$(dirname "$0")/.." && pwd)/lib.sh"
AUTH="user:${SMOKE_PASSWORD}"

# location_of <curl args...>: the Location header of the response ("" when there is none).
location_of() {
  curl -s --noproxy '*' --max-time 30 -o /dev/null -D - "$@" | tr -d '\r' | sed -n 's/^[Ll]ocation: //p' | head -1
}

step "the gate, without a session"
expect_code "the root redirects to the login page" 302 "$BASE/"
expect_match "… carrying the page to come back to" '^/auth/login\?next=' "$(location_of "$BASE/")"
expect_code "the login page itself is open" 200 "$BASE/auth/login"
expect_code "the sync page redirects" 302 "$BASE/sync/"
expect_code "a gated API route redirects too" 302 "$BASE/sync/api/doctor"
expect_code "the desktop redirects" 302 "$BASE/vnc/"
expect_code "a forged session cookie is no session" 302 -H 'Cookie: sdw_session=v1.1.2.deadbeef' "$BASE/"
expect_code "HTTP Basic credentials alone do not open the gate" 302 -u "$AUTH" "$BASE/"
expect_code "the gate's own check says 401 without a session" 401 "$BASE/auth/verify"
UPGRADE=(-H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13'
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H 'Sec-WebSocket-Protocol: binary'
  -H "Origin: ${BASE}")
expect_code "a WebSocket upgrade without a session is turned away" 302 "${UPGRADE[@]}" "$BASE/websockify"

step "logging in"
expect_code "a wrong password is refused" 401 -X POST --data-urlencode "password=not-the-password" "$BASE/auth/login"
LOGIN="$(curl -s --noproxy '*' --max-time 30 -i -X POST --data-urlencode "password=${SMOKE_PASSWORD}" \
  --data-urlencode "next=/sync/" "$BASE/auth/login" | tr -d '\r')"
expect_match "the right password redirects (303) to where the user was going" '^HTTP/[0-9.]+ 303' "$LOGIN"
expect_match "… to the requested page" '^[Ll]ocation: /sync/' "$LOGIN"
expect_match "the session cookie is HttpOnly" '^[Ss]et-[Cc]ookie: sdw_session=.*HttpOnly' "$LOGIN"
expect_match "… and SameSite=Lax" '^[Ss]et-[Cc]ookie: sdw_session=.*SameSite=Lax' "$LOGIN"
for evil in '//evil.example/x' '/\evil.example'; do
  expect_eq "a next that could leave the site ($evil) falls back to /" "/" \
    "$(location_of -X POST --data-urlencode "password=${SMOKE_PASSWORD}" --data-urlencode "next=$evil" "$BASE/auth/login")"
done
PAGE_HEADERS="$(curl -s --noproxy '*' --max-time 30 -D - -o /dev/null "$BASE/auth/login" | tr -d '\r')"
expect_match "the login page cannot be framed" '^[Xx]-[Ff]rame-[Oo]ptions: DENY' "$PAGE_HEADERS"
expect_match "… and is not cached" '^[Cc]ache-[Cc]ontrol: no-store' "$PAGE_HEADERS"

step "with a session"
GATE="$(gate_login)"
GATE_HDR="Cookie: $GATE"
expect_match "the login gave a session cookie" '^sdw_session=v1\.' "$GATE"
expect_code "the gate's own check now says 200" 200 -H "$GATE_HDR" "$BASE/auth/verify"
step "response headers"
SYNC_HEADERS="$(curl -s --noproxy '*' --max-time 30 -D - -o /dev/null -H "$GATE_HDR" "$BASE/sync/" | tr -d '\r')"
expect_match "the sync page has a CSP that allows scripts only from itself" \
  "^[Cc]ontent-[Ss]ecurity-[Pp]olicy: .*script-src 'self'" "$SYNC_HEADERS"
expect_match "… and cannot be framed (it holds the self-destruct form)" '^[Xx]-[Ff]rame-[Oo]ptions: DENY' "$SYNC_HEADERS"
expect_match "… with MIME sniffing off" '^[Xx]-[Cc]ontent-[Tt]ype-[Oo]ptions: nosniff' "$SYNC_HEADERS"
expect_no_match "the server does not announce itself" '^[Ss]erver:' "$SYNC_HEADERS"
BACKEND_HEADERS="$(curl -s --noproxy '*' --max-time 30 -D - -o /dev/null -H "$GATE_HDR" "$BASE/" | tr -d '\r')"
expect_match "what a backend serves gets the baseline too" '^[Rr]eferrer-[Pp]olicy: same-origin' "$BACKEND_HEADERS"
expect_match "… sniffing off" '^[Xx]-[Cc]ontent-[Tt]ype-[Oo]ptions: nosniff' "$BACKEND_HEADERS"
expect_no_match "no HSTS promise over plain HTTP" '^[Ss]trict-[Tt]ransport-[Ss]ecurity:' "$BACKEND_HEADERS"
# What Cloudflare's edge adds to a request that came in over HTTPS.
HTTPS_EDGE=(-H 'X-Forwarded-Proto: https' -H 'CF-Visitor: {"scheme":"https"}')
HTTPS_HEADERS="$(curl -s --noproxy '*' --max-time 30 -D - -o /dev/null -H "$GATE_HDR" "${HTTPS_EDGE[@]}" "$BASE/" | tr -d '\r')"
expect_match "behind an HTTPS edge it is sent" '^[Ss]trict-[Tt]ransport-[Ss]ecurity: max-age=[0-9]+' "$HTTPS_HEADERS"
HTTPS_LOGIN="$(curl -s --noproxy '*' --max-time 30 -i -X POST "${HTTPS_EDGE[@]}" \
  --data-urlencode "password=${SMOKE_PASSWORD}" "$BASE/auth/login" | tr -d '\r')"
expect_match "and the session cookie is Secure" '^[Ss]et-[Cc]ookie: sdw_session=.*; Secure' "$HTTPS_LOGIN"
expect_no_match "while on plain HTTP it is not (Safari would not keep it on localhost)" \
  '^[Ss]et-[Cc]ookie: sdw_session=.*Secure' "$LOGIN"

step "WebSocket origin"
CROSS=(-H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13'
  -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H 'Sec-WebSocket-Protocol: binary'
  -H 'Origin: https://other.example.com')
expect_code "a page on another origin cannot open a socket with the visitor's session (403)" 403 \
  -H "$GATE_HDR" "${CROSS[@]}" "$BASE/websockify"
expect_code "… nor can the same handshake ask the gate directly" 403 -H "$GATE_HDR" \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Origin: null' "$BASE/auth/verify"
expect_code "the page's own origin is let through (the socket opens)" 101 \
  --max-time 6 -H "$GATE_HDR" "${UPGRADE[@]}" "$BASE/websockify"
expect_code "a request without a WebSocket upgrade is not an origin question" 200 \
  -H "$GATE_HDR" -H 'Origin: https://other.example.com' "$BASE/auth/verify"

step "code-server (behind the gate, with no password of its own)"
expect_code "the editor is served on the session alone" 200 -L -H "$GATE_HDR" "$BASE/"
expect_match "… it is the editor, not a login page" 'vscode|workbench' \
  "$(curl -sL --noproxy '*' --max-time 30 -H "$GATE_HDR" "$BASE/" | head -c 20000)"
expect_no_match "there is no second password prompt" 'name="password"' \
  "$(curl -sL --noproxy '*' --max-time 30 -H "$GATE_HDR" "$BASE/" | head -c 20000)"

step "desktop (KasmVNC, behind the gate, with no password of its own)"
expect_code "/vnc/ without a session is sent to the login" 302 "$BASE/vnc/"
expect_code "/vnc/ with the session is served, with no second prompt" 200 -H "$GATE_HDR" "$BASE/vnc/"
expect_code "the WebSocket without a session is turned away" 302 "${UPGRADE[@]}" "$BASE/websockify"
# curl never finishes an upgraded connection, so read the status line and stop.
WS_STATUS="$(curl -s --noproxy '*' --max-time 6 -i -N -H "$GATE_HDR" "${UPGRADE[@]}" "$BASE/websockify" 2>/dev/null | head -1 | tr -d '\r')"
expect_match "the WebSocket upgrade with a session switches protocols" '^HTTP/1.1 101' "$WS_STATUS"
expect_false "no KasmVNC password file exists" dx sh -c 'ls /workspace/.desktop-vnc/.kasmpasswd'
STACK="$(dx cat /etc/sdw-desktop-stack 2>/dev/null)"
case "$STACK" in
  both | selkies)
    # Selkies 2.x serves its client, its API and its WebSocket from one server on loopback, under
    # /desktop. Its own Basic auth is off (it would refuse to start with it on and no password), so
    # everything it answers is behind the session gate and nothing else.
    step "desktop (Selkies, behind the gate, with no password of its own)"
    SEL_UPGRADE=(-H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13'
      -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' -H "Origin: ${BASE}")
    expect_code "/desktop/ without a session redirects" 302 "$BASE/desktop/"
    expect_code "/desktop/ with a session is served" 200 -H "$GATE_HDR" "$BASE/desktop/"
    expect_no_match "… with no second prompt" '^[Ww]ww-[Aa]uthenticate:' \
      "$(curl -s --noproxy '*' --max-time 30 -D - -o /dev/null -H "$GATE_HDR" "$BASE/desktop/" | tr -d '\r')"
    expect_code "its API without a session redirects" 302 "$BASE/desktop/api/sessions"
    expect_code "its WebSocket without a session is turned away" 302 "${SEL_UPGRADE[@]}" "$BASE/desktop/api/websockets"
    SEL_WS_STATUS="$(curl -s --noproxy '*' --max-time 6 -i -N -H "$GATE_HDR" "${SEL_UPGRADE[@]}" "$BASE/desktop/api/websockets" 2>/dev/null | head -1 | tr -d '\r')"
    expect_match "… with a session it switches protocols" '^HTTP/1.1 101' "$SEL_WS_STATUS"
    expect_code "a page on another origin cannot open it with the visitor's session (403)" 403 \
      -H "$GATE_HDR" "${CROSS[@]}" "$BASE/desktop/api/websockets"
    expect_match "Selkies stays up (not a crash loop)" '^desktop:desktop-selkies +RUNNING' "$(dx supervisorctl status 2>&1)"
    # The kernel's own table of listening sockets (the image has no ss or netstat): state 0A is
    # LISTEN, 17C0 is port 6080, and 0100007F is 127.0.0.1 (little-endian). Any other address would
    # list as something else, and the assertion compares the whole list.
    expect_eq "Selkies listens on 127.0.0.1:6080 and on no other address" "0100007F:17C0" \
      "$(dx awk '$4 == "0A" && $2 ~ /:17C0$/ { print $2 }' /proc/net/tcp /proc/net/tcp6)"
    ;;
esac

step "sync service and remote self-destruct"
expect_code "the sync page is served" 200 -H "$GATE_HDR" "$BASE/sync/"
expect_match "the sync service reports ready with a key" '"state":"ready".*"sync_key_available":true' \
  "$(curl -s --noproxy '*' --max-time 30 -H "$GATE_HDR" "$BASE/sync/api/doctor")"
expect_code "listing without proof of the key is refused by the service itself" 401 -H "$GATE_HDR" "$BASE/sync/api/download"
expect_code "metrics are off unless METRICS_ENABLED (404)" 404 -H "$GATE_HDR" "$BASE/sync/api/metrics"
# The self-destruct endpoint is the break-glass route: public (no session) on purpose, guarded by its
# own passphrase, rate limit and 404-for-everything-else.
expect_code "a wrong destruct passphrase is indistinguishable from no endpoint (404)" 404 \
  -X POST -H 'content-type: application/json' -d '{"passphrase":"nope"}' "$BASE/sync/api/destruct"
expect_code "a destruct request without a body is a 404 too" 404 -X POST "$BASE/sync/api/destruct"

step "logging out"
LOGOUT="$(curl -s --noproxy '*' --max-time 30 -i -H "$GATE_HDR" -X POST "$BASE/auth/logout" | tr -d '\r')"
expect_match "logout redirects to the login page" '^[Ll]ocation: /auth/login' "$LOGOUT"
expect_match "… and clears the cookie" '^[Ss]et-[Cc]ookie: sdw_session=; .*Max-Age=0' "$LOGOUT"
finish
