//! The start-up banner — a port of packages/core/src/workspace/banner.ts. What is NOT working is
//! part of the last thing on screen, not a warning buried in the scroll-back. Tests mirror the
//! renderBanner half of packages/core/tests/auth-banner.test.ts.

use crate::desktop::domain::DesktopTarget;
use crate::egress::domain::{describe_egress, EgressReport, EgressStatus};

pub struct BannerInput {
    pub password: String,
    /// Host port the web UI is published on.
    pub host_port: String,
    /// Which desktop stacks the image contains.
    pub stacks: Vec<DesktopTarget>,
    pub vault_mounted: bool,
    pub egress: Option<EgressReport>,
    /// Boot steps that failed under the "degrade" policy.
    pub degraded: Vec<String>,
}

const INNER_WIDTH: usize = 50;

fn draw_box(rows: &[String]) -> Vec<String> {
    let bar = "═".repeat(INNER_WIDTH);
    let (title, body) = rows.split_first().expect("at least a title row");
    let mut out = vec![
        format!("╔{bar}╗"),
        format!("║{title:<INNER_WIDTH$}║"),
        format!("╠{bar}╣"),
    ];
    for row in body {
        out.push(format!("║{row:<INNER_WIDTH$}║"));
    }
    out.push(format!("╚{bar}╝"));
    out
}

/// Short status word for the box; the full sentence goes below it when something is off.
fn egress_word(egress: Option<&EgressReport>) -> &'static str {
    match egress {
        None => "UNKNOWN",
        Some(r) => match r.status {
            EgressStatus::Confirmed => "confirmed",
            EgressStatus::Degraded => "DEGRADED",
            EgressStatus::Down => "DOWN",
        },
    }
}

pub fn render_banner(input: &BannerInput) -> Vec<String> {
    let origin = format!("http://localhost:{}", input.host_port);

    let mut rows = vec![
        format!("{}Secure Dev Workspace Ready", " ".repeat(10)),
        format!("  Password:  {}", input.password),
        format!("  Code:      {origin}"),
    ];
    if input.stacks.contains(&DesktopTarget::Selkies) {
        rows.push(format!("  Desktop:   {origin}/desktop/"));
    }
    if input.stacks.contains(&DesktopTarget::Vnc) {
        rows.push(format!("  VNC:       {origin}/vnc/"));
    }
    rows.push(format!("  Auth:      user / {}", input.password));
    rows.push(format!(
        "  Vault:     {}",
        if input.vault_mounted {
            "unlocked"
        } else {
            "LOCKED"
        }
    ));
    rows.push(format!(
        "  Egress:    {}",
        egress_word(input.egress.as_ref())
    ));

    let mut attention: Vec<String> = Vec::new();
    if !input.vault_mounted {
        attention.push("Vault is locked: run `unlock-vault` to open /workspace.".to_string());
    }
    match &input.egress {
        None => attention
            .push("Egress was not checked (the start-up step failed). Run `doctor`.".to_string()),
        Some(e) if e.status != EgressStatus::Confirmed => {
            attention.push(format!("Egress {}. Run `doctor`.", describe_egress(e)));
        }
        _ => {}
    }
    if !input.degraded.is_empty() {
        attention.push(format!(
            "Start-up steps that failed (continuing without them): {}.",
            input.degraded.join(", ")
        ));
    }

    let mut out = vec![String::new()];
    out.extend(draw_box(&rows));
    if !attention.is_empty() {
        out.push(String::new());
        for line in attention {
            out.push(format!("⚠ {line}"));
        }
    }
    out.push(String::new());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn confirmed() -> EgressReport {
        EgressReport {
            status: EgressStatus::Confirmed,
            checked_at: "t".to_string(),
            provider_nodes: Some(9),
        }
    }
    fn base() -> BannerInput {
        BannerInput {
            password: "s3cret".to_string(),
            host_port: "18080".to_string(),
            stacks: vec![DesktopTarget::Selkies, DesktopTarget::Vnc],
            vault_mounted: true,
            egress: Some(confirmed()),
            degraded: vec![],
        }
    }
    fn text(input: &BannerInput) -> String {
        render_banner(input).join("\n")
    }

    #[test]
    fn box_lines_are_equal_width() {
        let lines: Vec<String> = render_banner(&base())
            .into_iter()
            .filter(|l| matches!(l.chars().next(), Some('╔' | '║' | '╠' | '╚')))
            .collect();
        assert!(lines.len() > 5);
        let widths: std::collections::HashSet<usize> =
            lines.iter().map(|l| l.chars().count()).collect();
        assert_eq!(widths.len(), 1);
    }

    #[test]
    fn shows_login_and_where() {
        let out = text(&base());
        assert!(out.contains("Password:  s3cret"));
        assert!(out.contains("user / s3cret"));
        assert!(out.contains("http://localhost:18080/desktop/"));
        assert!(out.contains("http://localhost:18080/vnc/"));
    }

    #[test]
    fn prints_the_actual_host_port() {
        let out = text(&BannerInput {
            host_port: "18081".to_string(),
            ..base()
        });
        assert!(out.contains("Code:      http://localhost:18081"));
        assert!(out.contains("http://localhost:18081/desktop/"));
        assert!(!out.contains("18080"));
    }

    #[test]
    fn lists_only_contained_desktops() {
        assert!(!text(&BannerInput {
            stacks: vec![DesktopTarget::Vnc],
            ..base()
        })
        .contains("/desktop/"));
        assert!(!text(&BannerInput {
            stacks: vec![DesktopTarget::Selkies],
            ..base()
        })
        .contains("/vnc/"));
    }

    #[test]
    fn healthy_start_has_no_attention() {
        let out = text(&base());
        assert!(out.contains("Vault:     unlocked"));
        assert!(out.contains("Egress:    confirmed"));
        assert!(!out.contains('⚠'));
    }

    #[test]
    fn locked_vault_says_so() {
        let out = text(&BannerInput {
            vault_mounted: false,
            ..base()
        });
        assert!(out.contains("Vault:     LOCKED"));
        assert!(out.contains("⚠ Vault is locked: run `unlock-vault`"));
    }

    #[test]
    fn degraded_and_down_egress_in_box_and_below() {
        let degraded = EgressReport {
            status: EgressStatus::Degraded,
            ..confirmed()
        };
        let out = text(&BannerInput {
            egress: Some(degraded),
            ..base()
        });
        assert!(out.contains("Egress:    DEGRADED"));
        assert!(out.contains("⚠ Egress DEGRADED:"));
        let down = EgressReport {
            status: EgressStatus::Down,
            ..confirmed()
        };
        let out = text(&BannerInput {
            egress: Some(down),
            ..base()
        });
        assert!(out.contains("Egress:    DOWN"));
        assert!(out.contains("⚠ Egress DOWN:"));
    }

    #[test]
    fn failed_egress_step_reads_unknown() {
        let out = text(&BannerInput {
            egress: None,
            ..base()
        });
        assert!(out.contains("Egress:    UNKNOWN"));
        assert!(out.contains("⚠ Egress was not checked"));
    }

    #[test]
    fn names_failed_startup_steps() {
        let out = text(&BannerInput {
            degraded: vec!["prepare-desktops".to_string(), "prepare-backup".to_string()],
            ..base()
        });
        assert!(out.contains("prepare-desktops, prepare-backup"));
    }
}
