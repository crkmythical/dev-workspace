//! Egress use case: start the proxy, wait for it, and observe whether traffic gets out — a port
//! of packages/core/src/egress/usecases.ts.
//!
//! It never fails. A proxy that does not come up is a finding (status "down"), not an error: the
//! workspace stays usable and the caller makes the finding visible. The waits are bounded by
//! DEFAULT_EGRESS_TIMING, which is also the worst-case delay this adds to a start-up.

use crate::egress::domain::{
    describe_egress, egress_status, EgressObservation, EgressReport, EgressStatus, EgressTiming,
    Wait,
};
use crate::egress::ports::EgressPorts;

fn seconds(w: Wait) -> u64 {
    ((w.attempts as u64 * w.delay_ms) as f64 / 1000.0).round() as u64
}

/// Try up to `attempts` times, sleeping between tries (not after the last). A failing check (an
/// `Err`) is a miss.
fn wait_for<P: EgressPorts>(
    ports: &P,
    check: impl Fn(&P) -> Result<bool, String>,
    wait: Wait,
) -> bool {
    for i in 0..wait.attempts {
        if check(ports).unwrap_or(false) {
            return true;
        }
        if i < wait.attempts - 1 {
            ports.sleep(wait.delay_ms);
        }
    }
    false
}

/// Wait for the provider to load at least one node; returns the last count seen.
fn wait_for_nodes<P: EgressPorts>(ports: &P, wait: Wait) -> Option<i64> {
    let mut last: Option<i64> = None;
    for i in 0..wait.attempts {
        last = ports.provider_nodes().unwrap_or(None);
        if last.is_some_and(|n| n > 0) {
            return last;
        }
        if i < wait.attempts - 1 {
            ports.sleep(wait.delay_ms);
        }
    }
    last
}

pub fn bring_up_egress<P: EgressPorts>(ports: &P, timing: EgressTiming) -> EgressReport {
    ports.log("Starting Clash...");
    if let Err(err) = ports.start_proxy() {
        // Maybe it is already running; the wait below finds out.
        ports.log(&format!("WARNING: could not start Clash: {err}"));
    }

    let listening = wait_for(ports, |p| p.proxy_listening(), timing.proxy_wait);
    let ready_msg = if listening {
        "Clash ready.".to_string()
    } else {
        format!(
            "WARNING: Clash not ready after {}s.",
            seconds(timing.proxy_wait)
        )
    };
    ports.log(&ready_msg);

    let mut provider_nodes: Option<i64> = None;
    let mut probe_ok = false;
    if listening {
        provider_nodes = wait_for_nodes(ports, timing.provider_wait);
        if provider_nodes.is_none_or(|n| n <= 0) {
            // The start-up fetch may simply have lost a race with the network; Clash would not try
            // again for an hour, so ask once, now.
            ports.log(&format!(
                "Provider has no nodes after {}s; asking Clash to fetch the subscription again.",
                seconds(timing.provider_wait)
            ));
            if let Err(e) = ports.refresh_provider() {
                ports.log(&format!("WARNING: subscription refresh failed: {e}"));
            }
            provider_nodes = wait_for_nodes(ports, timing.refresh_wait);
        }
        let loaded_msg = match provider_nodes {
            Some(n) if n > 0 => {
                format!("Provider loaded: {n} nodes (url-test auto-selecting fastest).")
            }
            _ => "WARNING: Provider still has no nodes (Clash will retry on its own schedule)."
                .to_string(),
        };
        ports.log(&loaded_msg);

        // Give Clash a moment to establish its first connection before probing.
        ports.sleep(timing.settle_ms);
        probe_ok = wait_for(ports, |p| p.probe(), timing.probe);
    }

    let report = EgressReport {
        status: egress_status(EgressObservation {
            proxy_listening: listening,
            probe_ok,
        }),
        checked_at: ports.now(),
        provider_nodes,
    };
    let final_msg = if report.status == EgressStatus::Confirmed {
        format!("Egress {}.", describe_egress(&report))
    } else {
        format!("WARNING: egress {}", describe_egress(&report))
    };
    ports.log(&final_msg);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::domain::DEFAULT_EGRESS_TIMING;
    use std::cell::{Cell, RefCell};

    const NEVER: u32 = u32::MAX;

    struct Scenario {
        /// Attempt (1-based) at which the proxy starts listening; NEVER = never.
        listens_at_attempt: u32,
        /// Node counts returned per call; the last repeats. None = controller unreachable.
        nodes: Vec<Option<i64>>,
        /// The start-up fetch failed: no nodes until refresh_provider is called, then this many.
        nodes_after_refresh: Option<i64>,
        refresh_throws: bool,
        /// Attempt (1-based) at which the probe starts succeeding; NEVER = never.
        probe_ok_at_attempt: u32,
        start_throws: bool,
    }

    impl Default for Scenario {
        fn default() -> Self {
            Scenario {
                listens_at_attempt: 1,
                nodes: vec![Some(12)],
                nodes_after_refresh: None,
                refresh_throws: false,
                probe_ok_at_attempt: 1,
                start_throws: false,
            }
        }
    }

    /// bring_up_egress against a fake proxy and a fake clock. `sleep` only advances a counter, so
    /// the worst-case delay a start-up can spend waiting is asserted, not experienced.
    struct FakePorts {
        s: Scenario,
        calls: RefCell<Vec<String>>,
        logs: RefCell<Vec<String>>,
        elapsed_ms: Cell<u64>,
        listen_checks: Cell<u32>,
        node_checks: Cell<usize>,
        probes: Cell<u32>,
        refreshed: Cell<bool>,
    }

    impl FakePorts {
        fn new(s: Scenario) -> Self {
            FakePorts {
                s,
                calls: RefCell::new(Vec::new()),
                logs: RefCell::new(Vec::new()),
                elapsed_ms: Cell::new(0),
                listen_checks: Cell::new(0),
                node_checks: Cell::new(0),
                probes: Cell::new(0),
                refreshed: Cell::new(false),
            }
        }
        fn count(&self, name: &str) -> usize {
            self.calls.borrow().iter().filter(|c| *c == name).count()
        }
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn logs(&self) -> Vec<String> {
            self.logs.borrow().clone()
        }
    }

    impl EgressPorts for FakePorts {
        fn start_proxy(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("start".to_string());
            if self.s.start_throws {
                return Err("clash: not found".to_string());
            }
            Ok(())
        }
        fn proxy_listening(&self) -> Result<bool, String> {
            let n = self.listen_checks.get() + 1;
            self.listen_checks.set(n);
            self.calls.borrow_mut().push("listening?".to_string());
            Ok(n >= self.s.listens_at_attempt)
        }
        fn provider_nodes(&self) -> Result<Option<i64>, String> {
            self.calls.borrow_mut().push("nodes?".to_string());
            if let Some(after) = self.s.nodes_after_refresh {
                return Ok(Some(if self.refreshed.get() { after } else { 0 }));
            }
            let i = self.node_checks.get();
            self.node_checks.set(i + 1);
            Ok(self.s.nodes[i.min(self.s.nodes.len() - 1)])
        }
        fn refresh_provider(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("refresh".to_string());
            if self.s.refresh_throws {
                return Err("controller refused".to_string());
            }
            self.refreshed.set(true);
            Ok(())
        }
        fn probe(&self) -> Result<bool, String> {
            let p = self.probes.get() + 1;
            self.probes.set(p);
            self.calls.borrow_mut().push("probe".to_string());
            Ok(p >= self.s.probe_ok_at_attempt)
        }
        fn sleep(&self, ms: u64) {
            self.elapsed_ms.set(self.elapsed_ms.get() + ms);
        }
        fn now(&self) -> String {
            "2026-10-17T12:00:00.000Z".to_string()
        }
        fn log(&self, message: &str) {
            self.logs.borrow_mut().push(message.to_string());
        }
    }

    fn fake(s: Scenario) -> FakePorts {
        FakePorts::new(s)
    }

    const T: EgressTiming = DEFAULT_EGRESS_TIMING;

    fn last_log(f: &FakePorts) -> String {
        f.logs().last().cloned().unwrap_or_default()
    }

    #[test]
    fn a_healthy_proxy_is_confirmed_after_one_settle_pause() {
        let f = fake(Scenario::default());
        let report = bring_up_egress(&f, T);
        assert_eq!(
            report,
            EgressReport {
                status: EgressStatus::Confirmed,
                checked_at: "2026-10-17T12:00:00.000Z".to_string(),
                provider_nodes: Some(12),
            }
        );
        assert_eq!(f.calls(), ["start", "listening?", "nodes?", "probe"]);
        assert_eq!(f.elapsed_ms.get(), T.settle_ms);
        assert!(f.logs().iter().any(|l| l == "Clash ready."));
        assert!(last_log(&f).contains("Egress confirmed"));
    }

    #[test]
    fn a_proxy_that_never_listens_is_down_without_probing() {
        let f = fake(Scenario {
            listens_at_attempt: NEVER,
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.status, EgressStatus::Down);
        assert_eq!(report.provider_nodes, None);
        assert_eq!(f.count("listening?"), T.proxy_wait.attempts as usize);
        assert!(!f.calls().iter().any(|c| c == "nodes?"));
        assert!(!f.calls().iter().any(|c| c == "probe"));
        // The most a dead proxy can delay a start-up: 29 gaps of one second, nothing after.
        assert_eq!(
            f.elapsed_ms.get(),
            (T.proxy_wait.attempts as u64 - 1) * T.proxy_wait.delay_ms
        );
        assert!(f
            .logs()
            .iter()
            .any(|l| l.contains("WARNING: Clash not ready after 30s")));
        assert!(last_log(&f).contains("WARNING: egress DOWN"));
    }

    #[test]
    fn a_proxy_that_comes_up_late_is_waited_for() {
        let f = fake(Scenario {
            listens_at_attempt: 5,
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.status, EgressStatus::Confirmed);
        assert_eq!(f.count("listening?"), 5);
        assert_eq!(f.elapsed_ms.get(), 4 * T.proxy_wait.delay_ms + T.settle_ms);
    }

    #[test]
    fn listens_but_cannot_reach_out_is_degraded_after_the_probe_budget() {
        let f = fake(Scenario {
            probe_ok_at_attempt: NEVER,
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.status, EgressStatus::Degraded);
        assert_eq!(f.count("probe"), T.probe.attempts as usize);
        assert_eq!(
            f.elapsed_ms.get(),
            T.settle_ms + (T.probe.attempts as u64 - 1) * T.probe.delay_ms
        );
        assert!(last_log(&f).contains("WARNING: egress DEGRADED"));
    }

    #[test]
    fn a_flaky_first_probe_is_retried() {
        let f = fake(Scenario {
            probe_ok_at_attempt: 2,
            ..Default::default()
        });
        assert_eq!(bring_up_egress(&f, T).status, EgressStatus::Confirmed);
        assert_eq!(f.count("probe"), 2);
    }

    #[test]
    fn an_empty_subscription_is_recorded_and_still_probed() {
        let f = fake(Scenario {
            nodes: vec![Some(0)],
            probe_ok_at_attempt: NEVER,
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.status, EgressStatus::Degraded);
        assert_eq!(report.provider_nodes, Some(0));
        assert_eq!(
            f.count("nodes?"),
            (T.provider_wait.attempts + T.refresh_wait.attempts) as usize
        );
        assert!(describe_egress(&report).contains("CLASH_SUBSCRIPTION_URL"));
        assert!(f
            .logs()
            .iter()
            .any(|l| l.contains("Provider still has no nodes")));
    }

    #[test]
    fn nodes_that_load_after_a_few_polls_stop_the_wait_early() {
        let f = fake(Scenario {
            nodes: vec![None, Some(0), Some(0), Some(9)],
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.provider_nodes, Some(9));
        assert_eq!(f.count("nodes?"), 4);
        assert!(f
            .logs()
            .iter()
            .any(|l| l == "Provider loaded: 9 nodes (url-test auto-selecting fastest)."));
    }

    #[test]
    fn a_lost_race_is_recovered_by_one_refresh() {
        // Clash fetches the subscription once at start and would not try again for an hour.
        let f = fake(Scenario {
            nodes_after_refresh: Some(5),
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.status, EgressStatus::Confirmed);
        assert_eq!(report.provider_nodes, Some(5));
        assert_eq!(f.count("refresh"), 1);
        // the refresh happens only after the full first wait, never instead of it
        let calls = f.calls();
        let refresh_at = calls.iter().position(|c| c == "refresh").unwrap();
        let before_refresh = &calls[..refresh_at];
        assert_eq!(
            before_refresh.iter().filter(|c| *c == "nodes?").count(),
            T.provider_wait.attempts as usize
        );
        assert!(f
            .logs()
            .iter()
            .any(|l| l.contains("asking Clash to fetch the subscription again")));
        assert!(f
            .logs()
            .iter()
            .any(|l| l == "Provider loaded: 5 nodes (url-test auto-selecting fastest)."));
    }

    #[test]
    fn a_healthy_subscription_is_never_refreshed() {
        let f = fake(Scenario::default());
        bring_up_egress(&f, T);
        assert_eq!(f.count("refresh"), 0);
    }

    #[test]
    fn a_refused_refresh_changes_nothing() {
        let f = fake(Scenario {
            nodes: vec![Some(0)],
            refresh_throws: true,
            probe_ok_at_attempt: NEVER,
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(f.count("refresh"), 1);
        assert_eq!(report.status, EgressStatus::Degraded);
        assert_eq!(report.provider_nodes, Some(0));
    }

    #[test]
    fn the_refresh_adds_at_most_its_own_short_wait() {
        let f = fake(Scenario {
            nodes: vec![Some(0)],
            probe_ok_at_attempt: NEVER,
            ..Default::default()
        });
        bring_up_egress(&f, T);
        let first_wait = (T.provider_wait.attempts as u64 - 1) * T.provider_wait.delay_ms;
        let second_wait = (T.refresh_wait.attempts as u64 - 1) * T.refresh_wait.delay_ms;
        let probe_budget = T.settle_ms + (T.probe.attempts as u64 - 1) * T.probe.delay_ms;
        assert_eq!(f.elapsed_ms.get(), first_wait + second_wait + probe_budget);
    }

    #[test]
    fn an_unreachable_controller_leaves_the_node_count_unknown() {
        let f = fake(Scenario {
            nodes: vec![None],
            ..Default::default()
        });
        let report = bring_up_egress(&f, T);
        assert_eq!(report.provider_nodes, None);
        assert_eq!(report.status, EgressStatus::Confirmed);
    }

    #[test]
    fn failing_to_launch_is_logged_and_the_wait_still_decides() {
        let dead = fake(Scenario {
            start_throws: true,
            listens_at_attempt: NEVER,
            ..Default::default()
        });
        assert_eq!(bring_up_egress(&dead, T).status, EgressStatus::Down);
        assert!(dead
            .logs()
            .iter()
            .any(|l| l.contains("could not start Clash: clash: not found")));

        // ... for instance when a proxy from an earlier run is already listening.
        let already_up = fake(Scenario {
            start_throws: true,
            ..Default::default()
        });
        assert_eq!(
            bring_up_egress(&already_up, T).status,
            EgressStatus::Confirmed
        );
    }

    struct AlwaysThrows;
    impl EgressPorts for AlwaysThrows {
        fn start_proxy(&self) -> Result<(), String> {
            Err("boom".to_string())
        }
        fn proxy_listening(&self) -> Result<bool, String> {
            Err("boom".to_string())
        }
        fn provider_nodes(&self) -> Result<Option<i64>, String> {
            Err("boom".to_string())
        }
        fn refresh_provider(&self) -> Result<(), String> {
            Err("boom".to_string())
        }
        fn probe(&self) -> Result<bool, String> {
            Err("boom".to_string())
        }
        fn sleep(&self, _ms: u64) {}
        fn now(&self) -> String {
            "t".to_string()
        }
        fn log(&self, _message: &str) {}
    }

    #[test]
    fn never_throws_even_when_every_port_misbehaves() {
        assert_eq!(bring_up_egress(&AlwaysThrows, T).status, EgressStatus::Down);
    }

    #[test]
    fn honors_a_custom_timing() {
        let f = fake(Scenario {
            listens_at_attempt: NEVER,
            ..Default::default()
        });
        let custom = EgressTiming {
            proxy_wait: Wait {
                attempts: 3,
                delay_ms: 10,
            },
            ..T
        };
        bring_up_egress(&f, custom);
        assert_eq!(f.count("listening?"), 3);
        assert_eq!(f.elapsed_ms.get(), 20);
    }
}
