//! Value interpretation (HX-007; D-335): [`at`], [`Values`], [`Interpreted`], [`format`](fn@format), and the value
//! panel's choice of columns and integer radix [`format_with`] (W13a, D-360).

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::document::HexDocument;
use crate::error::HexError;
use crate::vocab::Radix;

/// The bytes at an offset read as numbers of one byte order (`None` where fewer bytes remain). Floats are read
/// as they are (NaN and infinities kept).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Interpreted {
    /// Signed 8-bit.
    pub i8: Option<i8>,
    /// Unsigned 8-bit.
    pub u8: Option<u8>,
    /// Signed 16-bit.
    pub i16: Option<i16>,
    /// Unsigned 16-bit.
    pub u16: Option<u16>,
    /// Signed 32-bit.
    pub i32: Option<i32>,
    /// Unsigned 32-bit.
    pub u32: Option<u32>,
    /// Signed 64-bit.
    pub i64: Option<i64>,
    /// Unsigned 64-bit.
    pub u64: Option<u64>,
    /// IEEE 754 single.
    pub f32: Option<f32>,
    /// IEEE 754 double.
    pub f64: Option<f64>,
    /// The scalar value that starts at the offset when the bytes there are a valid UTF-8 sequence (the same in
    /// both byte orders).
    pub utf8: Option<char>,
}

/// The value panel of one offset: little and big endian.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Values {
    /// Little endian.
    pub little: Interpreted,
    /// Big endian.
    pub big: Interpreted,
}

/// The values of the bytes at `offset` of `doc` (up to 8 bytes read through the page cache).
///
/// # Errors
///
/// A read error; [`HexError::Truncated`].
pub fn at(doc: &HexDocument, offset: u64) -> Result<Values, HexError> {
    let mut buf = [0u8; 8];
    let n = doc.read(offset, &mut buf)?;
    Ok(from_bytes(&buf[..n]))
}

/// [`at`] from the page cache only (W13a review: the UI thread never reads a file): `None` when a page of the up to 8
/// bytes is not cached (nothing is read), else what `at` returns.
pub fn at_cached(doc: &HexDocument, offset: u64) -> Option<Result<Values, HexError>> {
    let mut buf = [0u8; 8];
    Some(
        doc.read_cached(offset, &mut buf)?
            .map(|n| from_bytes(&buf[..n])),
    )
}

/// The values of the first bytes of `bytes` (at most 8 are used).
pub fn from_bytes(bytes: &[u8]) -> Values {
    let utf8 = utf8_at(bytes);
    let get = |n: usize| -> Option<[u8; 8]> {
        let mut w = [0u8; 8];
        w.get_mut(..n)?.copy_from_slice(bytes.get(..n)?);
        Some(w)
    };
    let read = |little: bool| {
        let take2 = get(2).map(|w| [w[0], w[1]]);
        let take4 = get(4).map(|w| [w[0], w[1], w[2], w[3]]);
        let take8 = get(8);
        let u16v = take2.map(|b| {
            if little {
                u16::from_le_bytes(b)
            } else {
                u16::from_be_bytes(b)
            }
        });
        let u32v = take4.map(|b| {
            if little {
                u32::from_le_bytes(b)
            } else {
                u32::from_be_bytes(b)
            }
        });
        let u64v = take8.map(|b| {
            if little {
                u64::from_le_bytes(b)
            } else {
                u64::from_be_bytes(b)
            }
        });
        Interpreted {
            i8: bytes.first().map(|&b| b as i8),
            u8: bytes.first().copied(),
            i16: u16v.map(|v| v as i16),
            u16: u16v,
            i32: u32v.map(|v| v as i32),
            u32: u32v,
            i64: u64v.map(|v| v as i64),
            u64: u64v,
            f32: u32v.map(f32::from_bits),
            f64: u64v.map(f64::from_bits),
            utf8,
        }
    };
    Values {
        little: read(true),
        big: read(false),
    }
}

/// The scalar value of a valid UTF-8 sequence at the start of `bytes`.
fn utf8_at(bytes: &[u8]) -> Option<char> {
    let first = *bytes.first()?;
    let len = match first {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return None,
    };
    std::str::from_utf8(bytes.get(..len)?).ok()?.chars().next()
}

/// A float as read: `NaN`, `+Infinity`, `-Infinity`, else the shortest text that reads back the same value.
fn float<F: std::fmt::Debug>(v: F, nan: bool, infinite: bool, positive: bool) -> String {
    if nan {
        "NaN".to_owned()
    } else if infinite {
        if positive { "+Infinity" } else { "-Infinity" }.to_owned()
    } else {
        format!("{v:?}")
    }
}

/// What the value panel shows (W13a, D-360): the little and big endian blocks, and the integers' radix (floats and
/// UTF-8 are the same in both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueFormat {
    /// The *Little Endian Values* block.
    pub little: bool,
    /// The *Big Endian Values* block.
    pub big: bool,
    /// Integers in decimal or in hexadecimal (`0x` and the type's width in digits for unsigned types; a sign, `0x`
    /// and the magnitude for signed ones: `-0x1`).
    pub radix: Radix,
}

impl Default for ValueFormat {
    /// Both blocks, integers in decimal: [`format`](fn@format)'s text.
    fn default() -> ValueFormat {
        ValueFormat {
            little: true,
            big: true,
            radix: Radix::Decimal,
        }
    }
}

/// The panel's lines (W13 and `hex dump --values` print the same text): a *Little Endian Values* and a *Big Endian
/// Values* block, one `name: value` line per type (`n/a` where fewer bytes remain). Equals [`format_with`] with both
/// blocks and decimal integers.
pub fn format(values: &Values) -> String {
    format_with(values, ValueFormat::default())
}

/// An unsigned integer of `bits` in `radix`.
fn unsigned(v: Option<u64>, bits: u32, radix: Radix) -> String {
    match (v, radix) {
        (None, _) => "n/a".to_owned(),
        (Some(v), Radix::Decimal) => v.to_string(),
        (Some(v), Radix::Hex) => format!("0x{v:0width$X}", width = (bits / 4) as usize),
    }
}

/// A signed integer in `radix`.
fn signed(v: Option<i64>, radix: Radix) -> String {
    match (v, radix) {
        (None, _) => "n/a".to_owned(),
        (Some(v), Radix::Decimal) => v.to_string(),
        (Some(v), Radix::Hex) if v < 0 => format!("-0x{:X}", v.unsigned_abs()),
        (Some(v), Radix::Hex) => format!("0x{v:X}"),
    }
}

/// The value panel's lines with a choice of blocks and integer radix (W13a, D-360): the blocks of [`format`] that
/// `fmt` shows, in that order.
pub fn format_with(values: &Values, fmt: ValueFormat) -> String {
    let mut out = String::new();
    let r = fmt.radix;
    for (title, v, shown) in [
        ("Little Endian Values", &values.little, fmt.little),
        ("Big Endian Values", &values.big, fmt.big),
    ] {
        if !shown {
            continue;
        }
        let _ = writeln!(out, "{title}");
        let lines = [
            ("Int8", signed(v.i8.map(i64::from), r)),
            ("UInt8", unsigned(v.u8.map(u64::from), 8, r)),
            ("Int16", signed(v.i16.map(i64::from), r)),
            ("UInt16", unsigned(v.u16.map(u64::from), 16, r)),
            ("Int32", signed(v.i32.map(i64::from), r)),
            ("UInt32", unsigned(v.u32.map(u64::from), 32, r)),
            ("Int64", signed(v.i64, r)),
            ("UInt64", unsigned(v.u64, 64, r)),
            (
                "Float32",
                v.f32.map_or_else(
                    || "n/a".to_owned(),
                    |f| float(f, f.is_nan(), f.is_infinite(), f > 0.0),
                ),
            ),
            (
                "Float64",
                v.f64.map_or_else(
                    || "n/a".to_owned(),
                    |f| float(f, f.is_nan(), f.is_infinite(), f > 0.0),
                ),
            ),
            (
                "UTF-8",
                v.utf8.map_or_else(
                    || "n/a".to_owned(),
                    |c| {
                        if c.is_control() {
                            format!("U+{:04X}", u32::from(c))
                        } else {
                            format!("U+{:04X} {c}", u32::from(c))
                        }
                    },
                ),
            ),
        ];
        for (name, value) in lines {
            let _ = writeln!(out, "  {:<8} {value}", format!("{name}:"));
        }
    }
    out
}
