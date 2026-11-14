#![no_main]
//! Coverage-guided fuzzing of the egress allowlist parser (EGRESS_ALLOW_CIDRS). The value is the
//! operator's, but what it produces is firewall rules: it must never panic, and an accepted list
//! must only ever render networks narrower than the limits (a /8 for IPv4, a /16 for IPv6).
use libfuzzer_sys::fuzz_target;
use sdw_core::egress::allowlist::{AllowList, MIN_V4_PREFIX, MIN_V6_PREFIX};

fuzz_target!(|data: &[u8]| {
    let Ok(raw) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(list) = AllowList::parse(raw) else {
        return;
    };
    for block in list.v4() {
        let (_, len) = block.split_once('/').expect("a rendered block has a prefix");
        assert!(len.parse::<u8>().unwrap() >= MIN_V4_PREFIX, "{block}");
    }
    for block in list.v6() {
        let (_, len) = block.split_once('/').expect("a rendered block has a prefix");
        assert!(len.parse::<u8>().unwrap() >= MIN_V6_PREFIX, "{block}");
    }
});
