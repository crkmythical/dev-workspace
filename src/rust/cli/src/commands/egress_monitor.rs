//! `sdw egress-monitor` — a supervised daemon that watches the egress path after boot.
//!
//! The boot probe records the egress status once. Nothing noticed a node dying or Clash crashing
//! afterwards, so `doctor` was the only way to find out. Every minute this takes the same quick
//! live observation as `doctor`; when the status has changed it rewrites /var/run/egress-status
//! (what the banner, `doctor` and the vault notice read) and writes an `egress.change` audit event.
//!
//! A change counts only after two probes in a row agree, so one probe lost to a busy node does not
//! flap the log. This reports and blocks nothing: what keeps traffic off the direct path is the
//! egress firewall (on by default), not this monitor.

use sdw_core::audit::{event, outcome, AuditEvent};
use sdw_core::egress::domain::{egress_status, EgressReport, EgressStatus};
use sdw_infra::audit::record_default;
use sdw_infra::egress::{observe_egress, read_egress_status, write_egress_status};
use sdw_infra::time::{now_iso, now_ms};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_secs(60);

/// What one probe means for the recorded status: the new status to record, if any, and the
/// candidate (a different status seen once, waiting for a second probe to agree).
pub fn step(
    recorded: Option<EgressStatus>,
    candidate: Option<EgressStatus>,
    observed: EgressStatus,
) -> (Option<EgressStatus>, Option<EgressStatus>) {
    if Some(observed) == recorded {
        return (None, None); // nothing changed; forget any one-off blip
    }
    if candidate == Some(observed) {
        return (Some(observed), None); // seen twice in a row: it is real
    }
    (None, Some(observed))
}

pub fn run(_args: &[String]) -> i32 {
    let mut recorded = read_egress_status().map(|r| r.status);
    let mut candidate = None;
    loop {
        std::thread::sleep(INTERVAL);
        let observed = egress_status(observe_egress());
        let (change, next_candidate) = step(recorded, candidate, observed);
        candidate = next_candidate;
        let Some(new_status) = change else { continue };
        let report = EgressReport {
            status: new_status,
            checked_at: now_iso(),
            // The boot probe's node count is stale by now and the monitor does not ask the
            // controller; None reads as "not known".
            provider_nodes: None,
        };
        if let Err(e) = write_egress_status(&report) {
            eprintln!("egress-monitor: could not record the egress status: {e}");
        }
        record_default(
            &AuditEvent::new(now_ms(), event::EGRESS, outcome::CHANGED)
                .with("from", recorded.map_or("none", EgressStatus::as_str))
                .with("to", new_status.as_str())
                .with("source", "monitor"),
        );
        println!(
            "egress-monitor: {} -> {}",
            recorded.map_or("none", EgressStatus::as_str),
            new_status.as_str()
        );
        recorded = Some(new_status);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EgressStatus::{Confirmed, Degraded, Down};

    #[test]
    fn an_unchanged_status_changes_nothing_and_clears_a_blip() {
        assert_eq!(step(Some(Confirmed), None, Confirmed), (None, None));
        // A one-off Degraded probe, then Confirmed again: no change was ever recorded.
        let (change, cand) = step(Some(Confirmed), None, Degraded);
        assert_eq!((change, cand), (None, Some(Degraded)));
        assert_eq!(step(Some(Confirmed), cand, Confirmed), (None, None));
    }

    #[test]
    fn two_probes_in_a_row_make_a_change_real() {
        let (change, cand) = step(Some(Confirmed), None, Down);
        assert_eq!(change, None);
        assert_eq!(step(Some(Confirmed), cand, Down), (Some(Down), None));
    }

    #[test]
    fn a_different_second_status_restarts_the_count() {
        let (_, cand) = step(Some(Confirmed), None, Degraded);
        assert_eq!(step(Some(Confirmed), cand, Down), (None, Some(Down)));
    }

    #[test]
    fn the_first_observation_with_no_record_is_confirmed_by_a_second() {
        let (change, cand) = step(None, None, Confirmed);
        assert_eq!(change, None);
        assert_eq!(step(None, cand, Confirmed), (Some(Confirmed), None));
    }
}
