//! Adapters for the core ports — a Rust port of packages/infra.
//!
//! Each adapter drives one of the container's pinned tools (gocryptfs/FUSE, Clash, restic, git,
//! supervisord, croc, ...) to satisfy a core trait. Pure helpers (argv builders, output parsers,
//! key derivation) are unit-tested here; the process-spawning shells are validated by tests/smoke
//! on the real image at the single stage-3 cutover.

pub mod cmd;
pub mod desktop;
pub mod destroy;
pub mod destruct_key;
pub mod egress;
pub mod gocryptfs;
pub mod holders;
pub mod notifications;
pub mod restic;
pub mod shred;
pub mod state;
pub mod supervisor;
pub mod time;
