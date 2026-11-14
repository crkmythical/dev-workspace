#![no_main]
//! Coverage-guided fuzzing of the session-cookie verifier. The cookie value is attacker-controlled
//! text that reaches `verify_session` on every request to a gated route, before any login; it must
//! never panic, and (checked by the proptests in core::auth::session) must never accept a token
//! the key did not sign.
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    if let Ok(token) = std::str::from_utf8(data) {
        let _ = sdw_core::auth::session::verify_session(b"fuzz-session-key", token, 1_700_000_000);
    }
});
