//! Fail-closed egress firewall (nftables). Pure ruleset generation; the apply lives in infra.
//!
//! Default-DROP on the OUTPUT hook. Only these things may leave the box: loopback (so every app
//! reaches Clash on 127.0.0.1 for the proxy ports and DNS); the Clash process itself, matched by
//! its uid (so it can reach the subscription nodes and its own DoH resolver); replies on
//! already-established connections (so inbound services like Caddy can answer); and the networks
//! the operator listed in `EGRESS_ALLOW_CIDRS`, if any. Everything else is dropped, so a process
//! that ignores the proxy, speaks UDP/QUIC, or runs under `noproxy` can no longer egress directly,
//! and DNS cannot leak past Clash.
//!
//! This is the ENFORCED counterpart to the best-effort proxy env vars. It is on by default
//! (`EGRESS_ENFORCE=0` turns it off). It needs CAP_NET_ADMIN and a Clash that runs as its own uid;
//! when it is wanted but cannot be applied, the boot step fails closed rather than booting leaky.

use crate::egress::allowlist::AllowList;
use crate::egress::layout::CLASH_HTTP_PORT;

/// nftables table name. The `inet` family covers IPv4 and IPv6 in one table.
pub const EGRESS_TABLE: &str = "sdw_egress";

/// The full `nft -f` program. Idempotent: the add-then-delete header lets `delete table` succeed
/// even on the very first apply, so re-applying (e.g. a Clash restart) never errors.
pub fn egress_ruleset(clash_uid: u32, allow: &AllowList) -> String {
    let mut allowed = String::new();
    for block in allow.v4() {
        allowed.push_str(&format!("\t\tip daddr {block} accept\n"));
    }
    for block in allow.v6() {
        allowed.push_str(&format!("\t\tip6 daddr {block} accept\n"));
    }
    format!(
        "add table inet {t}\n\
         delete table inet {t}\n\
         table inet {t} {{\n\
         \tchain output {{\n\
         \t\ttype filter hook output priority 0; policy drop;\n\
         \t\tct state established,related accept\n\
         \t\toifname \"lo\" accept\n\
         \t\tip daddr 127.0.0.0/8 accept\n\
         \t\tip6 daddr ::1/128 accept\n\
         \t\tmeta skuid {uid} accept\n\
         {allowed}\
         \t}}\n\
         }}\n",
        t = EGRESS_TABLE,
        uid = clash_uid,
    )
}

/// `ssh` reads neither `http_proxy` nor `all_proxy`, so with the firewall on a Git remote over SSH
/// (or any `ssh host`) would run into the DROP. This client configuration sends every connection
/// through Clash's SOCKS5 listener, with the host NAME passed along so the proxy resolves it.
///
/// Loopback hosts are exempt (a local tunnel must stay local). `ssh_config` takes the FIRST value
/// it finds for an option and reads the user's own `~/.ssh/config` before this file, so a user can
/// still say `ProxyCommand none` for a host on an allowed network.
pub fn ssh_proxy_config() -> String {
    format!(
        "# Written at boot by `sdw entrypoint` while the egress firewall is on (EGRESS_ENFORCE).\n\
         # ssh ignores the proxy environment variables; send it through Clash like everything else.\n\
         Host localhost 127.* ::1\n\
         \tProxyCommand none\n\
         \n\
         Host *\n\
         \tProxyCommand nc -X 5 -x 127.0.0.1:{CLASH_HTTP_PORT} %h %p\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> AllowList {
        AllowList::default()
    }

    #[test]
    fn ruleset_is_fail_closed_and_allows_only_the_intended_paths() {
        let rs = egress_ruleset(7890, &none());
        // Default DROP on egress.
        assert!(rs.contains("hook output priority 0; policy drop"));
        // Exactly the three intended escape hatches.
        assert!(rs.contains("ct state established,related accept"));
        assert!(rs.contains("oifname \"lo\" accept"));
        assert!(rs.contains("meta skuid 7890 accept"));
        // Idempotent header so a re-apply does not fail on the first run.
        assert!(rs.starts_with("add table inet sdw_egress\ndelete table inet sdw_egress\n"));
        // Nothing that would defeat the drop policy.
        assert!(!rs.contains("policy accept"));
        // Without an allowlist there is no destination rule beyond loopback.
        let destinations: Vec<&str> = rs
            .lines()
            .filter(|l| l.contains("daddr"))
            .map(str::trim)
            .collect();
        assert_eq!(
            destinations,
            ["ip daddr 127.0.0.0/8 accept", "ip6 daddr ::1/128 accept"]
        );
    }

    #[test]
    fn the_clash_uid_is_interpolated_not_hardcoded() {
        assert!(egress_ruleset(12345, &none()).contains("meta skuid 12345 accept"));
        assert!(!egress_ruleset(12345, &none()).contains("meta skuid 7890"));
    }

    #[test]
    fn the_allowlist_adds_one_accept_per_block_inside_the_chain() {
        let allow = AllowList::parse("192.168.1.0/24, 10.0.0.5, fd00::/16").unwrap();
        let rs = egress_ruleset(7890, &allow);
        for rule in [
            "ip daddr 10.0.0.5/32 accept",
            "ip daddr 192.168.1.0/24 accept",
            "ip6 daddr fd00::/16 accept",
        ] {
            assert!(rs.contains(&format!("\t\t{rule}\n")), "{rule}");
        }
        // The rules sit inside the chain, before its closing brace, and the policy is still drop.
        let chain_end = rs.rfind("\t}\n}").expect("the chain closes");
        assert!(rs.find("ip daddr 10.0.0.5/32").unwrap() < chain_end);
        assert!(rs.contains("policy drop"));
        assert!(!rs.contains("policy accept"));
        // Braces balance: the program is one table with one chain.
        assert_eq!(rs.matches('{').count(), rs.matches('}').count());
    }

    #[test]
    fn an_empty_allowlist_and_none_at_all_give_the_same_program() {
        assert_eq!(
            egress_ruleset(7890, &none()),
            egress_ruleset(7890, &AllowList::parse("  ").unwrap())
        );
    }

    #[test]
    fn ssh_goes_through_clash_except_for_loopback() {
        let conf = ssh_proxy_config();
        assert!(conf.contains("ProxyCommand nc -X 5 -x 127.0.0.1:7890 %h %p"));
        // The loopback stanza comes first: ssh_config keeps the first value it sees.
        let local = conf.find("Host localhost 127.* ::1").unwrap();
        let all = conf.find("Host *").unwrap();
        assert!(local < all);
        assert!(conf[local..all].contains("ProxyCommand none"));
    }
}
