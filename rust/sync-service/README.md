# sync-service (Rust/Axum PoC)

A proof-of-concept reimplementation of the TypeScript `sync-service`
(`packages/server`) in Rust + Axum, built to answer one question: **can the
tunnel-exposed daemon be a single memory-safe static binary without breaking
the wire protocol it shares with the browser SPA?**

## What it proves

The daemon speaks the exact same wire protocol as the TS server, verified by
`tests/parity.rs` against golden vectors generated from the **authoritative TS
implementation**:

| Contract | Source of truth | Rust module |
|----------|-----------------|-------------|
| HKDF-SHA256 key derivation | `packages/infra/src/sync-crypto.ts` | `src/crypto.rs` |
| AES-256-GCM (`ciphertext‖tag`) | `packages/infra/src/sync-crypto.ts` | `src/crypto.rs` |
| AAD framing `op\|ts\|…` | `packages/core/src/sync/aad.ts` | `src/aad.rs` |
| PNG camouflage body | `packages/core/src/sync/protocol.ts`, `envelope.ts` | `src/wire.rs` |
| List authenticator (`X-Sync-Auth`) | `packages/core/src/sync/usecases.ts` (`listShared`) | `src/list_auth.rs` |
| Relative-path validation | `packages/core/src/sync/path-safety.ts` | `src/path_safety.rs` |
| Replay window | `packages/core/src/sync/replay-window.ts` | `src/replay.rs` |

If the TS and Rust sides ever drift, the parity tests fail.

## Run

```bash
# 1. (Re)generate golden vectors from the TS implementation (repo root):
bun run packages/server/scripts/gen-wire-vectors.ts

# 2. Prove the Rust side reproduces them:
cd rust/sync-service && cargo test

# 3. Run the daemon (expects the vault-backed paths to exist in a container):
cargo run
```

## Scope / non-goals

This is a PoC, intentionally narrow:

- **Implemented**: `/sync/api/upload` (put-chunk, put-finalize), `/sync/api/download`
  (authenticated listing only, see below), `/sync/api/doctor`, replay protection,
  path safety, vault-lock detection via the passphrase file.
- **Listing needs proof of the sync key.** `GET /sync/api/download` requires
  `X-Sync-Ts` and `X-Sync-Auth` (base64 of `nonce‖tag`, an AEAD over an empty
  message with AAD `list|<ts>`), exactly like the TS server. File content is not
  served at all: the old `?path=` branch answered in plaintext without any
  authentication and is answered with 404 now.
- **Replay-window order.** A nonce is claimed only *after* its envelope
  authenticates, in upload and listing alike. The window is a bounded ring buffer;
  if forgeries could claim slots, a flood of them would push a genuine nonce out and
  let a captured request be replayed. `tests/parity.rs` pins this for listing; the
  upload route follows the same order but is untested here (it needs the
  container's vault paths).
- **Out of scope**: `/sync/api/destruct` (shells out and crosses the irreversible
  self-destruct boundary — not needed to prove wire parity), the WebSocket
  `/sync/api/events` channel, the per-IP/global destruct rate limiter (ported
  cleanly from `packages/core/src/shared/rate-limit.ts` if this graduates), SPA
  static-file serving (Caddy can serve the SPA directly), and the `SYNC_ENABLED`
  runtime flag (the TS server answers 404 on every sync route unless it is set; this
  PoC always serves).
- **PoC shortcuts**: file I/O uses synchronous `std::fs` inside async handlers
  (no `.await` is held across a lock); production would use `tokio::fs` +
  `spawn_blocking` or an actor owning the vault FS.

## The tradeoff this PoC makes concrete

The browser SPA stays TypeScript (File System Access API + WebCrypto), so the
shared `packages/core` wire logic now exists **twice** — once in TS, once here.
`gen-wire-vectors.ts` + `tests/parity.rs` are the seam that keeps the two copies
honest. Adopting Rust for the daemon means accepting (and automating) that
duplication; this PoC shows it is small and testable.
