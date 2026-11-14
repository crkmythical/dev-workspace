#![no_main]
//! Coverage-guided fuzzing of the WebSocket origin check. `Origin` and `Host` are request headers,
//! so both are attacker-controlled; the check must never panic, and (by the proptests in
//! core::auth::request) must never call a different host "the same origin".
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    // One input, two headers: the first line is the Origin, the rest the Host.
    let (origin, host) = text.split_once('\n').unwrap_or((text, ""));
    let _ = sdw_core::auth::request::origin_matches_host(origin, host);
});
