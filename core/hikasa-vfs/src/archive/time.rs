//! Member times (normative, D-146; §1.5.8). The zone is passed in, from the injected clock (T1): nothing here reads
//! the system zone.

use hikasa_core::FileTime;
use jiff::civil::{Date, DateTime, Time};
use jiff::tz::TimeZone;

/// 100 ns intervals between 1601-01-01 and the Unix epoch.
const NTFS_EPOCH: i128 = 116_444_736_000_000_000;

/// DOS date and time: **local wall time** read in `zone` (`compatible`: a time in a gap moves forward with the old
/// offset, a time in a fold takes the earlier instant). `None` for invalid fields: month 0 or over 12, day 0 or
/// past the month, hour over 23, minute over 59, second field over 29, and the all-zero date.
pub fn dos_time(date: u16, time: u16, zone: &TimeZone) -> Option<FileTime> {
    if date == 0 {
        return None;
    }
    let year = 1980 + i16::try_from(date >> 9).ok()?;
    let month = i8::try_from((date >> 5) & 0xf).ok()?;
    let day = i8::try_from(date & 0x1f).ok()?;
    let hour = i8::try_from(time >> 11).ok()?;
    let minute = i8::try_from((time >> 5) & 0x3f).ok()?;
    let seconds = time & 0x1f;
    if seconds > 29 {
        return None;
    }
    let second = i8::try_from(seconds * 2).ok()?;
    let date = Date::new(year, month, day).ok()?;
    let time = Time::new(hour, minute, second, 0).ok()?;
    let instant = zone
        .to_ambiguous_timestamp(DateTime::from_parts(date, time))
        .compatible()
        .ok()?;
    FileTime::from_unix(instant.as_second(), 0)
}

/// A Windows `FILETIME` (100 ns since 1601). Zero is no time.
pub fn ntfs_time(value: u64) -> Option<FileTime> {
    if value == 0 {
        return None;
    }
    let intervals = i128::from(value) - NTFS_EPOCH;
    let secs = i64::try_from(intervals.div_euclid(10_000_000)).ok()?;
    let nanos = u32::try_from(intervals.rem_euclid(10_000_000) * 100).ok()?;
    FileTime::from_unix(secs, nanos)
}

/// A PAX `mtime` value: decimal seconds with an optional fraction, possibly negative. `None` when it does not parse
/// or is out of range.
pub fn pax_time(text: &str) -> Option<FileTime> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let secs: i64 = whole.parse().ok()?;
    let mut nanos: u32 = 0;
    for position in 0..9 {
        let digit = fraction.as_bytes().get(position).map_or(0, |b| b - b'0');
        nanos = nanos * 10 + u32::from(digit);
    }
    if !negative {
        return FileTime::from_unix(secs, nanos);
    }
    if nanos == 0 {
        FileTime::from_unix(-secs, 0)
    } else {
        FileTime::from_unix(-secs - 1, 1_000_000_000 - nanos)
    }
}

/// A tar header's seconds (UTC); `None` out of range.
pub(crate) fn tar_time(seconds: u64) -> Option<FileTime> {
    FileTime::from_unix(i64::try_from(seconds).ok()?, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::offset;

    const fn dos_date(year: u16, month: u16, day: u16) -> u16 {
        ((year - 1980) << 9) | (month << 5) | day
    }

    const fn dos_clock(hour: u16, minute: u16, second: u16) -> u16 {
        (hour << 11) | (minute << 5) | (second / 2)
    }

    fn text(t: Option<FileTime>) -> String {
        t.map_or("none".to_owned(), FileTime::to_rfc3339)
    }

    /// Rows 1–4 and 7 of §1.5.8.
    #[test]
    fn dos_rows() {
        let plus2 = offset(2).to_time_zone();
        let minus5 = offset(-5).to_time_zone();
        let cet = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        let noon = dos_clock(12, 0, 0);
        let date = dos_date(2026, 7, 15);
        assert_eq!(text(dos_time(date, noon, &plus2)), "2026-07-15T10:00:00Z");
        assert_eq!(text(dos_time(date, noon, &minus5)), "2026-07-15T17:00:00Z");
        let gap = dos_time(dos_date(2026, 3, 29), dos_clock(2, 30, 0), &cet);
        assert_eq!(text(gap), "2026-03-29T01:30:00Z");
        let fold = dos_time(dos_date(2026, 10, 25), dos_clock(2, 30, 0), &cet);
        assert_eq!(text(fold), "2026-10-25T00:30:00Z");
        let utc = TimeZone::UTC;
        assert_eq!(dos_time(dos_date(2026, 13, 1), noon, &utc), None);
        assert_eq!(dos_time(dos_date(2026, 0, 1), noon, &utc), None);
        assert_eq!(dos_time(dos_date(2026, 2, 30), noon, &utc), None);
        assert_eq!(dos_time(date & !0x1f, noon, &utc), None);
        assert_eq!(dos_time(date, noon | 30, &utc), None);
        assert_eq!(dos_time(date, 24 << 11, &utc), None);
        assert_eq!(dos_time(date, 60 << 5, &utc), None);
        assert_eq!(dos_time(0, noon, &utc), None);
    }

    #[test]
    fn ntfs_and_pax() {
        let t = ntfs_time(116_444_736_000_000_000 + 17_924_155_205_000_000);
        assert_eq!(text(t), "2026-10-19T13:12:00.5Z");
        assert_eq!(ntfs_time(0), None);
        assert_eq!(text(ntfs_time(1)), "1601-01-01T00:00:00.0000001Z");
        assert_eq!(text(pax_time("1792415520.5")), "2026-10-19T13:12:00.5Z");
        assert_eq!(text(pax_time("1792415520")), "2026-10-19T13:12:00Z");
        assert_eq!(text(pax_time("-1.5")), "1969-12-31T23:59:58.5Z");
        assert_eq!(pax_time("abc"), None);
        assert_eq!(pax_time(""), None);
        assert_eq!(pax_time("1.x"), None);
        assert_eq!(pax_time("99999999999999999999999"), None);
        assert_eq!(text(tar_time(0)), "1970-01-01T00:00:00Z");
        assert_eq!(tar_time(u64::MAX), None);
    }
}
