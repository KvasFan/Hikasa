//! Regional settings and the parsing of numbers, dates, times and booleans from text (contract T-19, TB-001,
//! TB-005). Pure functions: the engine never reads the locale (T1); *use system* is resolved by the host.

use jiff::civil::{Date, DateTime, Time};
use serde::{Deserialize, Serialize};

use crate::cell::datetime_to_ms;

/// Order of day, month and year in a date that is not ISO 8601.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateOrder {
    /// Year, month, day (the default).
    #[default]
    Ymd,
    /// Day, month, year.
    Dmy,
    /// Month, day, year.
    Mdy,
}

impl DateOrder {
    /// Parses `ymd`, `dmy`, `mdy` (ASCII case-insensitive).
    pub fn parse(word: &str) -> Result<DateOrder, String> {
        match word.to_ascii_lowercase().as_str() {
            "ymd" => Ok(DateOrder::Ymd),
            "dmy" => Ok(DateOrder::Dmy),
            "mdy" => Ok(DateOrder::Mdy),
            _ => Err(format!(
                "unknown date order `{word}` (expected ymd, dmy or mdy)"
            )),
        }
    }

    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            DateOrder::Ymd => "ymd",
            DateOrder::Dmy => "dmy",
            DateOrder::Mdy => "mdy",
        }
    }
}

/// Regional settings (T-19). Defaults: `.` decimal, `,` thousands, YMD, any date separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Regional {
    /// Decimal separator.
    pub decimal: char,
    /// Thousands separator (`None`: digits are never grouped).
    pub thousands: Option<char>,
    /// Date order of non-ISO dates.
    pub date_order: DateOrder,
    /// Date separator (`None`: any of `-`, `/`, `.`).
    pub date_sep: Option<char>,
}

impl Default for Regional {
    fn default() -> Regional {
        Regional {
            decimal: '.',
            thousands: Some(','),
            date_order: DateOrder::Ymd,
            date_sep: None,
        }
    }
}

/// A decimal number as written: `mantissa × 10^exponent`, the mantissa without trailing zeros (contract T-6, F1).
/// Zero is `{ negative: false, mantissa: 0, exponent: 0 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    /// The number is below zero.
    pub negative: bool,
    /// The significant digits (at most 38).
    pub mantissa: u128,
    /// The power of ten.
    pub exponent: i32,
}

impl Decimal {
    /// Number of significant digits (0 for zero).
    pub fn digits(&self) -> u32 {
        if self.mantissa == 0 {
            0
        } else {
            self.mantissa.ilog10() + 1
        }
    }
}

/// What a number's text says beyond its value (T-6, F1, F2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberText {
    /// The value (the nearest `f64`).
    pub value: f64,
    /// Significant digits: from the first non-zero digit to the last one.
    pub significant: u32,
    /// The integer part has more than one digit and starts with `0` (`007`, `01234`; not `0` or `0.5`).
    pub leading_zero: bool,
    /// The exact decimal (`None` above 38 significant digits).
    pub decimal: Option<Decimal>,
}

/// Significant digits an `f64` holds exactly: a decimal with at most this many converts to a unique `f64` and back.
pub const EXACT_DIGITS: u32 = 15;

/// Digit accounting of a clean ASCII number (`[+-]digits[.digits][e[+-]digits]`, what `f64::from_str` takes).
fn account(clean: &str) -> Option<(u32, bool, Option<Decimal>)> {
    let b = clean.as_bytes();
    let mut i = 0;
    let negative = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let mut mantissa: u128 = 0;
    let mut overflow = false;
    let mut significant: u32 = 0;
    let mut pending: u32 = 0; // zeros after the last non-zero digit
    let mut started = false;
    let mut int_digits = 0u32;
    let mut first_int = 0u8;
    let mut frac_len: i64 = 0;
    let mut digit = |d: u8, mantissa: &mut u128, overflow: &mut bool| {
        if !started {
            if d == 0 {
                return;
            }
            started = true;
            *mantissa = u128::from(d);
            significant = 1;
            return;
        }
        if d == 0 {
            pending += 1;
            return;
        }
        let scale = 10u128.checked_pow(pending + 1);
        match scale
            .and_then(|s| mantissa.checked_mul(s))
            .and_then(|m| m.checked_add(u128::from(d)))
        {
            Some(m) => *mantissa = m,
            None => *overflow = true,
        }
        significant += pending + 1;
        pending = 0;
    };
    while i < b.len() && b[i].is_ascii_digit() {
        if int_digits == 0 {
            first_int = b[i];
        }
        int_digits += 1;
        digit(b[i] - b'0', &mut mantissa, &mut overflow);
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            frac_len += 1;
            digit(b[i] - b'0', &mut mantissa, &mut overflow);
            i += 1;
        }
    }
    let mut exp: i64 = 0;
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        let neg_exp = match b.get(i) {
            Some(b'-') => {
                i += 1;
                true
            }
            Some(b'+') => {
                i += 1;
                false
            }
            _ => false,
        };
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            exp = (exp * 10 + i64::from(b[i] - b'0')).min(1 << 40);
            i += 1;
        }
        if i == start {
            return None;
        }
        if neg_exp {
            exp = -exp;
        }
    }
    if i != b.len() || int_digits + frac_len as u32 == 0 {
        return None;
    }
    let leading_zero = int_digits > 1 && first_int == b'0';
    let decimal = if !started {
        Some(Decimal {
            negative: false,
            mantissa: 0,
            exponent: 0,
        })
    } else if overflow {
        None
    } else {
        let e = exp - frac_len + i64::from(pending);
        i32::try_from(e).ok().map(|exponent| Decimal {
            negative,
            mantissa,
            exponent,
        })
    };
    Some((significant, leading_zero, decimal))
}

/// A value parsed from text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Parsed {
    /// A number.
    Number(f64),
    /// A date-time in milliseconds since 1970-01-01T00:00 (civil).
    DateTime(i64),
    /// A boolean (`true`/`false`, any case).
    Bool(bool),
}

impl Regional {
    /// Parses a number: optional sign, digit groups of three around the thousands separator, one decimal
    /// separator, an exponent, a trailing `%` (divides by 100); surrounding blanks ignored. `None` for anything
    /// else (`NaN`, `inf`, `1,23,4`, empty).
    pub fn parse_number(&self, text: &str) -> Option<f64> {
        self.scan_number(text).map(|n| n.value)
    }

    /// Parses a number like [`Regional::parse_number`] and accounts for its digits: the significant digits, a
    /// leading zero, the exact decimal (contract T-6: numbers with more than 15 significant digits compare by
    /// their decimal, never through `f64`).
    pub fn scan_number(&self, text: &str) -> Option<NumberText> {
        let s = text.trim();
        let (s, percent) = match s.strip_suffix('%') {
            Some(rest) => (rest.trim_end(), true),
            None => (s, false),
        };
        let bytes = s.as_bytes();
        if bytes.is_empty() {
            return None;
        }
        // Fast path: plain ASCII numbers with `.` as the decimal separator (the common case of large tables).
        if self.decimal == '.'
            && bytes
                .iter()
                .all(|b| matches!(b, b'0'..=b'9' | b'+' | b'-' | b'.' | b'e' | b'E'))
        {
            if !bytes.iter().any(u8::is_ascii_digit) || matches!(bytes[0], b'e' | b'E') {
                return None;
            }
            let v: f64 = s.parse().ok()?;
            return finish_number(v, s, percent);
        }
        // Anything that cannot start a number is refused before any allocation.
        let first = s.trim_start_matches(['+', '-']).chars().next()?;
        if !first.is_ascii_digit() && first != self.decimal {
            return None;
        }
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        if let Some(&c) = chars.peek()
            && (c == '-' || c == '+')
        {
            out.push(c);
            chars.next();
        }
        // Integer part with optional grouping.
        let mut int_digits = 0usize;
        let mut group_len: Option<usize> = None; // digits since the last thousands separator
        let mut grouped = false;
        let mut first_group = 0usize;
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                out.push(c);
                int_digits += 1;
                if let Some(g) = group_len.as_mut() {
                    *g += 1;
                    if *g > 3 {
                        return None;
                    }
                } else {
                    first_group += 1;
                }
                chars.next();
            } else if Some(c) == self.thousands && c != self.decimal {
                if int_digits == 0 || group_len.is_some_and(|g| g != 3) {
                    return None;
                }
                if !grouped && first_group > 3 {
                    return None;
                }
                grouped = true;
                group_len = Some(0);
                chars.next();
            } else {
                break;
            }
        }
        if grouped && group_len != Some(3) {
            return None;
        }
        let mut frac_digits = 0usize;
        if chars.peek() == Some(&self.decimal) {
            chars.next();
            out.push('.');
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    out.push(c);
                    frac_digits += 1;
                    chars.next();
                } else {
                    break;
                }
            }
        }
        if int_digits + frac_digits == 0 {
            return None;
        }
        if let Some(&c) = chars.peek()
            && (c == 'e' || c == 'E')
        {
            chars.next();
            out.push('e');
            if let Some(&c) = chars.peek()
                && (c == '-' || c == '+')
            {
                out.push(c);
                chars.next();
            }
            let mut exp_digits = 0;
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    out.push(c);
                    exp_digits += 1;
                    chars.next();
                } else {
                    break;
                }
            }
            if exp_digits == 0 {
                return None;
            }
        }
        if chars.next().is_some() {
            return None;
        }
        let v: f64 = out.parse().ok()?;
        finish_number(v, &out, percent)
    }

    /// Parses a date, a date-time or a time of day (T-19); the result is milliseconds since 1970-01-01T00:00. A
    /// time alone is a time on 1899-12-30 (the spreadsheets' day 0, T-14).
    pub fn parse_datetime(&self, text: &str) -> Option<i64> {
        let s = text.trim();
        if s.is_empty() || !s.as_bytes()[0].is_ascii_digit() {
            return None;
        }
        // Split a date part and a time part at `T` or the first blank.
        let (date_part, time_part) = match s.find(['T', ' ']) {
            Some(at) => (&s[..at], Some(s[at + 1..].trim())),
            None => (s, None),
        };
        if let Some(date) = self.parse_date(date_part) {
            let time = match time_part {
                None => Time::midnight(),
                Some(t) => parse_time(t)?,
            };
            return Some(datetime_to_ms(DateTime::from_parts(date, time)));
        }
        if time_part.is_none() || time_part.is_some_and(is_meridiem) {
            let time = parse_time(s)?;
            let day0 = Date::constant(1899, 12, 30);
            return Some(datetime_to_ms(DateTime::from_parts(day0, time)));
        }
        None
    }

    fn parse_date(&self, s: &str) -> Option<Date> {
        let sep = s.chars().find(|c| !c.is_ascii_digit())?;
        if !matches!(sep, '-' | '/' | '.') {
            return None;
        }
        let parts: Vec<&str> = s.split(sep).collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.len() > 4) {
            return None;
        }
        if parts.iter().any(|p| !p.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        let n: Vec<i32> = parts.iter().filter_map(|p| p.parse().ok()).collect();
        if n.len() != 3 {
            return None;
        }
        // ISO: four-digit year first, `-` separator: always accepted.
        if sep == '-' && parts[0].len() == 4 {
            return make_date(n[0], n[1], n[2]);
        }
        if self.date_sep.is_some_and(|d| d != sep) {
            return None;
        }
        let (y, m, d, ylen) = match self.date_order {
            DateOrder::Ymd => (n[0], n[1], n[2], parts[0].len()),
            DateOrder::Dmy => (n[2], n[1], n[0], parts[2].len()),
            DateOrder::Mdy => (n[2], n[0], n[1], parts[2].len()),
        };
        let y = match ylen {
            4 => y,
            2 => {
                if y < 30 {
                    2000 + y
                } else {
                    1900 + y
                }
            }
            _ => return None,
        };
        make_date(y, m, d)
    }

    /// `true`/`false` in any case.
    pub fn parse_bool(&self, text: &str) -> Option<bool> {
        let t = text.trim();
        if t.eq_ignore_ascii_case("true") {
            Some(true)
        } else if t.eq_ignore_ascii_case("false") {
            Some(false)
        } else {
            None
        }
    }

    /// Number, then date-time, then bool.
    pub fn parse_any(&self, text: &str) -> Option<Parsed> {
        if let Some(v) = self.parse_number(text) {
            return Some(Parsed::Number(v));
        }
        if let Some(ms) = self.parse_datetime(text) {
            return Some(Parsed::DateTime(ms));
        }
        self.parse_bool(text).map(Parsed::Bool)
    }
}

/// The [`NumberText`] of a parsed value and its clean ASCII spelling.
fn finish_number(v: f64, clean: &str, percent: bool) -> Option<NumberText> {
    if !v.is_finite() {
        return None;
    }
    let (significant, leading_zero, mut decimal) = account(clean)?;
    if percent && let Some(d) = decimal.as_mut().filter(|d| d.mantissa != 0) {
        d.exponent = d.exponent.checked_sub(2)?;
    }
    let value = if percent { v / 100.0 } else { v };
    Some(NumberText {
        value: if value == 0.0 { 0.0 } else { value },
        significant,
        leading_zero,
        decimal,
    })
}

fn is_meridiem(s: &str) -> bool {
    s.eq_ignore_ascii_case("am") || s.eq_ignore_ascii_case("pm")
}

fn make_date(y: i32, m: i32, d: i32) -> Option<Date> {
    Date::new(
        i16::try_from(y).ok()?,
        i8::try_from(m).ok()?,
        i8::try_from(d).ok()?,
    )
    .ok()
}

/// `HH:MM[:SS[.fff]]` with an optional `AM`/`PM` (blank-separated or attached).
fn parse_time(s: &str) -> Option<Time> {
    let s = s.trim();
    let lower = s.to_ascii_lowercase();
    let (body, meridiem) = if let Some(rest) = lower.strip_suffix("am") {
        (rest.trim_end().to_owned(), Some(false))
    } else if let Some(rest) = lower.strip_suffix("pm") {
        (rest.trim_end().to_owned(), Some(true))
    } else {
        (lower, None)
    };
    let mut parts = body.split(':');
    let h: i32 = digits(parts.next()?, 1, 2)?;
    let m: i32 = digits(parts.next()?, 2, 2)?;
    let (sec, nanos) = match parts.next() {
        None => (0, 0),
        Some(sec) => {
            let (whole, frac) = match sec.split_once(['.', ',']) {
                Some((w, f)) => (w, Some(f)),
                None => (sec, None),
            };
            let s: i32 = digits(whole, 2, 2)?;
            let nanos = match frac {
                None => 0,
                Some(f) => {
                    if f.is_empty() || f.len() > 9 || !f.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    let padded = format!("{f:0<9}");
                    padded.parse::<i32>().ok()?
                }
            };
            (s, nanos)
        }
    };
    if parts.next().is_some() {
        return None;
    }
    let h = match meridiem {
        None => h,
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            (h % 12) + if pm { 12 } else { 0 }
        }
    };
    // Milliseconds are the stored precision.
    let nanos = nanos / 1_000_000 * 1_000_000;
    Time::new(
        i8::try_from(h).ok()?,
        i8::try_from(m).ok()?,
        i8::try_from(sec).ok()?,
        nanos,
    )
    .ok()
}

fn digits(s: &str, min: usize, max: usize) -> Option<i32> {
    if s.len() < min || s.len() > max || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::ms_to_datetime;

    fn dt(text: &str, r: &Regional) -> Option<String> {
        r.parse_datetime(text)
            .map(|ms| crate::cell::format_datetime(ms_to_datetime(ms)))
    }

    #[test]
    fn numbers() {
        let r = Regional::default();
        assert_eq!(r.parse_number("1,234.5"), Some(1234.5));
        assert_eq!(r.parse_number(" -1e3 "), Some(-1000.0));
        assert_eq!(r.parse_number("12%"), Some(0.12));
        assert_eq!(r.parse_number("+.5"), Some(0.5));
        assert_eq!(r.parse_number("5."), Some(5.0));
        for bad in [
            "", "-", "1,23,4", "12,34", "1234,567", "NaN", "inf", "1e", "1.2.3", "abc", ".", "1 2",
        ] {
            assert_eq!(r.parse_number(bad), None, "{bad}");
        }
        assert_eq!(r.parse_number("1,234,567"), Some(1_234_567.0));
        let de = Regional {
            decimal: ',',
            thousands: Some('.'),
            ..Regional::default()
        };
        assert_eq!(de.parse_number("1.234,5"), Some(1234.5));
        assert_eq!(de.parse_number("3,25"), Some(3.25));
        assert_eq!(de.parse_number("1,234.5"), None);
        let none = Regional {
            thousands: None,
            ..Regional::default()
        };
        assert_eq!(none.parse_number("1,234"), None);
    }

    #[test]
    fn number_digits() {
        let r = Regional::default();
        let scan = |t: &str| r.scan_number(t).unwrap();
        let n = scan("007");
        assert!(n.leading_zero);
        assert_eq!((n.value, n.significant), (7.0, 1));
        for t in ["0", "0.5", "-0.25", "+0", ".5", "10", "0e3"] {
            assert!(!scan(t).leading_zero, "{t}");
        }
        for t in ["01234", "-007", "00.5", "000", "0,123"] {
            assert!(scan(t).leading_zero, "{t}");
        }
        let n = scan("12345678901234567");
        assert_eq!(n.significant, 17);
        assert_eq!(
            n.decimal,
            Some(Decimal {
                negative: false,
                mantissa: 12_345_678_901_234_567,
                exponent: 0
            })
        );
        assert_eq!(scan("1,200.50").decimal, scan("1.2005e3").decimal);
        assert_eq!(scan("1,200.50").significant, 5);
        assert_eq!(
            scan("-0.00012")
                .decimal
                .map(|d| (d.negative, d.mantissa, d.exponent)),
            Some((true, 12, -5))
        );
        assert_eq!(scan("12.5%").decimal, scan("0.125").decimal);
        assert_eq!(scan("-0").decimal, scan("0").decimal);
        assert_eq!(scan("0.000").significant, 0);
        // Past 38 digits there is no exact decimal.
        assert_eq!(scan(&"9".repeat(39)).decimal, None);
        assert_eq!(scan(&"9".repeat(38)).decimal.map(|d| d.digits()), Some(38));
        let de = Regional {
            decimal: ',',
            thousands: Some('.'),
            ..Regional::default()
        };
        assert_eq!(
            de.scan_number("1.234,50").unwrap().decimal,
            scan("1234.5").decimal
        );
    }

    #[test]
    fn dates_and_times() {
        let ymd = Regional::default();
        assert_eq!(dt("2024-01-31", &ymd).as_deref(), Some("2024-01-31"));
        assert_eq!(
            dt("2024-01-31T12:30:00.250", &ymd).as_deref(),
            Some("2024-01-31 12:30:00.250")
        );
        assert_eq!(
            dt("2024/01/31 1:05 pm", &ymd).as_deref(),
            Some("2024-01-31 13:05:00")
        );
        assert_eq!(dt("12:30", &ymd).as_deref(), Some("1899-12-30 12:30:00"));
        assert_eq!(dt("12:00 AM", &ymd).as_deref(), Some("1899-12-30"));
        let dmy = Regional {
            date_order: DateOrder::Dmy,
            date_sep: Some('.'),
            ..Regional::default()
        };
        assert_eq!(dt("31.01.2024", &dmy).as_deref(), Some("2024-01-31"));
        assert_eq!(dt("31.01.24", &dmy).as_deref(), Some("2024-01-31"));
        assert_eq!(dt("31.01.95", &dmy).as_deref(), Some("1995-01-31"));
        assert_eq!(dt("31/01/2024", &dmy), None);
        assert_eq!(dt("2024-01-31", &dmy).as_deref(), Some("2024-01-31"));
        let mdy = Regional {
            date_order: DateOrder::Mdy,
            ..Regional::default()
        };
        assert_eq!(dt("1/31/2024", &mdy).as_deref(), Some("2024-01-31"));
        assert_eq!(dt("31/1/2024", &mdy), None);
        for bad in [
            "2024-02-30",
            "2024-13-01",
            "1",
            "24:00",
            "12:60",
            "x2024-01-01",
            "2024-01-01 junk",
        ] {
            assert_eq!(dt(bad, &ymd), None, "{bad}");
        }
        assert_eq!(ymd.parse_bool(" TRUE "), Some(true));
        assert_eq!(ymd.parse_any("1"), Some(Parsed::Number(1.0)));
        assert_eq!(ymd.parse_any("false"), Some(Parsed::Bool(false)));
    }
}
