//! The container's native command surface as one Rust multi-call binary.
//!
//! Like busybox, one binary serves every command. It is installed once and symlinked per command
//! name; argv[0]'s basename selects the command, while `sdw <command>` is used by entrypoint,
//! supervisor and cron.

pub mod commands;
mod dispatch;
pub mod passphrase;
pub mod prompt;

pub use dispatch::run;
