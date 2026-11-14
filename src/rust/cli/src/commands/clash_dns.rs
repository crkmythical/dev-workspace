//! `set-clash-dns` — view or change the resolvers Clash uses, at runtime, from the code-server /
//! VNC terminal. No rebuild, no .env edit, no container restart.
//!
//!   set-clash-dns                         show the DNS in effect and where it came from
//!   set-clash-dns 223.5.5.5,119.29.29.29  set a runtime override (persists), re-render, reload
//!   set-clash-dns --clear                 drop the override, fall back to .env / auto-detect
//!
//! The override is a file on the clash-config volume, so it outlives restarts and recreation. The
//! effective value follows egress::domain::choose_clash_dns: override > CLASH_DNS env > auto-detect.
//! `resolve_dns` is shared with the boot render (entrypoint render-clash-config) and `setup`.

use crate::commands::system_env;
use sdw_core::egress::domain::choose_clash_dns;
use sdw_core::egress::layout::CLASH_DNS_OVERRIDE_PATH;
use sdw_core::workspace::config::load_config;
use sdw_infra::clash::{write_config, ConfigOptions, SecretPolicy};
use sdw_infra::egress::{foreign_dns_reachable, reload_clash};
use std::fs;

/// The nameservers Clash should use right now and a short label for where the choice came from.
/// The boot probe (`foreign_dns_reachable`) runs ONLY when neither an override file nor CLASH_DNS
/// is set, so a configured workspace never pays for it.
pub fn resolve_dns() -> (String, String) {
    let config = load_config(system_env);
    let override_raw = fs::read_to_string(CLASH_DNS_OVERRIDE_PATH).ok();
    let override_value = override_raw.as_deref();
    let env_value = config.clash_dns.as_deref();

    let set = |o: Option<&str>| o.map(|s| !s.trim().is_empty()).unwrap_or(false);
    let (reachable, source) = if set(override_value) {
        (true, "runtime override (set-clash-dns)".to_string())
    } else if set(env_value) {
        (true, "CLASH_DNS from .env".to_string())
    } else if foreign_dns_reachable() {
        (true, "auto-detect: global DoH reachable".to_string())
    } else {
        (
            false,
            "auto-detect: foreign DNS blocked -> domestic".to_string(),
        )
    };
    let dns = choose_clash_dns(override_value, env_value, reachable).to_string();
    (dns, source)
}

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            let (dns, source) = resolve_dns();
            println!("Clash DNS in effect: {dns}");
            println!("  source: {source}");
            println!();
            println!("Change it (persists across restarts, no rebuild):");
            println!("  set-clash-dns 223.5.5.5,119.29.29.29   # mainland China");
            println!("  set-clash-dns --clear                  # back to .env / auto-detect");
            0
        }
        Some("--clear") => {
            match fs::remove_file(CLASH_DNS_OVERRIDE_PATH) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    eprintln!("set-clash-dns: cannot clear override: {e}");
                    return 1;
                }
            }
            apply("cleared runtime DNS override")
        }
        Some(flag) if flag.starts_with('-') => {
            eprintln!("usage: set-clash-dns [<ns1,ns2,...> | --clear]");
            2
        }
        Some(csv) => {
            let cleaned = normalize_csv(csv);
            if cleaned.is_empty() {
                eprintln!("set-clash-dns: give at least one resolver, e.g. 223.5.5.5,119.29.29.29");
                return 2;
            }
            if let Err(e) = fs::write(CLASH_DNS_OVERRIDE_PATH, format!("{cleaned}\n")) {
                eprintln!("set-clash-dns: cannot write {CLASH_DNS_OVERRIDE_PATH}: {e}");
                return 1;
            }
            apply(&format!("DNS override set -> {cleaned}"))
        }
    }
}

/// Re-render config.yaml with the now-effective DNS and hot-reload Clash. Shared by set and clear.
fn apply(msg: &str) -> i32 {
    let config = load_config(system_env);
    if config.clash_subscription_url.is_empty() {
        eprintln!("set-clash-dns: CLASH_SUBSCRIPTION_URL is not set; nothing to render.");
        return 1;
    }
    let (dns, source) = resolve_dns();
    if let Err(e) = write_config(&ConfigOptions {
        subscription_url: &config.clash_subscription_url,
        dns_nameservers: &dns,
        allow_lan: false,
        // Clash is running and holds the current secret; a new one in the file would leave the
        // controller and the file disagreeing until the next restart.
        secret: SecretPolicy::Keep,
    }) {
        eprintln!("set-clash-dns: {e}");
        return 1;
    }
    println!("{msg}");
    println!("effective DNS: {dns}  [{source}]");
    if reload_clash() {
        println!("\u{2713} Clash reloaded — new DNS is live.");
    } else {
        println!("\u{26a0} Clash not reachable to hot-reload; it applies on the next start.");
    }
    0
}

/// Trim entries and drop blanks, keeping comma separation. The renderer escapes each entry, so
/// this is tidy-up, not a security boundary.
fn normalize_csv(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_and_drops_blanks() {
        assert_eq!(
            normalize_csv("223.5.5.5, 119.29.29.29"),
            "223.5.5.5,119.29.29.29"
        );
        assert_eq!(normalize_csv("  , 1.1.1.1 ,, "), "1.1.1.1");
        assert_eq!(normalize_csv("   "), "");
        assert_eq!(normalize_csv(""), "");
    }
}
