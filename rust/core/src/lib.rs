//! Pure domain logic and use cases — a Rust port of packages/core.
//!
//! No I/O; the only dependency is serde (for the persisted forms the TS side builds with
//! JSON). Every rule is unit-testable on its own, and the tests mirror the TypeScript tests
//! so the two implementations cannot drift apart while both exist (see docs/rust-migration.md).
//! The infra/cli crates provide the adapters.

pub mod backup;
pub mod desktop;
pub mod egress;
pub mod shared;
pub mod sync;
pub mod vault;
pub mod workspace;
