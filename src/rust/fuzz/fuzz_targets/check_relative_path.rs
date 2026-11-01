#![no_main]
//! Coverage-guided fuzzing of the vault path-traversal guard. Companion to the proptest campaign
//! in core::sync::path_safety; this must never panic, and (checked by that proptest) must never
//! accept an unsafe path.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = sdw_core::sync::path_safety::check_relative_path(s);
    }
});
