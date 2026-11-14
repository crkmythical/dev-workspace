# Coverage-guided fuzzing (cargo-fuzz / libFuzzer)

The parsers that read bytes an attacker can send or set are fuzzed **continuously** by proptest in
the normal stable gate (`cargo test --workspace` runs the properties in `core::sync::wire`,
`core::sync::path_safety`, `core::auth` and `core::egress::allowlist` with thousands of randomized
adversarial inputs). That is the assurance the CI enforces on every push.

These libFuzzer targets add a **deeper, coverage-guided** campaign, which
`.github/workflows/fuzz.yml` runs once a week (and on demand). They require a **nightly**
toolchain (a `libfuzzer-sys` requirement) that the project does not otherwise use, so this crate is
**detached** from the main workspace (its own empty `[workspace]` table) and is never built by
`cargo test --workspace`.

## Run

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cd src/rust/fuzz
cargo +nightly fuzz run parse_upload_body        # or bound it: -- -max_total_time=120
cargo +nightly fuzz run verify_session
```

A crash writes a reproducer under `fuzz/artifacts/`; replay with
`cargo +nightly fuzz run <target> fuzz/artifacts/<...>`.

## Targets

| target | function | why it matters |
|---|---|---|
| `parse_upload_body`   | `sdw_core::sync::wire::parse_upload_body`           | parses the PNG-camouflaged upload body from **unauthenticated** wire bytes |
| `check_relative_path` | `sdw_core::sync::path_safety::check_relative_path`  | the only guard on where an authenticated upload lands in the vault (traversal defense) |
| `verify_session`      | `sdw_core::auth::session::verify_session`           | the session cookie, read on every request to a gated route before any login |
| `verify_access_jwt`   | `sdw_core::auth::access_jwt::verify_access_jwt_identity` | the `Cf-Access-Jwt-Assertion` header, from whoever can reach the origin |
| `origin_matches_host` | `sdw_core::auth::request::origin_matches_host`      | the WebSocket `Origin` check (cross-site hijacking of the desktop); both headers are attacker-controlled |
| `parse_allow_list`    | `sdw_core::egress::allowlist::AllowList::parse`     | `EGRESS_ALLOW_CIDRS` becomes firewall rules; also asserts that no accepted block is wider than the limits |
