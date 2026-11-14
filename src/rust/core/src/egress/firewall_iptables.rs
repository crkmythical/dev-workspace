//! Fail-closed egress firewall — iptables-legacy backend. Pure ruleset generation; the apply lives
//! in infra. This is the FALLBACK for kernels whose nftables is non-functional even though they
//! have netfilter: notably Android's AVF (pKVM) Terminal guest kernel, where `nft` returns
//! "Invalid argument" but the legacy x_tables path (`iptables-legacy`) works. nftables
//! (`firewall::egress_ruleset`) stays the default everywhere it works; this is only reached when
//! that fails.
//!
//! Same policy as the nftables table, rule for rule and in the same order: default-DROP on OUTPUT;
//! only loopback, replies on established connections, the Clash uid and the operator's allowlist
//! may leave. nftables says it in one `inet` table that covers IPv4 and IPv6 together; iptables
//! needs two programs — one for `iptables-legacy` (v4), one for `ip6tables-legacy` (v6) — each fed
//! to `{,ip6}tables-legacy-restore`, which commits the whole filter table in ONE atomic
//! transaction, so a re-apply (e.g. a Clash restart) never leaves a window where egress is open.
//!
//! The filter table's OUTPUT policy is set to DROP here; INPUT and FORWARD are left ACCEPT, matching
//! the nftables backend, which never filtered them (Caddy must still answer inbound). The backend
//! owns the filter table on the boxes it runs on (a workspace container, or the Android VM running
//! the stack natively) — nothing else manages iptables there.

use crate::egress::allowlist::AllowList;

/// An `iptables-legacy-restore` program for the filter table. `loopback` is the family's loopback
/// network (`127.0.0.0/8` for v4, `::1/128` for v6); `blocks` are the allowlist entries for that
/// family, already rendered as `addr/len`.
fn restore_program(clash_uid: u32, loopback: &str, blocks: &[String]) -> String {
    let mut out = String::from(
        "*filter\n\
         :INPUT ACCEPT [0:0]\n\
         :FORWARD ACCEPT [0:0]\n\
         :OUTPUT DROP [0:0]\n\
         -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n\
         -A OUTPUT -o lo -j ACCEPT\n",
    );
    out.push_str(&format!("-A OUTPUT -d {loopback} -j ACCEPT\n"));
    out.push_str(&format!(
        "-A OUTPUT -m owner --uid-owner {clash_uid} -j ACCEPT\n"
    ));
    for block in blocks {
        out.push_str(&format!("-A OUTPUT -d {block} -j ACCEPT\n"));
    }
    out.push_str("COMMIT\n");
    out
}

/// The IPv4 program, for `iptables-legacy-restore`.
pub fn egress_rules_v4(clash_uid: u32, allow: &AllowList) -> String {
    restore_program(clash_uid, "127.0.0.0/8", &allow.v4())
}

/// The IPv6 program, for `ip6tables-legacy-restore`.
pub fn egress_rules_v6(clash_uid: u32, allow: &AllowList) -> String {
    restore_program(clash_uid, "::1/128", &allow.v6())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> AllowList {
        AllowList::default()
    }

    // The exact bytes, pinned. The real-kernel test feeds these same programs to
    // iptables-legacy-restore; asserting the whole string here means the thing proven to load and
    // enforce on a real kernel is byte-for-byte what the code emits.
    #[test]
    fn v4_program_is_exactly_this() {
        assert_eq!(
            egress_rules_v4(7890, &none()),
            "*filter\n\
             :INPUT ACCEPT [0:0]\n\
             :FORWARD ACCEPT [0:0]\n\
             :OUTPUT DROP [0:0]\n\
             -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n\
             -A OUTPUT -o lo -j ACCEPT\n\
             -A OUTPUT -d 127.0.0.0/8 -j ACCEPT\n\
             -A OUTPUT -m owner --uid-owner 7890 -j ACCEPT\n\
             COMMIT\n"
        );
    }

    #[test]
    fn v6_program_is_exactly_this() {
        assert_eq!(
            egress_rules_v6(7890, &none()),
            "*filter\n\
             :INPUT ACCEPT [0:0]\n\
             :FORWARD ACCEPT [0:0]\n\
             :OUTPUT DROP [0:0]\n\
             -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT\n\
             -A OUTPUT -o lo -j ACCEPT\n\
             -A OUTPUT -d ::1/128 -j ACCEPT\n\
             -A OUTPUT -m owner --uid-owner 7890 -j ACCEPT\n\
             COMMIT\n"
        );
    }

    #[test]
    fn output_policy_is_drop_and_never_accept() {
        for program in [egress_rules_v4(7890, &none()), egress_rules_v6(7890, &none())] {
            assert!(program.contains(":OUTPUT DROP [0:0]"));
            assert!(!program.contains(":OUTPUT ACCEPT"));
            // INPUT/FORWARD stay open, like the nftables backend.
            assert!(program.contains(":INPUT ACCEPT [0:0]"));
            assert!(program.contains(":FORWARD ACCEPT [0:0]"));
        }
    }

    #[test]
    fn the_four_fixed_accepts_are_present_in_order() {
        let p = egress_rules_v4(7890, &none());
        let established = p.find("--ctstate ESTABLISHED,RELATED -j ACCEPT").unwrap();
        let lo = p.find("-A OUTPUT -o lo -j ACCEPT").unwrap();
        let loopback = p.find("-d 127.0.0.0/8 -j ACCEPT").unwrap();
        let owner = p.find("--uid-owner 7890 -j ACCEPT").unwrap();
        assert!(established < lo && lo < loopback && loopback < owner);
    }

    #[test]
    fn the_clash_uid_is_interpolated_not_hardcoded() {
        assert!(egress_rules_v4(12345, &none()).contains("--uid-owner 12345 -j ACCEPT"));
        assert!(!egress_rules_v4(12345, &none()).contains("--uid-owner 7890"));
    }

    #[test]
    fn the_allowlist_adds_one_accept_per_block_in_the_right_family() {
        let allow = AllowList::parse("192.168.1.0/24, 10.0.0.5, fd00::/16").unwrap();
        let v4 = egress_rules_v4(7890, &allow);
        let v6 = egress_rules_v6(7890, &allow);
        // v4 blocks only in the v4 program, v6 only in the v6 program.
        assert!(v4.contains("-A OUTPUT -d 10.0.0.5/32 -j ACCEPT"));
        assert!(v4.contains("-A OUTPUT -d 192.168.1.0/24 -j ACCEPT"));
        assert!(!v4.contains("fd00::/16"));
        assert!(v6.contains("-A OUTPUT -d fd00::/16 -j ACCEPT"));
        assert!(!v6.contains("10.0.0.5"));
        // Allowlist accepts sit before COMMIT, so the DROP policy still catches everything else.
        assert!(v4.find("10.0.0.5/32").unwrap() < v4.find("COMMIT").unwrap());
    }

    #[test]
    fn an_empty_allowlist_and_none_at_all_give_the_same_program() {
        assert_eq!(
            egress_rules_v4(7890, &none()),
            egress_rules_v4(7890, &AllowList::parse("  ").unwrap())
        );
        assert_eq!(
            egress_rules_v6(7890, &none()),
            egress_rules_v6(7890, &AllowList::parse("  ").unwrap())
        );
    }
}
