//! self-destruct — emergency data destruction. A port of packages/cli/src/self-destruct.ts.
//!
//!   self-destruct [--force] [--skip-shred] [--remote]
//!
//! Runs core's ordered destroy over every vault, then stops all services and kills PID 1 so the
//! container exits. Unless --force, it demands the word "destroy" on the prompt.

use crate::commands::system_env;
use crate::prompt::prompt_line;
use sdw_core::plugins::all_vaults;
use sdw_core::vault::usecases::{
    destroy_workspace, DestroyDeps, DestroyOptions as DestroyRun, DestroyReport,
};
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

    // Decide the exit code from the report BEFORE tearing the container down: a destroy that did
    // not complete (e.g. key erasure failed) must not exit 0 as if it had. In the container the
    // kill below usually preempts this return (tini SIGTERMs us), but a caller that survives — or
    // a test — gets the honest code instead of a blanket success.
    let code = destroy_exit_code(&report);

    // Post-destroy: stop all services then kill PID 1 (container exits). tini propagates SIGTERM to
    // all children including this process — expected.
    cmd::ok("supervisorctl", &["stop", "all"]);
    cmd::ok("kill", &["-TERM", "1"]);
    code
}

/// 0 only when every phase succeeded; 1 otherwise. Mirrors the `i32::from(aborted)` convention the
/// boot path uses, so "did it work?" reads the same everywhere.
fn destroy_exit_code(report: &DestroyReport) -> i32 {
    i32::from(!report.complete)
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

    fn report(complete: bool, keys_destroyed: bool) -> DestroyReport {
        DestroyReport {
            phases: vec![],
            keys_destroyed,
            complete,
        }
    }

    #[test]
    fn exit_code_is_zero_only_when_the_destroy_completed() {
        assert_eq!(destroy_exit_code(&report(true, true)), 0);
    }

    #[test]
    fn exit_code_is_nonzero_when_a_phase_failed() {
        // e.g. key erasure failed: complete is false, so the process must not look successful.
        assert_eq!(destroy_exit_code(&report(false, false)), 1);
        // shred-data failed but keys were destroyed: still incomplete, still nonzero.
        assert_eq!(destroy_exit_code(&report(false, true)), 1);
    }
}
