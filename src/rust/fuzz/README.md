# Coverage-guided fuzzing (cargo-fuzz / libFuzzer)

The two untrusted parsers are fuzzed **continuously** by proptest in the normal stable gate
(`cargo test --workspace` runs `core::sync::wire::tests` and `core::sync::path_safety::tests` with
tens of thousands of randomized adversarial inputs). That is the assurance the CI enforces.

These libFuzzer targets add a **deeper, coverage-guided** campaign. They require a **nightly**
toolchain (a `libfuzzer-sys` requirement) that the project does not otherwise use, so this crate is
**detached** from the main workspace (its own empty `[workspace]` table) and is never built by
`cargo test --workspace`.

## Run

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cd src/rust/fuzz
cargo +nightly fuzz run parse_upload_body        # or bound it: -- -max_total_time=120
cargo +nightly fuzz run check_relative_path
```

A crash writes a reproducer under `fuzz/artifacts/`; replay with
`cargo +nightly fuzz run <target> fuzz/artifacts/<...>`.

## Targets

| target | function | why it matters |
|---|---|---|
| `parse_upload_body`  | `sdw_core::sync::wire::parse_upload_body`     | parses the PNG-camouflaged upload body from **unauthenticated** wire bytes |
| `check_relative_path`| `sdw_core::sync::path_safety::check_relative_path` | the only guard on where an authenticated upload lands in the vault (traversal defense) |
