# Ephemeral authenticated workspaces (Cloudflare Access + Tunnel) — PoC

Batch/automated, use-and-destroy workspace instances. Each instance gets its own Cloudflare
Tunnel + DNS hostname + Access application, protected by **two** auth methods at the edge:

- **Human** — browser login (email one-time PIN), via an Access `Allow` policy on your email.
- **Machine** — an Access **Service Auth** policy bound to a per-instance service token
  (`CF-Access-Client-Id` / `CF-Access-Client-Secret` headers), for headless/automated clients.

`provision.sh <id>` creates everything and starts the container; `teardown.sh <id>` destroys the
edge objects and the container (use-and-destroy). Driven by the Cloudflare API (no browser, no
Terraform state) so it fits batch automation.

## What you provide (once)

1. Fill `secrets.env` (copy from `secrets.env.example`; it is **gitignored** — never commit, never
   paste the token into chat). Needs: `CF_API_TOKEN`, `CF_ACCOUNT_ID`, `CF_ZONE_ID`,
   `CF_ZONE_NAME`, `WS_SUBDOMAIN`, `ALLOW_EMAIL`.
2. Create the API token with exactly these permissions (short expiry; delete after the PoC):
   - Account · Cloudflare Tunnel · Edit
   - Account · Access: Apps and Policies · Edit
   - Account · Access: Service Tokens · Edit
   - Zone · DNS · Edit
3. **Zero Trust must be onboarded once**: dash.cloudflare.com → Zero Trust → pick a team name →
   Free plan. (Email OTP needs no identity provider.) Tell me your team name is set.
4. `cloudflared` installed on this host (already present here).

## Security

- The token can edit tunnels, Access, and DNS on your account/zone. Scope it to the one zone,
  give it a short TTL, and delete it when the PoC is done.
- `secrets.env` and `instances/` (per-instance state, including the service-token secret) are
  gitignored. The scripts never echo secrets.

## Validation (what the PoC must prove)

For one provisioned instance `inst-abc.<ws>.<domain>`:
- Unauthenticated request → **blocked by Access** (302 to the login page), never reaches the container.
- Human: browser → Access email-OTP login → then the container's own login (code-server / VNC).
- Machine: `curl -H "CF-Access-Client-Id: …" -H "CF-Access-Client-Secret: …"` → passes Access,
  reaches the container (no browser login).
- `/`, `/vnc/`, `/sync/` are all behind Access (the app covers the whole hostname, not just `/`).
- `teardown.sh` → the hostname stops resolving / returns no Access app; container and volumes gone.

> Status: scaffolding. The Cloudflare calls are validated live against a real account during the
> PoC (they cannot be exercised offline). Scripts are added and iterated here as that proceeds.
