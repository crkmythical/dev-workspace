//! backup-watcher — event-driven restic backup with debounce. A port of
//! packages/cli/src/backup-watcher.ts.
//!
//! Supervised program (autostart=false), started on vault mount when RESTIC_REPOSITORY is set.
//! Watches /workspace via inotify, debounces, triggers `restic backup`. WHAT to do WHEN is the
//! pure state machine in sdw_core::backup::loop_ (replayed in tests); this owns the effects only.
//!
//! FUSE-unmount safety: this process holds inotify watches + read fds on /workspace, so it MUST be
//! stopped before `fusermount -u` (lock-vault/destroy do that).

use crate::commands::system_env;
use notify::{RecursiveMode, Watcher};
use sdw_core::backup::domain::{
    evaluate_backup_gate, is_excluded_path, plan_prune, DebounceDecision, GateResult,
    PruneDecision, PruneState, BACKUP_EXCLUSION_SET, BACKUP_FAILURE_THRESHOLD, BACKUP_KEEP_DAILY,
    BACKUP_KEEP_HOURLY, BACKUP_KEEP_WEEKLY,
};
use sdw_core::backup::layout::BACKUP_STATE_DIR;
use sdw_core::backup::loop_::{
    decide_next, new_loop, on_backup_finished, on_backup_started, on_fs_event, BackupLoop,
    BackupOutcome,
};
use sdw_core::egress::layout::CLASH_HTTP_PORT;
use sdw_core::shared::failure::FailureTracker;
use sdw_core::shared::layout::WORKSPACE_MOUNT;
use sdw_core::workspace::config::load_config;
use sdw_infra::cmd;
use sdw_infra::failure_tracker::FileFailureTracker;
use sdw_infra::time::now_ms;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// restic goes through the same proxy as everything else; stderr is captured. The child's pid is
/// published so the shutdown path can kill it. Returns (succeeded, stderr).
fn run_restic(args: &[String], restic_pid: &AtomicU32) -> Result<(bool, String), String> {
    let proxy = format!("http://127.0.0.1:{CLASH_HTTP_PORT}");
    let mut command = Command::new("restic");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for key in ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"] {
        command.env(key, &proxy);
    }
    // The repository password and S3 keys may live in files (RESTIC_PASSWORD_FILE, ...).
    sdw_infra::secrets::pass_secrets(&mut command, sdw_infra::secrets::RESTIC_SECRETS);
    let child = command.spawn().map_err(|e| e.to_string())?;
    restic_pid.store(child.id(), Ordering::SeqCst);
    let result = child.wait_with_output();
    restic_pid.store(0, Ordering::SeqCst);
    let out = result.map_err(|e| e.to_string())?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

fn load_last_prune() -> Option<i64> {
    // Stored as epoch millis (internal to this daemon), so no date parsing is needed.
    fs::read_to_string(format!("{BACKUP_STATE_DIR}/last-prune"))
        .ok()?
        .trim()
        .parse::<i64>()
        .ok()
}

fn run_prune(restic_pid: &AtomicU32) {
    let args = vec![
        "forget".to_string(),
        "--prune".to_string(),
        "--keep-hourly".to_string(),
        BACKUP_KEEP_HOURLY.to_string(),
        "--keep-daily".to_string(),
        BACKUP_KEEP_DAILY.to_string(),
        "--keep-weekly".to_string(),
        BACKUP_KEEP_WEEKLY.to_string(),
    ];
    match run_restic(&args, restic_pid) {
        Ok((true, _)) => {
            let _ = fs::write(
                format!("{BACKUP_STATE_DIR}/last-prune"),
                now_ms().to_string(),
            );
            println!("Prune completed successfully.");
        }
        Ok((false, _)) => eprintln!("Prune failed (non-fatal). Will retry next cadence."),
        Err(e) => eprintln!("Prune error (non-fatal): {e}"),
    }
}

fn run_backup(
    loop_state: &Mutex<BackupLoop>,
    tracker: &mut FileFailureTracker,
    restic_pid: &AtomicU32,
) {
    {
        let mut l = loop_state.lock().unwrap();
        if l.backup_in_flight {
            return; // guard against re-entry from a tick overlap
        }
        *l = on_backup_started(*l, now_ms());
    }

    let mut args = vec!["backup".to_string(), WORKSPACE_MOUNT.to_string()];
    for pattern in BACKUP_EXCLUSION_SET {
        args.push("--exclude".to_string());
        args.push(pattern.to_string());
    }

    let mut failures: Option<u32> = None;
    match run_restic(&args, restic_pid) {
        Ok((true, _)) => {
            tracker.record_success();
            // Prune is serialized with the backup: never concurrent with it.
            if plan_prune(PruneState {
                last_prune_ms: load_last_prune(),
                now_ms: now_ms(),
            }) == PruneDecision::PruneDue
            {
                run_prune(restic_pid);
            }
        }
        Ok((false, stderr)) => {
            let count = tracker.record_failure().count;
            failures = Some(count);
            let detail: String = stderr.chars().take(200).collect();
            eprintln!("Backup failed (attempt {count}): {detail}");
        }
        Err(e) => {
            let count = tracker.record_failure().count;
            failures = Some(count);
            eprintln!("Backup failed (attempt {count}): {e}");
        }
    }

    let mut l = loop_state.lock().unwrap();
    let outcome = match failures {
        None => BackupOutcome::Ok,
        Some(f) => BackupOutcome::Failed { failures: f },
    };
    *l = on_backup_finished(*l, outcome, now_ms());
}

pub fn run(_args: &[String]) -> i32 {
    match evaluate_backup_gate(&load_config(system_env).backup) {
        GateResult::Dormant => {
            println!("Backup not configured (RESTIC_REPOSITORY absent). Exiting cleanly.");
            return 0;
        }
        GateResult::Misconfigured { missing } => {
            eprintln!(
                "Backup misconfigured: missing {}. Exiting.",
                missing.join(", ")
            );
            return 1;
        }
        GateResult::Enabled => {}
    }
    if !cmd::ok("mountpoint", &["-q", WORKSPACE_MOUNT]) {
        eprintln!("Workspace not mounted. Exiting.");
        return 1;
    }
    let _ = fs::create_dir_all(BACKUP_STATE_DIR);

    let mut tracker = FileFailureTracker::new(
        BACKUP_STATE_DIR,
        BACKUP_FAILURE_THRESHOLD,
        "backup-failure.md",
        |n| {
            format!(
                "# ⚠ Backup Failure\n\nFailed {n} consecutive times. Run `doctor` to diagnose.\n"
            )
        },
    );

    let loop_state = Arc::new(Mutex::new(new_loop(tracker.consecutive_failures())));
    let stopping = Arc::new(AtomicBool::new(false));
    let restic_pid = Arc::new(AtomicU32::new(0));

    // Signal thread: on SIGTERM/SIGINT, stop and kill any in-flight restic (TERM then KILL).
    {
        let stopping = stopping.clone();
        let restic_pid = restic_pid.clone();
        match signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGINT,
        ]) {
            Ok(mut signals) => {
                std::thread::spawn(move || {
                    if signals.forever().next().is_some() {
                        stopping.store(true, Ordering::SeqCst);
                        let pid = restic_pid.load(Ordering::SeqCst);
                        if pid != 0 {
                            cmd::ok("kill", &["-TERM", &pid.to_string()]);
                            std::thread::sleep(Duration::from_secs(5));
                            cmd::ok("kill", &["-KILL", &pid.to_string()]);
                        }
                    }
                });
            }
            Err(e) => eprintln!("WARNING: could not install signal handler: {e}"),
        }
    }

    // inotify watcher: fold filesystem events into the loop state on its own thread.
    let watcher_loop = loop_state.clone();
    let watcher_stop = stopping.clone();
    let mut watcher =
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if watcher_stop.load(Ordering::SeqCst) {
                return;
            }
            if let Ok(event) = res {
                let relevant = event.paths.is_empty()
                    || event
                        .paths
                        .iter()
                        .any(|p| !is_excluded_path(&p.to_string_lossy()));
                if relevant {
                    let mut l = watcher_loop.lock().unwrap();
                    *l = on_fs_event(*l, now_ms());
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Failed to start filesystem watcher: {e}");
                return 1;
            }
        };
    if let Err(e) = watcher.watch(Path::new(WORKSPACE_MOUNT), RecursiveMode::Recursive) {
        eprintln!("Failed to start filesystem watcher: {e}");
        return 1;
    }
    println!("Backup watcher started. Monitoring /workspace for changes.");

    // Main loop: a 1-second tick.
    while !stopping.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(1000));
        if stopping.load(Ordering::SeqCst) {
            break;
        }
        let decision = {
            let l = loop_state.lock().unwrap();
            decide_next(&l, now_ms())
        };
        if decision == DebounceDecision::Trigger {
            run_backup(&loop_state, &mut tracker, &restic_pid);
        }
    }

    drop(watcher);
    println!("Backup watcher stopped gracefully.");
    0
}
