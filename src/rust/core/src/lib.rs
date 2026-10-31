//! Pure domain logic and use cases — a Rust port of packages/core.
//!
//! No I/O. Most of it depends only on serde (for the persisted forms the TS side builds with
//! JSON); `sync::{crypto,wire,aad,replay,path_safety,rate_limit}` add the sync AEAD primitives
//! (RustCrypto: aes-gcm/hkdf/sha2/hex), which are likewise pure computation — the TS core held
//! the same crypto. Every rule is unit-testable on its own, and the tests mirror the TypeScript
//! tests so the two implementations cannot drift apart while both exist (see
//! docs/rust-migration.md). The infra/cli crates provide the adapters; the sync-service crate
//! consumes these primitives over the wire.

pub mod backup;
pub mod desktop;
pub mod egress;
pub mod plugins;
pub mod shared;
pub mod sync;
pub mod transfer;
pub mod vault;
pub mod workspace;
