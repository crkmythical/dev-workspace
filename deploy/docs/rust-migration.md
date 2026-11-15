# Rust-first architecture

All maintained product and host-control behavior is implemented in the Cargo workspace under
`src/rust/`.

## Current language boundary

| Area | Implementation | Reason |
|---|---|---|
| Domain rules and protocol | Rust (`sdw-core`) | One implementation shared by every adapter |
| Container CLI, entrypoint and daemons | Rust (`sdw-cli`) | One static musl multi-call binary |
| Auth and sync HTTP service | Rust (`sync-service`) | Axum adapter over `sdw-core` |
| Browser sync UI | Rust (`sdw-web`) compiled to WebAssembly | No duplicate TypeScript protocol or frontend build stack |
| Native protocol smoke probe | Rust (`sdw-smoke-client`) | Exercises the exact request builders used by WASM |
| Host control plane | Rust (`sdw-host`) | Typed state, rollback, PID/path/context checks, maintenance and secrets |
| Bootstrap and OS recipes | Thin Bash | Work before a binary exists or list platform-specific commands |
| Full-browser smoke | Small JavaScript Playwright driver | Browser-control adapter only; no product logic |
| Third-party runtime | Caddy, mihomo, gocryptfs, restic, code-server, desktops, supervisor | Pinned external tools, not project implementation languages |

There is no maintained TypeScript source, Bun workspace, Vite build, Biome config or TypeScript
compiler step. `wasm-bindgen --target web --no-typescript` emits the required ES-module loader; it
is generated adapter code, not a second application implementation.

## Cargo workspace

```
src/rust/
├── core/           Pure rules, use cases, crypto and sync request construction
├── infra/          Linux/container adapters
├── cli/            `sdw` multi-call container executable
├── sync-service/   Axum ingress/auth/sync adapter, linked into `sdw`
├── web/            Browser UI and file scanner compiled to wasm32-unknown-unknown
├── smoke-client/   Native end-to-end sync protocol probe
└── host/           macOS/Linux control plane and maintenance tools
```

`core` knows no browser, HTTP server or OS process. WASM and native smoke adapters both use
`core::sync::client`; the server consumes the matching parser, crypto and constants. `host` stays
independent of container crates and uses only std, serde, sha2 and zeroize around installed host
commands.

## Browser design

`sdw-web` uses `wasm-bindgen`/`web-sys` for DOM, Fetch, File System Access and browser randomness,
and RustCrypto through `sdw-core` for PBKDF2, AES-256-GCM and framing.

- The passphrase field is cleared after derivation.
- The key stays only in the current WebAssembly instance and explicit Rust copies are zeroed.
- Reloading discards the key; IndexedDB no longer persists it.
- The client verifies the derived key before polling.
- Polling is serialized; only successful finalize updates the baseline.
- SHA-256 is streamed per chunk and every Fetch has an AbortController timeout.

WebAssembly linear memory remains inspectable by script already executing in the same origin. CSP,
asset integrity and ingress integrity therefore remain part of the boundary.

## Build and release

The image's digest-pinned Rust builder creates the Linux/musl `sdw` binary and browser WASM. The
wasm-bindgen CLI version is read from Cargo.lock and declarations are disabled.

Tag releases also build `sdw-host` natively on four GitHub runners:

- Linux amd64 and arm64
- macOS amd64 and arm64

The release has exactly nine assets: four native archives, four adjacent SHA-256 files and one
combined `SHA256SUMS`. Image candidates are smoked and manifests created before a draft GitHub
Release is published. A published release is never overwritten by a rerun. `install-sdw-host.sh`
selects the local platform and verifies the checksum before installation.

```bash
make web-tools
make build-web
make host
make install-host
deploy/scripts/install-sdw-host.sh --repo OWNER/REPO
```

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p sdw-web --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace --locked
make test-host
make lint-sh
go run github.com/rhysd/actionlint/cmd/actionlint@v1.7.12
```

The final local gate runs actionlint over every workflow. CI additionally compiles `sdw-host` on
macOS, builds the WASM output without `.d.ts`, and runs exact-image smoke before publishing
manifests.

## Host migration assessment

| Area | Difficulty | Benefit | Decision |
|---|---:|---:|---|
| Destructive cleanup | Medium | Very high | Rust: canonical paths, Compose JSON/labels, verified leftovers, context lock |
| Self-destruct watcher | Medium | High | Rust: typed Docker inspect, checked PID/process groups, detached cleanup |
| Ephemeral Cloudflare Access | Medium-high | Very high | Rust: API JSON, atomic resumable state, rollback after each resource |
| Named Cloudflare tunnel | Medium | High | Rust: scoped config, strict inputs, no unverified binary download |
| File-secret initialization | Low | High | Rust: shared parser, validate-before-write, atomic 0600 output |
| systemd/LaunchAgent rendering | Low-medium | Medium | Rust: escaping and captured Docker context |
| Tool-pin updater | High | Medium-high | Rust: typed APIs, checksum cross-check, streamed hash, transactional writes |
| Git history purge | Medium | High due irreversibility | Rust: clean-tree/root gates, verified bundle, post-rewrite proof |
| Prebuilt host distribution | Medium | High usability | Four native archives, checksum verification, release after all gates |
| Bootstrap | Medium | Low | Keep Shell as the pre-binary entry point |
| Kata/AVF installation | High | Low | Keep Shell: privileged OS command recipes, little reusable policy |
| Smoke checks | High rewrite cost | Negative | Keep Shell/Playwright as black-box adapters |

The 13 migrated compatibility entrypoints fell from roughly 1,370 lines of host Shell to about 70
lines. They contain no state or parsing. Rust is more verbose in raw lines, but there is one typed
implementation and one failure model instead of Shell, jq, embedded Python and service-specific
copies.

## Maintenance behavior

`sdw-host update-tools` supports all prior tools and options. GitHub credentials are kept out of
argv and child environments. GitHub's asset SHA-256 is cross-checked with project checksum files
where available; IDEA uses JetBrains checksum links; Burp is streamed through SHA-256. No pin is
written until every selected update has planned successfully, and write failure rolls earlier files
back. Selkies dependency locking still calls `unzip` and `uv`, because those are the authoritative
wheel metadata and Python resolver adapters.

`sdw-host purge-history` defaults to dry-run. Confirmed execution requires `git-filter-repo`, a
clean tracked tree and an already-untracked `.env`; it creates and verifies a full bundle before
rewriting, proves `.env` is absent from every ref afterwards, and only then expires reflogs and
prunes objects. There is no deprecated `filter-branch` production fallback.

## Shell reduction policy

Keep Shell only where it is a short bootstrap or a declarative platform recipe. Current examples
are initial bootstrap, Kata/AVF installation and system smoke assertions. Do not translate these
mechanically into long sequences of Rust `Command` calls. Move a remaining script only if it owns
policy, parses untrusted structured data, maintains state, handles secrets or duplicates Rust rules.
