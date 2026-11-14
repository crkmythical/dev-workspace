# Rust-first architecture

The project migration from Bun/TypeScript to Rust is complete. All maintained product behavior is
implemented in the Cargo workspace under `src/rust/`.

## Current language boundary

| Area | Implementation | Reason |
|---|---|---|
| Domain rules and protocol | Rust (`sdw-core`) | One implementation shared by every adapter |
| Container CLI, entrypoint and daemons | Rust (`sdw-cli`) | One static musl multi-call binary |
| Auth and sync HTTP service | Rust (`sync-service`) | Axum adapter over `sdw-core` |
| Browser sync UI | Rust (`sdw-web`) compiled to WebAssembly | Removes the duplicate TypeScript protocol and frontend build stack |
| Native protocol smoke probe | Rust (`sdw-smoke-client`) | Exercises the exact request builders used by WASM |
| Host install/orchestration | Bash, kept thin | Must run before the image exists and primarily composes OS tools |
| Full-browser smoke | Small JavaScript Playwright driver | Playwright's browser-control API is the adapter; no product logic lives there |
| Third-party runtime | Caddy, mihomo, gocryptfs, restic, code-server, desktops, supervisor | External pinned tools, not project implementation languages |

There is no maintained TypeScript source, Bun workspace, Vite build, Biome config or TypeScript
compiler step. `wasm-bindgen --target web --no-typescript` emits a small generated ES-module loader
because browsers cannot instantiate the Rust module directly. That generated loader is not a
second application implementation.

## Cargo workspace

```
src/rust/
├── core/           Pure rules, use cases, crypto and sync request construction
├── infra/          Linux/container adapters
├── cli/            `sdw` multi-call executable
├── sync-service/   Axum ingress/auth/sync adapter, linked into `sdw`
├── web/            Browser UI and file scanner compiled to wasm32-unknown-unknown
└── smoke-client/   Native end-to-end sync protocol probe
```

Dependency direction remains inward. `core` knows no browser, HTTP server or OS process. The WASM
and native smoke adapters both use `core::sync::client`; the server consumes the corresponding
`core::sync` parser, crypto and constants.

## Browser design

`sdw-web` uses `wasm-bindgen` and `web-sys` for DOM, Fetch, File System Access and browser random
bytes. It uses RustCrypto through `sdw-core` for PBKDF2, AES-256-GCM and protocol framing.

The migration also changes key handling deliberately:

- The passphrase field is cleared after derivation.
- The sync key stays only in the current WebAssembly instance; Clear zeroes explicit Rust copies.
- Reloading or closing the page discards the key. It is no longer persisted in IndexedDB.
- The client verifies a derived key against the authenticated list endpoint before polling.
- Polling is serialized. A slow upload cannot start a duplicate interval upload.
- A file enters the baseline only after finalize succeeds, so failures retry.
- SHA-256 is updated per chunk rather than reading a 500 MiB file into one buffer.
- Every Fetch has an AbortController timeout.

Trade-off: WebAssembly linear memory is inspectable by script already executing in the same origin,
whereas the former WebCrypto key was marked non-extractable. CSP, generated-asset integrity and the
ingress boundary therefore remain important. Session-only storage reduces the persistence window
but does not turn an active compromised page into a trusted environment.

## Build

The image uses one digest-pinned Rust builder stage:

1. Build `sdw` for Linux/musl.
2. Build `sdw-web` for `wasm32-unknown-unknown` with the `wasm` Cargo profile.
3. Run the version-matched `wasm-bindgen` CLI with `--no-typescript`.
4. Copy the generated loader/module and static HTML/CSS to `/opt/sdw-web`.

Local commands:

```bash
make web-tools       # one-time wasm target + pinned wasm-bindgen CLI
make build-web       # output: src/rust/target/sdw-web
make lint            # native + wasm clippy and rustfmt
make test            # whole Cargo workspace
make build            # full image
```

The runtime image contains no Rust compiler, Bun, Vite or frontend package manager. Node, Python
and Java remain optional user development runtimes inside the workspace; this project does not use
them to build or run itself.

## Validation

The fast gate is:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo clippy -p sdw-web --target wasm32-unknown-unknown -- -D warnings
cargo test --workspace --locked
```

CI also builds the WASM artifact and asserts that wasm-bindgen emitted no `.d.ts`. The system smoke
suite uses the native Rust protocol client for API coverage and a real Chrome, driven by the small
Playwright adapter, for the actual WASM page and desktop streams.

Frozen wire vectors remain in `sync-service/testdata/` as compatibility fixtures. They no longer
mediate between two live language implementations; both sides now use Rust and the native/WASM
end-to-end checks detect adapter errors.

## Shell reduction policy

Do not mechanically rewrite every shell file. Keep shell where it is a short composition of
`docker`, `launchctl`, `systemctl`, `cloudflared`, `crictl` or test assertions. Move code to Rust
when a script begins owning policy, parsing untrusted structured data, maintaining state, handling
secrets, or duplicating rules already in `sdw-core`.

The next consolidation candidate is a host-side `sdw-host` Rust binary for Cloudflare orchestration,
Docker-context safeguards and destructive cleanup. Bootstrap must retain a minimal shell entry until
a trusted prebuilt host binary or a required host Rust toolchain is part of the installation model.
