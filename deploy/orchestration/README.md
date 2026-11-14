# Ephemeral authenticated workspaces

`sdw-host` provisions disposable workspaces behind Cloudflare Tunnel + Access. The files in this
directory are compatibility launchers of four lines; resource state, API parsing, rollback, PID
checks, secret transport and Docker execution live in the Rust crate `src/rust/host`.

## Commands

```bash
deploy/orchestration/up.sh <id> [port]       # edge + container
deploy/orchestration/down.sh <id>             # container/volumes + edge

deploy/orchestration/provision.sh <id> [port] # edge only
deploy/orchestration/teardown.sh <id>         # edge only
deploy/orchestration/run-tunnel.sh <id>       # foreground connector from saved state
```

The launcher uses `SDW_HOST_BIN` when set, then `src/rust/target/release/sdw-host`; if neither
exists it builds the release binary with Cargo. Use `make install-host` to place a reusable binary
at `~/.local/bin/sdw-host` for service installation or packaging.

An instance receives:

- a remotely managed tunnel and one-level DNS hostname;
- a Cloudflare Access self-hosted application;
- a human email allow policy;
- a machine service-token policy;
- its own Compose project, container, volumes and loopback origin port.

When `CF_TEAM_DOMAIN` is configured, the application's AUD and team domain are passed to the
workspace gate, so an Access-authenticated browser does not see a second login.

## Configuration

Copy `secrets.env.example` to `secrets.env`, fill it, and set mode 0600:

```bash
install -m 600 deploy/orchestration/secrets.env.example deploy/orchestration/secrets.env
```

Required values are `CF_API_TOKEN`, `CF_ACCOUNT_ID`, `CF_ZONE_ID`, `CF_ZONE_NAME` and
`ALLOW_EMAIL`. `WS_SUBDOMAIN` defaults to `ws`; `CF_TEAM_DOMAIN` enables SSO wiring.

The API token should be limited to the chosen account and zone, short-lived, and granted only:

- Cloudflare Tunnel · Edit
- Access Apps and Policies · Edit
- Access Service Tokens · Edit
- Zone DNS · Edit

## State and rollback

State is an atomic, mode-0600 JSON document at `instances/<id>.json`. It is updated after every
successful remote operation, so rollback and teardown consume what actually exists rather than
reconstructing it from shell variables. Legacy `instances/<id>.env` files remain readable for
migration and are removed after a successful teardown.

A failed provision deletes only resources already recorded. A failed container start removes its
Compose project and then rolls the edge back. If any deletion fails, state is retained and teardown
returns non-zero; rerun the same command instead of losing the identifiers needed for recovery.

Instance IDs accept only 1–40 ASCII letters, digits and internal hyphens. Ports are parsed as `u16`.
A free default port is selected by binding loopback instead of parsing `nc` output.

## Secret handling

- `secrets.env` is parsed as data and is never executed.
- Group/world-readable secrets and state files are refused.
- The Cloudflare API token is written to a temporary 0600 curl config and removed by RAII.
- JSON request bodies go through curl stdin.
- The tunnel token reaches only the connector's environment and never argv.
- Rust zeroizes the owned token/state strings when their structs drop.
- Connector teardown verifies the saved PID still belongs to a cloudflared command before sending
  SIGTERM to its dedicated process group.

## Verification

`src/tests/scripts/host-orchestration.test.sh` runs the complete lifecycle against fake Cloudflare,
Docker and cloudflared processes. It checks private typed state, token non-disclosure, partial API
rollback, container-start rollback, PID cleanup, unsafe ID rejection and secret-file modes. It does
not contact a real Cloudflare account.
