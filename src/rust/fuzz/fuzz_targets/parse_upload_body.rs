#![no_main]
//! Coverage-guided fuzzing of the UNAUTHENTICATED upload-body parser (runs before any decrypt, so
//! it sees attacker-controlled bytes straight off the wire). Companion to the proptest campaign in
//! core::sync::wire; this must never panic on any input.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = sdw_core::sync::wire::parse_upload_body(data);
});
