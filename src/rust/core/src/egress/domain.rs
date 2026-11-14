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
    pub fn as_str(self) -> &'static str {
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
pub const CLASH_DNS_PLACEHOLDER: &str = "{{CLASH_DNS_NAMESERVER}}";
pub const CLASH_SECRET_PLACEHOLDER: &str = "{{CLASH_CONTROLLER_SECRET}}";
pub const CLASH_LAN_PLACEHOLDER: &str = "{{CLASH_ALLOW_LAN}}";

/// Every placeholder the template must contain. A template that lacks one would render without
/// the corresponding setting (an open controller, a Clash that ignores the subscription), so
/// `render_clash_config` refuses it instead of producing a config that merely looks fine.
const CLASH_PLACEHOLDERS: [&str; 4] = [
    CLASH_URL_PLACEHOLDER,
    CLASH_DNS_PLACEHOLDER,
    CLASH_SECRET_PLACEHOLDER,
    CLASH_LAN_PLACEHOLDER,
];

/// DNS resolvers Clash uses to resolve proxy-node and subscription hostnames (comma-separated).
/// Global anycast DoH: correct and fast on most networks. Inside mainland China these are blocked
/// or poisoned, which leaves Clash with zero usable nodes; override with CLASH_DNS there (for
/// example 223.5.5.5,119.29.29.29).
pub const DEFAULT_CLASH_DNS: &str = "https://1.1.1.1/dns-query,https://8.8.8.8/dns-query";

/// Mainland-China fallback: plain-UDP domestic resolvers (AliDNS, DNSPod) that need no bootstrap
/// and are not blocked there. What auto-detection picks when the global DoH probe fails.
pub const DEFAULT_CLASH_DNS_CN: &str = "223.5.5.5,119.29.29.29";

/// Pick the effective nameservers by precedence: a runtime override (written by `set-clash-dns`)
/// wins, then the CLASH_DNS env var, then auto-detection. `foreign_dns_reachable` is a boot probe
/// (is global DoH reachable?) consulted ONLY when neither of the first two is set: true keeps the
/// global default, false — a censored network — uses the mainland-China resolvers. Blank/whitespace
/// values count as unset so an empty override or env var does not wedge Clash.
pub fn choose_clash_dns<'a>(
    override_value: Option<&'a str>,
    env_value: Option<&'a str>,
    foreign_dns_reachable: bool,
) -> &'a str {
    fn nonempty(s: &str) -> Option<&str> {
        let t = s.trim();
        (!t.is_empty()).then_some(t)
    }
    if let Some(s) = override_value.and_then(nonempty) {
        return s;
    }
    if let Some(s) = env_value.and_then(nonempty) {
        return s;
    }
    if foreign_dns_reachable {
        DEFAULT_CLASH_DNS
    } else {
        DEFAULT_CLASH_DNS_CN
    }
}

/// What differs from one rendering of the Clash template to the next.
#[derive(Debug, Clone, Copy)]
pub struct ClashRender<'a> {
    pub subscription_url: &'a str,
    /// Comma-separated nameservers (see `choose_clash_dns`).
    pub dns_nameservers: &'a str,
    /// Bearer token the controller on 127.0.0.1 demands. Without it any process in the container
    /// can re-route or reconfigure Clash.
    pub controller_secret: &'a str,
    /// Listen on every interface rather than loopback only. The standalone client needs this for a
    /// published container port to reach Clash; the workspace never does.
    pub allow_lan: bool,
}

/// `s` as the inside of a double-quoted YAML scalar (JSON string escaping, which YAML accepts):
/// a quote, backslash, `#` or line break in a value is escaped, never honoured.
fn yaml_dq_inner(s: &str) -> String {
    let quoted = serde_json::to_string(s).expect("a string serializes");
    quoted[1..quoted.len() - 1].to_string()
}

/// Fill the template's placeholders in ONE pass over the template, so a value is never rescanned:
/// a subscription URL that happens to contain another placeholder's text is not expanded, and
/// `$&`/`$1` in a value are not special. User-supplied values are escaped for their YAML context.
fn fill_template(template: &str, p: &ClashRender) -> String {
    let values = [
        (CLASH_URL_PLACEHOLDER, yaml_dq_inner(p.subscription_url)),
        (CLASH_DNS_PLACEHOLDER, clash_dns_block(p.dns_nameservers)),
        (CLASH_SECRET_PLACEHOLDER, yaml_dq_inner(p.controller_secret)),
        (
            CLASH_LAN_PLACEHOLDER,
            if p.allow_lan { "true" } else { "false" }.to_string(),
        ),
    ];
    let mut out = String::with_capacity(template.len() + 256);
    let mut rest = template;
    while let Some(at) = rest.find("{{") {
        out.push_str(&rest[..at]);
        let here = &rest[at..];
        match values.iter().find(|(name, _)| here.starts_with(name)) {
            Some((name, value)) => {
                out.push_str(value);
                rest = &here[name.len()..];
            }
            None => {
                out.push_str("{{");
                rest = &here[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Render the Clash config from its template. Refuses a template that lacks any placeholder, and
/// checks the result reads back with the secret it was given, so a template edit cannot silently
/// drop the controller secret (or the subscription) from the config.
pub fn render_clash_config(template: &str, p: &ClashRender) -> Result<String, String> {
    if p.controller_secret.is_empty() {
        return Err("the controller secret is empty".to_string());
    }
    for placeholder in CLASH_PLACEHOLDERS {
        if !template.contains(placeholder) {
            return Err(format!("the Clash template has no {placeholder}"));
        }
    }
    let out = fill_template(template, p);
    if parse_controller_secret(&out).as_deref() != Some(p.controller_secret) {
        return Err("the rendered config does not carry the controller secret".to_string());
    }
    Ok(out)
}

/// The controller secret of a config we rendered: the top-level `secret: "<json string>"` line.
/// None when absent, empty or not a quoted string (which our renderer never writes).
pub fn parse_controller_secret(config: &str) -> Option<String> {
    let value = config
        .lines()
        .find_map(|line| line.strip_prefix("secret:"))?;
    let secret: String = serde_json::from_str(value.trim()).ok()?;
    (!secret.is_empty()).then_some(secret)
}

/// The comma-separated nameservers as an indented YAML list to drop under `nameserver:`. Each is a
/// double-quoted, JSON-escaped scalar, so a space, `#` or newline inside one cannot break out of
/// the list. Blank entries are dropped; an empty list falls back to DEFAULT_CLASH_DNS so Clash is
/// never left without a resolver.
fn clash_dns_block(dns_nameservers: &str) -> String {
    fn yaml_items(list: &str) -> Vec<String> {
        list.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|ns| {
                format!(
                    "    - {}",
                    serde_json::to_string(ns).expect("a string serializes")
                )
            })
            .collect()
    }
    let items = yaml_items(dns_nameservers);
    let items = if items.is_empty() {
        yaml_items(DEFAULT_CLASH_DNS)
    } else {
        items
    };
    items.join("\n")
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

    /// `fill_template` with the secret and LAN flag fixed, for the tests that are about one value.
    fn fill(template: &str, url: &str, dns: &str) -> String {
        fill_template(
            template,
            &ClashRender {
                subscription_url: url,
                dns_nameservers: dns,
                controller_secret: "s3cret",
                allow_lan: false,
            },
        )
    }

    #[test]
    fn render_clash_config_cases() {
        let dns = "223.5.5.5";
        let template =
            format!("proxy-providers:\n  subscription:\n    url: \"{CLASH_URL_PLACEHOLDER}\"\n");
        assert_eq!(
            fill(&template, "https://sub.example/api?token=abc", dns),
            "proxy-providers:\n  subscription:\n    url: \"https://sub.example/api?token=abc\"\n"
        );

        let twice = format!("{CLASH_URL_PLACEHOLDER} {CLASH_URL_PLACEHOLDER}");
        assert_eq!(fill(&twice, "u", dns), "u u");

        // A string replacement would paste the placeholder back for `$&`; ours does not.
        for url in [
            "https://x/?a=$&b",
            "https://x/$'y",
            "https://x/$1$$",
            "https://x/$`z",
        ] {
            assert_eq!(
                fill(&format!("url: \"{CLASH_URL_PLACEHOLDER}\""), url, dns),
                format!("url: \"{url}\"")
            );
        }

        // Escapes what would end the quoted YAML scalar.
        assert_eq!(
            fill(
                &format!("url: \"{CLASH_URL_PLACEHOLDER}\""),
                "https://x/\"q\"\\n",
                dns
            ),
            "url: \"https://x/\\\"q\\\"\\\\n\""
        );

        // Keeps a url with a real line break on one line.
        let out = fill(
            &format!("url: \"{CLASH_URL_PLACEHOLDER}\""),
            "https://x\nevil: 1",
            dns,
        );
        assert_eq!(out.lines().count(), 1);
    }

    #[test]
    fn a_value_is_never_rescanned_for_placeholders() {
        // The subscription URL is user input. If it contained another placeholder's text, a
        // sequence of replaces would expand it (here: paste the controller secret into the URL).
        let template =
            format!("url: \"{CLASH_URL_PLACEHOLDER}\"\nsecret: \"{CLASH_SECRET_PLACEHOLDER}\"\n");
        let out = fill(
            &template,
            &format!("https://x/{CLASH_SECRET_PLACEHOLDER}"),
            "d",
        );
        assert_eq!(
            out,
            format!("url: \"https://x/{CLASH_SECRET_PLACEHOLDER}\"\nsecret: \"s3cret\"\n")
        );
        // A lone `{{` that is not one of ours passes through untouched.
        assert_eq!(fill("a {{ b }} {{", "u", "d"), "a {{ b }} {{");
    }

    #[test]
    fn render_clash_dns_cases() {
        let template = format!("dns:\n  nameserver:\n{CLASH_DNS_PLACEHOLDER}\n");

        // Comma-separated nameservers become an indented, quoted YAML list; surrounding
        // whitespace is trimmed and blank entries dropped.
        assert_eq!(
            fill(&template, "u", "223.5.5.5, 119.29.29.29"),
            "dns:\n  nameserver:\n    - \"223.5.5.5\"\n    - \"119.29.29.29\"\n"
        );
        assert_eq!(
            fill(&template, "u", "  ,, 1.1.1.1 ,"),
            "dns:\n  nameserver:\n    - \"1.1.1.1\"\n"
        );

        // DoH/DoT forms (with `#` params) survive intact inside the quoted scalar.
        assert_eq!(
            fill(&template, "u", "https://doh.pub/dns-query#h3=true"),
            "dns:\n  nameserver:\n    - \"https://doh.pub/dns-query#h3=true\"\n"
        );

        // An empty (or all-blank) list falls back to the global default, never an empty list.
        for empty in ["", "   ", " , , "] {
            let out = fill(&template, "u", empty);
            assert!(
                out.contains("    - \"https://1.1.1.1/dns-query\""),
                "{empty:?}"
            );
            assert!(
                out.contains("    - \"https://8.8.8.8/dns-query\""),
                "{empty:?}"
            );
        }

        // A nameserver cannot break out of the list: a newline/quote is escaped, not honoured.
        let evil = fill(&template, "u", "1.1.1.1\nrules: []");
        assert!(evil.contains(r#"    - "1.1.1.1\nrules: []""#));
        assert!(!evil.contains("\nrules: []"));
    }

    fn params<'a>(url: &'a str, secret: &'a str, allow_lan: bool) -> ClashRender<'a> {
        ClashRender {
            subscription_url: url,
            dns_nameservers: "223.5.5.5",
            controller_secret: secret,
            allow_lan,
        }
    }

    /// A template with every placeholder, the shape of the shipped one.
    fn full_template() -> String {
        format!(
            "allow-lan: {CLASH_LAN_PLACEHOLDER}\nsecret: \"{CLASH_SECRET_PLACEHOLDER}\"\n\
             dns:\n  nameserver:\n{CLASH_DNS_PLACEHOLDER}\nurl: \"{CLASH_URL_PLACEHOLDER}\"\n"
        )
    }

    #[test]
    fn render_fills_every_placeholder_and_the_secret_reads_back() {
        let out = render_clash_config(&full_template(), &params("https://sub/x", "abc123", true))
            .unwrap();
        assert!(out.contains("allow-lan: true\n"));
        assert!(out.contains("url: \"https://sub/x\"\n"));
        assert!(out.contains("    - \"223.5.5.5\""));
        assert!(!out.contains("{{"));
        assert_eq!(parse_controller_secret(&out).as_deref(), Some("abc123"));

        let closed = render_clash_config(&full_template(), &params("u", "abc123", false)).unwrap();
        assert!(closed.contains("allow-lan: false\n"));
    }

    #[test]
    fn a_template_missing_a_placeholder_is_refused() {
        // Each placeholder in turn removed: the result would silently lack that setting.
        for missing in CLASH_PLACEHOLDERS {
            let template = full_template().replace(missing, "x");
            let err = render_clash_config(&template, &params("u", "abc123", false)).unwrap_err();
            assert!(err.contains(missing), "{err}");
        }
        // An empty secret would render `secret: ""`, which means no authentication.
        assert!(render_clash_config(&full_template(), &params("u", "", false)).is_err());
    }

    #[test]
    fn the_secret_survives_a_hostile_value_and_reads_back_exactly() {
        // A secret with quote, backslash and a line break still round-trips through the file.
        let secret = "a\"b\\c\nd";
        let out = render_clash_config(&full_template(), &params("u", secret, false)).unwrap();
        assert_eq!(parse_controller_secret(&out).as_deref(), Some(secret));
        assert_eq!(out.lines().filter(|l| l.starts_with("secret:")).count(), 1);
    }

    #[test]
    fn parse_controller_secret_cases() {
        assert_eq!(
            parse_controller_secret("a: 1\nsecret: \"xyz\"\nb: 2\n").as_deref(),
            Some("xyz")
        );
        // Only the top-level key counts; an indented one belongs to something else.
        assert_eq!(parse_controller_secret("x:\n  secret: \"nested\"\n"), None);
        // An empty, unquoted or absent secret is no secret.
        assert_eq!(parse_controller_secret("secret: \"\"\n"), None);
        assert_eq!(parse_controller_secret("secret: bare\n"), None);
        assert_eq!(parse_controller_secret("a: 1\n"), None);
        assert_eq!(parse_controller_secret(""), None);
    }

    /// The template that ships in the image: renders with every placeholder consumed and carries
    /// the secret. (Whether Clash accepts the result is the smoke suite's question.)
    const SHIPPED_TEMPLATE: &str =
        include_str!("../../../../image/share/clash/config.yaml.template");

    #[test]
    fn the_shipped_template_renders_completely() {
        let out = render_clash_config(
            SHIPPED_TEMPLATE,
            &ClashRender {
                subscription_url: "https://sub.example/api?token=abc",
                dns_nameservers: "223.5.5.5,119.29.29.29",
                controller_secret: "0123456789abcdef",
                allow_lan: false,
            },
        )
        .unwrap();
        assert!(!out.contains("{{"), "an unfilled placeholder is left");
        assert_eq!(
            parse_controller_secret(&out).as_deref(),
            Some("0123456789abcdef")
        );
        assert!(out.contains("\nallow-lan: false\n"));
        assert!(out.contains("\nexternal-controller: 127.0.0.1:9090\n"));
        assert!(out.contains("url: \"https://sub.example/api?token=abc\"\n"));
        // The controller stays on loopback even when the proxy ports are opened up.
        let lan =
            render_clash_config(SHIPPED_TEMPLATE, &params("u", "0123456789abcdef", true)).unwrap();
        assert!(lan.contains("\nallow-lan: true\n"));
        assert!(lan.contains("\nexternal-controller: 127.0.0.1:9090\n"));
        // The names the rest of the code looks up (layout.rs) are defined by the template.
        assert!(out.contains("\n  subscription:\n"));
        assert!(out.contains("name: auto-select\n"));
        // An empty subscription must end in REJECT, not in the DIRECT an empty group falls back to.
        assert!(out.contains("proxies:\n      - REJECT\n    include-all-providers: true\n"));
        // Routing is declared, not selected at run time: a `global` mode would send everything
        // DIRECT until something picked the auto-select group.
        assert!(out.contains("\nmode: rule\n"));
        assert!(out.contains("\n  - MATCH,auto-select\n"));
    }

    #[test]
    fn choose_clash_dns_precedence() {
        // Override beats env and auto-detect.
        assert_eq!(
            choose_clash_dns(Some("1.0.0.1"), Some("8.8.8.8"), true),
            "1.0.0.1"
        );
        assert_eq!(choose_clash_dns(Some(" 1.0.0.1 "), None, false), "1.0.0.1");
        // Env beats auto-detect when there is no override.
        assert_eq!(choose_clash_dns(None, Some("8.8.4.4"), false), "8.8.4.4");
        assert_eq!(
            choose_clash_dns(Some("  "), Some("8.8.4.4"), true),
            "8.8.4.4"
        );
        // Neither set → auto-detect: reachable keeps global, blocked uses domestic.
        assert_eq!(choose_clash_dns(None, None, true), DEFAULT_CLASH_DNS);
        assert_eq!(choose_clash_dns(None, None, false), DEFAULT_CLASH_DNS_CN);
        // Blank override AND blank env also fall through to auto-detect.
        assert_eq!(
            choose_clash_dns(Some(""), Some("  "), false),
            DEFAULT_CLASH_DNS_CN
        );
        assert_eq!(
            choose_clash_dns(Some(" "), Some(""), true),
            DEFAULT_CLASH_DNS
        );
    }
}
