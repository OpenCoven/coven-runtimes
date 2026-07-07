//! Dependency-free ISO-8601 (UTC) timestamps.
//!
//! `conjure registry build` stamps `published_at` on a newly published
//! `(runtime, version)` pair. That is the only place we need the wall clock, and
//! it is a single formatted string — not worth a `chrono`/`time` dependency (the
//! same reasoning that keeps [`crate::sha256`] in-tree). The civil-date
//! conversion is Howard Hinnant's `civil_from_days` algorithm, valid for all
//! dates in the proleptic Gregorian calendar.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current UTC time as an ISO-8601 `YYYY-MM-DDTHH:MM:SSZ` string.
///
/// Falls back to the Unix epoch if the system clock is set before 1970 (which
/// only makes `published_at` slightly wrong, never panics).
pub fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso8601_utc(secs)
}

/// Format seconds-since-Unix-epoch as an ISO-8601 UTC timestamp.
pub fn iso8601_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let (hour, min, sec) = (tod / 3_600, (tod % 3_600) / 60, tod % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Convert a count of days since 1970-01-01 to a `(year, month, day)` civil date.
/// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms".
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_unix_zero() {
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_timestamp_matches() {
        // 2026-07-06T00:00:00Z — the seed date used in the registry index.
        assert_eq!(iso8601_utc(1_783_296_000), "2026-07-06T00:00:00Z");
    }

    #[test]
    fn time_of_day_is_formatted() {
        // 2000-01-01T23:59:59Z
        assert_eq!(iso8601_utc(946_771_199), "2000-01-01T23:59:59Z");
    }

    #[test]
    fn now_is_well_formed() {
        let s = now_iso8601();
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
    }
}
