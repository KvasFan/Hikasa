//! [`FileTime`]: a UTC instant with nanosecond precision and its RFC 3339 text form (T5, §4.1 rule 5).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use jiff::SignedDuration;
use jiff::civil::DateTime;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Classify, ErrorKind};

const NANOS_PER_SECOND: u32 = 1_000_000_000;

/// The Unix epoch as a civil date-time (UTC).
const EPOCH: DateTime = DateTime::constant(1970, 1, 1, 0, 0, 0, 0);

/// Unix seconds of `0000-01-01T00:00:00Z`, the first instant RFC 3339 can write (4-digit years).
const RFC3339_FIRST_SECOND: i64 = -62_167_219_200;
/// Unix seconds of `9999-12-31T23:59:59Z`, the last second RFC 3339 can write.
const RFC3339_LAST_SECOND: i64 = 253_402_300_799;

/// The serde field under which the `toml` crate hands a native TOML date-time to a deserializer (a one-entry map
/// whose value is the date-time text).
const TOML_DATETIME_FIELD: &str = "$__toml_private_datetime";

/// A UTC instant with nanosecond precision (INV-TIME-1: `nanos < 1_000_000_000`).
/// Serialises as RFC 3339 UTC via jiff (`2026-10-19T13:12:00.5Z`; fractional digits only when non-zero).
/// Instants that RFC 3339 cannot write (UTC years before 0000 or after 9999, which includes everything outside
/// jiff's range) serialise as `"@<secs>.<9-digit nanos>"`, and that form parses back.
///
/// The derived order is chronological (`secs` is the floor of the Unix time, `nanos` the remainder).
///
/// ```
/// use hikasa_core::time::FileTime;
///
/// let t = FileTime::from_unix(1_792_415_520, 500_000_000).unwrap();
/// assert_eq!(t.to_rfc3339(), "2026-10-19T13:12:00.5Z");
/// assert_eq!(FileTime::parse("2026-10-19T15:12:00.5+02:00").unwrap(), t);
/// assert_eq!(FileTime::from_unix(-1, 500_000_000).unwrap().to_rfc3339(), "1969-12-31T23:59:59.5Z");
/// assert_eq!(FileTime::parse("9999-12-31T23:59:59Z").unwrap().to_rfc3339(), "9999-12-31T23:59:59Z");
/// assert_eq!(FileTime::from_unix(-62_167_219_201, 0).unwrap().to_rfc3339(), "@-62167219201.000000000");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileTime {
    secs: i64,
    nanos: u32,
}

/// `FileTime::parse` failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid timestamp {input:?}: {reason}")]
pub struct TimeParseError {
    /// The rejected text.
    pub input: String,
    /// Stable reason: `"empty"`, `"not RFC 3339"` (the text does not have the RFC 3339 shape), `"invalid date"`,
    /// `"invalid time"`, `"invalid offset"` (a field out of range) or `"malformed @secs.nanos form"`.
    pub reason: &'static str,
}

impl Classify for TimeParseError {
    /// Always `invalid_input`.
    fn kind(&self) -> ErrorKind {
        ErrorKind::InvalidInput
    }
}

impl FileTime {
    /// From Unix seconds and nanoseconds (`None` if `nanos >= 1e9`).
    pub const fn from_unix(secs: i64, nanos: u32) -> Option<FileTime> {
        if nanos >= NANOS_PER_SECOND {
            None
        } else {
            Some(FileTime { secs, nanos })
        }
    }

    /// From `SystemTime` (handles instants before the epoch; saturates at the `i64` second range). The
    /// precision is the platform's (100 ns on Windows, 1 ns elsewhere).
    pub fn from_system_time(t: SystemTime) -> FileTime {
        match t.duration_since(UNIX_EPOCH) {
            Ok(after) => FileTime {
                secs: i64::try_from(after.as_secs()).unwrap_or(i64::MAX),
                nanos: after.subsec_nanos(),
            },
            Err(before) => {
                let before = before.duration();
                let whole = i64::try_from(before.as_secs()).unwrap_or(i64::MAX);
                match before.subsec_nanos() {
                    0 => FileTime {
                        secs: whole.saturating_neg(),
                        nanos: 0,
                    },
                    sub => FileTime {
                        secs: whole.saturating_neg().saturating_sub(1),
                        nanos: NANOS_PER_SECOND - sub,
                    },
                }
            }
        }
    }

    /// From a signed duration since the Unix epoch (jiff gives seconds and nanoseconds the same sign;
    /// `FileTime` floors the seconds).
    fn from_since_epoch(since: SignedDuration) -> FileTime {
        let secs = since.as_secs();
        let sub = since.subsec_nanos();
        if sub < 0 {
            FileTime {
                secs: secs.saturating_sub(1),
                nanos: (sub + 1_000_000_000).unsigned_abs(),
            }
        } else {
            FileTime {
                secs,
                nanos: sub.unsigned_abs(),
            }
        }
    }

    /// The civil UTC date-time of this instant when RFC 3339 can write it (years 0000–9999), else `None`.
    fn civil_utc(self) -> Option<DateTime> {
        if !(RFC3339_FIRST_SECOND..=RFC3339_LAST_SECOND).contains(&self.secs) {
            return None;
        }
        let nanos = i32::try_from(self.nanos).ok()?;
        EPOCH
            .checked_add(SignedDuration::new(self.secs, nanos))
            .ok()
    }

    /// Unix seconds (floor).
    pub const fn unix_seconds(self) -> i64 {
        self.secs
    }

    /// Sub-second nanoseconds.
    pub const fn subsec_nanos(self) -> u32 {
        self.nanos
    }

    /// `other - self` in nanoseconds (exact; i128 cannot overflow).
    pub const fn nanos_until(self, other: FileTime) -> i128 {
        (other.secs as i128 - self.secs as i128) * NANOS_PER_SECOND as i128
            + (other.nanos as i128 - self.nanos as i128)
    }

    /// RFC 3339 text (same as `Display`).
    pub fn to_rfc3339(self) -> String {
        self.to_string()
    }

    /// Parses RFC 3339 (any offset, normalised to UTC) or the `@secs.nanos` form.
    ///
    /// The RFC 3339 grammar (§5.6) is checked strictly: `YYYY-MM-DD`, `T`, `hh:mm:ss`, an optional fraction and
    /// `Z` or `±hh:mm`. As RFC 3339 allows, `t`, `z` and a space instead of `T` are accepted. A leap second
    /// (`:60`) is read as `:59`, as jiff does; fractional digits beyond nanoseconds are truncated. Every year
    /// 0000–9999 is accepted, also beyond jiff's own range (`9999-12-31T23:59:59Z`).
    pub fn parse(s: &str) -> Result<FileTime, TimeParseError> {
        let error = |reason| TimeParseError {
            input: s.to_owned(),
            reason,
        };
        if s.is_empty() {
            return Err(error("empty"));
        }
        if let Some(rest) = s.strip_prefix('@') {
            return parse_epoch_form(rest).ok_or_else(|| error("malformed @secs.nanos form"));
        }
        parse_rfc3339(s.as_bytes()).map_err(error)
    }
}

/// `<secs>.<9 digits>` with an optional leading `-`.
fn parse_epoch_form(rest: &str) -> Option<FileTime> {
    let (secs, nanos) = rest.split_once('.')?;
    let digits = secs.strip_prefix('-').unwrap_or(secs);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if nanos.len() != 9 || !nanos.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    FileTime::from_unix(secs.parse().ok()?, nanos.parse().ok()?)
}

const NOT_RFC3339: &str = "not RFC 3339";

/// The decimal value of `text[range]`, which must be non-empty ASCII digits.
fn digits(text: &[u8], range: std::ops::Range<usize>) -> Result<i64, &'static str> {
    let part = text.get(range).ok_or(NOT_RFC3339)?;
    if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
        return Err(NOT_RFC3339);
    }
    Ok(part
        .iter()
        .fold(0i64, |acc, &d| acc * 10 + i64::from(d - b'0')))
}

/// True when `text[at]` is one of `allowed`.
fn byte_in(text: &[u8], at: usize, allowed: &[u8]) -> bool {
    text.get(at).is_some_and(|b| allowed.contains(b))
}

/// A strict RFC 3339 `date-time` (§5.6), see [`FileTime::parse`].
fn parse_rfc3339(text: &[u8]) -> Result<FileTime, &'static str> {
    // full-date "T" partial-time: fixed positions.
    let year = digits(text, 0..4)?;
    let month = digits(text, 5..7)?;
    let day = digits(text, 8..10)?;
    let hour = digits(text, 11..13)?;
    let minute = digits(text, 14..16)?;
    let second = digits(text, 17..19)?;
    if !(byte_in(text, 4, b"-")
        && byte_in(text, 7, b"-")
        && byte_in(text, 10, b"Tt ")
        && byte_in(text, 13, b":")
        && byte_in(text, 16, b":"))
    {
        return Err(NOT_RFC3339);
    }
    // time-secfrac: one or more digits; beyond nanoseconds they are truncated.
    let mut at = 19;
    let mut nanos = 0i64;
    if byte_in(text, at, b".") {
        let fraction = text.get(at + 1..).unwrap_or(&[]);
        let len = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
        if len == 0 {
            return Err(NOT_RFC3339);
        }
        for k in 0..9 {
            let digit = fraction
                .get(k)
                .filter(|_| k < len)
                .map_or(0, |&d| i64::from(d - b'0'));
            nanos = nanos * 10 + digit;
        }
        at += 1 + len;
    }
    // time-offset: "Z" / ("+" / "-") hh ":" mm
    let offset_seconds = match text.get(at) {
        Some(b'Z' | b'z') => {
            at += 1;
            0
        }
        Some(&sign @ (b'+' | b'-')) => {
            let hours = digits(text, at + 1..at + 3)?;
            let minutes = digits(text, at + 4..at + 6)?;
            if !byte_in(text, at + 3, b":") {
                return Err(NOT_RFC3339);
            }
            if hours > 23 || minutes > 59 {
                return Err("invalid offset");
            }
            at += 6;
            let total = hours * 3600 + minutes * 60;
            if sign == b'-' { -total } else { total }
        }
        _ => return Err(NOT_RFC3339),
    };
    if at != text.len() {
        return Err(NOT_RFC3339);
    }
    if !(1..=12).contains(&month) || day == 0 {
        return Err("invalid date");
    }
    if hour > 23 || minute > 59 || second > 60 {
        return Err("invalid time");
    }
    let field = |value: i64| i8::try_from(value).map_err(|_| NOT_RFC3339);
    let civil = DateTime::new(
        i16::try_from(year).map_err(|_| NOT_RFC3339)?,
        field(month)?,
        field(day)?,
        field(hour)?,
        field(minute)?,
        // A leap second is read as the last second of its minute (jiff does the same).
        field(second.min(59))?,
        i32::try_from(nanos).map_err(|_| NOT_RFC3339)?,
    )
    .map_err(|_| "invalid date")?;
    let since_epoch = civil
        .duration_since(EPOCH)
        .checked_sub(SignedDuration::from_secs(offset_seconds))
        .ok_or("invalid offset")?;
    Ok(FileTime::from_since_epoch(since_epoch))
}

impl fmt::Display for FileTime {
    /// RFC 3339 UTC as jiff writes it, or `@<secs>.<9-digit nanos>` when RFC 3339 cannot write the instant.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.civil_utc() {
            Some(civil) => write!(f, "{civil}Z"),
            None => write!(f, "@{}.{:09}", self.secs, self.nanos),
        }
    }
}

impl Serialize for FileTime {
    /// The `Display` string.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FileTime {
    /// [`FileTime::parse`] of a string. A native TOML offset date-time (`mtime = 2026-10-19T13:12:00Z`) is
    /// accepted as well: the `toml` crate hands it over as a one-entry map holding the date-time text.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<FileTime, D::Error> {
        deserializer.deserialize_str(FileTimeVisitor)
    }
}

/// Serde visitor of [`FileTime`].
struct FileTimeVisitor;

impl<'de> Visitor<'de> for FileTimeVisitor {
    type Value = FileTime;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an RFC 3339 timestamp or an `@<secs>.<nanos>` string")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<FileTime, E> {
        FileTime::parse(value).map_err(E::custom)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<FileTime, A::Error> {
        let key: Option<String> = map.next_key()?;
        if key.as_deref() != Some(TOML_DATETIME_FIELD) {
            return Err(de::Error::invalid_type(de::Unexpected::Map, &self));
        }
        let text: String = map.next_value()?;
        FileTime::parse(&text).map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn constructors() {
        assert_eq!(
            FileTime::from_unix(0, 999_999_999).map(FileTime::subsec_nanos),
            Some(999_999_999)
        );
        assert_eq!(FileTime::from_unix(0, 1_000_000_000), None);
        // `SystemTime` offsets are multiples of 100 ns: Windows keeps no finer precision.
        let before = UNIX_EPOCH - Duration::new(1, 250_000_000);
        let t = FileTime::from_system_time(before);
        assert_eq!((t.unix_seconds(), t.subsec_nanos()), (-2, 750_000_000));
        let exact = FileTime::from_system_time(UNIX_EPOCH - Duration::from_secs(5));
        assert_eq!((exact.unix_seconds(), exact.subsec_nanos()), (-5, 0));
        let after = FileTime::from_system_time(UNIX_EPOCH + Duration::new(3, 700));
        assert_eq!((after.unix_seconds(), after.subsec_nanos()), (3, 700));
        assert_eq!(
            FileTime::from_since_epoch(SignedDuration::new(-1, -1)),
            FileTime::from_unix(-2, 999_999_999).unwrap()
        );
    }

    #[test]
    fn differences_are_exact() {
        let a = FileTime::from_unix(-1, 999_999_999).unwrap();
        let b = FileTime::from_unix(1, 1).unwrap();
        assert_eq!(a.nanos_until(b), 1_000_000_002);
        assert_eq!(b.nanos_until(a), -1_000_000_002);
        let min = FileTime::from_unix(i64::MIN, 0).unwrap();
        let max = FileTime::from_unix(i64::MAX, 999_999_999).unwrap();
        assert!(min.nanos_until(max) > 0);
        assert!(min < max);
    }

    #[test]
    fn text_forms() {
        let t = FileTime::from_unix(0, 0).unwrap();
        assert_eq!(t.to_rfc3339(), "1970-01-01T00:00:00Z");
        assert_eq!(
            FileTime::from_unix(1, 1).unwrap().to_string(),
            "1970-01-01T00:00:01.000000001Z"
        );
        let far = FileTime::from_unix(-400_000_000_000, 5).unwrap();
        assert_eq!(far.to_string(), "@-400000000000.000000005");
        assert_eq!(FileTime::parse(&far.to_string()).unwrap(), far);
        let huge = FileTime::from_unix(i64::MAX, 999_999_999).unwrap();
        assert_eq!(FileTime::parse(&huge.to_string()).unwrap(), huge);
        assert_eq!(
            FileTime::parse("1969-12-31T23:59:59.5Z").unwrap(),
            FileTime::from_unix(-1, 500_000_000).unwrap()
        );
    }

    #[test]
    fn the_whole_rfc3339_range_is_text() {
        let first = FileTime::from_unix(RFC3339_FIRST_SECOND, 0).unwrap();
        assert_eq!(first.to_string(), "0000-01-01T00:00:00Z");
        assert_eq!(FileTime::parse("0000-01-01T00:00:00Z").unwrap(), first);
        let last = FileTime::from_unix(RFC3339_LAST_SECOND, 999_999_999).unwrap();
        assert_eq!(last.to_string(), "9999-12-31T23:59:59.999999999Z");
        assert_eq!(FileTime::parse(&last.to_string()).unwrap(), last);
        // Beyond jiff's `Timestamp::MAX` (9999-12-30T22:00:00.999999999Z) but valid RFC 3339.
        for text in ["9999-12-31T23:59:59Z", "9999-12-30T22:00:01Z"] {
            assert_eq!(FileTime::parse(text).unwrap().to_string(), text);
        }
        // Years RFC 3339 cannot write use the @ form.
        for (secs, nanos) in [
            (RFC3339_FIRST_SECOND - 1, 0),
            (RFC3339_LAST_SECOND + 1, 0),
            (-62_184_456_000, 500_000_000),
        ] {
            let t = FileTime::from_unix(secs, nanos).unwrap();
            assert_eq!(t.to_string(), format!("@{secs}.{nanos:09}"));
            assert_eq!(FileTime::parse(&t.to_string()).unwrap(), t);
        }
        // A local time in year 9999 or 0000 whose UTC instant leaves the range still parses.
        let late = FileTime::parse("9999-12-31T23:59:59-23:59").unwrap();
        assert_eq!(
            late.unix_seconds(),
            RFC3339_LAST_SECOND + 23 * 3600 + 59 * 60
        );
        assert!(late.to_string().starts_with('@'));
        let early = FileTime::parse("0000-01-01T00:00:00+00:01").unwrap();
        assert_eq!(early.unix_seconds(), RFC3339_FIRST_SECOND - 60);
    }

    #[test]
    fn rfc3339_variants() {
        let utc = FileTime::parse("2026-10-19T13:12:00Z").unwrap();
        for text in [
            "2026-10-19t13:12:00z",
            "2026-10-19 13:12:00Z",
            "2026-10-19T13:12:00+00:00",
            "2026-10-19T13:12:00-00:00",
            "2026-10-19T14:12:00+01:00",
            "2026-10-19T13:12:00.000Z",
        ] {
            assert_eq!(FileTime::parse(text).unwrap(), utc, "{text}");
        }
        assert_eq!(
            FileTime::parse("2026-10-19T13:12:00.1234567891234Z")
                .unwrap()
                .subsec_nanos(),
            123_456_789,
            "digits beyond nanoseconds are truncated"
        );
        assert_eq!(
            FileTime::parse("2016-12-31T23:59:60Z").unwrap(),
            FileTime::parse("2016-12-31T23:59:59Z").unwrap(),
            "a leap second is read as :59"
        );
        assert_eq!(
            FileTime::parse("2024-02-29T00:00:00Z").unwrap().to_string(),
            "2024-02-29T00:00:00Z"
        );
    }

    #[test]
    fn parse_errors() {
        for (input, reason) in [
            ("", "empty"),
            ("2026-10-19T13:12:00", "not RFC 3339"),
            ("yesterday", "not RFC 3339"),
            ("2026-10-19T13:12Z", "not RFC 3339"),
            ("2026-10-19T13:12:00+0200", "not RFC 3339"),
            ("2026-10-19T13:12:00+02", "not RFC 3339"),
            ("2026-10-19T13:12:00Z[UTC]", "not RFC 3339"),
            ("2026-10-19T13:12:00.Z", "not RFC 3339"),
            ("2026-10-19T13:12:00ZZ", "not RFC 3339"),
            ("2026-10-19 13:12:00 Z", "not RFC 3339"),
            ("-000001-01-01T00:00:00Z", "not RFC 3339"),
            ("+002026-10-19T13:12:00Z", "not RFC 3339"),
            ("20261019T131200Z", "not RFC 3339"),
            ("2026-1-19T13:12:00Z", "not RFC 3339"),
            ("２０２６-10-19T13:12:00Z", "not RFC 3339"),
            ("2026-13-01T00:00:00Z", "invalid date"),
            ("2026-00-01T00:00:00Z", "invalid date"),
            ("2026-02-29T00:00:00Z", "invalid date"),
            ("2026-04-31T00:00:00Z", "invalid date"),
            ("2026-10-00T00:00:00Z", "invalid date"),
            ("2026-10-19T24:00:00Z", "invalid time"),
            ("2026-10-19T13:60:00Z", "invalid time"),
            ("2026-10-19T13:12:61Z", "invalid time"),
            ("2026-10-19T13:12:00+24:00", "invalid offset"),
            ("2026-10-19T13:12:00-00:60", "invalid offset"),
            ("@12", "malformed @secs.nanos form"),
            ("@12.5", "malformed @secs.nanos form"),
            ("@x.000000000", "malformed @secs.nanos form"),
            ("@-.000000000", "malformed @secs.nanos form"),
            (
                "@99999999999999999999.000000000",
                "malformed @secs.nanos form",
            ),
        ] {
            assert_eq!(
                FileTime::parse(input),
                Err(TimeParseError {
                    input: input.into(),
                    reason
                }),
                "{input:?}"
            );
        }
    }

    #[test]
    fn toml_native_datetimes_deserialize() {
        use serde::de::value::{Error, MapDeserializer};

        let native = MapDeserializer::<_, Error>::new(
            [(TOML_DATETIME_FIELD, "2026-10-19T15:12:00.5+02:00")].into_iter(),
        );
        assert_eq!(
            FileTime::deserialize(native).unwrap(),
            FileTime::from_unix(1_792_415_520, 500_000_000).unwrap()
        );
        let local = MapDeserializer::<_, Error>::new(
            [(TOML_DATETIME_FIELD, "2026-10-19T13:12:00")].into_iter(),
        );
        assert!(
            FileTime::deserialize(local)
                .unwrap_err()
                .to_string()
                .contains("not RFC 3339"),
            "a local date-time has no offset"
        );
        let other = MapDeserializer::<_, Error>::new([("at", "2026-10-19T13:12:00Z")].into_iter());
        assert!(FileTime::deserialize(other).is_err());
    }
}
