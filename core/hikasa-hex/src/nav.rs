//! Navigation, Go To and the Current Byte Address (HX-005, HX-007; D-333): [`step`], [`scan_next_difference`],
//! [`parse_address`], [`AddressQuery`], [`address_text`].

use std::ops::ControlFlow;

use hikasa_core::{Ctl, Side};
use hikasa_vfs::{StreamOptions, visit_chunk_pairs};
use serde::{Deserialize, Serialize};

use crate::align::{HexAlignment, SegmentKind};
use crate::document::HexDocument;
use crate::error::HexError;
use crate::vocab::{Radix, VocabError};

/// A navigation step from a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// The start of the next section (Ctrl+N).
    NextDifference,
    /// The start of the previous section — from inside a section, the one before it (Ctrl+P).
    PrevDifference,
    /// The next column holding a byte of that side (skipping its gaps).
    NextExisting(Side),
    /// The previous column holding a byte of that side.
    PrevExisting(Side),
    /// The first column.
    First,
    /// The last column.
    Last,
}

/// The column a step from `from_column` lands on (`None` when there is none). `NextDifference` past the last
/// stored section of a truncated list gives `None`; [`scan_next_difference`] then finds it (a job).
pub fn step(alignment: &HexAlignment, from_column: u64, step: Step) -> Option<u64> {
    let store = &alignment.segments;
    let sections = &alignment.sections;
    let columns = store.columns();
    match step {
        Step::First => (columns > 0).then_some(0),
        Step::Last => columns.checked_sub(1),
        Step::NextDifference => {
            let k = sections.first_ending_after(from_column);
            (k..sections.len())
                .filter_map(|i| sections.columns(i))
                .find(|c| c.start > from_column)
                .map(|c| c.start)
        }
        Step::PrevDifference => {
            // The last section that ends at or before the column: from inside a section, the one before it.
            let k = sections.first_ending_after(from_column);
            k.checked_sub(1)
                .and_then(|i| sections.columns(i))
                .map(|c| c.start)
        }
        Step::NextExisting(side) => {
            let mut col = from_column.checked_add(1)?;
            while let Some((i, start)) = store.at_column(col) {
                let seg = store.get(i)?;
                if holds(seg.kind, side) {
                    return Some(col);
                }
                col = start + seg.len;
            }
            None
        }
        Step::PrevExisting(side) => {
            let mut col = from_column.min(columns).checked_sub(1)?;
            loop {
                let (i, start) = store.at_column(col)?;
                let seg = store.get(i)?;
                if holds(seg.kind, side) {
                    return Some(col);
                }
                col = start.checked_sub(1)?;
            }
        }
    }
}

fn holds(kind: SegmentKind, side: Side) -> bool {
    match kind {
        SegmentKind::Paired => true,
        SegmentKind::LeftOnly => side != Side::Right,
        SegmentKind::RightOnly => side == Side::Right,
    }
}

/// `NextDifference` past a truncated section list: the first non-same column after `from_column` (a one-sided
/// segment's first column, or a differing paired byte found by reading), scanning forward through the visitor
/// (cancellable; run it in a job).
///
/// # Errors
///
/// A read error, [`HexError::Truncated`], cancellation.
pub fn scan_next_difference(
    left: &HexDocument,
    right: &HexDocument,
    alignment: &HexAlignment,
    from_column: u64,
    stream: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<Option<u64>, HexError> {
    let store = &alignment.segments;
    let mut col = from_column.saturating_add(1);
    while let Some((i, start)) = store.at_column(col) {
        let Some(seg) = store.get(i) else {
            break;
        };
        if seg.kind != SegmentKind::Paired {
            return Ok(Some(col));
        }
        let within = col - start;
        let (l, r, len) = (seg.left + within, seg.right + within, seg.len - within);
        let found = if l == r {
            let f =
                |offset: u64, a: &[u8], b: &[u8]| match a.iter().zip(b).position(|(x, y)| x != y) {
                    Some(p) => ControlFlow::Break(Some(offset + p as u64)),
                    None => ControlFlow::Continue(None),
                };
            let visit =
                visit_chunk_pairs(left.source(), right.source(), l..l + len, stream, ctl, &f)
                    .map_err(|e| HexError::from_vfs(left.display(), e))?;
            visit.values.last().copied().flatten().map(|at| at - l)
        } else {
            first_difference_by_reads(left, l, right, r, len, ctl)?
        };
        if let Some(d) = found {
            return Ok(Some(col + d));
        }
        col = start + seg.len;
    }
    Ok(None)
}

/// The first differing byte of `left[l..l+len)` and `right[r..r+len)` by positional reads of 1 MiB.
fn first_difference_by_reads(
    left: &HexDocument,
    l: u64,
    right: &HexDocument,
    r: u64,
    len: u64,
    ctl: &Ctl<'_>,
) -> Result<Option<u64>, HexError> {
    const STEP: u64 = 1 << 20;
    let mut done = 0;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    while done < len {
        ctl.check()?;
        let n = STEP.min(len - done);
        a.resize(n as usize, 0);
        b.resize(n as usize, 0);
        left.read_bulk(l + done, &mut a)?;
        right.read_bulk(r + done, &mut b)?;
        if let Some(p) = a.iter().zip(&b).position(|(x, y)| x != y) {
            return Ok(Some(done + p as u64));
        }
        done += n;
    }
    Ok(None)
}

/// A parsed Go To address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressQuery {
    /// An offset.
    Absolute(u64),
    /// Relative to the caret (`+n`, `-n`).
    Relative(i128),
    /// The last byte (`end`).
    End,
}

impl AddressQuery {
    /// The byte offset for a caret at `caret` in a file of `len` bytes, and a warning text when the address was
    /// past the end (or before the start) and was clamped to the last (first) byte. `None` for an empty file.
    pub fn resolve(self, caret: u64, len: u64) -> Option<(u64, Option<String>)> {
        let last = len.checked_sub(1)?;
        let wanted: i128 = match self {
            AddressQuery::Absolute(n) => i128::from(n),
            AddressQuery::Relative(d) => i128::from(caret) + d,
            AddressQuery::End => i128::from(last),
        };
        if wanted < 0 {
            return Some((
                0,
                Some("the address is before the start: the first byte".into()),
            ));
        }
        if wanted > i128::from(last) {
            return Some((
                last,
                Some(format!(
                    "the address is past the end ({len} bytes): the last byte"
                )),
            ));
        }
        Some((wanted as u64, None))
    }
}

/// Parses a Go To address: `0x1F`, `1Fh` and `$1F` are hex; `#31` and `31d` decimal; bare digits in `default`
/// (the dialog's radix, default hex; the CLI's `--offset` passes decimal); `+n`/`-n` are relative to the caret
/// (with the same spellings); `end`. `_` separators are allowed (the address text copies with them).
///
/// # Errors
///
/// [`VocabError`] for empty, malformed or overflowing text.
pub fn parse_address(text: &str, default: Radix) -> Result<AddressQuery, VocabError> {
    let expected = "an address: 0x1F, 1Fh, $1F, #31, 31d, digits, +n, -n or end";
    let err = || VocabError::new("address", text, expected);
    let t = text.trim();
    if t.eq_ignore_ascii_case("end") {
        return Ok(AddressQuery::End);
    }
    let (sign, body) = match t.as_bytes().first() {
        Some(b'+') => (Some(1i128), &t[1..]),
        Some(b'-') => (Some(-1i128), &t[1..]),
        _ => (None, t),
    };
    let body: String = body.trim().chars().filter(|&c| c != '_').collect();
    let lower = body.to_ascii_lowercase();
    let (digits, radix) = if let Some(rest) = lower.strip_prefix("0x") {
        (rest, 16)
    } else if let Some(rest) = lower.strip_prefix('$') {
        (rest, 16)
    } else if let Some(rest) = lower.strip_prefix('#') {
        (rest, 10)
    } else if let Some(rest) = lower.strip_suffix('h') {
        (rest, 16)
    } else if let Some(rest) = lower.strip_suffix('d')
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        (rest, 10)
    } else {
        (
            lower.as_str(),
            match default {
                Radix::Hex => 16,
                Radix::Decimal => 10,
            },
        )
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return Err(err());
    }
    let n = u64::from_str_radix(digits, radix).map_err(|_| err())?;
    Ok(match sign {
        Some(s) => AddressQuery::Relative(s * i128::from(n)),
        None => AddressQuery::Absolute(n),
    })
}

/// The Current Byte Address text (HX-007) W13 shows and copies: hex `0x0000_1F40` (`width` digits, at least the
/// offset's, with `_` every 4 digits from the right), decimal plain. The same text for both sides.
pub fn address_text(_side: Side, offset: u64, radix: Radix, width: u8) -> String {
    match radix {
        Radix::Decimal => offset.to_string(),
        Radix::Hex => {
            let digits = format!("{offset:0width$X}", width = usize::from(width));
            let mut out = String::with_capacity(digits.len() + digits.len() / 4 + 2);
            out.push_str("0x");
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(4) {
                    out.push('_');
                }
                out.push(c);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_spellings() {
        use AddressQuery::*;
        let hex = Radix::Hex;
        for (text, q) in [
            ("0x1F", Absolute(31)),
            ("1Fh", Absolute(31)),
            ("$1f", Absolute(31)),
            ("#31", Absolute(31)),
            ("31d", Absolute(31)),
            ("31", Absolute(0x31)),
            ("dead", Absolute(0xDEAD)),
            ("0x0000_1F40", Absolute(0x1F40)),
            ("+10", Relative(16)),
            ("-#10", Relative(-10)),
            ("END", End),
        ] {
            assert_eq!(parse_address(text, hex), Ok(q), "{text}");
        }
        assert_eq!(parse_address("31", Radix::Decimal), Ok(Absolute(31)));
        for bad in [
            "",
            "0x",
            "xyz",
            "12g",
            "0x1_0000_0000_0000_0000",
            "+",
            "1.5",
        ] {
            assert!(parse_address(bad, hex).is_err(), "{bad}");
        }
        assert_eq!(Absolute(5).resolve(0, 10), Some((5, None)));
        assert_eq!(Absolute(50).resolve(0, 10).unwrap().0, 9);
        assert!(Absolute(50).resolve(0, 10).unwrap().1.is_some());
        assert_eq!(Relative(-3).resolve(1, 10).unwrap().0, 0);
        assert_eq!(End.resolve(3, 10), Some((9, None)));
        assert_eq!(End.resolve(3, 0), None);
    }

    #[test]
    fn address_texts() {
        assert_eq!(
            address_text(Side::Left, 0x1F40, Radix::Hex, 8),
            "0x0000_1F40"
        );
        assert_eq!(
            address_text(Side::Left, 0x1F40, Radix::Hex, 9),
            "0x0_0000_1F40"
        );
        assert_eq!(
            address_text(Side::Right, 0x12_3456_789A, Radix::Hex, 8),
            "0x12_3456_789A"
        );
        assert_eq!(address_text(Side::Left, 8000, Radix::Decimal, 8), "8000");
    }
}
