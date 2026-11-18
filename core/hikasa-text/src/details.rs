//! Details data (TC-033; D-287; contract §1.10.20): [`hex_details`] (the bytes a save would write for a line),
//! [`char_alignment`] and [`char_alignment_of_row`] (Alignment Details: the characters of two lines aligned by M1's
//! inline engine and classified as the row's inline ranges are, so the colours equal the panes') and [`ruler`]. Text
//! Details is an editor on the same `EditableDocument` (W11). Pure.

use std::ops::Range;

use hikasa_diff::{InlineDiff, InlineOptions, inline_diff};
use hikasa_formats::{Encoder, TextEncoding, unmappable_in};
use serde::{Deserialize, Serialize};

use crate::compare::{InlineRange, RowIdx, RowState, TextComparison};
use crate::document::LineIdx;
use crate::edit::EditableDocument;

/// Hex Details of one line (TC-033).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexDetails {
    /// The line's text encoded with the document's save encoding (capped at `max_bytes`).
    pub bytes: Vec<u8>,
    /// The encoding.
    pub encoding: TextEncoding,
    /// The terminator's bytes in that encoding (empty for the last line without one and for a CPL piece).
    pub eol: Vec<u8>,
    /// `bytes` was cut at `max_bytes`.
    pub truncated: bool,
    /// Byte ranges of `bytes` written as `?` because the encoding cannot write their character.
    pub unmappable: Vec<Range<u32>>,
}

/// The bytes of line `line` as a save would write it in the document's encoding (an unchanged line: its bytes in the
/// file, except malformed bytes and the characters an encoding can write in two ways), then its terminator's; capped
/// at `max_bytes` (W11: 4,096). A stateful encoding (ISO-2022-JP) starts in its initial state for the line.
pub fn hex_details(doc: &EditableDocument, line: LineIdx, max_bytes: u32) -> HexDetails {
    let (encoding, _bom) = doc.encoding();
    let text = doc.line(line);
    let unmappable_chars: Vec<usize> = unmappable_in(&text, encoding)
        .into_iter()
        .map(|(i, _)| i)
        .collect();
    let mut encoder = Encoder::new(encoding);
    let mut bytes = Vec::with_capacity(text.len());
    let mut unmappable = Vec::new();
    let mut next = unmappable_chars.iter().peekable();
    // Encoded per character run: an unmappable character's `?` is located exactly.
    let mut run_start = 0usize;
    for (ci, (bi, c)) in text.char_indices().enumerate() {
        if next.peek() == Some(&&ci) {
            next.next();
            encoder.encode(&text[run_start..bi], false, &mut bytes);
            let at = bytes.len() as u32;
            encoder.encode(&text[bi..bi + c.len_utf8()], false, &mut bytes);
            unmappable.push(at..bytes.len() as u32);
            run_start = bi + c.len_utf8();
        }
    }
    let eol_text = doc.line_ending(line).as_text();
    encoder.encode(&text[run_start..], eol_text.is_empty(), &mut bytes);
    let mut eol = Vec::new();
    if !eol_text.is_empty() {
        encoder.encode(eol_text, true, &mut eol);
    }
    let cap = max_bytes as usize;
    let truncated = bytes.len() > cap;
    if truncated {
        bytes.truncate(cap);
        unmappable.retain(|r| (r.start as usize) < cap);
        for r in &mut unmappable {
            r.end = r.end.min(max_bytes);
        }
    }
    HexDetails {
        bytes,
        encoding,
        eol,
        truncated,
        unmappable,
    }
}

/// The class of one aligned character pair (TC-033's colours).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellClass {
    /// The same character on both sides.
    Same,
    /// Different characters in an important difference.
    Changed,
    /// Characters in an unimportant difference.
    Unimportant,
    /// A character of the left line only.
    LeftOnly,
    /// A character of the right line only.
    RightOnly,
}

/// One column of Alignment Details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CharCell {
    /// The left character.
    pub left: Option<char>,
    /// The right character.
    pub right: Option<char>,
    /// Its class.
    pub class: CellClass,
}

/// Alignment Details of two texts with every difference important: M1's inline engine with `opts`.
pub fn char_alignment(left: &str, right: &str, opts: &InlineOptions) -> Vec<CharCell> {
    let diff = inline_diff(left, right, opts);
    let mark = |ranges: &[Range<u32>]| -> Vec<InlineRange> {
        ranges
            .iter()
            .map(|r| InlineRange {
                range: r.clone(),
                important: true,
            })
            .collect()
    };
    cells(left, right, &mark(&diff.left), &mark(&diff.right))
}

/// Alignment Details of a row of a comparison: its two lines and their classified inline ranges
/// ([`TextComparison::inline`]), so the classes equal the panes' colours. A same row is all `Same`; a one-sided row
/// is all `LeftOnly` or `RightOnly`.
pub fn char_alignment_of_row(cmp: &TextComparison, row: RowIdx) -> Option<Vec<CharCell>> {
    let r = cmp.row(row)?;
    let text = |side, line: Option<LineIdx>| line.map(|l| cmp.document(side).line(l).into_owned());
    let left = text(hikasa_core::Side::Left, r.left);
    let right = text(hikasa_core::Side::Right, r.right);
    match (left, right) {
        (Some(l), Some(rt)) if r.state == RowState::Same => Some(
            l.chars()
                .zip(rt.chars())
                .map(|(a, b)| CharCell {
                    left: Some(a),
                    right: Some(b),
                    class: CellClass::Same,
                })
                .collect(),
        ),
        (Some(l), Some(rt)) => {
            let inline = cmp.inline(row).unwrap_or_default();
            Some(cells(&l, &rt, &inline.left, &inline.right))
        }
        (Some(l), None) => Some(
            l.chars()
                .map(|c| CharCell {
                    left: Some(c),
                    right: None,
                    class: CellClass::LeftOnly,
                })
                .collect(),
        ),
        (None, Some(rt)) => Some(
            rt.chars()
                .map(|c| CharCell {
                    left: None,
                    right: Some(c),
                    class: CellClass::RightOnly,
                })
                .collect(),
        ),
        (None, None) => None,
    }
}

/// One side split by its ranges: the common characters, and each range as (common characters before it, its
/// characters, important).
type Split = (Vec<char>, Vec<(usize, Vec<char>, bool)>);

fn split(text: &str, ranges: &[InlineRange]) -> Split {
    let mut common = Vec::new();
    let mut hunks = Vec::new();
    let mut at = 0usize;
    for x in ranges {
        let (s, e) = (x.range.start as usize, x.range.end as usize);
        common.extend(text.get(at..s).unwrap_or("").chars());
        hunks.push((
            common.len(),
            text.get(s..e).unwrap_or("").chars().collect(),
            x.important,
        ));
        at = e.max(at);
    }
    common.extend(text.get(at..).unwrap_or("").chars());
    (common, hunks)
}

/// Aligns the characters: outside the ranges the common text pairs one to one (`Same`); the ranges at the same offset
/// of the common text form a hunk whose characters pair by position (`Changed`, or `Unimportant` when no range of it
/// is important), the longer side's rest one-sided.
fn cells(left: &str, right: &str, l: &[InlineRange], r: &[InlineRange]) -> Vec<CharCell> {
    let (lc, lh) = split(left, l);
    let (rc, rh) = split(right, r);
    let mut out = Vec::with_capacity(lc.len().max(rc.len()));
    let (mut i, mut j) = (0usize, 0usize);
    let n = lc.len().max(rc.len());
    for k in 0..=n {
        let mut a: Vec<char> = Vec::new();
        let mut b: Vec<char> = Vec::new();
        let mut important = false;
        while let Some(h) = lh.get(i).filter(|h| h.0 == k) {
            a.extend(&h.1);
            important |= h.2;
            i += 1;
        }
        while let Some(h) = rh.get(j).filter(|h| h.0 == k) {
            b.extend(&h.1);
            important |= h.2;
            j += 1;
        }
        for x in 0..a.len().max(b.len()) {
            let (p, q) = (a.get(x).copied(), b.get(x).copied());
            let class = match (p, q) {
                (Some(_), Some(_)) if important => CellClass::Changed,
                (Some(_), Some(_)) => CellClass::Unimportant,
                (Some(_), None) => CellClass::LeftOnly,
                _ => CellClass::RightOnly,
            };
            out.push(CharCell {
                left: p,
                right: q,
                class,
            });
        }
        if k < n {
            let (p, q) = (lc.get(k).copied(), rc.get(k).copied());
            let class = match (p, q) {
                (Some(_), Some(_)) => CellClass::Same,
                (Some(_), None) => CellClass::LeftOnly,
                _ => CellClass::RightOnly,
            };
            out.push(CharCell {
                left: p,
                right: q,
                class,
            });
        }
    }
    out
}

/// One mark of the ruler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RulerMark {
    /// The display column (1-based).
    pub column: u32,
    /// A label every 10 columns.
    pub label: Option<u32>,
    /// A tab stop is here (after a multiple of the tab size).
    pub tab_stop: bool,
}

/// The ruler's marks of display columns `from..=to` (1-based): a tick every column, a label every 10, the tab stops.
pub fn ruler(tab_size: u8, from: u32, to: u32) -> Vec<RulerMark> {
    let ts = u32::from(tab_size.max(1));
    (from.max(1)..=to)
        .map(|column| RulerMark {
            column,
            label: (column % 10 == 0).then_some(column),
            tab_stop: (column - 1) % ts == 0,
        })
        .collect()
}

/// The inline difference of two texts with the default options (Alignment Details without a comparison).
pub fn inline_of(a: &str, b: &str) -> InlineDiff {
    inline_diff(a, b, &InlineOptions::DEFAULT)
}
