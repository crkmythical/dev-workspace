//! The audit trail: one JSON object per line saying who did what, when, and with what result.
//! Logins, logouts, Cloudflare Access sign-ins, self-destruct attempts and egress status changes
//! are recorded; `doctor` and `audit-log` read them back.
//!
//! Pure: this module builds and parses the lines, defines the [`AuditSink`] port and the cap that
//! keeps an attacker from flooding the log. The adapter that appends to a file lives in sdw-infra;
//! the sync-service and the entrypoint decide when to record.
//!
//! "Who" is a claim, not a proof: the client address comes from proxy headers (CF-Connecting-IP,
//! X-Forwarded-For) a caller can influence when it reaches the origin directly. The login has one
//! shared master password, so there is no user name; behind Cloudflare Access the Access identity
//! (e-mail or subject) is recorded as well.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Map, Value};

/// Where the audit log lives. docker-compose.yml mounts a named volume here, so the trail
/// survives a container re-creation (not `docker compose down -v`).
pub const AUDIT_LOG_PATH: &str = "/var/log/sdw/audit.jsonl";

/// Longest value kept for any field. Header-supplied strings are attacker-controlled.
const MAX_FIELD_CHARS: usize = 200;

/// Event names (the `event` field): a closed set, so summaries and dashboards can group on them.
pub mod event {
    pub const LOGIN: &str = "auth.login";
    pub const LOGOUT: &str = "auth.logout";
    pub const ACCESS: &str = "auth.access";
    pub const DESTRUCT: &str = "destruct.attempt";
    pub const EGRESS: &str = "egress.change";
    /// A WebSocket handshake from a page on another origin, refused by the gate.
    pub const GATE_ORIGIN: &str = "gate.origin";
    /// Stands in for events a flood made us drop; carries `of`, `of_outcome` and `count`.
    pub const SUPPRESSED: &str = "audit.suppressed";
}

/// Outcomes (the `outcome` field).
pub mod outcome {
    pub const SUCCESS: &str = "success";
    pub const FAILURE: &str = "failure";
    pub const THROTTLED: &str = "throttled";
    pub const REJECTED: &str = "rejected";
    pub const TRIGGERED: &str = "triggered";
    pub const ERROR: &str = "error";
    pub const CHANGED: &str = "changed";
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub ts_ms: i64,
    pub event: &'static str,
    pub outcome: &'static str,
    fields: Vec<(&'static str, String)>,
}

impl AuditEvent {
    pub fn new(ts_ms: i64, event: &'static str, outcome: &'static str) -> Self {
        Self {
            ts_ms,
            event,
            outcome,
            fields: Vec::new(),
        }
    }

    /// Add a field. The value is made safe for a log line: control characters (an escape sequence
    /// in a User-Agent would otherwise reach the terminal that reads the log) become `?`, and it is
    /// cut to a fixed length.
    pub fn with(mut self, key: &'static str, value: impl AsRef<str>) -> Self {
        let clean: String = value
            .as_ref()
            .chars()
            .map(|c| if c.is_control() { '?' } else { c })
            .take(MAX_FIELD_CHARS)
            .collect();
        self.fields.push((key, clean));
        self
    }

    /// [`with`](Self::with) when there is a value, otherwise unchanged.
    pub fn with_opt(self, key: &'static str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.with(key, v),
            None => self,
        }
    }

    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The JSON line (without the trailing newline).
    pub fn to_json_line(&self) -> String {
        let mut m = Map::new();
        m.insert("ts".into(), Value::String(rfc3339_ms(self.ts_ms)));
        m.insert("ts_ms".into(), Value::from(self.ts_ms));
        m.insert("event".into(), Value::from(self.event));
        m.insert("outcome".into(), Value::from(self.outcome));
        for (k, v) in &self.fields {
            m.insert((*k).to_string(), Value::String(v.clone()));
        }
        Value::Object(m).to_string()
    }
}

/// Where audit events go. Recording must never fail the operation being audited, so there is no
/// error to return: an adapter reports its own trouble (stderr) and carries on.
pub trait AuditSink: Send + Sync {
    fn record(&self, event: &AuditEvent);
}

/// Drops everything (tests, or an audit log that cannot be opened).
pub struct NoopSink;

impl AuditSink for NoopSink {
    fn record(&self, _event: &AuditEvent) {}
}

/// Keeps the JSON lines in memory (tests and dry runs).
#[derive(Default)]
pub struct MemorySink {
    lines: Mutex<Vec<String>>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl AuditSink for MemorySink {
    fn record(&self, event: &AuditEvent) {
        if let Ok(mut l) = self.lines.lock() {
            l.push(event.to_json_line());
        }
    }
}

/// Keeps attacker-triggerable events (wrong passwords, refused self-destruct calls) from flooding
/// the log and pushing the rest out through rotation. At most `max` are admitted per window; the
/// rest are counted per (event, outcome) and reported as one `audit.suppressed` line when the next
/// window opens.
pub struct NoisyGate {
    window_ms: i64,
    max: u32,
    window_start: i64,
    seen: u32,
    dropped: HashMap<(&'static str, &'static str), u64>,
}

impl NoisyGate {
    pub fn new(window_ms: i64, max: u32) -> Self {
        Self {
            window_ms,
            max,
            window_start: i64::MIN,
            seen: 0,
            dropped: HashMap::new(),
        }
    }

    /// Decide for one event at `now_ms`: whether to record it, plus the suppression summaries of
    /// the window that just ended (record those first).
    pub fn admit(
        &mut self,
        now_ms: i64,
        event: &'static str,
        outcome: &'static str,
    ) -> (bool, Vec<AuditEvent>) {
        let mut flushed = Vec::new();
        if self.window_start == i64::MIN || now_ms - self.window_start >= self.window_ms {
            let mut keys: Vec<_> = self.dropped.drain().collect();
            keys.sort();
            for ((ev, out), count) in keys {
                flushed.push(
                    AuditEvent::new(now_ms, crate::audit::event::SUPPRESSED, outcome::ERROR)
                        .with("of", ev)
                        .with("of_outcome", out)
                        .with("count", count.to_string()),
                );
            }
            self.window_start = now_ms;
            self.seen = 0;
        }
        if self.seen < self.max {
            self.seen += 1;
            (true, flushed)
        } else {
            *self.dropped.entry((event, outcome)).or_insert(0) += 1;
            (false, flushed)
        }
    }
}

/// `2026-11-03T10:15:12.345Z` for a Unix time in milliseconds.
pub fn rfc3339_ms(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1000);
    let ms = ts_ms.rem_euclid(1000);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar
/// (H. Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// An audit line read back from the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEvent {
    pub ts_ms: i64,
    pub event: String,
    pub outcome: String,
    /// Every other field, in key order.
    pub fields: Vec<(String, String)>,
}

impl ParsedEvent {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// `2026-11-03 10:15:12Z  auth.login  failure  ip=203.0.113.9 ua=curl/8.7`
    pub fn human(&self) -> String {
        let t = rfc3339_ms(self.ts_ms);
        let rest: Vec<String> = self
            .fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!(
            "{} {}Z  {:<17} {:<9} {}",
            &t[..10],
            &t[11..19],
            self.event,
            self.outcome,
            rest.join(" ")
        )
        .trim_end()
        .to_string()
    }
}

/// Parse one log line; None for anything that is not a well-formed audit event.
pub fn parse_line(line: &str) -> Option<ParsedEvent> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let obj = v.as_object()?;
    let ts_ms = obj.get("ts_ms")?.as_i64()?;
    let event = obj.get("event")?.as_str()?.to_string();
    let outcome = obj.get("outcome")?.as_str()?.to_string();
    let fields = obj
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "ts" | "ts_ms" | "event" | "outcome"))
        .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
        .collect();
    Some(ParsedEvent {
        ts_ms,
        event,
        outcome,
        fields,
    })
}

/// What happened in a window, for `doctor`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuditSummary {
    pub logins_ok: u64,
    pub logins_failed: u64,
    pub logins_throttled: u64,
    pub access_ok: u64,
    pub destruct_attempts: u64,
    pub destruct_triggered: u64,
    pub egress_changes: u64,
    /// When, and from which claimed address, the latest wrong password came.
    pub last_failed_login: Option<(i64, Option<String>)>,
    /// The latest egress change as "from -> to".
    pub last_egress: Option<(i64, String)>,
}

/// Count what the lines in `[now_ms - window_ms, now_ms]` say. Lines that do not parse are skipped.
pub fn summarize<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    now_ms: i64,
    window_ms: i64,
) -> AuditSummary {
    let mut s = AuditSummary::default();
    let since = now_ms - window_ms;
    for e in lines.into_iter().filter_map(parse_line) {
        if e.ts_ms < since {
            continue;
        }
        // A suppression line stands for `count` events of another kind.
        let (ev, out, n) = if e.event == event::SUPPRESSED {
            let n = e
                .field("count")
                .and_then(|c| c.parse::<u64>().ok())
                .unwrap_or(0);
            (
                e.field("of").unwrap_or_default().to_string(),
                e.field("of_outcome").unwrap_or_default().to_string(),
                n,
            )
        } else {
            (e.event.clone(), e.outcome.clone(), 1)
        };
        match (ev.as_str(), out.as_str()) {
            (event::LOGIN, outcome::SUCCESS) => s.logins_ok += n,
            (event::LOGIN, outcome::FAILURE) => {
                s.logins_failed += n;
                if e.event != event::SUPPRESSED
                    && s.last_failed_login
                        .as_ref()
                        .is_none_or(|(t, _)| e.ts_ms >= *t)
                {
                    s.last_failed_login = Some((e.ts_ms, e.field("ip").map(str::to_string)));
                }
            }
            (event::LOGIN, outcome::THROTTLED) => s.logins_throttled += n,
            (event::ACCESS, outcome::SUCCESS) => s.access_ok += n,
            (event::DESTRUCT, outcome::TRIGGERED) => {
                s.destruct_attempts += n;
                s.destruct_triggered += n;
            }
            (event::DESTRUCT, _) => s.destruct_attempts += n,
            (event::EGRESS, _) => {
                s.egress_changes += n;
                if s.last_egress.as_ref().is_none_or(|(t, _)| e.ts_ms >= *t) {
                    let change = format!(
                        "{} -> {}",
                        e.field("from").unwrap_or("?"),
                        e.field("to").unwrap_or("?")
                    );
                    s.last_egress = Some((e.ts_ms, change));
                }
            }
            _ => {}
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_known_instants() {
        assert_eq!(rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339_ms(1_709_210_096_789), "2024-02-29T12:34:56.789Z");
        assert_eq!(rfc3339_ms(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(rfc3339_ms(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(rfc3339_ms(1_793_700_000_000), "2026-11-03T10:00:00.000Z");
    }

    #[test]
    fn a_line_round_trips_and_is_one_line() {
        let e = AuditEvent::new(1_793_700_000_123, event::LOGIN, outcome::FAILURE)
            .with("ip", "203.0.113.9")
            .with("ua", "curl/8");
        let line = e.to_json_line();
        assert!(!line.contains('\n'));
        let p = parse_line(&line).expect("parses");
        assert_eq!(p.ts_ms, 1_793_700_000_123);
        assert_eq!(p.event, "auth.login");
        assert_eq!(p.outcome, "failure");
        assert_eq!(p.field("ip"), Some("203.0.113.9"));
        assert_eq!(
            p.human(),
            "2026-11-03 10:00:00Z  auth.login        failure   ip=203.0.113.9 ua=curl/8"
        );
    }

    #[test]
    fn control_characters_and_length_are_neutralised() {
        let nasty = format!("evil\x1b[31m\nred{}", "x".repeat(500));
        let e = AuditEvent::new(0, event::LOGIN, outcome::FAILURE).with("ua", nasty);
        let ua = e.field("ua").unwrap();
        assert!(!ua.chars().any(char::is_control));
        assert_eq!(ua.chars().count(), 200);
        assert!(!e.to_json_line().contains('\n'));
    }

    #[test]
    fn garbage_lines_are_skipped_not_fatal() {
        assert!(parse_line("").is_none());
        assert!(parse_line("not json").is_none());
        assert!(parse_line("{\"event\":\"x\"}").is_none());
        assert!(parse_line("[1,2]").is_none());
    }

    #[test]
    fn noisy_gate_admits_a_budget_then_summarises_the_rest() {
        let mut g = NoisyGate::new(60_000, 2);
        assert_eq!(g.admit(0, event::LOGIN, outcome::FAILURE), (true, vec![]));
        assert_eq!(g.admit(1, event::LOGIN, outcome::FAILURE), (true, vec![]));
        assert_eq!(g.admit(2, event::LOGIN, outcome::FAILURE), (false, vec![]));
        assert_eq!(
            g.admit(3, event::DESTRUCT, outcome::REJECTED),
            (false, vec![])
        );
        assert_eq!(g.admit(4, event::LOGIN, outcome::FAILURE), (false, vec![]));
        // The next window opens: the drops are reported once, then recording resumes.
        let (ok, flushed) = g.admit(60_000, event::LOGIN, outcome::FAILURE);
        assert!(ok);
        assert_eq!(flushed.len(), 2);
        let login = flushed
            .iter()
            .find(|e| e.field("of") == Some("auth.login"))
            .unwrap();
        assert_eq!(login.event, event::SUPPRESSED);
        assert_eq!(login.field("count"), Some("2"));
        assert_eq!(login.field("of_outcome"), Some("failure"));
    }

    #[test]
    fn summary_counts_the_window_and_the_suppressed() {
        let now = 1_000_000_000;
        let day = 86_400_000;
        let lines = [
            AuditEvent::new(now - 10, event::LOGIN, outcome::SUCCESS).to_json_line(),
            AuditEvent::new(now - 20, event::LOGIN, outcome::FAILURE)
                .with("ip", "198.51.100.7")
                .to_json_line(),
            AuditEvent::new(now - 30, event::LOGIN, outcome::THROTTLED).to_json_line(),
            AuditEvent::new(now - 40, event::ACCESS, outcome::SUCCESS)
                .with("who", "me@example.com")
                .to_json_line(),
            AuditEvent::new(now - 50, event::DESTRUCT, outcome::REJECTED).to_json_line(),
            AuditEvent::new(now - 60, event::EGRESS, outcome::CHANGED)
                .with("from", "confirmed")
                .with("to", "degraded")
                .to_json_line(),
            AuditEvent::new(now - 70, event::SUPPRESSED, outcome::ERROR)
                .with("of", "auth.login")
                .with("of_outcome", "failure")
                .with("count", "40")
                .to_json_line(),
            // Older than the window: ignored.
            AuditEvent::new(now - 2 * day, event::LOGIN, outcome::FAILURE).to_json_line(),
            "garbage".to_string(),
        ];
        let s = summarize(lines.iter().map(String::as_str), now, day);
        assert_eq!(s.logins_ok, 1);
        assert_eq!(s.logins_failed, 41);
        assert_eq!(s.logins_throttled, 1);
        assert_eq!(s.access_ok, 1);
        assert_eq!(s.destruct_attempts, 1);
        assert_eq!(s.destruct_triggered, 0);
        assert_eq!(s.egress_changes, 1);
        assert_eq!(
            s.last_failed_login,
            Some((now - 20, Some("198.51.100.7".to_string())))
        );
        assert_eq!(
            s.last_egress,
            Some((now - 60, "confirmed -> degraded".into()))
        );
    }

    #[test]
    fn memory_sink_keeps_what_it_is_given() {
        let sink = MemorySink::new();
        sink.record(&AuditEvent::new(1, event::LOGOUT, outcome::SUCCESS));
        assert_eq!(sink.lines().len(), 1);
        NoopSink.record(&AuditEvent::new(1, event::LOGOUT, outcome::SUCCESS));
    }
}
