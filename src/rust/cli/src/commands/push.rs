//! push — send files out of the workspace with croc (end-to-end encrypted). A port of
//! packages/cli/src/push.ts.
//!
//!   push [--direct] [--code CODE] <path> [path...]
//!
//! Through the self-hosted relay in CROC_RELAY, by default via Clash (--socks5) so the relay sees
//! only ciphertext; --direct is for a relay on the LAN. Secrets go in the environment, never on
//! the command line: CROC_PASS (relay) and, for a chosen phrase, CROC_SECRET (croc v11 refuses
//! `send --code`).

use sdw_core::transfer::{build_croc_send, SendOptions, Socks};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
struct PushArgs {
    direct: bool,
    code: Option<String>,
    paths: Vec<String>,
}

fn parse(args: &[String]) -> PushArgs {
    let mut out = PushArgs {
        direct: false,
        code: None,
        paths: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--direct" => out.direct = true,
            "--code" => {
                i += 1;
                out.code = args.get(i).cloned();
            }
            a => out.paths.push(a.to_string()),
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
    if parsed.paths.is_empty() {
        eprintln!("Usage: push [--direct] [--code CODE] <path> [path...]");
        return 1;
    }

    let argv = build_croc_send(&SendOptions {
        relay,
        socks: if parsed.direct {
            Socks::Direct
        } else {
            Socks::Clash
        },
        paths: parsed.paths,
    });

    let mut cmd = Command::new("croc");
    cmd.args(&argv);
    // CROC_RELAY passes through the inherited environment; CROC_PASS may live in a file
    // (CROC_PASS_FILE) and is handed over explicitly. The code phrase (if chosen) goes in
    // CROC_SECRET, never on argv.
    sdw_infra::secrets::pass_secrets(&mut cmd, &["CROC_PASS"]);
    if let Some(code) = parsed.code {
        cmd.env("CROC_SECRET", code);
    }
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
    fn parses_flags_code_and_paths() {
        let p = parse(&[
            "--direct".into(),
            "a.txt".into(),
            "--code".into(),
            "secret".into(),
            "b.txt".into(),
        ]);
        assert_eq!(
            p,
            PushArgs {
                direct: true,
                code: Some("secret".to_string()),
                paths: vec!["a.txt".to_string(), "b.txt".to_string()],
            }
        );
    }

    #[test]
    fn defaults_to_clash_no_code_no_paths() {
        assert_eq!(
            parse(&[]),
            PushArgs {
                direct: false,
                code: None,
                paths: vec![],
            }
        );
    }

    #[test]
    fn a_trailing_code_flag_without_a_value_is_none() {
        let p = parse(&["x".into(), "--code".into()]);
        assert_eq!(p.code, None);
        assert_eq!(p.paths, vec!["x".to_string()]);
    }
}
