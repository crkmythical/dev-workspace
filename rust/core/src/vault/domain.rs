//! Vault domain: lifecycle rules and the destroy-phase table.
//!
//! A faithful port of packages/core/src/vault/domain.ts. Pure: no I/O and no runtime imports,
//! so every rule is testable without gocryptfs/FUSE. The tests below mirror
//! packages/core/tests/vault-domain.test.ts.

/// "workspace" for the main vault; a plugin that owns a volume picks its own id.
pub type VaultId = String;

/// One encrypted volume. The workspace and each plugin's volume are instances of one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultSpec {
    pub id: VaultId,
    pub cipher_dir: String,
    pub mount_point: String,
    /// Let users other than the one who mounted it use the volume. FUSE refuses every other uid
    /// by default; a volume that holds a root file system needs it (tor runs as debian-tor, apt
    /// downloads as _apt, and both would get "Permission denied" without it).
    pub allow_other: bool,
}

/// "destroyed" is terminal: the self-destruct completion marker exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultState {
    Uninitialized,
    Locked,
    Unlocked,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultCommand {
    Init,
    Unlock { passphrase: String },
    Lock,
    Destroy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoopReason {
    AlreadyInitialized,
    AlreadyUnlocked,
    AlreadyLocked,
    AlreadyDestroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    NotInitialized,
    EmptyPassphrase,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// `warn_short_passphrase` is only meaningful for unlock (false for init/lock/destroy).
    Perform {
        warn_short_passphrase: bool,
    },
    Noop(NoopReason),
    Reject(RejectReason),
}

/// Plain-language lines for a failed unlock, from what gocryptfs printed. A wrong passphrase and
/// a mount the container is not allowed to make both leave the vault locked, but they are fixed
/// in opposite places. `password_generated`: the passphrase tried is one this start-up made up
/// because PASSWORD is empty, and it does not survive recreating the container.
pub fn explain_unlock_failure(detail: &str, password_generated: bool) -> Vec<String> {
    let text = detail.to_lowercase();
    if text.contains("password incorrect") {
        if !password_generated {
            return vec!["The passphrase is wrong.".to_string()];
        }
        return vec![
            "The passphrase is wrong.".to_string(),
            "PASSWORD is not set, so this start generated a new password. A generated password lives".to_string(),
            "only in the container and does not survive recreating it (docker compose up -d after a".to_string(),
            "rebuild, docker rm). If the vault was made under an earlier one, open it with the old".to_string(),
            "password: docker exec -it dev-workspace unlock-vault. Then set PASSWORD in .env to it.".to_string(),
        ];
    }
    if text.contains("mount failed") && text.contains("permission denied") {
        return vec![
            "The container is not allowed to mount a FUSE filesystem.".to_string(),
            "On a Linux host with AppArmor (Ubuntu, Debian) add docker-compose.apparmor.yml: set".to_string(),
            "COMPOSE_FILE in .env (see .env.example) or start with -f docker-compose.yml -f docker-compose.apparmor.yml".to_string(),
        ];
    }
    if text.contains("fuse device not found") || text.contains("/dev/fuse") {
        return vec![
            "/dev/fuse is not available in the container (docker-compose.yml passes it as a device)."
                .to_string(),
        ];
    }
    vec![]
}

// --- Releasing a busy mount ---

/// The PIDs in `fuser`'s output: it prints them, space separated, on stdout. Zero is dropped
/// (a `kill` aimed at PID 0 would hit the whole process group).
pub fn parse_fuser_pids(stdout: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for token in stdout.split(|c: char| !c.is_ascii_digit()) {
        if token.is_empty() {
            continue;
        }
        if let Ok(n) = token.parse::<u64>() {
            if n > 0 {
                out.push(n as u32);
            }
        }
    }
    out
}

/// `pid` followed by every process above it, up to PID 1. Stops at a process whose parent cannot
/// be read (it has exited) and never loops.
pub fn lineage<F: Fn(u32) -> Option<u32>>(pid: u32, parent_of: F) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut current = Some(pid);
    while let Some(c) = current {
        if c == 0 || chain.contains(&c) {
            break;
        }
        chain.push(c);
        current = parent_of(c);
    }
    chain
}

/// Which of the processes using a mount may be killed to free it: all of them except `protect`.
/// The caller passes its own lineage; killing its shell would hang up the terminal and take the
/// caller down before the keys are erased.
pub fn killable(holders: &[u32], protect: &[u32]) -> Vec<u32> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &pid in holders {
        if seen.insert(pid) && !protect.contains(&pid) {
            out.push(pid);
        }
    }
    out
}

/// Only a locked vault needs a passphrase, so callers prompt lazily after this check.
pub fn needs_passphrase(state: VaultState) -> bool {
    state == VaultState::Locked
}

/// Decide what a command should do in a given state (init/unlock/lock idempotent; unlock refuses
/// an uninitialized vault and an empty passphrase; a destroyed vault rejects everything but a
/// no-op repeat of destroy). gocryptfs adjudicates the passphrase at mount time, not here.
pub fn decide(state: VaultState, cmd: &VaultCommand) -> Decision {
    use VaultState::*;
    if state == Destroyed {
        return match cmd {
            VaultCommand::Destroy => Decision::Noop(NoopReason::AlreadyDestroyed),
            _ => Decision::Reject(RejectReason::Destroyed),
        };
    }
    match cmd {
        VaultCommand::Init => {
            if state == Uninitialized {
                Decision::Perform {
                    warn_short_passphrase: false,
                }
            } else {
                Decision::Noop(NoopReason::AlreadyInitialized)
            }
        }
        VaultCommand::Unlock { passphrase } => {
            if state == Unlocked {
                return Decision::Noop(NoopReason::AlreadyUnlocked);
            }
            if state == Uninitialized {
                return Decision::Reject(RejectReason::NotInitialized);
            }
            if passphrase.is_empty() {
                return Decision::Reject(RejectReason::EmptyPassphrase);
            }
            Decision::Perform {
                warn_short_passphrase: passphrase.chars().count() < 8,
            }
        }
        VaultCommand::Lock => {
            if state == Unlocked {
                Decision::Perform {
                    warn_short_passphrase: false,
                }
            } else {
                Decision::Noop(NoopReason::AlreadyLocked)
            }
        }
        VaultCommand::Destroy => Decision::Perform {
            warn_short_passphrase: false,
        },
    }
}

// --- vault-sync (ciphertext replication) ---

/// Consecutive failed pushes before a notification is written.
pub const SYNC_FAILURE_THRESHOLD: u32 = 3;

/// The .gitignore written into the ciphertext repository when a vault is created.
pub const VAULT_GITIGNORE: &str = "node_modules/
target/
build/
dist/
.cache/
**/*.log
.idea/
.vscode/
__pycache__/
.DS_Store
*.tmp
*.swp
shared/
";

/// Large binaries go through git-lfs in the ciphertext repository.
pub const VAULT_GITATTRIBUTES: &str =
    "*.bin filter=lfs diff=lfs merge=lfs -text\n*.jar filter=lfs diff=lfs merge=lfs -text\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncState {
    /// Does the ciphertext repository exist and have an `origin` to push to? (Observed from the
    /// repository, not VAULT_GIT_REPO: cron does not pass the container's environment on.)
    pub has_remote: bool,
    /// Is the workspace mount present?
    pub mounted: bool,
    /// Did `git add -A` then `git diff --cached --quiet` report changes?
    pub has_staged_changes: bool,
    /// Commits that exist here but not on origin yet, because an earlier push failed.
    pub has_unpushed_commits: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    NoRemote,
    NotMounted,
    NoChanges,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPlan {
    Skip { reason: SkipReason, exit_code: i32 },
    CommitAndPush,
    PushOnly,
}

/// Opt-in (no remote → do nothing, never fail), locked-boundary (not mounted → never commit),
/// quiet-cycle (mounted, nothing staged or waiting → never commit) and retry (a commit whose
/// push failed goes out on a later cycle) invariants.
pub fn plan_sync(state: SyncState) -> SyncPlan {
    if !state.has_remote {
        return SyncPlan::Skip {
            reason: SkipReason::NoRemote,
            exit_code: 0,
        };
    }
    if !state.mounted {
        return SyncPlan::Skip {
            reason: SkipReason::NotMounted,
            exit_code: 0,
        };
    }
    if state.has_staged_changes {
        return SyncPlan::CommitAndPush;
    }
    if state.has_unpushed_commits {
        return SyncPlan::PushOnly;
    }
    SyncPlan::Skip {
        reason: SkipReason::NoChanges,
        exit_code: 0,
    }
}

// --- destruction ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestroyPhase {
    pub id: &'static str,
    pub irreversible: bool,
    pub label: &'static str,
}

/// Destroy phases, in the only order they may run. `shred-keys` is the Point of No Return.
pub const DESTROY_PHASES: &[DestroyPhase] = &[
    DestroyPhase {
        id: "quiesce",
        irreversible: false,
        label: "[Phase 0] Stopping vault-sync and optional remote cleanup...",
    },
    DestroyPhase {
        id: "unmount",
        irreversible: false,
        label: "[Phase 1] Unmounting vaults...",
    },
    DestroyPhase {
        id: "shred-keys",
        irreversible: true,
        label: "[Phase 2] Shredding vault master keys...",
    },
    DestroyPhase {
        id: "shred-data",
        irreversible: true,
        label: "[Phase 3] Shredding cipher data (this may take a while)...",
    },
    DestroyPhase {
        id: "scrub-credentials",
        irreversible: true,
        label: "[Phase 4] Cleaning credentials and config...",
    },
    DestroyPhase {
        id: "mark-complete",
        irreversible: true,
        label: "[Phase 5] Finalizing...",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn unlock(p: &str) -> VaultCommand {
        VaultCommand::Unlock {
            passphrase: p.to_string(),
        }
    }

    // A brace-free predicate: `prop_assert!` stringifies its condition into a format string, so
    // `matches!(d, Perform { .. })` inline would be read as a (broken) format placeholder.
    fn is_perform(d: Decision) -> bool {
        matches!(d, Decision::Perform { .. })
    }

    // Strategies mirroring the fast-check arbitraries (ASCII, so char count == TS .length).
    fn passphrase() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[ -~]{0,64}").unwrap()
    }
    fn non_empty() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[ -~]{1,64}").unwrap()
    }
    fn state() -> impl Strategy<Value = VaultState> {
        prop_oneof![
            Just(VaultState::Uninitialized),
            Just(VaultState::Locked),
            Just(VaultState::Unlocked),
            Just(VaultState::Destroyed),
        ]
    }
    fn command() -> impl Strategy<Value = VaultCommand> {
        prop_oneof![
            Just(VaultCommand::Init),
            passphrase().prop_map(|p| VaultCommand::Unlock { passphrase: p }),
            Just(VaultCommand::Lock),
            Just(VaultCommand::Destroy),
        ]
    }

    proptest! {
        #[test]
        fn decide_is_total(s in state(), c in command()) {
            // Always exactly one well-formed decision.
            let _ = decide(s, &c);
        }

        #[test]
        fn unlock_on_unlocked_is_always_noop(p in passphrase()) {
            prop_assert_eq!(decide(VaultState::Unlocked, &unlock(&p)), Decision::Noop(NoopReason::AlreadyUnlocked));
        }

        #[test]
        fn round_trip(p in non_empty()) {
            prop_assert!(is_perform(decide(VaultState::Locked, &unlock(&p))));
            prop_assert!(is_perform(decide(VaultState::Unlocked, &VaultCommand::Lock)));
            prop_assert!(matches!(decide(VaultState::Locked, &VaultCommand::Lock), Decision::Noop(_)));
        }

        #[test]
        fn repeated_unlocks_only_mount_once(p in non_empty(), repeats in 1u32..5) {
            prop_assert!(is_perform(decide(VaultState::Locked, &unlock(&p))));
            for _ in 0..repeats {
                prop_assert!(matches!(decide(VaultState::Unlocked, &unlock(&p)), Decision::Noop(_)));
            }
        }

        #[test]
        fn non_empty_passphrase_always_attempts(p in non_empty()) {
            prop_assert!(is_perform(decide(VaultState::Locked, &unlock(&p))));
        }

        #[test]
        fn uninitialized_never_mounts(p in passphrase()) {
            prop_assert_eq!(decide(VaultState::Uninitialized, &unlock(&p)), Decision::Reject(RejectReason::NotInitialized));
        }

        #[test]
        fn short_passphrase_is_flagged_not_blocked(p in non_empty()) {
            match decide(VaultState::Locked, &unlock(&p)) {
                Decision::Perform { warn_short_passphrase } => {
                    prop_assert_eq!(warn_short_passphrase, p.chars().count() < 8);
                }
                other => prop_assert!(false, "expected perform, got {:?}", other),
            }
        }

        #[test]
        fn destroyed_is_terminal(c in command()) {
            let d = decide(VaultState::Destroyed, &c);
            if c == VaultCommand::Destroy {
                prop_assert_eq!(d, Decision::Noop(NoopReason::AlreadyDestroyed));
            } else {
                prop_assert_eq!(d, Decision::Reject(RejectReason::Destroyed));
            }
        }

        #[test]
        fn sync_never_commits_when_not_mounted(staged in any::<bool>(), unpushed in any::<bool>()) {
            let plan = plan_sync(SyncState {
                has_remote: true, mounted: false, has_staged_changes: staged, has_unpushed_commits: unpushed,
            });
            prop_assert_eq!(plan, SyncPlan::Skip { reason: SkipReason::NotMounted, exit_code: 0 });
        }

        #[test]
        fn killable_never_touches_protected_and_is_a_subset(
            holders in prop::collection::vec(1u32..=50, 0..20),
            protect in prop::collection::vec(1u32..=50, 0..20),
        ) {
            let victims = killable(&holders, &protect);
            for pid in &victims {
                prop_assert!(!protect.contains(pid));
                prop_assert!(holders.contains(pid));
            }
            let unique: std::collections::HashSet<_> = victims.iter().collect();
            prop_assert_eq!(unique.len(), victims.len());
        }
    }

    #[test]
    fn empty_passphrase_never_mounts() {
        assert_eq!(
            decide(VaultState::Locked, &unlock("")),
            Decision::Reject(RejectReason::EmptyPassphrase)
        );
    }

    #[test]
    fn init_performs_only_on_uninitialized() {
        assert!(matches!(
            decide(VaultState::Uninitialized, &VaultCommand::Init),
            Decision::Perform { .. }
        ));
        for s in [VaultState::Locked, VaultState::Unlocked] {
            assert_eq!(
                decide(s, &VaultCommand::Init),
                Decision::Noop(NoopReason::AlreadyInitialized)
            );
        }
    }

    #[test]
    fn lock_performs_only_on_unlocked() {
        assert!(matches!(
            decide(VaultState::Unlocked, &VaultCommand::Lock),
            Decision::Perform { .. }
        ));
        for s in [VaultState::Locked, VaultState::Uninitialized] {
            assert_eq!(
                decide(s, &VaultCommand::Lock),
                Decision::Noop(NoopReason::AlreadyLocked)
            );
        }
    }

    #[test]
    fn destroy_performs_from_every_live_state() {
        for s in [
            VaultState::Uninitialized,
            VaultState::Locked,
            VaultState::Unlocked,
        ] {
            assert!(matches!(
                decide(s, &VaultCommand::Destroy),
                Decision::Perform { .. }
            ));
        }
    }

    #[test]
    fn needs_passphrase_only_when_locked() {
        assert!(needs_passphrase(VaultState::Locked));
        for s in [
            VaultState::Uninitialized,
            VaultState::Unlocked,
            VaultState::Destroyed,
        ] {
            assert!(!needs_passphrase(s));
        }
    }

    #[test]
    fn sync_gating_examples() {
        let base = SyncState {
            has_remote: true,
            mounted: true,
            has_staged_changes: false,
            has_unpushed_commits: false,
        };
        assert_eq!(
            plan_sync(base),
            SyncPlan::Skip {
                reason: SkipReason::NoChanges,
                exit_code: 0
            }
        );
        assert_eq!(
            plan_sync(SyncState {
                has_staged_changes: true,
                ..base
            }),
            SyncPlan::CommitAndPush
        );
        assert_eq!(
            plan_sync(SyncState {
                has_remote: false,
                has_staged_changes: true,
                ..base
            }),
            SyncPlan::Skip {
                reason: SkipReason::NoRemote,
                exit_code: 0
            }
        );
        assert_eq!(
            plan_sync(SyncState {
                has_unpushed_commits: true,
                ..base
            }),
            SyncPlan::PushOnly
        );
        assert_eq!(
            plan_sync(SyncState {
                has_staged_changes: true,
                has_unpushed_commits: true,
                ..base
            }),
            SyncPlan::CommitAndPush
        );
    }

    #[test]
    fn destroy_phases_order_and_point_of_no_return() {
        let ids: Vec<_> = DESTROY_PHASES.iter().map(|p| p.id).collect();
        assert_eq!(
            ids,
            [
                "quiesce",
                "unmount",
                "shred-keys",
                "shred-data",
                "scrub-credentials",
                "mark-complete"
            ]
        );
        let first = DESTROY_PHASES.iter().position(|p| p.irreversible).unwrap();
        assert_eq!(DESTROY_PHASES[first].id, "shred-keys");
        for phase in &DESTROY_PHASES[first..] {
            assert!(phase.irreversible);
        }
        assert_eq!(DESTROY_PHASES.last().unwrap().id, "mark-complete");
    }

    const APPARMOR_DENIED: &str = "/usr/bin/fusermount3: mount failed: Permission denied\nfs.Mount failed: fusermount exited with code 256";
    const WRONG_PASSPHRASE: &str =
        "failed to unlock master key: cipher: message authentication failed\nPassword incorrect.";
    const NO_FUSE_DEVICE: &str = "/usr/bin/fusermount3: fuse device not found, try 'modprobe fuse' first\nfs.Mount failed: fusermount exited with code 256";

    #[test]
    fn explain_unlock_failure_cases() {
        assert_eq!(
            explain_unlock_failure(WRONG_PASSPHRASE, false),
            vec!["The passphrase is wrong."]
        );

        let apparmor = explain_unlock_failure(APPARMOR_DENIED, false).join("\n");
        assert!(apparmor.contains("not allowed to mount"));
        assert!(apparmor.contains("docker-compose.apparmor.yml"));
        assert!(!apparmor.contains("passphrase"));

        let no_fuse = explain_unlock_failure(NO_FUSE_DEVICE, false).join("\n");
        assert!(no_fuse.contains("/dev/fuse"));
        assert!(!no_fuse.contains("AppArmor"));

        let generated = explain_unlock_failure(WRONG_PASSPHRASE, true);
        assert_eq!(generated[0], "The passphrase is wrong.");
        let text = generated.join(" ");
        assert!(text.contains("generated a new password"));
        assert!(text.contains("recreating"));
        assert!(text.contains("unlock-vault"));

        // Generation is irrelevant to a failure that is not about the passphrase.
        assert_eq!(
            explain_unlock_failure(APPARMOR_DENIED, true),
            explain_unlock_failure(APPARMOR_DENIED, false)
        );
        assert!(explain_unlock_failure("", false).is_empty());
        assert!(explain_unlock_failure("something unexpected", false).is_empty());
        assert_eq!(
            explain_unlock_failure("PASSWORD INCORRECT.", false),
            vec!["The passphrase is wrong."]
        );
    }

    #[test]
    fn parse_fuser_pids_cases() {
        assert_eq!(parse_fuser_pids(" 555 561"), vec![555, 561]);
        assert_eq!(parse_fuser_pids(" 555"), vec![555]);
        assert_eq!(parse_fuser_pids(""), Vec::<u32>::new());
        assert_eq!(parse_fuser_pids("\n"), Vec::<u32>::new());
        assert_eq!(parse_fuser_pids("/workspace:  555c 561f"), vec![555, 561]);
        assert_eq!(parse_fuser_pids("0 12 000"), vec![12]);
    }

    #[test]
    fn lineage_cases() {
        let parents: std::collections::HashMap<u32, u32> =
            [(900, 800), (800, 700), (700, 90), (90, 1), (1, 0)]
                .into_iter()
                .collect();
        let parent_of = |pid: u32| parents.get(&pid).copied();
        assert_eq!(lineage(900, parent_of), vec![900, 800, 700, 90, 1]);
        assert_eq!(
            lineage(900, |pid| if pid == 800 { None } else { parent_of(pid) }),
            vec![900, 800]
        );
        assert_eq!(lineage(4242, |_| None), vec![4242]);
        assert_eq!(
            lineage(1, |pid| if pid == 1 { Some(2) } else { Some(1) }),
            vec![1, 2]
        );
    }

    #[test]
    fn killable_cases() {
        assert_eq!(
            killable(&[900, 800, 700, 123, 456], &[900, 800, 700, 90, 1]),
            vec![123, 456]
        );
        assert_eq!(killable(&[5, 6, 5], &[1]), vec![5, 6]);
        assert_eq!(killable(&[], &[1]), Vec::<u32>::new());
    }
}
