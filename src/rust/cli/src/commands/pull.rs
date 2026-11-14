//! pull — receive files into the workspace with croc. A port of packages/cli/src/pull.ts.
//!
//!   pull [--direct] [--out DIR] <code>
//!
//! <code> is the phrase `push` printed on the other side. Files land in DIR (default: the vault
//! mount). Same relay and proxy rules as push; the code phrase goes in CROC_SECRET, not on argv.

use sdw_core::transfer::{build_croc_receive, ReceiveOptions, Socks, TRANSFER_DEFAULT_OUT};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
struct PullArgs {
    direct: bool,
    out: String,
    code: Option<String>,
}

fn parse(args: &[String]) -> PullArgs {
    let mut out = PullArgs {
        direct: false,
        out: TRANSFER_DEFAULT_OUT.to_string(),
        code: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--direct" => out.direct = true,
            "--out" => {
                i += 1;
                if let Some(dir) = args.get(i) {
                    out.out = dir.clone();
                }
            }
            a => out.code = Some(a.to_string()),
        }
        i += 1;
    }
    out
}

pub fn run(args: &[String]) -> i32 {
    let relay = match std::env::var("CROC_RELAY") {
        Ok(r) if !r.is_empty() => r,
        _ => {
            eprintln!(
                "File transfer is off. Set CROC_RELAY (and CROC_PASS) to a self-hosted croc relay."
            );
            return 1;
        }
    };
    let parsed = parse(args);
    let code = match parsed.code {
        Some(c) => c,
        None => {
            eprintln!("Usage: pull [--direct] [--out DIR] <code>");
            return 1;
        }
    };

    let argv = build_croc_receive(&ReceiveOptions {
        relay,
        socks: if parsed.direct {
            Socks::Direct
        } else {
            Socks::Clash
        },
        out: parsed.out,
    });

    let mut cmd = Command::new("croc");
    cmd.args(&argv);
    sdw_infra::secrets::pass_secrets(&mut cmd, &["CROC_PASS"]);
    cmd.env("CROC_SECRET", code);
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("could not run croc: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_direct_out_and_code() {
        let p = parse(&[
            "--direct".into(),
            "--out".into(),
            "/tmp/x".into(),
            "secret-code".into(),
        ]);
        assert_eq!(
            p,
            PullArgs {
                direct: true,
                out: "/tmp/x".to_string(),
                code: Some("secret-code".to_string()),
            }
        );
    }

    #[test]
    fn out_defaults_to_the_vault_mount_and_code_is_required() {
        let p = parse(&["the-code".into()]);
        assert!(!p.direct);
        assert_eq!(p.out, TRANSFER_DEFAULT_OUT);
        assert_eq!(p.code.as_deref(), Some("the-code"));
    }

    #[test]
    fn the_last_positional_wins_as_the_code() {
        let p = parse(&["first".into(), "second".into()]);
        assert_eq!(p.code.as_deref(), Some("second"));
    }
}
