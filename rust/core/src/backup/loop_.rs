//! The backup watcher's state machine, as pure transitions — a port of
//! packages/core/src/backup/loop.ts. (The file is `loop_` because `loop` is a Rust keyword.)
//!
//! The watcher (cli/backup-watcher) owns the effects: inotify, the 1-second tick, spawning restic.
//! Every change to its state goes through these functions instead of being scattered across
//! callbacks, so a whole day of events, ticks and backups can be replayed in a test (see below)
//! instead of being trusted.

use crate::backup::domain::{plan_debounce, DebounceDecision, DebounceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupLoop {
    /// Most recent filesystem event; never cleared (see plan_debounce rule 6).
    pub last_event_ms: Option<i64>,
    /// When the last backup STARTED, success or failure.
    pub last_trigger_ms: Option<i64>,
    pub backup_in_flight: bool,
    /// An event arrived while a backup was running.
    pub pending_during_flight: bool,
    pub consecutive_failures: u32,
    pub last_failure_ms: Option<i64>,
}

/// A fresh loop; `persisted_failures` is the counter that survived a restart.
pub fn new_loop(persisted_failures: u32) -> BackupLoop {
    BackupLoop {
        last_event_ms: None,
        last_trigger_ms: None,
        backup_in_flight: false,
        pending_during_flight: false,
        consecutive_failures: persisted_failures,
        last_failure_ms: None,
    }
}

pub fn on_fs_event(loop_: BackupLoop, now_ms: i64) -> BackupLoop {
    BackupLoop {
        last_event_ms: Some(now_ms),
        pending_during_flight: loop_.pending_during_flight || loop_.backup_in_flight,
        ..loop_
    }
}

/// What the tick should do now.
pub fn decide_next(loop_: &BackupLoop, now_ms: i64) -> DebounceDecision {
    plan_debounce(DebounceState {
        last_event_ms: loop_.last_event_ms,
        now_ms,
        backup_in_flight: loop_.backup_in_flight,
        last_trigger_ms: loop_.last_trigger_ms,
        consecutive_failures: loop_.consecutive_failures,
        last_failure_ms: loop_.last_failure_ms,
    })
}

pub fn on_backup_started(loop_: BackupLoop, now_ms: i64) -> BackupLoop {
    BackupLoop {
        backup_in_flight: true,
        last_trigger_ms: Some(now_ms),
        // Only events that arrive DURING this backup matter for the flag, not earlier ones.
        pending_during_flight: false,
        ..loop_
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupOutcome {
    Ok,
    /// `failures` is the consecutive count after this failure (the tracker's answer).
    Failed {
        failures: u32,
    },
}

pub fn on_backup_finished(loop_: BackupLoop, outcome: BackupOutcome, now_ms: i64) -> BackupLoop {
    let (consecutive_failures, last_failure_ms) = match outcome {
        BackupOutcome::Ok => (0, None),
        BackupOutcome::Failed { failures } => (failures, Some(now_ms)),
    };
    // Edits that landed mid-backup restart the quiet window, so the next backup does not fire the
    // instant this one ends while the user is still typing.
    let last_event_ms = if loop_.pending_during_flight {
        Some(now_ms)
    } else {
        loop_.last_event_ms
    };
    BackupLoop {
        consecutive_failures,
        last_failure_ms,
        backup_in_flight: false,
        last_event_ms,
        pending_during_flight: false,
        ..loop_
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::domain::{
        backoff_delay_ms, BACKUP_DEBOUNCE_MS, BACKUP_FALLBACK_INTERVAL_MS,
    };

    /// The outcome of replaying the loop on a fake clock.
    struct Run {
        /// Seconds at which a backup started.
        starts: Vec<i64>,
        loop_: BackupLoop,
    }

    struct SimOpts {
        seconds: i64,
        /// Seconds at which the filesystem reports an event.
        events: Vec<i64>,
        backup_ms: i64,
        /// Backup n (0-based) fails when this returns true.
        fails: fn(u32) -> bool,
        start_failures: u32,
    }

    impl Default for SimOpts {
        fn default() -> Self {
            SimOpts {
                seconds: 0,
                events: Vec::new(),
                backup_ms: 5_000,
                fails: |_| false,
                start_failures: 0,
            }
        }
    }

    /// The watcher's loop replayed on a fake clock: a 1-second tick, a backup that takes
    /// `backup_ms` and either succeeds or fails, and a script of filesystem events. What is
    /// asserted is the list of moments backups STARTED, because the bug this guards against (a
    /// backup storm after a single edit) is only visible over time, not in one decision.
    fn simulate(opts: SimOpts) -> Run {
        let event_at: std::collections::HashSet<i64> = opts.events.iter().copied().collect();
        let mut loop_ = new_loop(opts.start_failures);
        let mut finish_at: Option<i64> = None;
        let mut finish_ok = true;
        let mut failure_count = opts.start_failures;
        let mut n: u32 = 0;
        let mut starts: Vec<i64> = Vec::new();

        for t in 0..=opts.seconds {
            let now_ms = t * 1000;
            if event_at.contains(&t) {
                loop_ = on_fs_event(loop_, now_ms);
            }
            if finish_at.is_some_and(|f| now_ms >= f) {
                if finish_ok {
                    failure_count = 0;
                } else {
                    failure_count += 1;
                }
                let outcome = if finish_ok {
                    BackupOutcome::Ok
                } else {
                    BackupOutcome::Failed {
                        failures: failure_count,
                    }
                };
                loop_ = on_backup_finished(loop_, outcome, now_ms);
                finish_at = None;
            }
            if decide_next(&loop_, now_ms) == DebounceDecision::Trigger {
                starts.push(t);
                loop_ = on_backup_started(loop_, now_ms);
                finish_ok = !(opts.fails)(n);
                n += 1;
                finish_at = Some(now_ms + opts.backup_ms);
            }
        }
        Run { starts, loop_ }
    }

    // --- replayed over time (mirrors "backup loop over time") ---

    #[test]
    fn one_burst_of_edits_yields_one_extra_backup_not_a_storm() {
        let fallback_s = BACKUP_FALLBACK_INTERVAL_MS / 1000;
        let debounce_s = BACKUP_DEBOUNCE_MS / 1000;
        // Edits at t=1..5, then silence. The first run is the baseline at t=0; the burst is saved
        // once, 10 s after it went quiet; after that only the 5-minute fallback fires.
        let run = simulate(SimOpts {
            seconds: 3600,
            events: vec![1, 2, 3, 4, 5],
            ..Default::default()
        });
        let starts = run.starts;
        assert_eq!(starts[0], 0);
        assert_eq!(starts[1], 5 + debounce_s);
        let rest = &starts[2..];
        for (i, &r) in rest.iter().enumerate() {
            assert_eq!(r, starts[1] + (i as i64 + 1) * fallback_s);
        }
        let ceil_div = (3600 + fallback_s - 1) / fallback_s;
        assert!(starts.len() as i64 <= 2 + ceil_div);
    }

    #[test]
    fn no_activity_only_baseline_and_fallback() {
        let fallback_s = BACKUP_FALLBACK_INTERVAL_MS / 1000;
        let run = simulate(SimOpts {
            seconds: 1000,
            ..Default::default()
        });
        assert_eq!(
            run.starts,
            vec![0, fallback_s, 2 * fallback_s, 3 * fallback_s]
        );
    }

    #[test]
    fn continuous_editing_waits_for_quiet() {
        let debounce_s = BACKUP_DEBOUNCE_MS / 1000;
        let edits: Vec<i64> = (0..30).map(|i| 1 + i * 2).collect(); // every 2 s until t=59
        let run = simulate(SimOpts {
            seconds: 200,
            events: edits,
            ..Default::default()
        });
        // baseline, then ONE backup after the last edit (t=59) + 10 s of quiet
        assert_eq!(run.starts, vec![0, 59 + debounce_s]);
    }

    #[test]
    fn edit_during_a_backup_is_saved_after_the_quiet_window() {
        let debounce_s = BACKUP_DEBOUNCE_MS / 1000;
        // The baseline backup runs t=0..5; the edit at t=2 lands mid-flight.
        let run = simulate(SimOpts {
            seconds: 100,
            events: vec![2],
            ..Default::default()
        });
        assert_eq!(run.starts, vec![0, 5 + debounce_s]);
    }

    #[test]
    fn a_failed_backup_is_retried_after_its_backoff_then_settles() {
        // Backup 0 (the baseline) fails, backup 1 succeeds.
        let run = simulate(SimOpts {
            seconds: 120,
            fails: |n| n == 0,
            ..Default::default()
        });
        assert_eq!(run.starts[0], 0);
        // finished (failed) at t=5, backoff for 1 failure is 10 s -> retry at t=15
        assert_eq!(run.starts[1], 5 + backoff_delay_ms(1) / 1000);
        assert_eq!(run.starts.len(), 2);
        assert_eq!(run.loop_.consecutive_failures, 0);
    }

    #[test]
    fn repeated_failures_back_off_further_and_keep_retrying() {
        let run = simulate(SimOpts {
            seconds: 600,
            fails: |_| true,
            ..Default::default()
        });
        let starts = run.starts;
        assert!(starts.len() > 4);
        // idle time between attempts (each backup itself takes 5 s)
        let gaps: Vec<i64> = starts[1..]
            .iter()
            .enumerate()
            .map(|(i, &t)| t - starts[i] - 5)
            .collect();
        // backoff doubles (10, 20, 40, 80) and then stays capped at 80 s
        assert_eq!(&gaps[0..4], &[10, 20, 40, 80]);
        for &gap in &gaps[4..] {
            assert_eq!(gap, 80);
        }
    }

    #[test]
    fn a_persisted_counter_does_not_stop_the_baseline_run() {
        let run = simulate(SimOpts {
            seconds: 60,
            start_failures: 2,
            ..Default::default()
        });
        assert_eq!(run.starts[0], 0);
    }

    #[test]
    fn never_runs_two_backups_at_once() {
        let events: Vec<i64> = (0..200).collect();
        let run = simulate(SimOpts {
            seconds: 300,
            events,
            backup_ms: 8_000,
            ..Default::default()
        });
        let starts = run.starts;
        for i in 1..starts.len() {
            assert!(starts[i] - starts[i - 1] >= 8);
        }
    }

    // --- single transitions (mirrors "backup loop transitions") ---

    #[test]
    fn an_event_during_a_backup_sets_the_pending_flag_one_before_does_not() {
        let mut loop_ = on_fs_event(new_loop(0), 100);
        assert!(!loop_.pending_during_flight);
        loop_ = on_backup_started(loop_, 200);
        assert!(!loop_.pending_during_flight);
        loop_ = on_fs_event(loop_, 250);
        assert!(loop_.pending_during_flight);
    }

    #[test]
    fn finishing_restarts_the_quiet_window_only_if_something_happened_midflight() {
        let quiet = on_backup_finished(
            on_backup_started(on_fs_event(new_loop(0), 100), 200),
            BackupOutcome::Ok,
            900,
        );
        assert_eq!(quiet.last_event_ms, Some(100));

        let busy = on_backup_finished(
            on_fs_event(on_backup_started(new_loop(0), 200), 250),
            BackupOutcome::Ok,
            900,
        );
        assert_eq!(busy.last_event_ms, Some(900));
        assert!(!busy.pending_during_flight);
    }

    #[test]
    fn success_clears_the_failure_state_failure_records_it() {
        let failed = on_backup_finished(
            on_backup_started(new_loop(0), 0),
            BackupOutcome::Failed { failures: 3 },
            500,
        );
        assert_eq!(failed.consecutive_failures, 3);
        assert_eq!(failed.last_failure_ms, Some(500));
        assert!(!failed.backup_in_flight);

        let ok = on_backup_finished(on_backup_started(failed, 600), BackupOutcome::Ok, 900);
        assert_eq!(ok.consecutive_failures, 0);
        assert_eq!(ok.last_failure_ms, None);
    }

    #[test]
    fn transitions_do_not_mutate_their_input() {
        // Rust moves/copies rather than sharing, so this is a compile-time guarantee; the assert
        // documents the TS test's intent (a retained value is unchanged by the transitions).
        let before = new_loop(1);
        let _ = on_fs_event(before, 1);
        let _ = on_backup_started(before, 2);
        let _ = on_backup_finished(before, BackupOutcome::Failed { failures: 2 }, 3);
        assert_eq!(before, new_loop(1));
    }
}
