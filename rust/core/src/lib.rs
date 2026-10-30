//! Pure domain logic and use cases — a Rust port of packages/core.
//!
//! No I/O and no runtime dependencies: every rule is unit-testable on its own, and the
//! tests mirror the TypeScript tests so the two implementations cannot drift apart while
//! both exist (see docs/rust-migration.md). The infra/cli crates provide the adapters.

pub mod backup;
pub mod vault;
