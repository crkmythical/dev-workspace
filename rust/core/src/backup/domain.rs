//! Realtime backup — pure decision logic (no I/O).
//!
//! A faithful port of packages/core/src/backup/domain.ts. Constants, the debounce/prune/retry
//! state machine, the opt-in gate, the doctor status and the restic repository probe. Tests
//! mirror packages/core/tests/backup.property.test.ts.

// --- Constants ---

/// Debounce quiet window: 10 s of no filesystem events before a backup.
pub const BACKUP_DEBOUNCE_MS: i64 = 10_000;
/// Fallback: force a backup every 5 min even without inotify events (FUSE inotify is unreliable).
pub const BACKUP_FALLBACK_INTERVAL_MS: i64 = 300_000;
/// Consecutive failures before a notification is written.
pub const BACKUP_FAILURE_THRESHOLD: u32 = 3;
/// Doctor warns if the last success is older than this (10 min).
pub const BACKUP_STALENESS_MS: i64 = 600_000;
/// Maximum retry attempts before giving up on a single cycle.
pub const BACKUP_MAX_RETRIES: u32 = 5;
/// Base delay for exponential backoff (5 s).
pub const BACKUP_BASE_BACKOFF_MS: i64 = 5_000;
pub const BACKUP_KEEP_HOURLY: u32 = 24;
pub const BACKUP_KEEP_DAILY: u32 = 7;
pub const BACKUP_KEEP_WEEKLY: u32 = 4;
/// Prune cadence: `restic forget --prune` at most every 6 hours.
pub const BACKUP_PRUNE_CADENCE_MS: i64 = 6 * 60 * 60 * 1000;

/// Exclusion set — one source of truth for the watcher ignore and restic --exclude. Checked
/// against ALL path segments, so nested occurrences (project/node_modules) are excluded too.
pub const BACKUP_EXCLUSION_SET: &[&str] = &[
    "node_modules",
    ".git",
    ".cache",
    ".desktop",
    ".desktop-vnc",
    ".notifications",
    ".code-server",
    ".uploads",
    "dist",
    "build",
    ".vscode",
    ".idea",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "target",
    ".next",
    ".nuxt",
];

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebounceDecision {
    Trigger,
    Wait,
    Coalesce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebounceState {
    /// Most recent filesystem event (None = none received yet).
    pub last_event_ms: Option<i64>,
    pub now_ms: i64,
    /// Whether a restic backup subprocess is currently running.
    pub backup_in_flight: bool,
    /// Last backup START, success or failure (None = never triggered).
    pub last_trigger_ms: Option<i64>,
    /// Consecutive failures (0 = last backup succeeded or never ran).
    pub consecutive_failures: u32,
    /// Last backup failure (None = never failed).
    pub last_failure_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneDecision {
    PruneDue,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PruneState {
    pub last_prune_ms: Option<i64>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackupGateEnv {
    pub restic_repository: Option<String>,
    pub restic_password: Option<String>,
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    Dormant,
    Enabled,
    Misconfigured { missing: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorBackupStatus {
    Ok,
    Warn,
    Fail,
    NotConfigured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorBackupState {
    pub gate_result: GateResult,
    pub last_success_ms: Option<i64>,
    pub consecutive_failures: u32,
    pub now_ms: i64,
}

/// What `restic snapshots` says about the configured repository. Only an explicit "no repository
/// here" (exit 10 or "repository does not exist") may lead to an init; a wrong password is 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoProbe {
    Ready,
    Missing,
    WrongPassword,
    Unreachable,
}

// --- Pure functions ---

fn present(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.is_empty())
}

/// Trigger, wait, or coalesce. Priority: in-flight → coalesce; first run → trigger; inside the
/// post-failure backoff → wait; fallback elapsed → trigger; a failed attempt whose backoff is
/// over → trigger (retry); no event since the last backup started → wait; quiet ≥ 10 s → trigger.
pub fn plan_debounce(state: DebounceState) -> DebounceDecision {
    if state.backup_in_flight {
        return DebounceDecision::Coalesce;
    }
    let Some(last_trigger) = state.last_trigger_ms else {
        return DebounceDecision::Trigger;
    };
    let failing = state.consecutive_failures > 0;
    if failing {
        if let Some(last_failure) = state.last_failure_ms {
            if state.now_ms - last_failure < backoff_delay_ms(state.consecutive_failures) {
                return DebounceDecision::Wait;
            }
        }
    }
    if state.now_ms - last_trigger >= BACKUP_FALLBACK_INTERVAL_MS {
        return DebounceDecision::Trigger;
    }
    // A failed attempt saved nothing: what it was meant to save is still unsaved.
    if failing {
        return DebounceDecision::Trigger;
    }
    match state.last_event_ms {
        // No event since the last backup started: nothing is unsaved.
        None => DebounceDecision::Wait,
        Some(ev) if ev < last_trigger => DebounceDecision::Wait,
        Some(ev) => {
            if state.now_ms - ev >= BACKUP_DEBOUNCE_MS {
                DebounceDecision::Trigger
            } else {
                DebounceDecision::Wait
            }
        }
    }
}

/// Whether a prune cycle is due.
pub fn plan_prune(state: PruneState) -> PruneDecision {
    match state.last_prune_ms {
        None => PruneDecision::PruneDue,
        Some(last) => {
            if state.now_ms - last >= BACKUP_PRUNE_CADENCE_MS {
                PruneDecision::PruneDue
            } else {
                PruneDecision::Skip
            }
        }
    }
}

/// Exponential backoff: 5 s × 2^min(n, 4), capped at ~80 s.
pub fn backoff_delay_ms(consecutive_failures: u32) -> i64 {
    BACKUP_BASE_BACKOFF_MS * (1i64 << consecutive_failures.min(4))
}

/// Evaluate the opt-in gate from the environment.
pub fn evaluate_backup_gate(env: &BackupGateEnv) -> GateResult {
    if !present(&env.restic_repository) {
        return GateResult::Dormant;
    }
    let mut missing = Vec::new();
    if !present(&env.restic_password) {
        missing.push("RESTIC_PASSWORD".to_string());
    }
    if !present(&env.aws_access_key_id) {
        missing.push("AWS_ACCESS_KEY_ID".to_string());
    }
    if !present(&env.aws_secret_access_key) {
        missing.push("AWS_SECRET_ACCESS_KEY".to_string());
    }
    if missing.is_empty() {
        GateResult::Enabled
    } else {
        GateResult::Misconfigured { missing }
    }
}

/// Whether the failure count has reached the notification threshold.
pub fn should_notify(consecutive_failures: u32) -> bool {
    consecutive_failures >= BACKUP_FAILURE_THRESHOLD
}

/// Doctor status for the backup component.
pub fn plan_doctor_backup_status(state: &DoctorBackupState) -> DoctorBackupStatus {
    match state.gate_result {
        GateResult::Dormant => return DoctorBackupStatus::NotConfigured,
        GateResult::Misconfigured { .. } => return DoctorBackupStatus::Fail,
        GateResult::Enabled => {}
    }
    if state.consecutive_failures >= BACKUP_FAILURE_THRESHOLD {
        return DoctorBackupStatus::Fail;
    }
    match state.last_success_ms {
        None => DoctorBackupStatus::Warn,
        Some(last) if state.now_ms - last >= BACKUP_STALENESS_MS => DoctorBackupStatus::Warn,
        Some(_) => DoctorBackupStatus::Ok,
    }
}

/// Classify a `restic snapshots` result.
pub fn classify_repo_probe(exit_code: i32, stderr: &str) -> RepoProbe {
    if exit_code == 0 {
        return RepoProbe::Ready;
    }
    let text = stderr.to_lowercase();
    if exit_code == 12 || text.contains("wrong password") || text.contains("no key found") {
        return RepoProbe::WrongPassword;
    }
    if exit_code == 10 || text.contains("repository does not exist") {
        return RepoProbe::Missing;
    }
    RepoProbe::Unreachable
}

/// Whether a relative path (from /workspace) should be excluded — any segment in the set.
pub fn is_excluded_path(relative_path: &str) -> bool {
    relative_path
        .split('/')
        .any(|seg| BACKUP_EXCLUSION_SET.contains(&seg))
}

#[cfg(test)]
mod tests {
    // The expected-value chains below transcribe the TS decision trees branch for branch, so
    // consecutive branches sometimes share a result on purpose (parity over brevity).
    #![allow(clippy::if_same_then_else)]
    use super::*;
    use proptest::prelude::*;

    fn timestamp() -> impl Strategy<Value = i64> {
        1_000_000_000_000i64..=2_000_000_000_000i64
    }
    fn nullable_ts() -> impl Strategy<Value = Option<i64>> {
        prop_oneof![Just(None), timestamp().prop_map(Some)]
    }
    fn debounce_state() -> impl Strategy<Value = DebounceState> {
        (
            nullable_ts(),
            timestamp(),
            any::<bool>(),
            nullable_ts(),
            0u32..=20,
            nullable_ts(),
        )
            .prop_map(
                |(
                    last_event_ms,
                    now_ms,
                    backup_in_flight,
                    last_trigger_ms,
                    cf,
                    last_failure_ms,
                )| {
                    DebounceState {
                        last_event_ms,
                        now_ms,
                        backup_in_flight,
                        last_trigger_ms,
                        consecutive_failures: cf,
                        last_failure_ms,
                    }
                },
            )
    }
    fn safe_segment() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-zA-Z0-9._-]{1,20}")
            .unwrap()
            .prop_filter("not an excluded name", |s| {
                !BACKUP_EXCLUSION_SET.contains(&s.as_str())
            })
    }

    proptest! {
        // Property 1: the debounce decision matches the specified priority order.
        #[test]
        fn debounce_triggers_iff_conditions_met(
            state in debounce_state().prop_filter("not in flight", |s| !s.backup_in_flight),
        ) {
            let decision = plan_debounce(state);
            let in_backoff = state.consecutive_failures > 0
                && state.last_failure_ms.is_some()
                && state.now_ms - state.last_failure_ms.unwrap()
                    < backoff_delay_ms(state.consecutive_failures);
            let expected = if state.last_trigger_ms.is_none() {
                DebounceDecision::Trigger
            } else if in_backoff {
                DebounceDecision::Wait
            } else if state.now_ms - state.last_trigger_ms.unwrap() >= BACKUP_FALLBACK_INTERVAL_MS {
                DebounceDecision::Trigger
            } else if state.consecutive_failures > 0 {
                DebounceDecision::Trigger
            } else if state.last_event_ms.is_none()
                || state.last_event_ms.unwrap() < state.last_trigger_ms.unwrap()
            {
                DebounceDecision::Wait
            } else if state.now_ms - state.last_event_ms.unwrap() >= BACKUP_DEBOUNCE_MS {
                DebounceDecision::Trigger
            } else {
                DebounceDecision::Wait
            };
            prop_assert_eq!(decision, expected);
        }

        // Property 2: in-flight always coalesces.
        #[test]
        fn in_flight_always_coalesces(
            state in debounce_state().prop_filter("in flight", |s| s.backup_in_flight),
        ) {
            prop_assert_eq!(plan_debounce(state), DebounceDecision::Coalesce);
        }

        // Property 3: a path with any excluded segment is excluded; all-safe is not.
        #[test]
        fn excludes_any_matching_segment(
            prefix in prop::collection::vec(safe_segment(), 0..=3),
            excluded in prop::sample::select(BACKUP_EXCLUSION_SET.to_vec()),
            suffix in prop::collection::vec(safe_segment(), 0..=3),
        ) {
            let mut parts = prefix;
            parts.push(excluded.to_string());
            parts.extend(suffix);
            prop_assert!(is_excluded_path(&parts.join("/")));
        }

        #[test]
        fn does_not_exclude_safe_paths(segs in prop::collection::vec(safe_segment(), 1..=5)) {
            prop_assert!(!is_excluded_path(&segs.join("/")));
        }

        // Property 4: prune is due iff never pruned or the cadence elapsed.
        #[test]
        fn prune_due_iff_never_or_cadence_elapsed(last in nullable_ts(), now in timestamp()) {
            let decision = plan_prune(PruneState { last_prune_ms: last, now_ms: now });
            let expected = match last {
                None => PruneDecision::PruneDue,
                Some(l) if now - l >= BACKUP_PRUNE_CADENCE_MS => PruneDecision::PruneDue,
                Some(_) => PruneDecision::Skip,
            };
            prop_assert_eq!(decision, expected);
        }

        // Property 5 + 9: backoff is BASE * 2^min(n,4) and non-decreasing up to n=4.
        #[test]
        fn backoff_formula(n in 0u32..=20) {
            prop_assert_eq!(backoff_delay_ms(n), BACKUP_BASE_BACKOFF_MS * (1i64 << n.min(4)));
        }

        #[test]
        fn backoff_monotonic(a in 0u32..=3, off in 1u32..=4) {
            let b = a + off;
            if b <= 4 {
                prop_assert!(backoff_delay_ms(a) <= backoff_delay_ms(b));
            }
        }

        // Property 6: notify iff failures >= threshold.
        #[test]
        fn notify_threshold(n in 0u32..=20) {
            prop_assert_eq!(should_notify(n), n >= BACKUP_FAILURE_THRESHOLD);
        }

        // Property 7: dormant with no repo; enabled with all four present.
        #[test]
        fn gate_enabled_with_all_four(
            repo in "[ -~]{1,20}",
            pw in "[ -~]{1,20}",
            key in "[ -~]{1,20}",
            secret in "[ -~]{1,20}",
        ) {
            let gate = evaluate_backup_gate(&BackupGateEnv {
                restic_repository: Some(repo),
                restic_password: Some(pw),
                aws_access_key_id: Some(key),
                aws_secret_access_key: Some(secret),
            });
            prop_assert_eq!(gate, GateResult::Enabled);
        }

        // Property 8: doctor status across the state space.
        #[test]
        fn doctor_status(
            gate in prop_oneof![
                Just(GateResult::Dormant),
                Just(GateResult::Enabled),
                Just(GateResult::Misconfigured { missing: vec!["RESTIC_PASSWORD".to_string()] }),
            ],
            last_success in nullable_ts(),
            cf in 0u32..=20,
            now in timestamp(),
        ) {
            let state = DoctorBackupState {
                gate_result: gate.clone(),
                last_success_ms: last_success,
                consecutive_failures: cf,
                now_ms: now,
            };
            let status = plan_doctor_backup_status(&state);
            let expected = match gate {
                GateResult::Dormant => DoctorBackupStatus::NotConfigured,
                GateResult::Misconfigured { .. } => DoctorBackupStatus::Fail,
                GateResult::Enabled => {
                    if cf >= BACKUP_FAILURE_THRESHOLD {
                        DoctorBackupStatus::Fail
                    } else if last_success.is_none_or(|l| now - l >= BACKUP_STALENESS_MS) {
                        DoctorBackupStatus::Warn
                    } else {
                        DoctorBackupStatus::Ok
                    }
                }
            };
            prop_assert_eq!(status, expected);
        }
    }

    #[test]
    fn gate_dormant_and_misconfigured() {
        for repo in [None, Some(String::new())] {
            let gate = evaluate_backup_gate(&BackupGateEnv {
                restic_repository: repo,
                ..Default::default()
            });
            assert_eq!(gate, GateResult::Dormant);
        }
        let gate = evaluate_backup_gate(&BackupGateEnv {
            restic_repository: Some("s3:test".to_string()),
            ..Default::default()
        });
        match gate {
            GateResult::Misconfigured { missing } => {
                assert!(missing.contains(&"RESTIC_PASSWORD".to_string()));
                assert!(missing.contains(&"AWS_ACCESS_KEY_ID".to_string()));
                assert!(missing.contains(&"AWS_SECRET_ACCESS_KEY".to_string()));
            }
            other => panic!("expected misconfigured, got {other:?}"),
        }
    }

    #[test]
    fn classify_repo_probe_cases() {
        assert_eq!(classify_repo_probe(0, ""), RepoProbe::Ready);
        assert_eq!(classify_repo_probe(12, ""), RepoProbe::WrongPassword);
        assert_eq!(
            classify_repo_probe(1, "wrong password or no key found"),
            RepoProbe::WrongPassword
        );
        assert_eq!(
            classify_repo_probe(1, "no key found"),
            RepoProbe::WrongPassword
        );
        assert_eq!(classify_repo_probe(10, ""), RepoProbe::Missing);
        assert_eq!(
            classify_repo_probe(1, "repository does not exist"),
            RepoProbe::Missing
        );
        // A transient error is never mistaken for a missing repository.
        assert_eq!(
            classify_repo_probe(1, "connection refused"),
            RepoProbe::Unreachable
        );
    }

    #[test]
    fn excludes_nested_segments() {
        assert!(is_excluded_path("project/node_modules/pkg/index.js"));
        assert!(is_excluded_path(".git"));
        assert!(!is_excluded_path("src/main.rs"));
        assert!(!is_excluded_path("a/b/c"));
    }
}
