# Ephemeral authenticated workspaces (Cloudflare Access + Tunnel)

Batch, use-and-destroy workspace instances. Each gets its own Cloudflare Tunnel + one-level DNS
hostname + Access application, protected by **two** auth methods at the edge:

- **Human** — browser login (email one-time PIN), via an Access `allow` policy on your email.
- **Machine** — an Access `non_identity` policy bound to a per-instance **service token**
  (`CF-Access-Client-Id` / `CF-Access-Client-Secret` headers), for headless/automated clients.

Driven by the Cloudflare API (no browser, no Terraform state), so it fits batch automation.

## One command

```sh
./up.sh <id>        # provision the edge AND start a wired container; prints https://ws-<id>.<zone>/
./down.sh <id>      # stop + remove the container and its volumes, then tear down the edge
```

`up.sh` picks a free loopback port, runs `provision.sh`, and starts the workspace under a
per-instance compose project/name/volumes (`ws-<id>`) — so you can run several at once. Reverse
with `down.sh <id>` (use-and-destroy: `-v` wipes the instance's vault).

**Access single-sign-on (no second login behind Access):** when `CF_TEAM_DOMAIN` is set in
`secrets.env`, `up.sh` passes the provisioned app's AUD and your team domain to the container as
`ACCESS_AUD` / `ACCESS_TEAM_DOMAIN`. The workspace's auth gate then accepts the Access session
(the `Cf-Access-Jwt-Assertion` the edge injects, RS256-verified against the team JWKS) instead of
asking for its own login. Leave `CF_TEAM_DOMAIN` empty to skip SSO (the gate then prompts).

The lower-level primitives are still available: `provision.sh <id> [port]` (edge only — you run the
container yourself on `127.0.0.1:<port>`) and `teardown.sh <id>` (edge only).

## What you provide (once)

1. Fill `secrets.env` (copy from `secrets.env.example`; **gitignored** — never commit, never paste
   the token into chat). Needs `CF_API_TOKEN`, `CF_ACCOUNT_ID`, `CF_ZONE_ID`, `CF_ZONE_NAME`,
   `WS_SUBDOMAIN`, `ALLOW_EMAIL`; optional `CF_TEAM_DOMAIN` (enables SSO wiring).
2. API token with exactly these permissions (short expiry; delete after use):
   - Account · Cloudflare Tunnel · Edit
   - Account · Access: Apps and Policies · Edit
   - Account · Access: Service Tokens · Edit
   - Zone · DNS · Edit
3. **Zero Trust onboarded once**: dash.cloudflare.com → Zero Trust → team name → Free plan (email
   OTP needs no identity provider).
4. `cloudflared` installed on the host.

## Security

- The token can edit tunnels, Access and DNS on your account/zone. Scope it to the one zone, give
  it a short TTL, delete it when done.
- `secrets.env` and `instances/` (per-instance state, incl. the tunnel token and service-token
  secret, `chmod 600`) are gitignored; the scripts never echo secrets.

## Verified (live, against a real account)

- Unauthenticated → **blocked by Access** at the edge (302 to the team login), never reaches the
  container. Covers `/`, `/vnc/`, `/sync/` and the WebSocket upgrades (the app covers the whole
  hostname).
- Machine: `curl -H "CF-Access-Client-Id: …" -H "CF-Access-Client-Secret: …"` → passes Access, and
  with SSO wired the workspace gate accepts the Access assertion (no `/auth/login`); a wrong
  `ACCESS_AUD` is rejected by the origin (proves it verifies, not trusts the header).
- `down.sh` → container, volumes and every edge object removed; no connector left running.
