//! Timestamp comparison (FC-021; algorithm §2.4; D-28, D-52).

use hikasa_core::{FileTime, Side};
use serde::{Deserialize, Serialize};

use crate::rules::TimestampRule;

/// Nanoseconds per hour.
const HOUR_NANOS: i128 = 3_600 * 1_000_000_000;

/// Largest whole number of hours tried with *ignore time zone* (D-28: covers every real UTC offset pair).
const MAX_ZONE_HOURS: i128 = 26;

/// Timestamp relation (§2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeRelation {
    /// Equal within the rule's tolerance (and DST / time-zone allowances).
    Same,
    /// The left time is later.
    LeftNewer,
    /// The right time is later.
    RightNewer,
}

impl TimeRelation {
    /// Left↔Right (`Same` stays).
    pub fn mirrored(self) -> TimeRelation {
        match self {
            TimeRelation::Same => TimeRelation::Same,
            TimeRelation::LeftNewer => TimeRelation::RightNewer,
            TimeRelation::RightNewer => TimeRelation::LeftNewer,
        }
    }
}

/// Compares two modification times under `rule` (§2.4). Only meaningful when `rule.enabled`; the caller skips
/// the criterion otherwise (and when a time is missing).
///
/// Nanosecond resolution; the tolerance is inclusive (D-28). With `ignore_timezone`, differences of
/// `k` whole hours (1 ≤ k ≤ 26) ± tolerance are `Same`; otherwise with `ignore_dst`, exactly one hour ±
/// tolerance is `Same`. `rule.offset` first shifts one side by whole hours (D-52).
pub fn compare_times(l: FileTime, r: FileTime, rule: &TimestampRule) -> TimeRelation {
    let mut d: i128 = l.nanos_until(r);
    if let Some(offset) = rule.offset {
        let shift = i128::from(offset.hours) * HOUR_NANOS;
        match offset.side {
            // l + shift: d = r - (l + shift).
            Side::Left => d -= shift,
            Side::Right => d += shift,
            Side::Center | Side::Output => {}
        }
    }
    let tolerance = i128::from(rule.tolerance_secs) * 1_000_000_000;
    let abs = d.abs();
    if abs <= tolerance {
        return TimeRelation::Same;
    }
    if rule.ignore_timezone {
        if (1..=MAX_ZONE_HOURS).any(|k| (abs - k * HOUR_NANOS).abs() <= tolerance) {
            return TimeRelation::Same;
        }
    } else if rule.ignore_dst && (abs - HOUR_NANOS).abs() <= tolerance {
        return TimeRelation::Same;
    }
    if d > 0 {
        TimeRelation::RightNewer
    } else {
        TimeRelation::LeftNewer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::TzOffset;

    fn t(s: &str) -> FileTime {
        FileTime::parse(&format!("2026-10-19T{s}Z")).unwrap()
    }

    fn rule(tol: u32, dst: bool, tz: bool) -> TimestampRule {
        TimestampRule {
            enabled: true,
            tolerance_secs: tol,
            ignore_dst: dst,
            ignore_timezone: tz,
            offset: None,
        }
    }

    /// §2.4 table, rows 1–12 (row 13, `enabled = false`, is a quick-test rule: see `quick.rs`).
    #[test]
    fn timestamp_table() {
        use TimeRelation::*;
        let noon = t("12:00:00");
        let rows: [(FileTime, TimestampRule, TimeRelation); 11] = [
            (t("12:00:02"), rule(2, false, false), Same),
            (t("12:00:02.000000001"), rule(2, false, false), RightNewer),
            (t("11:59:58"), rule(2, false, false), Same),
            (t("13:00:00"), rule(2, true, false), Same),
            (t("13:00:03"), rule(2, true, false), RightNewer),
            (t("11:00:01"), rule(2, true, false), Same),
            (t("14:00:00"), rule(2, true, false), RightNewer),
            (t("17:00:01"), rule(2, false, true), Same),
            (t("12:30:00"), rule(2, false, true), RightNewer),
            (
                FileTime::from_unix(noon.unix_seconds() + 27 * 3600, 0).unwrap(),
                rule(2, false, true),
                RightNewer,
            ),
            (t("12:00:00.5"), rule(0, false, false), RightNewer),
        ];
        for (i, (right, rule, want)) in rows.iter().enumerate() {
            assert_eq!(compare_times(noon, *right, rule), *want, "row {}", i + 1);
        }
        // Row 12: offset left+2.
        let mut shifted = rule(2, false, false);
        shifted.offset = Some(TzOffset {
            side: Side::Left,
            hours: 2,
        });
        assert_eq!(compare_times(noon, t("14:00:01"), &shifted), Same);
        // The same shift on the right side.
        shifted.offset = Some(TzOffset {
            side: Side::Right,
            hours: -2,
        });
        assert_eq!(compare_times(noon, t("14:00:01"), &shifted), Same);
        assert_eq!(
            compare_times(t("14:00:00"), noon, &rule(2, false, false)),
            LeftNewer
        );
        // k = 26 is the largest covered multiple.
        let plus26 = FileTime::from_unix(noon.unix_seconds() + 26 * 3600, 0).unwrap();
        assert_eq!(compare_times(noon, plus26, &rule(2, false, true)), Same);
    }

    #[test]
    fn extremes_do_not_overflow() {
        let min = FileTime::from_unix(i64::MIN, 0).unwrap();
        let max = FileTime::from_unix(i64::MAX, 999_999_999).unwrap();
        let mut r = rule(u32::MAX, true, true);
        r.offset = Some(TzOffset {
            side: Side::Left,
            hours: -12,
        });
        assert_eq!(compare_times(min, max, &r), TimeRelation::RightNewer);
        assert_eq!(compare_times(max, min, &r), TimeRelation::LeftNewer);
    }
}
