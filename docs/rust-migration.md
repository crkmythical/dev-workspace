# Rust migration

Porting the workspace's own code from Bun/TypeScript to Rust, staged so each step
is validated and the work can stop at any boundary. Started 2026-10.

## Scope

"All Rust" means **our ~7,400 lines of source**, not the ecosystem:

- **Ported to Rust:** `core` (pure rules + use cases), `infra` (adapters), `cli`
  (the container commands + entrypoint), `server` (the sync-service daemon).
- **Stays as-is:** the pinned third-party binaries the image already carries —
  gocryptfs, Caddy, Clash/mihomo, restic, code-server, KasmVNC/Selkies,
  supervisord (several are Go; same posture as before). Adding croc (below) is
  one more such binary.
- **Stays TypeScript:** the browser SPA (`packages/spa`). Its sync key is a
  non-extractable WebCrypto `CryptoKey`; compiling the crypto to WASM would put
  the key bytes in page-readable memory, which is weaker. The shared wire logic
  therefore exists twice and is pinned by golden vectors (see "Keeping the two
  honest").
- **Stays bash:** the host-side scripts (`scripts/`, `destroy.sh`, the
  LaunchAgent/systemd units). They run before the container exists or after it is
  gone; zero-dependency bash is the right tool.

## Why there are only two image-cutover points

`core` is imported by `infra`, `cli`, `server` and `spa`. A Rust `core` ships
nothing until the whole `core`+`infra`+`cli` cluster is Rust, because nothing in
the image consumes it before then, and you cannot replace one `cli` command with
Rust while it shares a TypeScript `core`. So the image flips at exactly two
points:

1. **sync-service daemon → Rust** — self-contained; can ship on its own.
2. **the entire command surface + entrypoint → one Rust multi-call binary** — the
   Bun runtime leaves the image here.

Between those, module-by-module porting is validated by `cargo test` + golden
vectors, not by the smoke suite (there is no half-Rust image to boot).

## Validation strategy

- **Fast gate, every change:** `cargo test`, `cargo clippy -- -D warnings`,
  `cargo fmt --check`, and golden-vector parity against the authoritative TS.
  No Docker needed.
- **Smoke gate, every image cutover:** `tests/smoke` on the real image (needs
  Docker; ~23 min + build). Batched at the two cutovers, not per change — the
  Colima VM is slow and its disk has been fragile, so the fast gate carries the
  per-change load.

## Stages

| Stage | Content | Gate | Status |
|-------|---------|------|--------|
| 0 | Cargo workspace, release profile, CI (`cargo fmt`/`clippy`/`test`) | cargo | **done** |
| 1 | Graduate the sync-service daemon | cargo, then smoke `30-sync`/`20-auth`/`80-destruct`/`55-browser` | **code done & green; image cutover pending smoke** |
| +croc | Add croc as the CLI file-transfer path | cargo, then a new smoke check | designed (below), not built |
| 2 | Port `core` pure logic (vault/backup/egress/desktop/workspace) | cargo + vectors/proptest | not started |
| 3 | `infra`+`cli`+entrypoint → one multi-call binary; Bun leaves the image | cargo, then full smoke (298) | not started |

## Stage 1 — what is done

`rust/sync-service` is the full daemon, not a PoC:

- feature gates (`SYNC_ENABLED`, `REMOTE_DESTRUCT_ENABLED`; off → 404),
- `doctor` with the TS JSON shape (`disabled`/`vault-locked`/`ready`),
- upload (`put-chunk`/`put-finalize`) with the authenticate-then-store-then-claim
  order preserved, filesystem work on `spawn_blocking`,
- authenticated listing (no file content),
- remote destruct: per-IP + global-backstop rate limit, bcrypt verify, delayed
  trigger behind a trait (real trigger launches the `self-destruct` CLI, so the
  irreversible path stays in one place),
- static SPA serving (so the daemon is a drop-in for the TS one with no Caddy
  change; moving it to Caddy's `file_server` is a later, separately-smoked step).

Tests (all green, `cargo test`): wire-vector parity (8), bcrypt↔Bun parity (1),
HTTP routes incl. a real encrypted chunk+finalize and the destruct decision path
(13), unit tests for features/rate-limit/store (9). Release binary ~0.8 MB.

**Remaining before it ships:** the image cutover — build the binary in
`image/Dockerfile`, point `image/supervisord.conf` at it, drop the
`bun run .../server/src/index.ts` program — then pass the smoke checks above. Not
done here because the smoke suite needs Colima, which is deferred.

## croc — the CLI file-transfer path

croc moves files peer-to-peer, end-to-end encrypted (PAKE codephrase → AES-256-GCM),
through a relay that only ever sees ciphertext. It is added **alongside** the sync
page (which keeps its unique zero-install, browser-only, lands-in-the-vault
property), as the path for large or scripted transfers.

Integration (at the image-cutover milestone, smoke-gated):

- Pin `croc` in `image/Dockerfile` with a checksum, like `restic`/`caddy`.
- Self-host the relay (`croc relay`); never the default public relay. New env:
  `CROC_RELAY`, `CROC_PASS`.
- Route through Clash: `croc --socks5 <clash-socks> --relay $CROC_RELAY …`
  (croc resolves the relay's DNS proxy-side with `socks5h`). This keeps the
  egress posture; the relay only sees ciphertext.
- Two commands, `push` / `pull` (today thin wrappers over the croc CLI; after
  stage 3 they are two subcommands of the Rust multi-call binary). Transfers land
  in `/workspace` (the vault mount).
- Threat-model note: the relay sees metadata (timing, sizes, endpoint IPs); the
  codephrase is the shared secret.

## tailcat — optional alternative ingress (not default)

tailcat (`tailscale serve`-style, no account, no control plane, WireGuard E2E via
magicsock+DERP) can expose code-server / the desktops when the Cloudflare Tunnel
is unavailable. Deferred and opt-in because it **bypasses Clash** (it speaks UDP /
DERP-over-HTTPS, not an HTTP/SOCKS proxy) and the tailcat address is a bearer
capability (weaker than Access OAuth unless paired with its key-authed SSH mode).
Self-host DERP for independence.

## Keeping the two honest

While the SPA stays TypeScript, `rust/sync-service/tests/parity.rs` replays golden
vectors generated from the TS implementation
(`packages/server/scripts/gen-wire-vectors.ts`), and `tests/bcrypt_parity.rs` pins
Bun's bcrypt output against the `bcrypt` crate. Regenerate the vectors whenever the
protocol changes; a drift fails the Rust build.
