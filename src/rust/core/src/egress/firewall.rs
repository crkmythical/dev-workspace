//! Fail-closed egress firewall (nftables). Pure ruleset generation; the apply lives in infra.
//!
//! Default-DROP on the OUTPUT hook. Only three things may leave the box: loopback (so every app
//! reaches Clash on 127.0.0.1 for the proxy ports and DNS); the Clash process itself, matched by
//! its uid (so it can reach the subscription nodes and its own DoH resolver); and replies on
//! already-established connections (so inbound services like Caddy can answer). Everything else is
//! dropped, so a process that ignores the proxy, speaks UDP/QUIC, or runs under `noproxy` can no
//! longer egress directly, and DNS cannot leak past Clash.
//!
//! This is the ENFORCED counterpart to the best-effort proxy env vars. It needs CAP_NET_ADMIN and
//! a Clash that runs as its own uid, so it is opt-in via EGRESS_ENFORCE; when asked for but not
//! applicable (no NET_ADMIN), the boot step fails closed rather than booting leaky.

/// nftables table name. The `inet` family covers IPv4 and IPv6 in one table.
pub const EGRESS_TABLE: &str = "sdw_egress";

/// The full `nft -f` program. Idempotent: the add-then-delete header lets `delete table` succeed
/// even on the very first apply, so re-applying (e.g. a Clash restart) never errors.
pub fn egress_ruleset(clash_uid: u32) -> String {
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
         \t}}\n\
         }}\n",
        t = EGRESS_TABLE,
        uid = clash_uid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruleset_is_fail_closed_and_allows_only_the_intended_paths() {
        let rs = egress_ruleset(7890);
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
    }

    #[test]
    fn the_clash_uid_is_interpolated_not_hardcoded() {
        assert!(egress_ruleset(12345).contains("meta skuid 12345 accept"));
        assert!(!egress_ruleset(12345).contains("meta skuid 7890"));
    }
}
