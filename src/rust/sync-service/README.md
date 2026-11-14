# sync-service (Rust / Axum)

The loopback HTTP adapter for the unified auth gateway and encrypted browser upload protocol. It is
linked into the `sdw` multi-call binary and run as `sdw sync-service`; Caddy is the only published
listener and forwards `/auth/*` and `/sync/api/*` to `127.0.0.1:8081`.

## Routes

| Route | Behaviour |
|---|---|
| `GET/POST /auth/*` | Login, session verification and logout for the unified ingress gate |
| `POST /sync/api/upload` | AEAD chunk/finalize, replay window, strict chunk ordering and atomic commit |
| `GET /sync/api/download` | Key-authenticated metadata listing only; never returns file content |
| `GET /sync/api/doctor` | `disabled`, `vault-locked` or `ready` |
| `GET /sync/api/metrics` | Optional authenticated Prometheus counters |
| `POST /sync/api/destruct` | Feature-gated, rate-limited trigger; `202 scheduled`, never a premature completion claim |

`SYNC_ENABLED`, `REMOTE_DESTRUCT_ENABLED` and `METRICS_ENABLED` default off. Filesystem work runs
through `spawn_blocking`. Requests for one upload id share a sequencer that prevents concurrent
append/finalize races and enforces chunk indices; different uploads remain concurrent.

The service resolves its configuration before creating Tokio worker threads, then removes inherited
container variables other than process/TLS/proxy plumbing. A remote-destruction worker reloads
operator configuration from root-only PID 1. The process still runs as container root because the
current upload path and destroy trigger cross root-owned vault/runtime boundaries; splitting it into
an unprivileged HTTP process and a narrow privileged broker remains a hardening opportunity.

## One Rust protocol

The browser (`sdw-web`) and native smoke probe (`sdw-smoke-client`) both construct requests with
`sdw-core::sync::client`. This service consumes the matching Rust parser, crypto, constants and path
rules. Frozen vectors in `testdata/` preserve compatibility with previously shipped envelopes;
`tests/routes.rs` exercises the active HTTP contract, and full smoke runs native plus real-browser
end-to-end paths.

## Validate

```bash
cargo test --workspace --locked
cargo clippy --all-targets -- -D warnings
cargo clippy -p sdw-web --target wasm32-unknown-unknown -- -D warnings
```

See `deploy/docs/rust-migration.md` for the full language boundary and build path.
