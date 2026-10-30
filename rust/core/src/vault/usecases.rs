//! Vault use cases: lock, unlock and destroy — a port of packages/core/src/vault/usecases.ts.
//!
//! Each ordering invariant is written exactly once here and exercised with fake ports (no
//! container needed): holders are stopped BEFORE any unmount (FUSE EBUSY safety); a destroy runs
//! its phases in the DESTROY_PHASES order, keeps going when a phase fails, and reports honestly
//! instead of claiming success.

use crate::vault::domain::{
    decide, needs_passphrase, Decision, RejectReason, VaultCommand, VaultSpec, DESTROY_PHASES,
};
use crate::vault::ports::{
    DestroyEffects, HolderControl, RemoteRepoResult, Shredder, UnlockContext, UnlockHook,
    UnmountMode, VolumeDriver,
};

// --- lock ---

pub struct LockDeps<'a> {
    pub volume: &'a dyn VolumeDriver,
    pub holders: &'a dyn HolderControl,
    /// Where the volume reports what it had to do to release the mount (e.g. kill processes).
    pub log: Option<&'a dyn Fn(&str)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockResult {
    /// reason: already-locked.
    Noop,
    /// reason: destroyed.
    Rejected,
    Locked,
    /// The unmount was refused (open handles). The caller may retry with mode "lazy".
    Busy,
}

pub fn lock_vault(
    spec: &VaultSpec,
    deps: &LockDeps<'_>,
    mode: UnmountMode,
) -> Result<LockResult, String> {
    match decide(deps.volume.state(spec), &VaultCommand::Lock) {
        Decision::Noop(_) => return Ok(LockResult::Noop),
        Decision::Reject(_) => return Ok(LockResult::Rejected),
        Decision::Perform { .. } => {}
    }

    // The ordering invariant: holders first, then unmount.
    deps.holders.stop_all();
    let noop = |_: &str| {};
    let report: &dyn Fn(&str) = match deps.log {
        Some(l) => l,
        None => &noop,
    };
    let released = deps.volume.unmount(spec, mode, report)?;
    Ok(if released {
        LockResult::Locked
    } else {
        LockResult::Busy
    })
}

/// The lock came back "busy" and the caller chose not to force it. The vault stays mounted, so
/// what the attempt stopped (desktops, the backup watcher) must run again.
pub fn cancel_lock(holders: &dyn HolderControl) -> Vec<String> {
    holders.restore()
}

// --- unlock ---

pub struct UnlockDeps<'a> {
    pub volume: &'a dyn VolumeDriver,
    /// Run in order after a successful mount.
    pub hooks: &'a [&'a dyn UnlockHook],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockRejectReason {
    NotInitialized,
    EmptyPassphrase,
    Destroyed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlockResult {
    /// reason: already-unlocked.
    Noop,
    Rejected {
        reason: UnlockRejectReason,
    },
    /// The mount itself failed (wrong passphrase or mount error).
    Failed {
        detail: String,
    },
    Unlocked {
        warn_short_passphrase: bool,
        hook_failures: Vec<String>,
    },
}

/// One unlock path for both the interactive `unlock-vault` command and the container's boot-time
/// auto-unlock. The passphrase is read lazily, only when a mount is actually going to be attempted.
pub fn unlock_vault(
    spec: &VaultSpec,
    read_passphrase: impl Fn() -> String,
    deps: &UnlockDeps<'_>,
) -> UnlockResult {
    let state = deps.volume.state(spec);
    let passphrase = if needs_passphrase(state) {
        read_passphrase()
    } else {
        String::new()
    };
    let decision = decide(
        state,
        &VaultCommand::Unlock {
            passphrase: passphrase.clone(),
        },
    );
    let warn_short_passphrase = match decision {
        Decision::Noop(_) => return UnlockResult::Noop,
        Decision::Reject(reason) => {
            return UnlockResult::Rejected {
                reason: map_reject(reason),
            }
        }
        Decision::Perform {
            warn_short_passphrase,
        } => warn_short_passphrase,
    };

    let mounted = deps.volume.mount(spec, &passphrase);
    if !mounted.ok {
        return UnlockResult::Failed {
            detail: mounted.detail.unwrap_or_default(),
        };
    }

    let mut hook_failures: Vec<String> = Vec::new();
    for hook in deps.hooks {
        let ctx = UnlockContext {
            spec,
            passphrase: &passphrase,
        };
        if let Err(err) = hook.run(&ctx) {
            hook_failures.push(format!("{}: {}", hook.name(), err));
        }
    }
    UnlockResult::Unlocked {
        warn_short_passphrase,
        hook_failures,
    }
}

fn map_reject(reason: RejectReason) -> UnlockRejectReason {
    match reason {
        RejectReason::NotInitialized => UnlockRejectReason::NotInitialized,
        RejectReason::EmptyPassphrase => UnlockRejectReason::EmptyPassphrase,
        RejectReason::Destroyed => UnlockRejectReason::Destroyed,
    }
}

// --- destroy ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestroyOptions {
    pub skip_shred: bool,
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseReport {
    pub phase: &'static str,
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestroyReport {
    pub phases: Vec<PhaseReport>,
    /// True only if every vault's gocryptfs.conf is gone.
    pub keys_destroyed: bool,
    /// True only if every phase succeeded.
    pub complete: bool,
}

pub struct DestroyDeps<'a> {
    pub specs: &'a [VaultSpec],
    pub volume: &'a dyn VolumeDriver,
    pub holders: &'a dyn HolderControl,
    pub shredder: &'a dyn Shredder,
    pub effects: &'a dyn DestroyEffects,
    pub now: &'a dyn Fn() -> String,
    pub log: &'a dyn Fn(&str),
}

const KEY_FILE: &str = "gocryptfs.conf";
const KEY_SHRED_PASSES: u32 = 3;

fn label_of(id: &str) -> &'static str {
    DESTROY_PHASES
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.label)
        .unwrap_or("")
}

/// One phase's effects. `Ok(Some(detail))` records a note on a success (e.g. "skipped");
/// `Err(detail)` is a phase failure the loop records before moving on.
fn run_phase(
    id: &str,
    deps: &DestroyDeps<'_>,
    opts: DestroyOptions,
    keys_destroyed: &mut bool,
) -> Result<Option<String>, String> {
    let log = deps.log;
    match id {
        "quiesce" => {
            log(label_of("quiesce"));
            deps.effects.quiesce_services()?;
            if opts.remote {
                let result = deps.effects.delete_remote_repo()?;
                if result == RemoteRepoResult::SkippedNoToken {
                    log("  WARNING: No github-token found, cannot delete remote repo.");
                }
                if result == RemoteRepoResult::Failed {
                    return Err("remote repo deletion failed".to_string());
                }
            }
            Ok(None)
        }
        "unmount" => {
            log(label_of("unmount"));
            deps.holders.stop_all();
            let mut stuck: Vec<String> = Vec::new();
            for spec in deps.specs {
                if !deps.volume.unmount(spec, UnmountMode::Escalate, log)? {
                    stuck.push(spec.id.clone());
                }
            }
            if !stuck.is_empty() {
                return Err(format!("could not release mount(s): {}", stuck.join(", ")));
            }
            Ok(None)
        }
        "shred-keys" => {
            log(label_of("shred-keys"));
            let mut failed: Vec<String> = Vec::new();
            for spec in deps.specs {
                let path = format!("{}/{}", spec.cipher_dir, KEY_FILE);
                if !deps.shredder.shred(&path, KEY_SHRED_PASSES) {
                    failed.push(spec.id.clone());
                }
            }
            *keys_destroyed = failed.is_empty();
            if *keys_destroyed {
                log(&format!("  *** POINT OF NO RETURN *** {}", (deps.now)()));
                return Ok(None);
            }
            log(&format!(
                "  !!! KEY ERASURE FAILED for: {} (the vault may still be decryptable)",
                failed.join(", ")
            ));
            Err(format!("key erasure failed: {}", failed.join(", ")))
        }
        "shred-data" => {
            if opts.skip_shred {
                log("[Phase 3] Skipped (--skip-shred).");
                return Ok(Some("skipped".to_string()));
            }
            log(label_of("shred-data"));
            for spec in deps.specs {
                deps.effects.shred_tree(&spec.cipher_dir)?;
            }
            Ok(None)
        }
        "scrub-credentials" => {
            log(label_of("scrub-credentials"));
            deps.effects.scrub_credentials()?;
            Ok(None)
        }
        "mark-complete" => {
            log(label_of("mark-complete"));
            deps.effects.clear_shell_history()?;
            deps.effects.write_completion_marker(&(deps.now)())?;
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Best-effort, ordered destruction. A failing phase is recorded and the next phase still runs
/// (stopping half way would leave a half-destroyed vault), but the report never claims more than
/// what actually happened.
pub fn destroy_workspace(deps: &DestroyDeps<'_>, opts: DestroyOptions) -> DestroyReport {
    let mut keys_destroyed = false;
    let mut phases: Vec<PhaseReport> = Vec::new();
    for phase in DESTROY_PHASES {
        match run_phase(phase.id, deps, opts, &mut keys_destroyed) {
            Ok(detail) => phases.push(PhaseReport {
                phase: phase.id,
                ok: true,
                detail,
            }),
            Err(detail) => {
                (deps.log)(&format!("  WARNING: {} failed: {}", phase.id, detail));
                phases.push(PhaseReport {
                    phase: phase.id,
                    ok: false,
                    detail: Some(detail),
                });
            }
        }
    }
    let complete = phases.iter().all(|p| p.ok);
    (deps.log)(if complete {
        "destroyWorkspace complete."
    } else {
        "destroyWorkspace finished WITH FAILURES."
    });
    DestroyReport {
        phases,
        keys_destroyed,
        complete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::domain::VaultState;
    use crate::vault::ports::{MountOutcome, UnlockContext};
    use std::cell::{Cell, RefCell};
    use std::collections::HashSet;
    use std::rc::Rc;

    const NOW: &str = "2026-01-01T00:00:00.000Z";

    type Calls = Rc<RefCell<Vec<String>>>;

    fn new_calls() -> Calls {
        Rc::new(RefCell::new(Vec::new()))
    }

    fn workspace() -> VaultSpec {
        VaultSpec {
            id: "workspace".to_string(),
            cipher_dir: "/c/ws".to_string(),
            mount_point: "/m/ws".to_string(),
            allow_other: false,
        }
    }
    /// Any further encrypted volume (a plugin's): the use cases treat every VaultSpec alike.
    fn second() -> VaultSpec {
        VaultSpec {
            id: "second".to_string(),
            cipher_dir: "/c/2nd".to_string(),
            mount_point: "/m/2nd".to_string(),
            allow_other: false,
        }
    }
    fn specs() -> Vec<VaultSpec> {
        vec![workspace(), second()]
    }

    // --- fakes ---

    #[derive(Default, Clone, Copy)]
    struct UnmountResults {
        graceful: Option<bool>,
        lazy: Option<bool>,
        escalate: Option<bool>,
    }
    impl UnmountResults {
        fn get(self, mode: UnmountMode) -> bool {
            match mode {
                UnmountMode::Graceful => self.graceful,
                UnmountMode::Lazy => self.lazy,
                UnmountMode::Escalate => self.escalate,
            }
            .unwrap_or(true)
        }
    }

    #[derive(Default)]
    struct VolOpts {
        mount_ok: Option<bool>,
        mount_detail: Option<String>,
        unmount: UnmountResults,
        throw_on_unmount: bool,
        unmount_report: Option<String>,
    }

    struct FakeVolume {
        calls: Calls,
        state: Cell<VaultState>,
        opts: VolOpts,
    }
    fn fake_volume(initial: VaultState, calls: Calls, opts: VolOpts) -> FakeVolume {
        FakeVolume {
            calls,
            state: Cell::new(initial),
            opts,
        }
    }
    impl VolumeDriver for FakeVolume {
        fn state(&self, _spec: &VaultSpec) -> VaultState {
            self.state.get()
        }
        fn init(&self, spec: &VaultSpec, _passphrase: &str) -> bool {
            self.calls.borrow_mut().push(format!("init:{}", spec.id));
            self.state.set(VaultState::Locked);
            true
        }
        fn mount(&self, spec: &VaultSpec, _passphrase: &str) -> MountOutcome {
            self.calls.borrow_mut().push(format!("mount:{}", spec.id));
            if self.opts.mount_ok == Some(false) {
                return MountOutcome {
                    ok: false,
                    detail: Some(
                        self.opts
                            .mount_detail
                            .clone()
                            .unwrap_or_else(|| "bad passphrase".to_string()),
                    ),
                };
            }
            self.state.set(VaultState::Unlocked);
            MountOutcome {
                ok: true,
                detail: None,
            }
        }
        fn unmount(
            &self,
            spec: &VaultSpec,
            mode: UnmountMode,
            report: &dyn Fn(&str),
        ) -> Result<bool, String> {
            self.calls
                .borrow_mut()
                .push(format!("unmount:{}:{}", spec.id, mode.as_str()));
            if let Some(r) = &self.opts.unmount_report {
                report(r);
            }
            if self.opts.throw_on_unmount {
                return Err("fusermount exploded".to_string());
            }
            let released = self.opts.unmount.get(mode);
            if released {
                self.state.set(VaultState::Locked);
            }
            Ok(released)
        }
    }

    struct FakeHolders {
        calls: Calls,
    }
    impl HolderControl for FakeHolders {
        fn stop_all(&self) {
            self.calls.borrow_mut().push("holders.stopAll".to_string());
        }
        fn restore(&self) -> Vec<String> {
            self.calls.borrow_mut().push("holders.restore".to_string());
            vec!["backup watcher".to_string()]
        }
    }

    /// A holder with nothing to stop and no restore (the TS `{ stopAll: async () => {} }`).
    struct SilentHolders;
    impl HolderControl for SilentHolders {
        fn stop_all(&self) {}
    }

    struct FakeShredder {
        calls: Calls,
        fail_on: HashSet<String>,
    }
    impl Shredder for FakeShredder {
        fn shred(&self, path: &str, passes: u32) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("shred:{path}:{passes}"));
            !self.fail_on.contains(path)
        }
    }

    struct FakeEffects {
        calls: Calls,
        remote: RemoteRepoResult,
        scrub_throws: bool,
    }
    fn fake_effects(calls: Calls) -> FakeEffects {
        FakeEffects {
            calls,
            remote: RemoteRepoResult::Deleted,
            scrub_throws: false,
        }
    }
    impl DestroyEffects for FakeEffects {
        fn quiesce_services(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("effects.quiesce".to_string());
            Ok(())
        }
        fn delete_remote_repo(&self) -> Result<RemoteRepoResult, String> {
            self.calls
                .borrow_mut()
                .push("effects.deleteRemote".to_string());
            Ok(self.remote)
        }
        fn shred_tree(&self, dir: &str) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("effects.shredTree:{dir}"));
            Ok(())
        }
        fn scrub_credentials(&self) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push("effects.scrubCredentials".to_string());
            if self.scrub_throws {
                return Err("rm failed".to_string());
            }
            Ok(())
        }
        fn clear_shell_history(&self) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push("effects.clearHistory".to_string());
            Ok(())
        }
        fn write_completion_marker(&self, iso: &str) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("effects.marker:{iso}"));
            Ok(())
        }
    }

    struct FakeHook {
        name: String,
        calls: Calls,
        fail: bool,
    }
    fn hook(name: &str, calls: Calls) -> FakeHook {
        FakeHook {
            name: name.to_string(),
            calls,
            fail: false,
        }
    }
    fn failing_hook(name: &str, calls: Calls) -> FakeHook {
        FakeHook {
            name: name.to_string(),
            calls,
            fail: true,
        }
    }
    impl UnlockHook for FakeHook {
        fn name(&self) -> &str {
            &self.name
        }
        fn run(&self, _ctx: &UnlockContext<'_>) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("hook:{}", self.name));
            if self.fail {
                return Err(format!("{} exploded", self.name));
            }
            Ok(())
        }
    }

    struct SpyHook {
        seen: Rc<RefCell<Vec<(String, String)>>>,
    }
    impl UnlockHook for SpyHook {
        fn name(&self) -> &str {
            "spy"
        }
        fn run(&self, ctx: &UnlockContext<'_>) -> Result<(), String> {
            self.seen
                .borrow_mut()
                .push((ctx.spec.id.clone(), ctx.passphrase.to_string()));
            Ok(())
        }
    }

    const PLAIN: DestroyOptions = DestroyOptions {
        skip_shred: false,
        remote: false,
    };

    // --- lockVault ---

    #[test]
    fn lock_stops_every_holder_before_unmounting() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let result = lock_vault(
            &workspace(),
            &LockDeps {
                volume: &volume,
                holders: &holders,
                log: None,
            },
            UnmountMode::Graceful,
        )
        .unwrap();
        assert_eq!(result, LockResult::Locked);
        assert_eq!(
            *calls.borrow(),
            ["holders.stopAll", "unmount:workspace:graceful"]
        );
    }

    #[test]
    fn lock_is_a_noop_on_a_locked_vault() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Locked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let result = lock_vault(
            &workspace(),
            &LockDeps {
                volume: &volume,
                holders: &holders,
                log: None,
            },
            UnmountMode::Graceful,
        )
        .unwrap();
        assert_eq!(result, LockResult::Noop);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn lock_is_a_noop_on_an_uninitialized_vault() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Uninitialized, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let result = lock_vault(
            &workspace(),
            &LockDeps {
                volume: &volume,
                holders: &holders,
                log: None,
            },
            UnmountMode::Graceful,
        )
        .unwrap();
        assert_eq!(result, LockResult::Noop);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn lock_is_rejected_on_a_destroyed_vault_and_touches_nothing() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Destroyed, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let result = lock_vault(
            &workspace(),
            &LockDeps {
                volume: &volume,
                holders: &holders,
                log: None,
            },
            UnmountMode::Graceful,
        )
        .unwrap();
        assert_eq!(result, LockResult::Rejected);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn lock_reports_busy_when_a_graceful_unmount_is_refused() {
        let calls = new_calls();
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                unmount: UnmountResults {
                    graceful: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let result = lock_vault(
            &workspace(),
            &LockDeps {
                volume: &volume,
                holders: &holders,
                log: None,
            },
            UnmountMode::Graceful,
        )
        .unwrap();
        assert_eq!(result, LockResult::Busy);
    }

    #[test]
    fn a_forced_retry_stops_holders_again_and_detaches_lazily() {
        let calls = new_calls();
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                unmount: UnmountResults {
                    graceful: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let deps = LockDeps {
            volume: &volume,
            holders: &holders,
            log: None,
        };
        lock_vault(&workspace(), &deps, UnmountMode::Graceful).unwrap();
        calls.borrow_mut().clear();
        let forced = lock_vault(&workspace(), &deps, UnmountMode::Lazy).unwrap();
        assert_eq!(forced, LockResult::Locked);
        assert_eq!(
            *calls.borrow(),
            ["holders.stopAll", "unmount:workspace:lazy"]
        );
    }

    #[test]
    fn a_lock_the_user_declines_to_force_puts_back_what_it_stopped() {
        let calls = new_calls();
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                unmount: UnmountResults {
                    graceful: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let deps = LockDeps {
            volume: &volume,
            holders: &holders,
            log: None,
        };
        assert_eq!(
            lock_vault(&workspace(), &deps, UnmountMode::Graceful).unwrap(),
            LockResult::Busy
        );
        assert_eq!(
            cancel_lock(deps.holders),
            vec!["backup watcher".to_string()]
        );
        assert_eq!(
            *calls.borrow(),
            [
                "holders.stopAll",
                "unmount:workspace:graceful",
                "holders.restore"
            ]
        );
    }

    #[test]
    fn nothing_to_put_back_for_holders_that_cannot_be_restored() {
        assert_eq!(cancel_lock(&SilentHolders), Vec::<String>::new());
    }

    #[test]
    fn lets_the_volume_tell_the_user_what_it_had_to_do() {
        let logs = RefCell::new(Vec::<String>::new());
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        let volume = fake_volume(
            VaultState::Unlocked,
            new_calls(),
            VolOpts {
                unmount_report: Some("killed 2 process(es)".to_string()),
                ..Default::default()
            },
        );
        lock_vault(
            &second(),
            &LockDeps {
                volume: &volume,
                holders: &SilentHolders,
                log: Some(&log),
            },
            UnmountMode::Escalate,
        )
        .unwrap();
        assert_eq!(*logs.borrow(), ["killed 2 process(es)"]);
    }

    #[test]
    fn lock_works_for_any_further_vault_too() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let result = lock_vault(
            &second(),
            &LockDeps {
                volume: &volume,
                holders: &SilentHolders,
                log: None,
            },
            UnmountMode::Escalate,
        )
        .unwrap();
        assert_eq!(result, LockResult::Locked);
        assert_eq!(*calls.borrow(), ["unmount:second:escalate"]);
    }

    // --- unlockVault ---

    fn pass() -> String {
        "correct horse battery".to_string()
    }

    #[test]
    fn unlock_mounts_then_runs_the_hooks_in_order() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Locked, calls.clone(), VolOpts::default());
        let a = hook("a", calls.clone());
        let b = hook("b", calls.clone());
        let hooks: [&dyn UnlockHook; 2] = [&a, &b];
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(
            result,
            UnlockResult::Unlocked {
                warn_short_passphrase: false,
                hook_failures: vec![]
            }
        );
        assert_eq!(*calls.borrow(), ["mount:workspace", "hook:a", "hook:b"]);
    }

    #[test]
    fn unlock_hands_the_spec_and_passphrase_to_every_hook() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let volume = fake_volume(VaultState::Locked, new_calls(), VolOpts::default());
        let spy = SpyHook { seen: seen.clone() };
        let hooks: [&dyn UnlockHook; 1] = [&spy];
        unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(
            *seen.borrow(),
            [("workspace".to_string(), "correct horse battery".to_string())]
        );
    }

    #[test]
    fn an_already_unlocked_vault_is_a_noop() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let a = hook("a", calls.clone());
        let hooks: [&dyn UnlockHook; 1] = [&a];
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(result, UnlockResult::Noop);
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn asks_for_the_passphrase_only_when_a_mount_will_be_attempted() {
        let reads = Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            "correct horse battery".to_string()
        };
        for state in [
            VaultState::Unlocked,
            VaultState::Uninitialized,
            VaultState::Destroyed,
        ] {
            let volume = fake_volume(state, new_calls(), VolOpts::default());
            unlock_vault(
                &workspace(),
                read,
                &UnlockDeps {
                    volume: &volume,
                    hooks: &[],
                },
            );
        }
        assert_eq!(reads.get(), 0);
        let volume = fake_volume(VaultState::Locked, new_calls(), VolOpts::default());
        unlock_vault(
            &workspace(),
            read,
            &UnlockDeps {
                volume: &volume,
                hooks: &[],
            },
        );
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn unlock_rejects_an_uninitialized_vault() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Uninitialized, calls.clone(), VolOpts::default());
        let a = hook("a", calls.clone());
        let hooks: [&dyn UnlockHook; 1] = [&a];
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(
            result,
            UnlockResult::Rejected {
                reason: UnlockRejectReason::NotInitialized
            }
        );
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn unlock_rejects_an_empty_passphrase_without_mounting() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Locked, calls.clone(), VolOpts::default());
        let a = hook("a", calls.clone());
        let hooks: [&dyn UnlockHook; 1] = [&a];
        let result = unlock_vault(
            &workspace(),
            String::new,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(
            result,
            UnlockResult::Rejected {
                reason: UnlockRejectReason::EmptyPassphrase
            }
        );
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn unlock_rejects_a_destroyed_vault() {
        let volume = fake_volume(VaultState::Destroyed, new_calls(), VolOpts::default());
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &[],
            },
        );
        assert_eq!(
            result,
            UnlockResult::Rejected {
                reason: UnlockRejectReason::Destroyed
            }
        );
    }

    #[test]
    fn unlock_reports_a_failed_mount_and_runs_no_hooks() {
        let calls = new_calls();
        let volume = fake_volume(
            VaultState::Locked,
            calls.clone(),
            VolOpts {
                mount_ok: Some(false),
                mount_detail: Some("wrong password".to_string()),
                ..Default::default()
            },
        );
        let a = hook("a", calls.clone());
        let hooks: [&dyn UnlockHook; 1] = [&a];
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(
            result,
            UnlockResult::Failed {
                detail: "wrong password".to_string()
            }
        );
        assert_eq!(*calls.borrow(), ["mount:workspace"]);
    }

    #[test]
    fn unlock_isolates_hook_failures() {
        let calls = new_calls();
        let volume = fake_volume(VaultState::Locked, calls.clone(), VolOpts::default());
        let a = failing_hook("a", calls.clone());
        let b = hook("b", calls.clone());
        let hooks: [&dyn UnlockHook; 2] = [&a, &b];
        let result = unlock_vault(
            &workspace(),
            pass,
            &UnlockDeps {
                volume: &volume,
                hooks: &hooks,
            },
        );
        assert_eq!(*calls.borrow(), ["mount:workspace", "hook:a", "hook:b"]);
        assert_eq!(
            result,
            UnlockResult::Unlocked {
                warn_short_passphrase: false,
                hook_failures: vec!["a: a exploded".to_string()]
            }
        );
    }

    #[test]
    fn unlock_flags_a_short_passphrase() {
        let volume = fake_volume(VaultState::Locked, new_calls(), VolOpts::default());
        let result = unlock_vault(
            &workspace(),
            || "short".to_string(),
            &UnlockDeps {
                volume: &volume,
                hooks: &[],
            },
        );
        match result {
            UnlockResult::Unlocked {
                warn_short_passphrase,
                ..
            } => assert!(warn_short_passphrase),
            other => panic!("expected unlocked, got {other:?}"),
        }
    }

    // --- destroyWorkspace ---

    fn run_destroy(
        specs: &[VaultSpec],
        volume: &dyn VolumeDriver,
        holders: &dyn HolderControl,
        shredder: &dyn Shredder,
        effects: &dyn DestroyEffects,
        logs: &RefCell<Vec<String>>,
        opts: DestroyOptions,
    ) -> DestroyReport {
        let now = || NOW.to_string();
        let log = |m: &str| logs.borrow_mut().push(m.to_string());
        destroy_workspace(
            &DestroyDeps {
                specs,
                volume,
                holders,
                shredder,
                effects,
                now: &now,
                log: &log,
            },
            opts,
        )
    }

    #[test]
    fn destroy_runs_every_side_effect_in_the_one_valid_order() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert_eq!(
            *calls.borrow(),
            [
                "effects.quiesce",
                "holders.stopAll",
                "unmount:workspace:escalate",
                "unmount:second:escalate",
                "shred:/c/ws/gocryptfs.conf:3",
                "shred:/c/2nd/gocryptfs.conf:3",
                "effects.shredTree:/c/ws",
                "effects.shredTree:/c/2nd",
                "effects.scrubCredentials",
                "effects.clearHistory",
                "effects.marker:2026-01-01T00:00:00.000Z",
            ]
        );
        assert!(report.complete);
        assert!(report.keys_destroyed);
    }

    #[test]
    fn destroy_reports_the_phases_in_order() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        let ids: Vec<&str> = report.phases.iter().map(|p| p.phase).collect();
        let expected: Vec<&str> = DESTROY_PHASES.iter().map(|p| p.id).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn destroy_stops_holders_and_unmounts_before_shredding_keys_marker_last() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        let c = calls.borrow();
        let at = |needle: &str| c.iter().position(|x| x.starts_with(needle)).unwrap();
        assert!(at("holders.stopAll") < at("unmount:workspace"));
        assert!(at("unmount:second") < at("shred:"));
        assert!(at("shred:") < at("effects.shredTree"));
        assert_eq!(at("effects.marker"), c.len() - 1);
    }

    #[test]
    fn destroy_puts_what_the_unmount_had_to_kill_into_the_log() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                unmount_report: Some("killed 1 process(es)".to_string()),
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        // Once per vault: the workspace and the second one.
        assert_eq!(
            logs.borrow()
                .iter()
                .filter(|l| *l == "killed 1 process(es)")
                .count(),
            2
        );
    }

    #[test]
    fn destroy_logs_point_of_no_return_only_after_keys_are_gone() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert!(logs
            .borrow()
            .iter()
            .any(|l| l.contains("POINT OF NO RETURN")));
    }

    #[test]
    fn destroy_does_not_claim_success_when_key_erasure_fails() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let mut fail_on = HashSet::new();
        fail_on.insert("/c/ws/gocryptfs.conf".to_string());
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on,
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert!(!report.keys_destroyed);
        assert!(!report.complete);
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == "shred-keys")
                .map(|p| p.ok),
            Some(false)
        );
        assert!(!logs
            .borrow()
            .iter()
            .any(|l| l.contains("POINT OF NO RETURN")));
        assert!(logs
            .borrow()
            .iter()
            .any(|l| l.contains("KEY ERASURE FAILED")));
        // best effort: later phases still ran, down to the completion marker
        assert!(calls
            .borrow()
            .iter()
            .any(|c| c == "effects.shredTree:/c/ws"));
        assert_eq!(
            calls.borrow().last().map(String::as_str),
            Some("effects.marker:2026-01-01T00:00:00.000Z")
        );
    }

    #[test]
    fn destroy_keeps_going_when_a_phase_throws_and_reports_it() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                throw_on_unmount: true,
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == "unmount")
                .map(|p| p.ok),
            Some(false)
        );
        assert!(!report.complete);
        assert!(calls
            .borrow()
            .iter()
            .any(|c| c == "shred:/c/ws/gocryptfs.conf:3"));
        assert_eq!(
            calls.borrow().last().map(String::as_str),
            Some("effects.marker:2026-01-01T00:00:00.000Z")
        );
    }

    #[test]
    fn destroy_reports_an_unmount_that_could_not_release() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(
            VaultState::Unlocked,
            calls.clone(),
            VolOpts {
                unmount: UnmountResults {
                    escalate: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        let unmount = report.phases.iter().find(|p| p.phase == "unmount").unwrap();
        assert!(!unmount.ok);
        assert!(unmount.detail.as_deref().unwrap().contains("workspace"));
    }

    #[test]
    fn destroy_a_throwing_scrub_does_not_stop_the_completion_marker() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = FakeEffects {
            calls: calls.clone(),
            remote: RemoteRepoResult::Deleted,
            scrub_throws: true,
        };
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == "scrub-credentials")
                .map(|p| p.ok),
            Some(false)
        );
        assert!(report.keys_destroyed);
        assert_eq!(
            calls.borrow().last().map(String::as_str),
            Some("effects.marker:2026-01-01T00:00:00.000Z")
        );
    }

    #[test]
    fn destroy_skip_shred_leaves_data_phase_out_but_destroys_keys() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            DestroyOptions {
                skip_shred: true,
                remote: false,
            },
        );
        assert!(!calls
            .borrow()
            .iter()
            .any(|c| c.starts_with("effects.shredTree")));
        assert!(calls
            .borrow()
            .iter()
            .any(|c| c == "shred:/c/ws/gocryptfs.conf:3"));
        assert_eq!(
            report.phases.iter().find(|p| p.phase == "shred-data"),
            Some(&PhaseReport {
                phase: "shred-data",
                ok: true,
                detail: Some("skipped".to_string())
            })
        );
        assert!(report.complete);
    }

    #[test]
    fn destroy_deletes_the_remote_repo_only_when_asked() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = fake_effects(calls.clone());
        run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            PLAIN,
        );
        assert!(!calls.borrow().iter().any(|c| c == "effects.deleteRemote"));

        let calls2 = new_calls();
        let logs2 = RefCell::new(Vec::new());
        let volume2 = fake_volume(VaultState::Unlocked, calls2.clone(), VolOpts::default());
        let holders2 = FakeHolders {
            calls: calls2.clone(),
        };
        let shredder2 = FakeShredder {
            calls: calls2.clone(),
            fail_on: HashSet::new(),
        };
        let effects2 = fake_effects(calls2.clone());
        run_destroy(
            &specs(),
            &volume2,
            &holders2,
            &shredder2,
            &effects2,
            &logs2,
            DestroyOptions {
                skip_shred: false,
                remote: true,
            },
        );
        assert!(calls2.borrow().iter().any(|c| c == "effects.deleteRemote"));
    }

    #[test]
    fn destroy_warns_but_does_not_fail_when_no_token() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = FakeEffects {
            calls: calls.clone(),
            remote: RemoteRepoResult::SkippedNoToken,
            scrub_throws: false,
        };
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            DestroyOptions {
                skip_shred: false,
                remote: true,
            },
        );
        assert!(logs.borrow().iter().any(|l| l.contains("No github-token")));
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == "quiesce")
                .map(|p| p.ok),
            Some(true)
        );
    }

    #[test]
    fn destroy_fails_the_quiesce_phase_when_remote_deletion_errors() {
        let calls = new_calls();
        let logs = RefCell::new(Vec::new());
        let volume = fake_volume(VaultState::Unlocked, calls.clone(), VolOpts::default());
        let holders = FakeHolders {
            calls: calls.clone(),
        };
        let shredder = FakeShredder {
            calls: calls.clone(),
            fail_on: HashSet::new(),
        };
        let effects = FakeEffects {
            calls: calls.clone(),
            remote: RemoteRepoResult::Failed,
            scrub_throws: false,
        };
        let report = run_destroy(
            &specs(),
            &volume,
            &holders,
            &shredder,
            &effects,
            &logs,
            DestroyOptions {
                skip_shred: false,
                remote: true,
            },
        );
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == "quiesce")
                .map(|p| p.ok),
            Some(false)
        );
    }

    #[test]
    fn destroy_is_re_runnable() {
        let logs = RefCell::new(Vec::new());
        let calls1 = new_calls();
        let v1 = fake_volume(VaultState::Unlocked, calls1.clone(), VolOpts::default());
        let h1 = FakeHolders {
            calls: calls1.clone(),
        };
        let s1 = FakeShredder {
            calls: calls1.clone(),
            fail_on: HashSet::new(),
        };
        let e1 = fake_effects(calls1.clone());
        let first = run_destroy(&specs(), &v1, &h1, &s1, &e1, &logs, PLAIN);

        let calls2 = new_calls();
        let v2 = fake_volume(VaultState::Unlocked, calls2.clone(), VolOpts::default());
        let h2 = FakeHolders {
            calls: calls2.clone(),
        };
        let s2 = FakeShredder {
            calls: calls2.clone(),
            fail_on: HashSet::new(),
        };
        let e2 = fake_effects(calls2.clone());
        let second = run_destroy(&specs(), &v2, &h2, &s2, &e2, &logs, PLAIN);

        assert_eq!(second, first);
    }
}
