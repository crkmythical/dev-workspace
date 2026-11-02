//! Pure domain logic and use cases — a Rust port of packages/core.
//!
//! No I/O. Most of it depends only on serde (for the persisted forms the TS side builds with
//! JSON); `sync::{crypto,wire,aad,replay,path_safety,rate_limit}` add the sync AEAD primitives
//! (RustCrypto: aes-gcm/pbkdf2/sha2/hex), which are likewise pure computation — the TS core held
//! the same crypto. Every rule is unit-testable on its own. The Rust sync wire format is pinned to
//! a frozen golden-vector snapshot of the original TS server (sync-service/tests/parity.rs); the
//! live browser client is TS, so the authoritative cross-language check is the end-to-end smoke
//! suite, not a shared unit test (see deploy/docs/rust-migration.md). The infra/cli crates provide
//! the adapters; the sync-service crate consumes these primitives over the wire.

pub mod auth;
pub mod backup;
pub mod desktop;
pub mod egress;
pub mod plugins;
pub mod shared;
pub mod sync;
pub mod transfer;
pub mod vault;
pub mod workspace;
