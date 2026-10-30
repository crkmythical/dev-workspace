//! Pure builders for the croc command line — a port of packages/core/src/transfer/croc.ts. No
//! environment, no spawning: the cli `push` / `pull` adapters read the environment and run `croc`
//! with these.
//!
//! croc is end-to-end encrypted (PAKE from the code phrase, then AES-256-GCM); the relay only ever
//! sees ciphertext. Global flags (--relay, --socks5) come before the subcommand.
//!
//! Neither secret is ever an argument: croc reads the relay password from CROC_PASS and the code
//! phrase from CROC_SECRET (croc v11 refuses `send --code` on the command line for exactly this
//! reason). The cli adapters put them in the child's environment; these builders produce only the
//! non-secret argv.

use crate::transfer::layout::CLASH_SOCKS_ADDR;

/// How the relay connection is proxied. The TS side modelled this as `string | null | undefined`;
/// the three cases are named here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Socks {
    /// Default (TS `undefined`): route through Clash (CLASH_SOCKS_ADDR), for a remote relay.
    #[default]
    Clash,
    /// Go direct with no proxy (TS `null`), for a LAN/reachable relay.
    Direct,
    /// An explicit SOCKS5 proxy, host:port.
    Proxy(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOptions {
    /// Self-hosted relay, host:port.
    pub relay: String,
    pub socks: Socks,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveOptions {
    pub relay: String,
    pub socks: Socks,
    /// Directory received files are written to.
    pub out: String,
}

fn globals(relay: &str, socks: &Socks) -> Vec<String> {
    let mut args = vec!["--relay".to_string(), relay.to_string()];
    let proxy = match socks {
        Socks::Clash => Some(CLASH_SOCKS_ADDR),
        Socks::Direct => None,
        Socks::Proxy(p) => Some(p.as_str()),
    }
    // An empty proxy is falsy on the TS side; treat it as "no proxy".
    .filter(|p| !p.is_empty());
    if let Some(p) = proxy {
        args.push("--socks5".to_string());
        args.push(p.to_string());
    }
    args
}

pub fn build_croc_send(o: &SendOptions) -> Vec<String> {
    let mut args = globals(&o.relay, &o.socks);
    args.push("send".to_string());
    args.extend(o.paths.iter().cloned());
    args
}

pub fn build_croc_receive(o: &ReceiveOptions) -> Vec<String> {
    let mut args = globals(&o.relay, &o.socks);
    // --yes: no interactive "accept?" prompt (push/pull run scripted). The code comes from
    // CROC_SECRET, so there is no positional argument here.
    args.push("--yes".to_string());
    args.push("--out".to_string());
    args.push(o.out.clone());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_routes_through_clash_by_default_and_never_passes_the_code() {
        let args = build_croc_send(&SendOptions {
            relay: "r.example:9009".to_string(),
            socks: Socks::Clash,
            paths: vec!["a.txt".to_string()],
        });
        assert_eq!(
            args,
            [
                "--relay",
                "r.example:9009",
                "--socks5",
                CLASH_SOCKS_ADDR,
                "send",
                "a.txt"
            ]
        );
        assert!(!args.iter().any(|a| a == "--code"));
    }

    #[test]
    fn send_goes_direct_for_a_lan_relay() {
        let args = build_croc_send(&SendOptions {
            relay: "10.0.0.5:9009".to_string(),
            socks: Socks::Direct,
            paths: vec!["a".to_string(), "b".to_string()],
        });
        assert_eq!(args, ["--relay", "10.0.0.5:9009", "send", "a", "b"]);
    }

    #[test]
    fn receive_auto_accepts_into_a_dir_through_clash() {
        let args = build_croc_receive(&ReceiveOptions {
            relay: "r.example:9009".to_string(),
            socks: Socks::Clash,
            out: "/workspace".to_string(),
        });
        assert_eq!(
            args,
            [
                "--relay",
                "r.example:9009",
                "--socks5",
                CLASH_SOCKS_ADDR,
                "--yes",
                "--out",
                "/workspace"
            ]
        );
    }

    #[test]
    fn receive_direct_omits_the_proxy() {
        let args = build_croc_receive(&ReceiveOptions {
            relay: "10.0.0.5:9009".to_string(),
            socks: Socks::Direct,
            out: "/workspace".to_string(),
        });
        assert!(!args.iter().any(|a| a == "--socks5"));
        assert_eq!(
            args,
            ["--relay", "10.0.0.5:9009", "--yes", "--out", "/workspace"]
        );
    }
}
