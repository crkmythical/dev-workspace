//! Wall-clock timestamp for the destroy marker and logs. Shells out to `date -u` — the one ISO
//! need in the whole crate — so no date library is pulled into the static binary.

use crate::cmd;

/// An ISO-8601 UTC timestamp (second precision). Empty only if `date` is somehow absent; the
/// completion marker's value is cosmetic (its existence is what the entrypoint checks).
pub fn now_iso() -> String {
    let (ok, out) = cmd::capture("date", &["-u", "+%Y-%m-%dT%H:%M:%SZ"]);
    if ok {
        out.trim().to_string()
    } else {
        String::new()
    }
}
