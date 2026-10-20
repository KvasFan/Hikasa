//! P-TIME-1 (FileTime serde round trip; the text equals jiff's `Timestamp` Display wherever RFC 3339 can write the
//! instant) and P-CASE-1 (`simple_lower` is idempotent and preserves char counts), A1.3.

mod common;

use hikasa_core::case::{eq_folded, fold, simple_lower, simple_upper};
use hikasa_core::time::FileTime;
use proptest::prelude::*;

/// Unix seconds of `0000-01-01T00:00:00Z` and `9999-12-31T23:59:59Z`: the instants RFC 3339 can write.
const RFC3339_SECONDS: std::ops::RangeInclusive<i64> = -62_167_219_200..=253_402_300_799;

/// Seconds that jiff's `Timestamp` can hold and RFC 3339 can write (years 0000 to 9999-12-30T22:00:00Z).
fn jiff_rfc3339_seconds() -> std::ops::RangeInclusive<i64> {
    *RFC3339_SECONDS.start()..=jiff::Timestamp::MAX.as_second() - 1
}

/// jiff's representable second range (for the timeline comparison).
fn jiff_seconds() -> std::ops::RangeInclusive<i64> {
    jiff::Timestamp::MIN.as_second()..=jiff::Timestamp::MAX.as_second() - 1
}

fn jiff_timestamp(secs: i64, nanos: u32) -> jiff::Timestamp {
    jiff::Timestamp::new(secs, i32::try_from(nanos).unwrap()).unwrap()
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_time_case.txt"))]

    /// P-TIME-1 (every instant jiff and RFC 3339 share): serde round trip, and the text is jiff's `Timestamp`
    /// Display.
    #[test]
    fn p_time_1_round_trip_in_range(secs in jiff_rfc3339_seconds(), nanos in 0u32..1_000_000_000) {
        let t = FileTime::from_unix(secs, nanos).unwrap();
        let ts = jiff_timestamp(secs, nanos);
        prop_assert_eq!(t.to_rfc3339(), ts.to_string());
        let json = serde_json::to_string(&t).unwrap();
        prop_assert_eq!(&json, &format!("\"{ts}\""));
        prop_assert_eq!(serde_json::from_str::<FileTime>(&json).unwrap(), t);
        prop_assert_eq!(FileTime::parse(&t.to_rfc3339()).unwrap(), t);
    }

    /// P-TIME-1 (any instant): serde round trip; RFC 3339 text exactly for the years RFC 3339 can write, the
    /// `@secs.nanos` fallback otherwise.
    #[test]
    fn p_time_1_round_trip_any(
        secs in prop_oneof![any::<i64>(), RFC3339_SECONDS, -62_167_219_300i64..=-62_167_219_100, 253_402_300_700i64..=253_402_300_900],
        nanos in 0u32..1_000_000_000,
    ) {
        let t = FileTime::from_unix(secs, nanos).unwrap();
        let json = serde_json::to_string(&t).unwrap();
        prop_assert_eq!(serde_json::from_str::<FileTime>(&json).unwrap(), t);
        let text = t.to_rfc3339();
        prop_assert_eq!(FileTime::parse(&text).unwrap(), t);
        if RFC3339_SECONDS.contains(&secs) {
            prop_assert!(!text.starts_with('@') && text.ends_with('Z'), "{}", text);
            prop_assert_eq!(text.find('T'), Some(10), "4-digit year: {}", text);
        } else {
            prop_assert_eq!(text, format!("@{secs}.{nanos:09}"));
        }
    }

    /// P-TIME-1 (parsing): any offset is normalised to UTC; jiff's own RFC 3339 rendering with an offset is the
    /// oracle.
    #[test]
    fn p_time_1_offsets_normalise_to_utc(
        secs in jiff_rfc3339_seconds(),
        nanos in 0u32..1_000_000_000,
        offset_minutes in -(23 * 60 + 59)..=(23 * 60 + 59),
    ) {
        let ts = jiff_timestamp(secs, nanos);
        let offset = jiff::tz::Offset::from_seconds(offset_minutes * 60).unwrap();
        let text = ts.display_with_offset(offset).to_string();
        // Only texts with a 4-digit year are RFC 3339.
        prop_assume!(text.find('T') == Some(10));
        prop_assert_eq!(FileTime::parse(&text).unwrap(), FileTime::from_unix(secs, nanos).unwrap(), "{}", text);
    }

    /// Parsing is total: arbitrary text never panics, and accepted text re-renders to an equal instant.
    #[test]
    fn file_time_parse_is_total(text in "[-0-9:.TtZz+@ ]{0,40}", junk in any::<String>()) {
        for input in [text.as_str(), junk.as_str()] {
            if let Ok(t) = FileTime::parse(input) {
                prop_assert_eq!(FileTime::parse(&t.to_rfc3339()).unwrap(), t);
            }
        }
    }

    /// Ordering and differences agree with the jiff timeline.
    #[test]
    fn time_order_matches_nanos_until(a in (jiff_seconds(), 0u32..1_000_000_000), b in (jiff_seconds(), 0u32..1_000_000_000)) {
        let ta = FileTime::from_unix(a.0, a.1).unwrap();
        let tb = FileTime::from_unix(b.0, b.1).unwrap();
        let d = ta.nanos_until(tb);
        prop_assert_eq!(d.signum(), match ta.cmp(&tb) {
            std::cmp::Ordering::Less => 1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => -1,
        });
        let ja = jiff_timestamp(a.0, a.1);
        let jb = jiff_timestamp(b.0, b.1);
        prop_assert_eq!(d, jb.as_nanosecond() - ja.as_nanosecond());
    }

    /// P-CASE-1: `simple_lower` is idempotent and preserves char counts.
    #[test]
    fn p_case_1(s in any::<String>(), c in any::<char>()) {
        prop_assert_eq!(simple_lower(simple_lower(c)), simple_lower(c));
        let once = fold(&s);
        prop_assert_eq!(once.chars().count(), s.chars().count());
        let twice = fold(&once);
        prop_assert_eq!(twice.as_ref(), once.as_ref());
        prop_assert!(eq_folded(&s, &once));
        let upper: String = s.chars().map(simple_upper).collect();
        prop_assert_eq!(upper.chars().count(), s.chars().count());
    }
}
