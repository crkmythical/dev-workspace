//! Recording from the handlers: which events are always kept, which are capped so a flood cannot
//! push the rest out of the log, and the counters behind `/sync/api/metrics`.
//!
//! The event model, the file adapter and the summary live in `sdw_core::audit` and
//! `sdw_infra::audit`; this is the wiring the HTTP surface needs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use sdw_core::audit::{event, outcome, AuditEvent, AuditSink, NoisyGate};

/// Attacker-triggerable events (wrong passwords, refused self-destruct calls): at most this many
/// are written per window; the rest are counted and reported once as `audit.suppressed`.
const NOISY_WINDOW_MS: i64 = 60_000;
const NOISY_MAX: u32 = 30;
/// The gate checks a Cloudflare Access assertion on every request, so a sign-in is recorded once
/// per identity per this long.
const ACCESS_DEDUP_MS: i64 = 15 * 60 * 1000;

pub struct Auditor {
    sink: Arc<dyn AuditSink>,
    noisy: Mutex<NoisyGate>,
    access_seen: Mutex<HashMap<String, i64>>,
}

impl Auditor {
    pub fn new(sink: Arc<dyn AuditSink>) -> Self {
        Self {
            sink,
            noisy: Mutex::new(NoisyGate::new(NOISY_WINDOW_MS, NOISY_MAX)),
            access_seen: Mutex::new(HashMap::new()),
        }
    }

    /// Always written: a successful login, a logout, a self-destruct that fired.
    pub fn record(&self, e: AuditEvent) {
        self.sink.record(&e);
    }

    /// Written unless a flood has used up the window's budget.
    pub fn record_noisy(&self, e: AuditEvent) {
        let (admit, flushed) = match self.noisy.lock() {
            Ok(mut gate) => gate.admit(e.ts_ms, e.event, e.outcome),
            Err(_) => (true, Vec::new()),
        };
        for summary in &flushed {
            self.sink.record(summary);
        }
        if admit {
            self.sink.record(&e);
        }
    }

    /// A Cloudflare Access sign-in, once per identity per [`ACCESS_DEDUP_MS`].
    pub fn record_access(&self, now_ms: i64, who: &str, ip: &str) {
        {
            let Ok(mut seen) = self.access_seen.lock() else {
                return;
            };
            if seen.get(who).is_some_and(|t| now_ms - *t < ACCESS_DEDUP_MS) {
                return;
            }
            if seen.len() >= 256 {
                seen.retain(|_, t| now_ms - *t < ACCESS_DEDUP_MS);
            }
            // Bounded: a flood of distinct (but signature-valid) identities cannot grow it forever.
            if seen.len() < 1024 {
                seen.insert(who.to_string(), now_ms);
            }
        }
        self.sink.record(
            &AuditEvent::new(now_ms, event::ACCESS, outcome::SUCCESS)
                .with("who", who)
                .with("ip", ip),
        );
    }
}

/// Monotonic counters, rendered in the Prometheus text format.
#[derive(Default)]
pub struct Metrics {
    pub login_success: AtomicU64,
    pub login_failure: AtomicU64,
    pub login_throttled: AtomicU64,
    pub gate_allow_cookie: AtomicU64,
    pub gate_allow_access: AtomicU64,
    pub gate_deny: AtomicU64,
    pub gate_deny_origin: AtomicU64,
    pub destruct_rejected: AtomicU64,
    pub destruct_rate_limited: AtomicU64,
    pub destruct_triggered: AtomicU64,
    pub destruct_error: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut out = String::new();
        let mut family = |name: &str, help: &str, label: &str, rows: &[(&str, u64)]| {
            out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n"));
            for (value, n) in rows {
                out.push_str(&format!("{name}{{{label}=\"{value}\"}} {n}\n"));
            }
        };
        family(
            "sdw_login_total",
            "Logins at the ingress gate, by outcome.",
            "outcome",
            &[
                ("success", get(&self.login_success)),
                ("failure", get(&self.login_failure)),
                ("throttled", get(&self.login_throttled)),
            ],
        );
        family(
            "sdw_gate_requests_total",
            "Requests checked by the gate (/auth/verify), by result.",
            "result",
            &[
                ("allow_cookie", get(&self.gate_allow_cookie)),
                ("allow_access", get(&self.gate_allow_access)),
                ("deny", get(&self.gate_deny)),
                ("deny_origin", get(&self.gate_deny_origin)),
            ],
        );
        family(
            "sdw_destruct_total",
            "Remote self-destruct requests that reached the handler, by outcome.",
            "outcome",
            &[
                ("rejected", get(&self.destruct_rejected)),
                ("rate_limited", get(&self.destruct_rate_limited)),
                ("triggered", get(&self.destruct_triggered)),
                ("error", get(&self.destruct_error)),
            ],
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdw_core::audit::{parse_line, MemorySink};

    #[test]
    fn access_sign_ins_are_recorded_once_per_identity_per_window() {
        let sink = Arc::new(MemorySink::new());
        let a = Auditor::new(sink.clone());
        a.record_access(0, "me@example.com", "203.0.113.1");
        a.record_access(1_000, "me@example.com", "203.0.113.1");
        a.record_access(2_000, "svc-1", "203.0.113.2");
        a.record_access(ACCESS_DEDUP_MS + 1, "me@example.com", "203.0.113.1");
        let lines = sink.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        let first = parse_line(&lines[0]).unwrap();
        assert_eq!(first.event, "auth.access");
        assert_eq!(first.field("who"), Some("me@example.com"));
    }

    #[test]
    fn a_flood_of_noisy_events_is_capped_and_summarised() {
        let sink = Arc::new(MemorySink::new());
        let a = Auditor::new(sink.clone());
        for i in 0..100 {
            a.record_noisy(AuditEvent::new(i, event::LOGIN, outcome::FAILURE).with("ip", "x"));
        }
        assert_eq!(sink.lines().len(), NOISY_MAX as usize);
        // A real, critical event is never dropped, and the next window reports what was lost.
        a.record(AuditEvent::new(200, event::LOGIN, outcome::SUCCESS));
        a.record_noisy(AuditEvent::new(
            NOISY_WINDOW_MS + 10,
            event::LOGIN,
            outcome::FAILURE,
        ));
        let lines = sink.lines();
        assert!(lines.iter().any(|l| l.contains("\"outcome\":\"success\"")));
        let summary = lines
            .iter()
            .filter_map(|l| parse_line(l))
            .find(|e| e.event == event::SUPPRESSED)
            .expect("a suppression summary");
        assert_eq!(summary.field("count"), Some("70"));
    }

    #[test]
    fn metrics_render_as_prometheus_text() {
        let m = Metrics::default();
        Metrics::inc(&m.login_failure);
        Metrics::inc(&m.login_failure);
        Metrics::inc(&m.gate_deny);
        let text = m.render();
        assert!(text.contains("# TYPE sdw_login_total counter"));
        assert!(text.contains("sdw_login_total{outcome=\"failure\"} 2"));
        assert!(text.contains("sdw_gate_requests_total{result=\"deny\"} 1"));
        assert!(text.contains("sdw_destruct_total{outcome=\"triggered\"} 0"));
    }
}
