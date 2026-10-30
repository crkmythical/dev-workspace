//! backup-init — initialize the restic repository. A port of packages/cli/src/backup-init.ts.
//!
//! Safe to re-run: the repository is probed first, and only a location where restic says there is
//! no repository gets `restic init`. The entrypoint auto-inits on first boot; this is the manual
//! fallback / diagnostic.

use crate::commands::system_env;
use sdw_core::backup::domain::{evaluate_backup_gate, GateResult, RepoProbe};
use sdw_core::workspace::config::load_config;
use sdw_infra::restic::{init_repository, probe_repository};

pub fn run(_args: &[String]) -> i32 {
    let config = load_config(system_env);
    match evaluate_backup_gate(&config.backup) {
        GateResult::Dormant => {
            eprintln!("RESTIC_REPOSITORY not configured. Nothing to initialize.");
            return 1;
        }
        GateResult::Misconfigured { missing } => {
            eprintln!("Missing credentials: {}", missing.join(", "));
            return 1;
        }
        GateResult::Enabled => {}
    }

    let status = probe_repository();
    match status.verdict {
        RepoProbe::Ready => {
            println!("✓ Repository already initialized and readable with this password.");
            0
        }
        RepoProbe::WrongPassword => {
            eprintln!(
                "✗ A repository exists at that location, but RESTIC_PASSWORD does not open it."
            );
            1
        }
        RepoProbe::Unreachable => {
            eprintln!(
                "✗ Could not reach the repository (network or credentials?). Not initializing."
            );
            if let Some(last) = status.detail.lines().last() {
                if !last.is_empty() {
                    eprintln!("  {last}");
                }
            }
            1
        }
        RepoProbe::Missing => {
            println!("Initializing restic repository...");
            if init_repository() {
                println!("✓ Repository initialized.");
                0
            } else {
                eprintln!("✗ Repository initialization failed.");
                1
            }
        }
    }
}
