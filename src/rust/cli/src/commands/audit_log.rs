//! `audit-log [--json] [N]` — the last N audit events (default 20), oldest first: when, what, the
//! result and who (the address the proxies claim, the User-Agent, the Access identity). The log is
//! written by the auth gateway (logins, logouts, Cloudflare Access sign-ins, self-destruct
//! attempts) and by the egress monitor (status changes); see sdw_core::audit.

use sdw_core::audit::{parse_line, AUDIT_LOG_PATH};
use sdw_infra::audit::read_recent_lines;
use std::path::Path;

pub fn run(args: &[String]) -> i32 {
    let mut json = false;
    let mut count = 20usize;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            a => match a.parse::<usize>() {
                Ok(n) if n > 0 => count = n,
                _ => {
                    eprintln!("usage: audit-log [--json] [N]");
                    return 2;
                }
            },
        }
    }
    let lines = read_recent_lines(Path::new(AUDIT_LOG_PATH), 1 << 20);
    let parsed: Vec<_> = lines
        .iter()
        .filter_map(|l| parse_line(l).map(|e| (l, e)))
        .collect();
    if parsed.is_empty() {
        println!("No audit events yet ({AUDIT_LOG_PATH}).");
        return 0;
    }
    for (raw, event) in &parsed[parsed.len().saturating_sub(count)..] {
        if json {
            println!("{raw}");
        } else {
            println!("{}", event.human());
        }
    }
    0
}
