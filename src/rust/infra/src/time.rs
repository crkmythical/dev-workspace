//! Wall-clock timestamp for the destroy marker and logs. Shells out to `date -u` — the one ISO
//! need in the whole crate — so no date library is pulled into the static binary.

use crate::cmd;
use std::time::{SystemTime, UNIX_EPOCH};

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

/// Milliseconds since the Unix epoch (the equivalent of JS `Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 for a civil (proleptic Gregorian) date — Howard Hinnant's algorithm.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parse an ISO-8601 UTC timestamp ("YYYY-MM-DDTHH:MM:SS[.fff][Z]") to epoch millis. The equivalent
/// of JS `new Date(ts).getTime()` for the timestamps this project writes (always UTC). None on a
/// value it does not recognise.
pub fn parse_iso_to_ms(s: &str) -> Option<i64> {
    let (date, time) = s.trim().split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    let time = time.trim_end_matches('Z').split('.').next()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;

    let days = days_from_civil(year, month, day);
    Some((days * 86_400 + hour * 3_600 + minute * 60 + second) * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_timestamps_to_epoch_millis() {
        assert_eq!(parse_iso_to_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_to_ms("2000-01-01T00:00:00Z"),
            Some(946_684_800_000)
        );
        assert_eq!(
            parse_iso_to_ms("2021-01-01T00:00:00Z"),
            Some(1_609_459_200_000)
        );
        // Fractional seconds (JS toISOString) are dropped, not an error.
        assert_eq!(
            parse_iso_to_ms("2021-01-01T00:00:00.123Z"),
            Some(1_609_459_200_000)
        );
    }

    #[test]
    fn rejects_nonsense() {
        assert_eq!(parse_iso_to_ms(""), None);
        assert_eq!(parse_iso_to_ms("not-a-date"), None);
        assert_eq!(parse_iso_to_ms("2021-01-01"), None);
    }
}
