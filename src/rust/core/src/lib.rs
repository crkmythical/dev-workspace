//! Pure domain rules, use cases, crypto and cross-adapter protocol construction.
//!
//! This crate performs no I/O. Native CLI/infra, Axum sync service, browser WebAssembly and the
//! native smoke probe all depend inward on these rules, so the project has no second-language
//! protocol implementation. The infra and driving adapter crates own external effects.

pub mod audit;
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
