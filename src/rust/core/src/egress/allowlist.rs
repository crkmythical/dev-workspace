//! `EGRESS_ALLOW_CIDRS`: the networks the workspace may reach directly, without going through Clash,
//! while the fail-closed egress firewall is on. Pure parsing and rendering; the rules built from it
//! are in `firewall`.
//!
//! The list is the one deliberate hole in a default-DROP filter, so it is strict:
//!
//! * only literal IPv4 / IPv6 addresses and CIDR blocks (a name would need a lookup that the
//!   firewall forbids);
//! * nothing as wide as "the internet": a prefix of /0, and anything shorter than /8 (IPv4) or /16
//!   (IPv6), is refused;
//! * a malformed entry is an error that stops the boot, not a line that is skipped, so a typo can
//!   neither leave a host unreachable without a word nor open a different block than meant.

use std::net::{Ipv4Addr, Ipv6Addr};

/// The shortest IPv4 prefix accepted: a /8 is the size of one private block (10.0.0.0/8).
pub const MIN_V4_PREFIX: u8 = 8;
/// The shortest IPv6 prefix accepted.
pub const MIN_V6_PREFIX: u8 = 16;
/// The rules are a linear chain evaluated for every packet that leaves; keep it short.
pub const MAX_ENTRIES: usize = 256;

/// One network, with the host bits already cleared (`10.1.2.3/8` is `10.0.0.0/8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cidr {
    V4(Ipv4Addr, u8),
    V6(Ipv6Addr, u8),
}

impl Cidr {
    /// `a.b.c.d/len`, as nftables takes it.
    pub fn render(&self) -> String {
        match self {
            Cidr::V4(addr, len) => format!("{addr}/{len}"),
            Cidr::V6(addr, len) => format!("{addr}/{len}"),
        }
    }

    /// The form for a `no_proxy` list: a single host is written as the bare address, because
    /// `10.0.0.5/32` is understood by fewer tools than `10.0.0.5`.
    pub fn for_no_proxy(&self) -> String {
        match self {
            Cidr::V4(addr, 32) => addr.to_string(),
            Cidr::V6(addr, 128) => addr.to_string(),
            other => other.render(),
        }
    }
}

fn parse_entry(item: &str) -> Result<Cidr, String> {
    let bad = |why: &str| format!("'{item}': {why}");
    let (addr, prefix) = match item.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (item, None),
    };
    let is_v6 = addr.contains(':');
    let max: u8 = if is_v6 { 128 } else { 32 };
    let len = match prefix {
        None => max,
        Some(p) => {
            if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad("the prefix length after '/' must be a number"));
            }
            let len: u8 = p
                .parse()
                .map_err(|_| bad("the prefix length is out of range"))?;
            if len > max {
                return Err(bad(&format!("the prefix length is more than {max}")));
            }
            len
        }
    };
    if is_v6 {
        let ip: Ipv6Addr = addr
            .parse()
            .map_err(|_| bad("not an IPv6 address or a CIDR block"))?;
        if ip.to_ipv4_mapped().is_some() {
            return Err(bad("write an IPv4-mapped address as plain IPv4"));
        }
        if len < MIN_V6_PREFIX {
            return Err(bad(&format!(
                "too wide: an IPv6 block must be /{MIN_V6_PREFIX} or longer"
            )));
        }
        let mask = if len == 0 {
            0
        } else {
            u128::MAX << (128 - len)
        };
        Ok(Cidr::V6(Ipv6Addr::from(u128::from(ip) & mask), len))
    } else {
        let ip: Ipv4Addr = addr
            .parse()
            .map_err(|_| bad("not an IPv4 address or a CIDR block"))?;
        if len < MIN_V4_PREFIX {
            return Err(bad(&format!(
                "too wide: an IPv4 block must be /{MIN_V4_PREFIX} or longer"
            )));
        }
        let mask = if len == 0 { 0 } else { u32::MAX << (32 - len) };
        Ok(Cidr::V4(Ipv4Addr::from(u32::from(ip) & mask), len))
    }
}

/// The parsed `EGRESS_ALLOW_CIDRS`: sorted, without duplicates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowList {
    entries: Vec<Cidr>,
}

impl AllowList {
    /// Entries separated by commas and/or whitespace. An empty or blank value is an empty list.
    pub fn parse(raw: &str) -> Result<AllowList, String> {
        let mut entries = Vec::new();
        for item in raw
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
        {
            entries.push(parse_entry(item)?);
        }
        entries.sort();
        entries.dedup();
        if entries.len() > MAX_ENTRIES {
            return Err(format!(
                "{} entries; at most {MAX_ENTRIES} are accepted (use wider blocks)",
                entries.len()
            ));
        }
        Ok(AllowList { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The IPv4 blocks, as `a.b.c.d/len`.
    pub fn v4(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|c| matches!(c, Cidr::V4(..)))
            .map(Cidr::render)
            .collect()
    }

    /// The IPv6 blocks, as `addr/len`.
    pub fn v6(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|c| matches!(c, Cidr::V6(..)))
            .map(Cidr::render)
            .collect()
    }

    /// What to append to a `no_proxy` list so tools reach these networks directly (the firewall
    /// lets them out; a tool that still sent them to Clash would reach the proxy node's side).
    pub fn no_proxy_entries(&self) -> Vec<String> {
        self.entries.iter().map(Cidr::for_no_proxy).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> AllowList {
        AllowList::parse(raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"))
    }

    #[test]
    fn empty_and_blank_are_an_empty_list() {
        for raw in ["", "   ", ",", " , ,\n"] {
            assert!(parsed(raw).is_empty(), "{raw:?}");
        }
    }

    #[test]
    fn addresses_and_blocks_in_both_families() {
        let list = parsed("10.0.0.0/8, 192.168.1.5 fd00::/16\n2001:db8::1");
        assert_eq!(list.v4(), ["10.0.0.0/8", "192.168.1.5/32"]);
        assert_eq!(list.v6(), ["2001:db8::1/128", "fd00::/16"]);
    }

    #[test]
    fn host_bits_are_cleared_so_equal_blocks_collapse() {
        let list = parsed("10.1.2.3/8, 10.9.9.9/8, 10.0.0.0/8");
        assert_eq!(list.v4(), ["10.0.0.0/8"]);
        assert_eq!(list.len(), 1);
        assert_eq!(parsed("fd12:3456::1/16").v6(), ["fd12::/16"]);
        assert_eq!(parsed("192.168.1.77/24").v4(), ["192.168.1.0/24"]);
    }

    #[test]
    fn the_whole_internet_is_refused_however_it_is_written() {
        for raw in [
            "0.0.0.0/0",
            "0.0.0.0/1",
            "128.0.0.0/1",
            "10.0.0.0/7",
            "::/0",
            "::/1",
            "fc00::/15",
        ] {
            let err = AllowList::parse(raw).unwrap_err();
            assert!(err.contains("too wide"), "{raw}: {err}");
        }
        // The limits themselves are fine.
        assert!(AllowList::parse("10.0.0.0/8").is_ok());
        assert!(AllowList::parse("fd00::/16").is_ok());
    }

    #[test]
    fn malformed_entries_are_errors_that_name_the_entry() {
        for raw in [
            "example.com",
            "10.0.0",
            "10.0.0.256",
            "10.0.0.0/",
            "10.0.0.0/33",
            "10.0.0.0/+8",
            "10.0.0.0/-8",
            "10.0.0.0/8/8",
            "10.0.0.0/abc",
            "fd00::/129",
            "fd00:::1",
            "::ffff:10.0.0.1",
            "1.2.3.4:80",
        ] {
            let err = AllowList::parse(raw).unwrap_err();
            assert!(err.contains('\''), "{raw}: {err}");
        }
        // One bad entry spoils the list: nothing is skipped quietly.
        let err = AllowList::parse("10.0.0.0/8, nonsense, 192.168.0.0/16").unwrap_err();
        assert!(err.contains("nonsense"), "{err}");
    }

    #[test]
    fn the_list_has_a_ceiling() {
        let many: Vec<String> = (0..=MAX_ENTRIES)
            .map(|i| format!("10.{}.{}.0/24", i / 256, i % 256))
            .collect();
        assert!(AllowList::parse(&many.join(",")).is_err());
        assert!(AllowList::parse(&many[..MAX_ENTRIES].join(",")).is_ok());
    }

    #[test]
    fn no_proxy_uses_bare_addresses_for_single_hosts() {
        let list = parsed("10.0.0.5, 192.168.0.0/16, fd00::7, fd00::/16");
        assert_eq!(
            list.no_proxy_entries(),
            ["10.0.0.5", "192.168.0.0/16", "fd00::/16", "fd00::7"]
        );
    }

    #[test]
    fn order_does_not_matter() {
        assert_eq!(
            parsed("192.168.0.0/16,10.0.0.0/8"),
            parsed("10.0.0.0/8 192.168.0.0/16")
        );
    }
}
