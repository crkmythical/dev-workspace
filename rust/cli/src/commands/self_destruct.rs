//! self-destruct — emergency data destruction. A port of packages/cli/src/self-destruct.ts.
//!
//!   self-destruct [--force] [--skip-shred] [--remote]
//!
//! Runs core's ordered destroy over every vault, then stops all services and kills PID 1 so the
//! container exits. Unless --force, it demands the word "destroy" on the prompt.

use crate::commands::system_env;
use crate::prompt::prompt_line;
use sdw_core::plugins::all_vaults;
use sdw_core::vault::usecases::{destroy_workspace, DestroyDeps, DestroyOptions as DestroyRun};
use sdw_core::workspace::config::load_config;
use sdw_infra::cmd;
use sdw_infra::destroy::{DestroyOptions as EffectsOptions, ShellDestroyEffects};
use sdw_infra::gocryptfs::GocryptfsVolume;
use sdw_infra::holders::SupervisorHolders;
use sdw_infra::shred::FileShredder;
use sdw_infra::time::now_iso;

fn parse_flags(args: &[String]) -> (bool, bool, bool) {
    let has = |flag: &str| args.iter().any(|a| a == flag);
    (has("--force"), has("--skip-shred"), has("--remote"))
}

pub fn run(args: &[String]) -> i32 {
    let (force, skip_shred, remote) = parse_flags(args);

    if !force {
        let answer = prompt_line("Type 'destroy' to confirm: ");
        if answer.as_deref() != Some("destroy") {
            println!("Aborted.");
            return 0;
        }
    }

    let config = load_config(system_env);
    let specs = all_vaults();
    let volume = GocryptfsVolume;
    let holders = SupervisorHolders::new();
    let shredder = FileShredder;
    let effects = ShellDestroyEffects::new(EffectsOptions {
        vault_repo: config.git.vault_repo,
    });
    let now = now_iso;
    let log = |m: &str| println!("{m}");

    let report = destroy_workspace(
        &DestroyDeps {
            specs: specs.as_slice(),
            volume: &volume,
            holders: &holders,
            shredder: &shredder,
            effects: &effects,
            now: &now,
            log: &log,
        },
        DestroyRun { skip_shred, remote },
    );

    if !report.complete {
        eprintln!("Destroy finished with failures:");
        for p in report.phases.iter().filter(|p| !p.ok) {
            eprintln!("  {}: {}", p.phase, p.detail.as_deref().unwrap_or("failed"));
        }
    }

    // Post-destroy: stop all services then kill PID 1 (container exits). tini propagates SIGTERM to
    // all children including this process — expected.
    cmd::ok("supervisorctl", &["stop", "all"]);
    cmd::ok("kill", &["-TERM", "1"]);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_flags_in_any_order() {
        assert_eq!(parse_flags(&[]), (false, false, false));
        assert_eq!(
            parse_flags(&["--remote".into(), "--force".into()]),
            (true, false, true)
        );
        assert_eq!(parse_flags(&["--skip-shred".into()]), (false, true, false));
    }
}
