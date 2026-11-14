//! `sdw proxy-only` — the standalone Clash client (deploy/client): the workspace's Clash, template
//! and renderer, and none of the rest. One implementation of "turn a subscription into a local
//! proxy" instead of a second, hand-written config that could (and did) drift from it.
//!
//!   sdw proxy-only            render the config and run Clash in the foreground
//!   sdw proxy-only --check    exit 0 when Clash accepts connections and has loaded nodes (the
//!                             container HEALTHCHECK; run it as the clash user, who owns the config)

use crate::commands::system_env;
use std::os::unix::process::CommandExt;
use std::process::Command;

use sdw_core::egress::layout::{CLASH_HTTP_PORT, CLASH_USER};
use sdw_core::workspace::config::load_config;
use sdw_infra::clash::{write_config, ConfigOptions, SecretPolicy};
use sdw_infra::egress::{clash_command, port_accepts, prepare_clash_dir, provider_node_count};
use sdw_infra::workload::is_root;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => serve(),
        Some("--check") => check(),
        Some(_) => {
            eprintln!("usage: sdw proxy-only [--check]");
            2
        }
    }
}

/// Started as root (the container's user): hand `/etc/clash` to the clash user and run again AS that
/// user. A volume from an older image, or a bind mount, may be root's, and root without
/// `CAP_DAC_OVERRIDE` cannot write into a directory somebody else owns; this way the config, which
/// holds the controller secret, is created by the user that reads it, and root's capabilities are
/// gone before the subscription is fetched or parsed. Returns only on failure. `None`: carry on
/// as the current user (not root, or an image without the clash user, which `prepare_clash_dir`
/// has already warned about).
fn become_clash_user() -> Option<i32> {
    if !is_root() || prepare_clash_dir().is_none() {
        return None;
    }
    let exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("proxy-only: cannot find its own executable: {e}");
            return Some(1);
        }
    };
    let err = Command::new("setpriv")
        .args([
            &format!("--reuid={CLASH_USER}"),
            &format!("--regid={CLASH_USER}"),
            "--clear-groups",
        ])
        .arg(exe)
        .arg("proxy-only")
        .exec();
    eprintln!("proxy-only: cannot become the {CLASH_USER} user: {err}");
    Some(1)
}

fn serve() -> i32 {
    let config = load_config(system_env);
    if config.clash_subscription_url.is_empty() {
        eprintln!("proxy-only: CLASH_SUBSCRIPTION_URL is not set.");
        return 2;
    }
    if let Some(code) = become_clash_user() {
        return code;
    }
    let (dns, source) = crate::commands::clash_dns::resolve_dns();
    if let Err(e) = write_config(&ConfigOptions {
        subscription_url: &config.clash_subscription_url,
        dns_nameservers: &dns,
        // The container publishes the proxy ports on the host, and a published port reaches the
        // container on its network interface, not on loopback.
        allow_lan: true,
        secret: SecretPolicy::Rotate,
    }) {
        eprintln!("proxy-only: {e}");
        return 1;
    }
    println!("proxy-only: config rendered (DNS: {dns} [{source}]); starting Clash.");
    // exec: Clash replaces this process, so Docker's stop signal and its restart policy act on it
    // directly. There is no supervisor here to keep alive. It is already the clash user (or, in an
    // image without one, root), so there is no `setpriv` to put in front of it.
    let err = clash_command(false).exec();
    eprintln!("proxy-only: cannot start Clash: {err}");
    1
}

fn check() -> i32 {
    if !port_accepts(CLASH_HTTP_PORT) {
        eprintln!("proxy-only: nothing accepts connections on :{CLASH_HTTP_PORT}");
        return 1;
    }
    match provider_node_count() {
        Some(n) if n > 0 => {
            println!("ok: {n} nodes loaded");
            0
        }
        Some(_) => {
            eprintln!("proxy-only: the subscription has loaded no nodes");
            1
        }
        None => {
            eprintln!("proxy-only: the controller did not answer (is this run as the clash user?)");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_argument_is_a_usage_error() {
        assert_eq!(run(&["--bogus".to_string()]), 2);
    }
}
