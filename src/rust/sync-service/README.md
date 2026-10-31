# sync-service (Rust / Axum)

The tunnel-exposed file-sync daemon, in Rust: one small, memory-safe static
binary with no runtime, speaking the exact wire protocol and HTTP contract of the
TypeScript service it replaces (`packages/server` + `packages/core/sync`).

It began as a proof of concept (does a Rust daemon fit without breaking the wire
protocol the browser SPA shares?). That question is answered; this is now the
full surface, and the TS service is what it is checked against.

## What it serves

Caddy proxies all of `/sync/*` here (`:8081`).

| Route | Behaviour |
|-------|-----------|
| `GET /sync/` , `GET /sync/assets/*` | the compiled SPA (served with correct MIME; a later step can move this to Caddy) |
| `POST /sync/api/upload` | `put-chunk` / `put-finalize`, authenticated-then-stored, replay-protected |
| `GET /sync/api/download` | authenticated directory listing only (no file content) |
| `GET /sync/api/doctor` | `disabled` / `vault-locked` / `ready`, same JSON as the TS route |
| `POST /sync/api/destruct` | remote self-destruct: feature-gated, rate-limited, bcrypt-verified |

`SYNC_ENABLED` and `REMOTE_DESTRUCT_ENABLED` gate the API and the destruct route;
a feature that is off answers 404, indistinguishable from a route that is absent.
Filesystem work runs on blocking threads (`spawn_blocking`). The WebSocket
`/sync/api/events` channel is not reimplemented: it had no sender and no browser
consumer.

The destruct route keeps the irreversible step behind a trait. In production the
trigger launches the existing `self-destruct` CLI (which shreds the keys, stops
every service and SIGTERMs PID 1), so the destruction logic lives in one place
during the migration; tests use a fake trigger and never cross the boundary.

## How the two copies are kept honest

The browser SPA stays TypeScript (File System Access API + WebCrypto), so the
shared wire logic exists twice. Two seams pin them together:

- **`tests/parity.rs`** replays golden vectors from the authoritative TS side
  (`bun run packages/server/scripts/gen-wire-vectors.ts`): HKDF, AES-256-GCM,
  AAD framing, the PNG body, the list authenticator, path validation.
- **`tests/bcrypt_parity.rs`** pins that the `bcrypt` crate verifies hashes
  `Bun.password` produced — the remote destruct depends on it.
- **`tests/routes.rs`** drives the HTTP surface (gates, doctor shape, listing
  auth, a real encrypted chunk+finalize through the store, and the whole destruct
  decision path) to the same status codes `tests/smoke` asserts on the real image.

If either side drifts, a test fails.

## Run

```bash
cargo test                 # parity + bcrypt + routes + units
cargo clippy --all-targets -- -D warnings
cargo build --release      # ~0.8 MB static binary
cargo run                  # expects the vault-backed container paths to exist
```

## Shipped in the image

`image/Dockerfile` builds this crate in a musl `rust-builder` stage and copies the
static binary to `/usr/local/bin/sync-service`; `image/supervisord.conf` runs it in
place of `bun run .../server`. Validated by the smoke suite (checks 10/20/30/55/80
pass 93/0, including a real-browser upload into the vault and remote destruct). The
TS `packages/server` stays only as the parity source for `gen-wire-vectors.ts`. See
`deploy/docs/rust-migration.md`.
