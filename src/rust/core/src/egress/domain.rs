//! Egress domain: what we know about the container's route to the internet.
//!
//! A faithful port of packages/core/src/egress/domain.ts. Egress is BEST EFFORT, not enforced:
//! this reports what was OBSERVED, never what is guaranteed. Pure (the probing lives in infra).
//! Tests mirror packages/core/tests/egress-domain.test.ts.

use serde::Serialize;

/// confirmed = proxy listens and a request through it worked; degraded = listens but the request
/// failed; down = nothing listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EgressStatus {
    Confirmed,
    Degraded,
    Down,
}

impl EgressStatus {
    fn as_str(self) -> &'static str {
        match self {
            EgressStatus::Confirmed => "confirmed",
            EgressStatus::Degraded => "degraded",
            EgressStatus::Down => "down",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EgressObservation {
    pub proxy_listening: bool,
    pub probe_ok: bool,
}

pub fn egress_status(o: EgressObservation) -> EgressStatus {
    // A probe "through" a proxy nobody is listening on cannot have worked.
    if !o.proxy_listening {
        return EgressStatus::Down;
    }
    if o.probe_ok {
        EgressStatus::Confirmed
    } else {
        EgressStatus::Degraded
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressReport {
    pub status: EgressStatus,
    /// ISO timestamp of the observation.
    pub checked_at: String,
    /// Nodes the subscription provider had loaded; None when the controller did not answer.
    pub provider_nodes: Option<i64>,
}

/// One line for logs and the banner. Non-confirmed states are upper-case on purpose.
pub fn describe_egress(report: &EgressReport) -> String {
    match report.status {
        EgressStatus::Confirmed => "confirmed (a request through the proxy succeeded)".to_string(),
        EgressStatus::Degraded => {
            if report.provider_nodes == Some(0) {
                "DEGRADED: the proxy is up but its subscription loaded no nodes (check CLASH_SUBSCRIPTION_URL)".to_string()
            } else {
                "DEGRADED: the proxy is up but a request through it failed (nodes may be down or still starting)".to_string()
            }
        }
        EgressStatus::Down => "DOWN: nothing is listening on the proxy port".to_string(),
    }
}

/// The note left for the user while egress is not confirmed; None when there is nothing to say.
pub fn egress_notice(report: &EgressReport) -> Option<String> {
    if report.status == EgressStatus::Confirmed {
        return None;
    }
    let head = if report.status == EgressStatus::Down {
        "DOWN"
    } else {
        "DEGRADED"
    };
    Some(
        [
            format!("# ⚠ Egress {head}"),
            String::new(),
            format!("Checked at {}.", report.checked_at),
            String::new(),
            describe_egress(report),
            String::new(),
            "Tools that honor `http_proxy` (git, curl, npm, restic) will fail until the proxy recovers.".to_string(),
            "The workspace itself keeps running.".to_string(),
            String::new(),
            "Egress is best effort: nothing blocks a process that ignores the proxy settings (or runs".to_string(),
            "under `noproxy <cmd>`) from connecting directly.".to_string(),
            String::new(),
            "Run `doctor` for the live state. This note is removed at the next start once egress is confirmed.".to_string(),
            String::new(),
        ]
        .join("\n"),
    )
}

// --- persisted form (what `doctor` reads back) ---

pub fn serialize_egress_report(report: &EgressReport) -> String {
    format!(
        "{}\n",
        serde_json::to_string(report).expect("EgressReport serializes")
    )
}

/// None for anything that is not a report we wrote: the file is only a convenience. The field
/// providerNodes must be PRESENT (null or a finite number); a missing one is not our report.
pub fn parse_egress_report(raw: Option<&str>) -> Option<EgressReport> {
    let raw = raw?;
    if raw.is_empty() {
        return None;
    }
    let data: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = data.as_object()?;
    let status = match obj.get("status")?.as_str()? {
        "confirmed" => EgressStatus::Confirmed,
        "degraded" => EgressStatus::Degraded,
        "down" => EgressStatus::Down,
        _ => return None,
    };
    let checked_at = obj.get("checkedAt")?.as_str()?.to_string();
    let nodes_val = obj.get("providerNodes")?;
    let provider_nodes = if nodes_val.is_null() {
        None
    } else {
        Some(nodes_val.as_f64().filter(|f| f.is_finite())? as i64)
    };
    Some(EgressReport {
        status,
        checked_at,
        provider_nodes,
    })
}

// --- doctor ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressHealth {
    pub status: Health,
    pub detail: String,
}

/// The doctor row: the LIVE state, plus what the start-up probe saw. A degraded proxy is a
/// warning; only "nothing is listening" is a failure.
pub fn egress_health(
    live: EgressStatus,
    at_boot: Option<&EgressReport>,
    proxy_port: u16,
) -> EgressHealth {
    let boot = match at_boot {
        Some(r) => format!("; at start-up: {}", r.status.as_str()),
        None => String::new(),
    };
    match live {
        EgressStatus::Confirmed => {
            EgressHealth { status: Health::Ok, detail: format!("via 127.0.0.1:{proxy_port}{boot}") }
        }
        EgressStatus::Degraded => EgressHealth {
            status: Health::Warn,
            detail: format!(
                "proxy up on :{proxy_port} but a request through it failed (nodes down or starting){boot}"
            ),
        },
        EgressStatus::Down => {
            EgressHealth { status: Health::Fail, detail: format!("nothing listening on :{proxy_port}{boot}") }
        }
    }
}

// --- Clash config ---

pub const CLASH_URL_PLACEHOLDER: &str = "{{CLASH_SUBSCRIPTION_URL}}";

/// Fill the subscription URL into the config template. The template holds it inside a
/// double-quoted YAML scalar, so the URL is JSON-string-escaped first (serde_json gives exactly
/// what JSON.stringify does); the replace is literal, so `$&`/`$1` in the URL are not special.
pub fn render_clash_config(template: &str, subscription_url: &str) -> String {
    let quoted = serde_json::to_string(subscription_url).expect("a string serializes");
    let escaped = &quoted[1..quoted.len() - 1];
    template.replace(CLASH_URL_PLACEHOLDER, escaped)
}

// --- timing policy of the start-up bring-up ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wait {
    pub attempts: u32,
    pub delay_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EgressTiming {
    pub proxy_wait: Wait,
    pub provider_wait: Wait,
    pub refresh_wait: Wait,
    pub settle_ms: u64,
    pub probe: Wait,
}

pub const DEFAULT_EGRESS_TIMING: EgressTiming = EgressTiming {
    proxy_wait: Wait {
        attempts: 30,
        delay_ms: 1000,
    },
    provider_wait: Wait {
        attempts: 15,
        delay_ms: 2000,
    },
    refresh_wait: Wait {
        attempts: 3,
        delay_ms: 2000,
    },
    settle_ms: 2000,
    probe: Wait {
        attempts: 3,
        delay_ms: 3000,
    },
};

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn report(status: EgressStatus, provider_nodes: Option<i64>) -> EgressReport {
        EgressReport {
            status,
            checked_at: "2026-10-17T12:00:00.000Z".to_string(),
            provider_nodes,
        }
    }

    #[test]
    fn egress_status_mapping() {
        assert_eq!(
            egress_status(EgressObservation {
                proxy_listening: true,
                probe_ok: true
            }),
            EgressStatus::Confirmed
        );
        assert_eq!(
            egress_status(EgressObservation {
                proxy_listening: true,
                probe_ok: false
            }),
            EgressStatus::Degraded
        );
        assert_eq!(
            egress_status(EgressObservation {
                proxy_listening: false,
                probe_ok: false
            }),
            EgressStatus::Down
        );
        // A probe cannot succeed through a proxy nobody is listening on.
        assert_eq!(
            egress_status(EgressObservation {
                proxy_listening: false,
                probe_ok: true
            }),
            EgressStatus::Down
        );
    }

    proptest! {
        #[test]
        fn confirmed_iff_both(proxy_listening in any::<bool>(), probe_ok in any::<bool>()) {
            let confirmed = egress_status(EgressObservation { proxy_listening, probe_ok }) == EgressStatus::Confirmed;
            prop_assert_eq!(confirmed, proxy_listening && probe_ok);
        }
    }

    #[test]
    fn describe_and_notice() {
        assert!(describe_egress(&report(EgressStatus::Confirmed, Some(12))).contains("confirmed"));
        assert!(describe_egress(&report(EgressStatus::Degraded, Some(12))).contains("DEGRADED"));
        assert!(describe_egress(&report(EgressStatus::Down, Some(12))).contains("DOWN"));

        assert!(describe_egress(&report(EgressStatus::Degraded, Some(0)))
            .contains("CLASH_SUBSCRIPTION_URL"));
        assert!(!describe_egress(&report(EgressStatus::Degraded, Some(12)))
            .contains("CLASH_SUBSCRIPTION_URL"));
        assert!(!describe_egress(&report(EgressStatus::Degraded, None))
            .contains("CLASH_SUBSCRIPTION_URL"));

        assert_eq!(
            egress_notice(&report(EgressStatus::Confirmed, Some(12))),
            None
        );
        let down = egress_notice(&report(EgressStatus::Down, Some(12))).unwrap();
        assert!(down.contains("# ⚠ Egress DOWN"));
        assert!(down.contains("2026-10-17T12:00:00.000Z"));
        let degraded = egress_notice(&report(EgressStatus::Degraded, Some(12))).unwrap();
        assert!(degraded.contains("# ⚠ Egress DEGRADED"));
        assert!(degraded.contains("best effort"));
        assert!(degraded.contains("noproxy"));
    }

    #[test]
    fn persisted_report_round_trips() {
        for status in [
            EgressStatus::Confirmed,
            EgressStatus::Degraded,
            EgressStatus::Down,
        ] {
            for nodes in [Some(0i64), Some(7), None] {
                let r = report(status, nodes);
                let round = parse_egress_report(Some(&serialize_egress_report(&r)));
                assert_eq!(round, Some(r));
            }
        }
    }

    #[test]
    fn rejects_anything_not_ours() {
        let bad = [
            None,
            Some(""),
            Some("not json"),
            Some("null"),
            Some("[]"),
            Some(r#"{"status":"fine","checkedAt":"x","providerNodes":1}"#),
            Some(r#"{"status":"down","providerNodes":1}"#),
            Some(r#"{"status":"down","checkedAt":"x"}"#),
            Some(r#"{"status":"down","checkedAt":"x","providerNodes":"7"}"#),
        ];
        for raw in bad {
            assert_eq!(parse_egress_report(raw), None, "should reject {raw:?}");
        }
    }

    #[test]
    fn egress_health_rows() {
        assert_eq!(
            egress_health(EgressStatus::Confirmed, None, 7890).status,
            Health::Ok
        );
        assert_eq!(
            egress_health(EgressStatus::Degraded, None, 7890).status,
            Health::Warn
        );
        assert_eq!(
            egress_health(EgressStatus::Down, None, 7890).status,
            Health::Fail
        );

        let row = egress_health(
            EgressStatus::Confirmed,
            Some(&report(EgressStatus::Degraded, Some(12))),
            7890,
        );
        assert_eq!(row.status, Health::Ok);
        assert!(row.detail.contains("at start-up: degraded"));
        assert!(!egress_health(EgressStatus::Confirmed, None, 7890)
            .detail
            .contains("start-up"));
        assert!(egress_health(EgressStatus::Down, None, 7890)
            .detail
            .contains(":7890"));
    }

    #[test]
    fn render_clash_config_cases() {
        let template =
            format!("proxy-providers:\n  subscription:\n    url: \"{CLASH_URL_PLACEHOLDER}\"\n");
        assert_eq!(
            render_clash_config(&template, "https://sub.example/api?token=abc"),
            "proxy-providers:\n  subscription:\n    url: \"https://sub.example/api?token=abc\"\n"
        );

        let twice = format!("{CLASH_URL_PLACEHOLDER} {CLASH_URL_PLACEHOLDER}");
        assert_eq!(render_clash_config(&twice, "u"), "u u");

        // A string replacement would paste the placeholder back for `$&`; ours does not.
        for url in [
            "https://x/?a=$&b",
            "https://x/$'y",
            "https://x/$1$$",
            "https://x/$`z",
        ] {
            assert_eq!(
                render_clash_config(&format!("url: \"{CLASH_URL_PLACEHOLDER}\""), url),
                format!("url: \"{url}\"")
            );
        }

        // Escapes what would end the quoted YAML scalar.
        assert_eq!(
            render_clash_config(
                &format!("url: \"{CLASH_URL_PLACEHOLDER}\""),
                "https://x/\"q\"\\n"
            ),
            "url: \"https://x/\\\"q\\\"\\\\n\""
        );

        // Keeps a url with a real line break on one line.
        let out = render_clash_config(
            &format!("url: \"{CLASH_URL_PLACEHOLDER}\""),
            "https://x\nevil: 1",
        );
        assert_eq!(out.lines().count(), 1);
    }
}
