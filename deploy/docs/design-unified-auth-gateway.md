# Design — Unified Authentication Gateway

Status: IMPLEMENTED (the text below is the design as it was proposed, kept for the reasoning;
"today" in section 1 means before it). Since then the backends dropped their own passwords as well:
code-server runs with `--auth none` and KasmVNC with `-DisableBasicAuth`, so the gate is the only
login and there is no second prompt. A WebSocket handshake whose `Origin` is another site is refused
even with a valid cookie. The behaviour is checked by `src/tests/smoke/checks/20-auth.sh`.
Selkies was moved to 2.x on 2026-11-04 (the 1.x line was deleted upstream): it now serves its
client, API and WebSocket itself under `/desktop/` (the socket is `/desktop/api/websockets`, not
`/desktop/websockets` as in the text below), with its own Basic auth switched off. The argument is
unchanged: the gate is the only login, and it covers every path under `/desktop/`.

Supersedes the per-service auth patchwork at the Caddy ingress.

## 1. Problem — the disease, not the symptom

The workspace publishes several HTTP/WebSocket services through one Caddy ingress
on `:8080`, and today **each backend authenticates itself, independently and
inconsistently**:

| Route | Backend | Auth today | Covers its WebSocket? |
|-------|---------|-----------|------------------------|
| `/` | code-server :8082 | code-server's own password | yes (its own) |
| `/vnc/`, `/websockify` | KasmVNC :6081 | KasmVNC native basic-auth, end-to-end | yes (its own) |
| `/desktop/`, `/desktop/websockets` | Selkies :6080 | Caddy basic-auth on the **static page only** | **NO — WS is open** |
| `/sync/` (SPA) | Caddy file_server | none (page is open by design) | n/a |
| `/sync/api/*` | sync-service :8081 | per-endpoint (derived sync key / destruct passphrase) | n/a |
| `/sync/api/doctor` | sync-service :8081 | Caddy basic-auth | n/a |

Caddy is effectively **allow-by-default**: a route reaches its backend and the
backend is trusted to guard itself. The Selkies WebSocket hole (anyone who reaches
the tunnel can open `/desktop/websockets` and get the live desktop + input with no
password) is not a one-off bug — it is the **predictable failure mode of this
model**. The next service added will probably leak the same way.

Root cause of *why* it fragmented: **HTTP Basic Auth cannot ride a browser
WebSocket.** The `WebSocket` API forbids custom headers and browsers do not replay
basic-auth credentials to a JS-initiated handshake (a 401 there just yields a blank
stream). So every streaming service had to invent its own in-band auth, and the one
that didn't (Selkies in `--mode=websockets`) is wide open.

## 2. Goals / non-goals

Goals:
- **One** authentication boundary in front of **every** route, HTTP and WebSocket
  alike. Deny-by-default.
- Backends become trusted-internal; the perimeter never depends on a backend
  remembering to guard itself.
- Works across **all** deployment modes (Cloudflare Tunnel, frp, Tailscale, raw
  localhost, linux-systemd) — not tied to any one transport or to Cloudflare.
- Reuse the single master password that already unifies code-server / desktops /
  vault. One login for the whole workspace.
- Automation-friendly (a machine client presents a token / Access service token).

Non-goals (for this iteration):
- Multi-user / per-user identities, RBAC. Single shared master password stays the
  model.
- Replacing the vault passphrase or the destruct mechanism.
- TOTP/OIDC/SSO as an origin feature (delegated to Cloudflare Access at the edge if
  wanted — see §7).

## 3. Design overview

Introduce a **session-cookie authentication gate** terminated once at the ingress:

- A browser authenticates **once** with the master password and receives a signed,
  `HttpOnly` **session cookie**.
- Caddy is flipped to **deny-by-default**: a single `forward_auth` check runs in
  front of every route and consults the gate before any backend is reached.
- **The cookie rides WebSockets.** Unlike an `Authorization` header, a same-origin
  cookie IS sent on the WS upgrade handshake, so the *same* gate covers
  `/desktop/websockets`, `/websockify`, code-server's WS, everything. The Selkies
  hole closes as a consequence of the perimeter, not as a special case.
- Only a small, explicit **allowlist** stays public (the login endpoints and the
  break-glass destruct endpoint).

This is the standard authenticating-reverse-proxy / zero-trust pattern (what
Cloudflare Access, oauth2-proxy, and Authelia implement); we build the minimal form
of it into the service that is already always-on.

```
                         :8080  Caddy (deny-by-default)
  browser ──cookie──▶ ┌─────────────────────────────────────────────┐
                      │  forward_auth ──▶ sync-service /auth/verify   │
                      │        │ 2xx → pass            401 → /auth/login
                      │        ▼                                      │
                      │  /            → code-server :8082             │
                      │  /vnc/ , /websockify      → KasmVNC :6081     │
                      │  /desktop/ , /desktop/websockets → Selkies    │
                      │  /sync/ , /sync/api/*     → sync-service :8081 │
                      │  ── allowlist (no gate) ──                    │
                      │  /auth/*                  → sync-service       │
                      │  /sync/api/destruct       → sync-service       │
                      └─────────────────────────────────────────────┘
```

## 4. The gate — endpoints (in the always-on `sdw sync-service`)

`sync-service` already runs under supervisord with `autostart=true`
(independent of `SYNC_ENABLED`), so it is the natural always-on home. Add an
`/auth` router; no new process, no new port.

- `GET  /auth/login?next=<path>` — a minimal HTML login form (one password field).
- `POST /auth/login` — verify the submitted password; on success `Set-Cookie` and
  302 to `next` (default `/`).
- `GET  /auth/verify` — the `forward_auth` target. 200 if the request carries a
  valid session (cookie signature + not expired) **or** a valid edge assertion
  (§7); 401 otherwise. No request body is proxied; Caddy only forwards headers.
- `POST /auth/logout` — clear the cookie.

Password verification reuses the existing credential: bcrypt-compare the submitted
password against `DESKTOP_BCRYPT_HASH` (already derived from the master password by
the entrypoint and inherited through supervisord), or equivalently against the
master password the service can read. No new secret store.

## 5. Session cookie

- Name `sdw_session`; attributes `HttpOnly; Secure; SameSite=Strict; Path=/`.
- Value: `base64url(payload) "." base64url(HMAC-SHA256(key, payload))`, where
  `payload = {iat, exp}` (compact; no PII, single-user). Constant-time compare on
  verify.
- Signing key: a **32-byte random secret generated at boot, kept in memory only**
  (consistent with `design-memory-only-secrets.md`). Consequence: a container
  restart invalidates sessions → re-login. Acceptable, and arguably desirable.
  (Alternative for review: derive the key from the master password via HKDF-SHA256,
  as `derive_destruct_key` already does, so sessions survive a restart. Trade-off:
  persistence vs. forward-secrecy on restart.)
- Lifetime: `exp = iat + 12h` (sliding-renew optional). For review.
- Crypto: `hmac` + `sha2` (HKDF/SHA256 already vendored; add the `hmac` MAC crate).

## 6. Caddy — deny-by-default policy

Replace the current "each handler guards itself" layout with one gate plus an
explicit allowlist. Sketch:

```caddyfile
:8080 {
    # Public allowlist FIRST (most specific), no gate:
    @public path /auth/* /sync/api/destruct
    handle @public {
        reverse_proxy 127.0.0.1:8081
    }

    # Everything else: one gate, then route as before.
    forward_auth 127.0.0.1:8081 {
        uri /auth/verify
        copy_headers Set-Cookie     # pass refreshed cookie back
        # on 401, redirect browsers to the login page:
        @denied status 401
        handle_response @denied {
            redir * /auth/login?next={http.request.uri}
        }
    }
    import /etc/caddy/desktop.d/*.caddy   # /vnc/, /desktop/ (+ their WS)
    handle /sync/api/* { reverse_proxy 127.0.0.1:8081 }
    handle_path /sync/* { root * /opt/sdw-web; file_server }
    handle { reverse_proxy 127.0.0.1:8082 }   # code-server
}
```

`forward_auth` runs on the WS **upgrade GET** too, so the gate covers
`/desktop/websockets` and `/websockify` with no per-snippet auth. The desktop
snippets drop their own `basic_auth`; Selkies needs no change at all.
(Validation item: confirm `forward_auth` is in the pinned standard Caddy build — it
is a core directive; if a route's `handle_response` redirect proves awkward, fall
back to an explicit `reverse_proxy /auth/verify` matcher.)

## 7. Edge unification with Cloudflare Access (IMPLEMENTED, opt-in)

When the deployment is behind Cloudflare Access (the ephemeral-orchestration model),
Access already authenticates every request at the edge. To avoid a double login,
`/auth/verify` **also accepts a valid Access assertion**: verify the
`Cf-Access-Jwt-Assertion` JWT against the team's public keys (aud = the app's AUD,
exp valid). So:

- Behind Access → the Access session satisfies the gate (SSO, device posture, the
  dual human/service-token model from `deploy/orchestration/`).
- No Access (local / frp / Tailscale / systemd) → the origin session cookie
  satisfies the gate.

One gate, two accepted credentials, every transport. The origin gate is the source
of truth and always present; Access is an additive outer layer, never a requirement.
(Note: trusting `Cf-Access-Jwt-Assertion` is safe only because the origin is bound
to `127.0.0.1` and reachable solely through the tunnel, so a client cannot inject
the header. This is already true; keep it as an invariant.)

## 8. Backends become internal

- Selkies: no change — the gate protects its WS.
- KasmVNC / code-server: keep their own auth as **defense-in-depth** (recommended),
  at the cost of one extra native prompt (status quo for KasmVNC). Decision for
  review: optionally turn code-server/KasmVNC auth off for true single-sign-on,
  trading depth for UX. Default recommendation: keep depth.
- All backends stay bound to `127.0.0.1` (unchanged).

## 9. Public allowlist & rationale

- `/auth/*` — the login/verify/logout surface must be reachable to log in.
- `/sync/api/destruct` — **break-glass**: remote self-destruct must work precisely
  when you have lost normal access; it self-authenticates with the destruct
  passphrase (unchanged).
- Decision for review: the `/sync/` SPA and sync-key-authenticated `/sync/api/*`
  (upload/list) — gate them behind the session (cleaner perimeter; a human with the
  master password is logged in anyway) **or** keep open as today (they self-auth
  with the derived sync key). Recommendation: gate them; keep only destruct public.

## 10. Threat model delta (update `threat-model.md`)

- Closes: unauthenticated Selkies desktop stream + input over any exposed ingress;
  unauthenticated `doctor` recon; and the general "a new service forgets to guard
  itself" class.
- Perimeter is now one auditable, deny-by-default gate for HTTP and WS.
- Residual (unchanged, acceptable): the gate protects the ingress, not the
  `127.0.0.1` loopback inside the container — a process already in the container can
  reach a backend directly. Mitigated by least-privilege, no inbound ports, and
  tunnel-only exposure. The in-container-process threat is already covered by the
  container/microVM isolation discussion in the threat model.

## 11. Rollout / migration

- Build-time, no runtime config churn: new `/auth` routes compiled into
  `sync-service`; Caddyfile + desktop snippets edited; one image rebuild.
- Backwards compatible: the master password is unchanged; users just see a single
  login page instead of per-service browser prompts.
- Fail-closed by construction: deny-by-default means a mis-edited route is *closed*,
  not open (the opposite of today). If the gate (`sync-service`) is down, the
  workspace is locked, not exposed — matches the vault's fail-closed stance.
- `cargo` gate (fmt/clippy/test) + `bun` gate stay green; add tests below.

## 12. Testing / smoke

Unit (sync-service):
- cookie sign → verify round-trips; tampered signature / expired `exp` → reject;
  constant-time compare.
- `POST /auth/login` right/wrong password → set-cookie / 401.
- `/auth/verify`: valid cookie → 200; none/invalid → 401; valid Access JWT → 200;
  forged `Cf-Access-Jwt-Assertion` → 401.

Smoke (live container, the real proof):
- Unauthenticated `GET` of `/`, `/vnc/`, `/desktop/`, `/sync/`, and the **WS
  upgrades** `/desktop/websockets` + `/websockify` → all 401/redirect (no backend
  reached).
- After `POST /auth/login`, the cookie **rides the WS**: `/desktop/websockets`
  upgrades 101 and streams; `/websockify` connects.
- `/sync/api/destruct` reachable without the session (break-glass intact).
- Behind Access: unauth → Access login; with Access session → gate passes, no second
  login.

## 13. Open decisions (need your call)

1. Session key: in-memory per-boot (re-login after restart) vs. HKDF-from-master
   (survives restart). Recommend in-memory.
2. Gate the `/sync/` SPA + sync API, or keep them open as today. Recommend gate
   (destruct stays open).
3. Keep backend-native auth as defense-in-depth (extra prompt) vs. single-sign-on by
   turning it off. Recommend keep depth.
4. [DONE] Access-JWT unification (§7) is implemented: `/auth/verify` also accepts a valid
   `Cf-Access-Jwt-Assertion` (RS256 verified against the team JWKS) when `ACCESS_TEAM_DOMAIN`
   + `ACCESS_AUD` are set. Opt-in; unset = cookie-only, the header is never trusted.
