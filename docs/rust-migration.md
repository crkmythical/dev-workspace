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
| 1 | Graduate the sync-service daemon + wire it into the image | cargo, then smoke | **done — smoke 93/0 (checks 10/20/30/55/80)** |
| +croc | Add croc as the CLI file-transfer path | cargo, then smoke | **done — smoke 4/4 (services/40-croc)** |
| 2 | Port `core` pure logic (vault/backup/egress/desktop/workspace) | cargo + proptest | **in progress** — see below |
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

**Shipped:** `image/Dockerfile` builds the binary in a musl `rust-builder` stage and
copies it to `/usr/local/bin/sync-service`; `image/supervisord.conf` runs that
instead of `bun run .../server`. The TS `packages/server` stays in the tree (its
`gen-wire-vectors.ts` is the parity source) but no longer runs.

Validated 2026-10-30 on Colima/qemu (arm64): smoke checks 10/20/30/55/80 pass **93/0**
against an image carrying this binary — boot, Caddy-proxied SPA+API, the full
upload/replay/auth/path-safety protocol on the real FUSE vault, a real-Chrome upload
landing in the vault byte-for-byte, and remote destruct (rate-limited, bcrypt-verified,
CLI-triggered to an inert container). The `rust-builder` stage was confirmed to compile
the binary in-image. A full from-scratch production build was not completed locally
(mise's java/node downloads crawl at ~99 kB/s on this network, unrelated to this
change); CI builds it from scratch.

## Stage 2 — progress

A Rust `core` crate (`rust/core`, `sdw-core`): pure domain logic, no I/O, the only
dependency serde (for the persisted forms the TS builds with JSON). Each module is a
faithful port of its `packages/core` counterpart, and its tests mirror the TS tests
(proptest standing in for fast-check) so the two cannot drift while both exist. The
gate is `cargo test` + clippy `-D warnings` + rustfmt; nothing consumes the crate yet
(it joins infra/cli at stage 3).

Ported so far (46 tests):

- `vault/domain` — lifecycle `decide`, unlock-failure explainer, `lineage`/`killable`
  (self-destruct kill-safety), `plan_sync`, the DESTROY_PHASES table.
- `backup/domain` — the debounce/fallback/retry state machine, prune cadence, backoff,
  the opt-in gate, doctor status, the restic repo-probe classifier, path exclusion.
- `egress/domain` — observed-status model, notice/description, the persisted report,
  the doctor row, `render_clash_config`, the start-up timing policy.
- `desktop/domain` — the supervisord topology and FUSE-safe start/stop ordering.
- `workspace/features` — the optional-feature flags.
- `workspace/auth` + `workspace/banner` — master-password resolution, code-server
  config render/parse, the start-up banner.
- `workspace/boot` — BOOT_PLAN and run_boot (step order, abort/degrade, vault gates).
- `workspace/config` — load_config (the typed environment view) and env validation.
- `sync/reconcile` (+ `sync/types`) — the three-way reconciler (SPA side).
- `shared/failure` — the consecutive-failure threshold and tracker trait.

Remaining in stage 2: the plugin registry and the env schema (the big data table).
Then the sync primitives the sync-service already has (aad, crypto, wire, path-safety,
replay, rate-limit) fold into core so there is one copy (dedup, done as part of stage
3's merge).

## croc — the CLI file-transfer path

croc moves files peer-to-peer, end-to-end encrypted (PAKE codephrase → AES-256-GCM),
through a relay that only ever sees ciphertext. It is added **alongside** the sync
page (which keeps its unique zero-install, browser-only, lands-in-the-vault
property), as the path for large or scripted transfers.

Shipped:

- `image/Dockerfile` pins `croc` with a sha256, like `restic`/`caddy`.
- `CROC_RELAY` (host:port of a self-hosted `croc relay`) and `CROC_PASS` (its
  password) are documented in the env schema (`.env.example`). Empty = off. The
  public default relay is never used.
- Two commands, `push` / `pull` (`packages/cli/src/{push,pull}.ts`), over a pure
  arg-builder in `packages/core/src/transfer/croc.ts`. They route through Clash by
  default (`--socks5 127.0.0.1:7890`, Clash's mixed port); `push --direct` is for a
  LAN relay. `pull` lands files in `/workspace` (the vault) by default.
- Neither secret is ever on the command line: the relay password is `CROC_PASS`
  and the code phrase is `CROC_SECRET` (croc v11 refuses `send --code` on argv — a
  property the smoke test caught — so the code never shows in a process listing).
- `push` / `pull` migrate to subcommands of the Rust multi-call binary at stage 3;
  the pure `buildCrocSend` / `buildCrocReceive` and their tests carry over.

Validated 2026-10-30 (services/40-croc.sh, Colima/qemu arm64): a self-hosted relay,
an end-to-end round-trip into the vault with the bytes intact, and the relay's logs
holding only ciphertext (never the payload) — **4/4**. Threat-model note: the relay
still sees metadata (timing, sizes, endpoint IPs); the code phrase is the shared
secret.

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
