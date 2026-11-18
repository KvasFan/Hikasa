//! Cells (contract T-5, §4.1): the packed 12-byte [`Cell`], the borrowed [`Value`] view, [`CellError`] and display
//! texts.

use std::borrow::Cow;
use std::fmt::Write as _;

use jiff::SignedDuration;
use jiff::civil::{Date, DateTime};
use serde::{Deserialize, Serialize};

/// A spreadsheet error value (T-14). Compared by code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellError {
    /// `#DIV/0!`
    Div0,
    /// `#N/A`
    NA,
    /// `#NAME?`
    Name,
    /// `#NULL!`
    Null,
    /// `#NUM!`
    Num,
    /// `#REF!`
    Ref,
    /// `#VALUE!`
    Value,
    /// `#DATA!` (getting data)
    Data,
}

impl CellError {
    /// Every error, in code order.
    pub const ALL: [CellError; 8] = [
        CellError::Div0,
        CellError::NA,
        CellError::Name,
        CellError::Null,
        CellError::Num,
        CellError::Ref,
        CellError::Value,
        CellError::Data,
    ];

    /// The spreadsheet spelling (`#DIV/0!`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            CellError::Div0 => "#DIV/0!",
            CellError::NA => "#N/A",
            CellError::Name => "#NAME?",
            CellError::Null => "#NULL!",
            CellError::Num => "#NUM!",
            CellError::Ref => "#REF!",
            CellError::Value => "#VALUE!",
            CellError::Data => "#DATA!",
        }
    }

    fn code(self) -> u32 {
        CellError::ALL.iter().position(|e| *e == self).unwrap_or(0) as u32
    }

    fn from_code(code: u32) -> CellError {
        CellError::ALL
            .get(code as usize)
            .copied()
            .unwrap_or(CellError::Value)
    }
}

/// Stored kind of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellKind {
    /// No value.
    Empty,
    /// A number.
    Number,
    /// Text.
    Text,
    /// A boolean.
    Bool,
    /// A civil date-time.
    DateTime,
    /// A duration.
    Duration,
    /// An error value.
    Error,
}

impl CellKind {
    /// Stable snake_case name (`"date_time"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            CellKind::Empty => "empty",
            CellKind::Number => "number",
            CellKind::Text => "text",
            CellKind::Bool => "bool",
            CellKind::DateTime => "date_time",
            CellKind::Duration => "duration",
            CellKind::Error => "error",
        }
    }

    const fn tag(self) -> u32 {
        match self {
            CellKind::Empty => 0,
            CellKind::Number => 1,
            CellKind::Text => 2,
            CellKind::Bool => 3,
            CellKind::DateTime => 4,
            CellKind::Duration => 5,
            CellKind::Error => 6,
        }
    }

    const fn from_tag(tag: u32) -> CellKind {
        match tag & KIND_MASK {
            1 => CellKind::Number,
            2 => CellKind::Text,
            3 => CellKind::Bool,
            4 => CellKind::DateTime,
            5 => CellKind::Duration,
            6 => CellKind::Error,
            _ => CellKind::Empty,
        }
    }
}

const KIND_MASK: u32 = 0xFF;
/// The cell has a formula in the sheet's formula table.
pub(crate) const FLAG_FORMULA: u32 = 1 << 8;
/// A text cell whose span is in the unescaped tail (resolved when a sheet is finished).
pub(crate) const FLAG_TAIL: u32 = 1 << 10;

/// One stored cell: 12 bytes (T-5). Text cells are spans into their sheet's arena; read them through
/// [`crate::Sheet::cell`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C)]
pub struct Cell {
    tag: u32,
    a: u32,
    b: u32,
}

const _: () = assert!(std::mem::size_of::<Cell>() == 12);

impl Cell {
    /// The empty cell.
    pub const EMPTY: Cell = Cell { tag: 0, a: 0, b: 0 };

    fn wide(kind: CellKind, bits: u64) -> Cell {
        Cell {
            tag: kind.tag(),
            a: bits as u32,
            b: (bits >> 32) as u32,
        }
    }

    /// A number.
    pub fn number(v: f64) -> Cell {
        Cell::wide(CellKind::Number, v.to_bits())
    }

    /// A boolean.
    pub fn bool(v: bool) -> Cell {
        Cell {
            tag: CellKind::Bool.tag(),
            a: u32::from(v),
            b: 0,
        }
    }

    /// A date-time in milliseconds since 1970-01-01T00:00 (civil).
    pub fn date_time_ms(ms: i64) -> Cell {
        Cell::wide(CellKind::DateTime, ms as u64)
    }

    /// A duration in milliseconds.
    pub fn duration_ms(ms: i64) -> Cell {
        Cell::wide(CellKind::Duration, ms as u64)
    }

    /// An error value.
    pub fn error(e: CellError) -> Cell {
        Cell {
            tag: CellKind::Error.tag(),
            a: e.code(),
            b: 0,
        }
    }

    /// A text span `[offset, offset + len)` of the sheet's arena.
    pub(crate) fn text(offset: u32, len: u32) -> Cell {
        Cell {
            tag: CellKind::Text.tag(),
            a: offset,
            b: len,
        }
    }

    /// The stored kind.
    pub fn kind(&self) -> CellKind {
        CellKind::from_tag(self.tag)
    }

    /// True when the sheet's formula table has an entry for the cell.
    pub fn has_formula(&self) -> bool {
        self.tag & FLAG_FORMULA != 0
    }

    pub(crate) fn with_flag(mut self, flag: u32) -> Cell {
        self.tag |= flag;
        self
    }

    pub(crate) fn has_flag(&self, flag: u32) -> bool {
        self.tag & flag != 0
    }

    pub(crate) fn clear_flag(&mut self, flag: u32) {
        self.tag &= !flag;
    }

    pub(crate) fn span(&self) -> (u32, u32) {
        (self.a, self.b)
    }

    pub(crate) fn set_offset(&mut self, offset: u32) {
        self.a = offset;
    }

    fn bits(&self) -> u64 {
        u64::from(self.a) | (u64::from(self.b) << 32)
    }

    /// Resolves the cell against its arena.
    pub(crate) fn value<'a>(&self, arena: &'a str) -> Value<'a> {
        match self.kind() {
            CellKind::Empty => Value::Empty,
            CellKind::Number => Value::Number(f64::from_bits(self.bits())),
            CellKind::Text => {
                let (start, len) = (self.a as usize, self.b as usize);
                Value::Text(arena.get(start..start + len).unwrap_or(""))
            }
            CellKind::Bool => Value::Bool(self.a != 0),
            CellKind::DateTime => Value::DateTime(ms_to_datetime(self.bits() as i64)),
            CellKind::Duration => Value::Duration(SignedDuration::from_millis(self.bits() as i64)),
            CellKind::Error => Value::Error(CellError::from_code(self.a)),
        }
    }

    /// True when both cells hold the same payload (kind and value bits; not for text spans).
    pub(crate) fn payload_eq(&self, other: &Cell) -> bool {
        self.kind() == other.kind() && self.a == other.a && self.b == other.b
    }
}

/// A borrowed, typed cell value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    /// No value.
    Empty,
    /// A number.
    Number(f64),
    /// Text (for delimited files: the field as written, qualifiers removed).
    Text(&'a str),
    /// A boolean.
    Bool(bool),
    /// A civil date-time (no zone).
    DateTime(DateTime),
    /// A duration.
    Duration(SignedDuration),
    /// An error value.
    Error(CellError),
}

impl Value<'_> {
    /// The kind of the value.
    pub fn kind(&self) -> CellKind {
        match self {
            Value::Empty => CellKind::Empty,
            Value::Number(_) => CellKind::Number,
            Value::Text(_) => CellKind::Text,
            Value::Bool(_) => CellKind::Bool,
            Value::DateTime(_) => CellKind::DateTime,
            Value::Duration(_) => CellKind::Duration,
            Value::Error(_) => CellKind::Error,
        }
    }

    /// True for `Empty` and for text that is empty or only whitespace.
    pub fn is_blank(&self) -> bool {
        match self {
            Value::Empty => true,
            Value::Text(t) => t.trim().is_empty(),
            _ => false,
        }
    }

    /// The display text (§4.1).
    pub fn display(&self) -> Cow<'_, str> {
        match self {
            Value::Empty => Cow::Borrowed(""),
            Value::Text(t) => Cow::Borrowed(t),
            Value::Number(v) => Cow::Owned(format_number(*v)),
            Value::Bool(true) => Cow::Borrowed("TRUE"),
            Value::Bool(false) => Cow::Borrowed("FALSE"),
            Value::DateTime(dt) => Cow::Owned(format_datetime(*dt)),
            Value::Duration(d) => Cow::Owned(format_duration(*d)),
            Value::Error(e) => Cow::Borrowed(e.as_str()),
        }
    }
}

/// Rust's shortest round-trip form, `-0` as `0`.
pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        return "0".to_owned();
    }
    format!("{v}")
}

/// `YYYY-MM-DD`, or `YYYY-MM-DD HH:MM:SS[.mmm]` when the time is not midnight.
pub fn format_datetime(dt: DateTime) -> String {
    let mut out = format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day());
    let ms = dt.subsec_nanosecond() / 1_000_000;
    if dt.hour() != 0 || dt.minute() != 0 || dt.second() != 0 || ms != 0 {
        let _ = write!(
            out,
            " {:02}:{:02}:{:02}",
            dt.hour(),
            dt.minute(),
            dt.second()
        );
        if ms != 0 {
            let _ = write!(out, ".{ms:03}");
        }
    }
    out
}

/// `[-]H:MM:SS[.mmm]`.
pub fn format_duration(d: SignedDuration) -> String {
    let ms = d.as_millis();
    let neg = ms < 0;
    let ms = ms.unsigned_abs();
    let (h, rem) = (ms / 3_600_000, ms % 3_600_000);
    let (m, rem) = (rem / 60_000, rem % 60_000);
    let (s, milli) = (rem / 1000, rem % 1000);
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    let _ = write!(out, "{h}:{m:02}:{s:02}");
    if milli != 0 {
        let _ = write!(out, ".{milli:03}");
    }
    out
}

/// The civil epoch of date-time cells.
const EPOCH: DateTime = DateTime::constant(1970, 1, 1, 0, 0, 0, 0);

/// Milliseconds since 1970-01-01T00:00 → civil date-time (saturating at jiff's range).
pub fn ms_to_datetime(ms: i64) -> DateTime {
    EPOCH
        .checked_add(SignedDuration::from_millis(ms))
        .unwrap_or(if ms < 0 { DateTime::MIN } else { DateTime::MAX })
}

/// Civil date-time → milliseconds since 1970-01-01T00:00 (sub-millisecond parts truncated).
pub fn datetime_to_ms(dt: DateTime) -> i64 {
    dt.duration_since(EPOCH).as_millis() as i64
}

/// Days from a civil date to the epoch's date.
pub(crate) fn date_to_ms(date: Date) -> i64 {
    datetime_to_ms(date.to_datetime(jiff::civil::Time::midnight()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_round_trip_their_values() {
        let arena = "hello";
        assert_eq!(Cell::number(1.5).value(arena), Value::Number(1.5));
        assert_eq!(Cell::bool(true).value(arena), Value::Bool(true));
        assert_eq!(Cell::text(1, 3).value(arena), Value::Text("ell"));
        assert_eq!(Cell::EMPTY.value(arena), Value::Empty);
        for e in CellError::ALL {
            assert_eq!(Cell::error(e).value(arena), Value::Error(e));
        }
        let dt = DateTime::constant(2024, 1, 31, 12, 30, 0, 250_000_000);
        assert_eq!(
            Cell::date_time_ms(datetime_to_ms(dt)).value(arena),
            Value::DateTime(dt)
        );
        let old = DateTime::constant(1899, 12, 30, 6, 0, 0, 0);
        assert_eq!(ms_to_datetime(datetime_to_ms(old)), old);
        assert_eq!(
            Cell::duration_ms(-90_500).value(arena).display(),
            "-0:01:30.500"
        );
    }

    #[test]
    fn display_texts() {
        assert_eq!(Value::Number(5.0).display(), "5");
        assert_eq!(Value::Number(-0.0).display(), "0");
        assert_eq!(Value::Number(0.1).display(), "0.1");
        assert_eq!(
            Value::DateTime(DateTime::constant(2024, 1, 31, 0, 0, 0, 0)).display(),
            "2024-01-31"
        );
        assert_eq!(
            Value::DateTime(DateTime::constant(2024, 1, 31, 7, 5, 3, 20_000_000)).display(),
            "2024-01-31 07:05:03.020"
        );
        assert_eq!(Value::Bool(false).display(), "FALSE");
        assert_eq!(Value::Error(CellError::Div0).display(), "#DIV/0!");
    }
}
