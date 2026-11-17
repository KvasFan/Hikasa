//! Value interpretation (HX-007; D-335): [`at`], [`Values`], [`Interpreted`], [`format`].

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::document::HexDocument;
use crate::error::HexError;

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

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map_or_else(|| "n/a".to_owned(), |v| v.to_string())
}

/// The panel's lines (W13 and `hex dump --values` print the same text): a *Little Endian Values* and a *Big Endian
/// Values* block, one `name: value` line per type (`n/a` where fewer bytes remain).
pub fn format(values: &Values) -> String {
    let mut out = String::new();
    for (title, v) in [
        ("Little Endian Values", &values.little),
        ("Big Endian Values", &values.big),
    ] {
        let _ = writeln!(out, "{title}");
        let lines = [
            ("Int8", opt(v.i8)),
            ("UInt8", opt(v.u8)),
            ("Int16", opt(v.i16)),
            ("UInt16", opt(v.u16)),
            ("Int32", opt(v.i32)),
            ("UInt32", opt(v.u32)),
            ("Int64", opt(v.i64)),
            ("UInt64", opt(v.u64)),
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
