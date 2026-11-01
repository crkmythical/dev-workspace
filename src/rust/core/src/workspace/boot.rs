//! The container's start-up as DATA — a port of packages/core/src/workspace/boot.ts. Which steps
//! run, in which order, what a failure means (abort vs degrade), and which need the vault mounted.
//! The effects live in the cli composition root; this is the pure orchestration. Tests mirror
//! packages/core/tests/boot.test.ts.

use crate::egress::domain::EgressReport;

#[derive(Debug, Default)]
pub struct BootState {
    /// The workspace vault is mounted. Writing under the mount point while this is false would
    /// land in the container's own, UNencrypted filesystem.
    pub vault_mounted: bool,
    /// What bring-up-egress observed.
    pub egress: Option<EgressReport>,
    /// Ids of steps that failed under the "degrade" policy.
    pub degraded: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFail {
    Abort,
    Degrade,
}

pub struct BootStepSpec {
    pub id: &'static str,
    pub on_fail: OnFail,
    /// The step is skipped (not failed) while this returns false.
    pub when: Option<fn(&BootState) -> bool>,
}

fn vault_is_mounted(s: &BootState) -> bool {
    s.vault_mounted
}

pub const BOOT_PLAN: &[BootStepSpec] = &[
    BootStepSpec {
        id: "preflight",
        on_fail: OnFail::Abort,
        when: None,
    },
    BootStepSpec {
        id: "cleanup-stale-mounts",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "render-clash-config",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "git-globals",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "tune-inotify",
        on_fail: OnFail::Degrade,
        when: None,
    },
    // Fail-closed egress firewall. BEFORE bring-up-egress so Clash starts under the filter, and
    // Abort: if enforcement was asked for but cannot be applied, do not boot into a leaky state.
    // (A no-op returning Ok when EGRESS_ENFORCE is off, so Abort never triggers for the default.)
    BootStepSpec {
        id: "enforce-egress",
        on_fail: OnFail::Abort,
        when: None,
    },
    BootStepSpec {
        id: "bring-up-egress",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "resolve-auth",
        on_fail: OnFail::Abort,
        when: None,
    },
    BootStepSpec {
        id: "init-vault",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "prepare-replication",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "unlock-vault",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "notify-egress",
        on_fail: OnFail::Degrade,
        when: Some(vault_is_mounted),
    },
    BootStepSpec {
        id: "prepare-desktops",
        on_fail: OnFail::Degrade,
        when: Some(vault_is_mounted),
    },
    BootStepSpec {
        id: "prepare-backup",
        on_fail: OnFail::Degrade,
        when: Some(vault_is_mounted),
    },
    BootStepSpec {
        id: "banner",
        on_fail: OnFail::Degrade,
        when: None,
    },
    BootStepSpec {
        id: "supervisord",
        on_fail: OnFail::Abort,
        when: None,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    Ok,
    Skipped,
    Degraded,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    pub id: String,
    pub outcome: StepOutcome,
    /// The error message, for "degraded" and "aborted".
    pub detail: Option<String>,
}

#[derive(Debug)]
pub struct BootReport {
    pub steps: Vec<StepReport>,
    /// An abort-policy step failed; the later steps did not run.
    pub aborted: bool,
}

/// Run the plan. `run_step(id, state)` performs a step's effects and returns Err(message) on
/// failure; the cli adapter dispatches by id. A `when` that returns false skips the step.
pub fn run_boot(
    plan: &[BootStepSpec],
    mut run_step: impl FnMut(&str, &mut BootState) -> Result<(), String>,
    state: &mut BootState,
    mut log: impl FnMut(&str),
) -> BootReport {
    let mut steps = Vec::new();
    for step in plan {
        if let Some(when) = step.when {
            if !when(state) {
                steps.push(StepReport {
                    id: step.id.to_string(),
                    outcome: StepOutcome::Skipped,
                    detail: None,
                });
                continue;
            }
        }
        match run_step(step.id, state) {
            Ok(()) => steps.push(StepReport {
                id: step.id.to_string(),
                outcome: StepOutcome::Ok,
                detail: None,
            }),
            Err(detail) => {
                if step.on_fail == OnFail::Abort {
                    steps.push(StepReport {
                        id: step.id.to_string(),
                        outcome: StepOutcome::Aborted,
                        detail: Some(detail.clone()),
                    });
                    log(&format!("ERROR: boot aborted at \"{}\": {detail}", step.id));
                    return BootReport {
                        steps,
                        aborted: true,
                    };
                }
                state.degraded.push(step.id.to_string());
                steps.push(StepReport {
                    id: step.id.to_string(),
                    outcome: StepOutcome::Degraded,
                    detail: Some(detail.clone()),
                });
                log(&format!(
                    "WARNING: boot step \"{}\" failed, continuing without it: {detail}",
                    step.id
                ));
            }
        }
    }
    BootReport {
        steps,
        aborted: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::domain::EgressStatus;
    use std::cell::RefCell;

    fn ids() -> Vec<&'static str> {
        BOOT_PLAN.iter().map(|s| s.id).collect()
    }
    fn index_of(id: &str) -> usize {
        BOOT_PLAN
            .iter()
            .position(|s| s.id == id)
            .expect("id in plan")
    }

    /// Run the real plan with a recorder: every step records its id, named ids fail, and
    /// unlock-vault optionally mounts the vault.
    fn run_recorded(
        failing: &[(&str, &str)],
        mount: bool,
    ) -> (Vec<String>, Vec<String>, BootReport, BootState) {
        let ran = RefCell::new(Vec::<String>::new());
        let logs = RefCell::new(Vec::<String>::new());
        let mut state = BootState::default();
        let report = run_boot(
            BOOT_PLAN,
            |id, st| {
                ran.borrow_mut().push(id.to_string());
                if let Some((_, msg)) = failing.iter().find(|(fid, _)| *fid == id) {
                    return Err((*msg).to_string());
                }
                if id == "unlock-vault" && mount {
                    st.vault_mounted = true;
                }
                Ok(())
            },
            &mut state,
            |m| logs.borrow_mut().push(m.to_string()),
        );
        (ran.into_inner(), logs.into_inner(), report, state)
    }

    #[test]
    fn shape() {
        let ids = ids();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(ids.first(), Some(&"preflight"));
        assert_eq!(ids.last(), Some(&"supervisord"));

        let abort: Vec<_> = BOOT_PLAN
            .iter()
            .filter(|s| s.on_fail == OnFail::Abort)
            .map(|s| s.id)
            .collect();
        assert_eq!(
            abort,
            ["preflight", "enforce-egress", "resolve-auth", "supervisord"]
        );

        let egress = BOOT_PLAN
            .iter()
            .find(|s| s.id == "bring-up-egress")
            .unwrap();
        assert_eq!(egress.on_fail, OnFail::Degrade);
    }

    #[test]
    fn ordering_rules() {
        let must_precede = [
            ("cleanup-stale-mounts", "unlock-vault"),
            ("render-clash-config", "bring-up-egress"),
            ("enforce-egress", "bring-up-egress"),
            ("resolve-auth", "init-vault"),
            ("resolve-auth", "unlock-vault"),
            ("init-vault", "unlock-vault"),
            ("init-vault", "prepare-replication"),
            ("prepare-replication", "banner"),
            ("bring-up-egress", "notify-egress"),
            ("unlock-vault", "notify-egress"),
            ("unlock-vault", "prepare-desktops"),
            ("unlock-vault", "prepare-backup"),
            ("prepare-desktops", "banner"),
            ("prepare-backup", "banner"),
            ("notify-egress", "banner"),
            ("banner", "supervisord"),
        ];
        for (first, then) in must_precede {
            assert!(
                index_of(first) < index_of(then),
                "{first} must precede {then}"
            );
        }
    }

    #[test]
    fn mount_point_steps_gated_on_the_vault() {
        for id in ["notify-egress", "prepare-desktops", "prepare-backup"] {
            let step = BOOT_PLAN.iter().find(|s| s.id == id).unwrap();
            let when = step.when.expect("has a when guard");
            assert!(!when(&BootState {
                vault_mounted: false,
                ..Default::default()
            }));
            assert!(when(&BootState {
                vault_mounted: true,
                ..Default::default()
            }));
        }
    }

    #[test]
    fn runs_in_order_when_all_ok() {
        let (ran, _logs, report, _state) = run_recorded(&[], true);
        assert_eq!(ran, ids());
        assert!(report.steps.iter().all(|s| s.outcome == StepOutcome::Ok));
        assert!(!report.aborted);
    }

    #[test]
    fn degrade_step_recorded_and_boot_continues() {
        let (ran, logs, report, state) = run_recorded(
            &[
                ("bring-up-egress", "clash exploded"),
                ("prepare-backup", "x"),
            ],
            true,
        );
        assert!(!report.aborted);
        assert_eq!(ran.last().map(String::as_str), Some("supervisord"));
        assert_eq!(state.degraded, ["bring-up-egress", "prepare-backup"]);
        let egress = report
            .steps
            .iter()
            .find(|s| s.id == "bring-up-egress")
            .unwrap();
        assert_eq!(egress.outcome, StepOutcome::Degraded);
        assert_eq!(egress.detail.as_deref(), Some("clash exploded"));
        assert!(logs
            .iter()
            .any(|l| l.contains("\"bring-up-egress\"") && l.contains("clash exploded")));
    }

    #[test]
    fn abort_step_stops_the_boot() {
        let (ran, logs, report, _state) = run_recorded(&[("resolve-auth", "no password")], false);
        assert!(report.aborted);
        assert_eq!(ran.last().map(String::as_str), Some("resolve-auth"));
        assert!(!ran.iter().any(|r| r == "init-vault"));
        assert!(!ran.iter().any(|r| r == "supervisord"));
        let last = report.steps.last().unwrap();
        assert_eq!(last.id, "resolve-auth");
        assert_eq!(last.outcome, StepOutcome::Aborted);
        assert_eq!(last.detail.as_deref(), Some("no password"));
        assert!(logs
            .last()
            .unwrap()
            .contains("boot aborted at \"resolve-auth\": no password"));
    }

    #[test]
    fn skipped_steps_recorded_as_skipped() {
        let (_ran, _logs, report, _state) = run_recorded(&[], false);
        let skipped: Vec<_> = report
            .steps
            .iter()
            .filter(|s| s.outcome == StepOutcome::Skipped)
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            skipped,
            ["notify-egress", "prepare-desktops", "prepare-backup"]
        );
    }

    #[test]
    fn later_step_sees_earlier_state() {
        let mut state = BootState::default();
        let seen = RefCell::new(None);
        run_boot(
            BOOT_PLAN,
            |id, st| {
                match id {
                    "bring-up-egress" => {
                        st.egress = Some(EgressReport {
                            status: EgressStatus::Down,
                            checked_at: "t".to_string(),
                            provider_nodes: None,
                        })
                    }
                    "banner" => *seen.borrow_mut() = st.egress.clone(),
                    _ => {}
                }
                Ok(())
            },
            &mut state,
            |_| {},
        );
        assert_eq!(
            seen.into_inner().map(|e| e.status),
            Some(EgressStatus::Down)
        );
    }
}
