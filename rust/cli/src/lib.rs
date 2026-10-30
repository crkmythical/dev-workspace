//! The container's command surface as one multi-call binary — a Rust port of packages/cli.
//!
//! Like busybox, one binary serves every command. It is installed once and symlinked per command
//! name; `argv[0]`'s basename selects the command (or `sdw <command>` works directly, which the
//! entrypoint and the supervised/cron jobs use). This replaces the Bun launchers that `bun run`
//! one `.ts` entry each (packages/cli/scripts/install-bins.ts).
//!
//! Commands are ported one at a time; the Dockerfile keeps running Bun until every command and the
//! entrypoint live here (the single stage-3 image cutover).

pub mod commands;
mod dispatch;
pub mod passphrase;

pub use dispatch::run;
