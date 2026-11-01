# sync-service (Rust / Axum)

The tunnel-exposed file-sync daemon, in Rust: part of the `sdw` multi-call binary, run as
`sdw sync-service`. It speaks the wire protocol and HTTP contract the browser SPA expects and
replaced an earlier TypeScript service (`packages/server`), which has since been removed.

## What it serves

Caddy serves the compiled SPA statically and reverse-proxies only `/sync/api/*` here (`:8081`).

| Route | Behaviour |
|-------|-----------|
| `POST /sync/api/upload` | `put-chunk` / `put-finalize`, authenticated-then-stored, replay-protected |
| `GET /sync/api/download` | authenticated directory listing only (no file content) |
| `GET /sync/api/doctor` | `disabled` / `vault-locked` / `ready`; gated behind Caddy basic-auth |
| `POST /sync/api/destruct` | remote self-destruct: feature-gated, rate-limited, bcrypt-verified |

`SYNC_ENABLED` and `REMOTE_DESTRUCT_ENABLED` gate the API and the destruct route; a feature that is
off answers 404, indistinguishable from a route that is absent. Filesystem work runs on blocking
threads (`spawn_blocking`).

The destruct route keeps the irreversible step behind a trait. In production the trigger launches
the `self-destruct` CLI (which shreds the keys, stops every service and SIGTERMs PID 1), so the
destruction logic lives in one place; the route answers 500 if that launch fails, and tests use a
fake trigger that never crosses the boundary.

## How the two copies are kept honest

The browser SPA stays TypeScript (File System Access API + WebCrypto), so the shared wire logic
exists twice. The seams:

- **`tests/parity.rs`** checks the Rust side against a FROZEN golden-vector snapshot
  (`testdata/wire-vectors.json`) of the original TS server: HKDF, AES-256-GCM, AAD framing, the PNG
  body, the list authenticator, path validation. The generator was retired with `packages/server`,
  so the vectors cannot be regenerated here; this guards the Rust side from regressing but does not
  observe the live TS client.
- **`tests/bcrypt_parity.rs`** pins that the `bcrypt` crate verifies hashes `Bun.password`
  produced — the remote destruct depends on it.
- **`tests/routes.rs`** drives the HTTP surface (gates, doctor shape, listing auth, a real
  encrypted chunk+finalize through the store, and the whole destruct decision path).

The authoritative end-to-end cross-language check is the smoke suite (`src/tests/smoke`,
`make smoke`), which boots the image and drives the real browser SPA against this daemon. Unit
parity alone cannot catch a change made only on the live TS side.

## Run

```bash
cargo test                 # parity + bcrypt + routes + units
cargo clippy --all-targets -- -D warnings
cargo build --release
```

## Shipped in the image

`image/Dockerfile` builds the `sdw` binary in a musl `rust-builder` stage;
`image/supervisord.conf` runs the daemon as `sdw sync-service`. Validated by the smoke suite
(a real-browser upload into the vault and remote destruct). See `deploy/docs/rust-migration.md`.
