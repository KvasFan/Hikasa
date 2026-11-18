//! Copy of a selection as text (HX-001, W13a; D-357): [`selection_range`], [`selection_text`], [`CopyForm`].
//!
//! A selection is a range of aligned columns; the bytes of one side in it are consecutive offsets of that side's file
//! (a gap column holds no byte of the side and contributes nothing). Copy writes them as hex pairs (`de ad be ef`, the
//! form `parse_hex_bytes` and the Find panel's hex field accept) or as the characters column's text. Pure over the
//! document: reads go through its page cache.

use std::ops::Range;

use hikasa_core::Side;

use crate::align::SegmentStore;
use crate::document::HexDocument;
use crate::error::HexError;
use crate::vocab::CharSet;

/// The most bytes Copy takes by default (16 MiB, W13a open default).
pub const COPY_LIMIT: u64 = 16 << 20;

/// What Copy writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CopyForm {
    /// Two lower-case hex digits a byte, one space between bytes (`de ad be ef`).
    Hex,
    /// The characters column's text under a character set (`CharSet::None` copies as ASCII: *Copy as Text* always
    /// copies characters).
    Chars(CharSet),
}

/// Why a copy gave no text.
#[derive(Debug, thiserror::Error)]
pub enum CopyError {
    /// More bytes than the limit.
    #[error("Copy takes at most {} MiB; select less", limit >> 20)]
    TooLarge {
        /// Bytes selected.
        bytes: u64,
        /// The limit.
        limit: u64,
    },
    /// A read error.
    #[error(transparent)]
    Read(#[from] HexError),
}

/// The byte range of `side` in the aligned `columns` (empty when they hold only gaps of that side).
pub fn selection_range(store: &SegmentStore, columns: Range<u64>, side: Side) -> Range<u64> {
    let end = columns.end.min(store.columns());
    let start = columns.start.min(end);
    let pick = |(l, r): (u64, u64)| if side == Side::Right { r } else { l };
    pick(store.offsets_at(start))..pick(store.offsets_at(end))
}

/// The text of the bytes of `side` in the aligned `columns` of `doc` (the document of that side) in `form`.
///
/// # Errors
///
/// [`CopyError::TooLarge`] beyond `limit` bytes; a read error.
pub fn selection_text(
    doc: &HexDocument,
    store: &SegmentStore,
    columns: Range<u64>,
    side: Side,
    form: CopyForm,
    limit: u64,
) -> Result<String, CopyError> {
    let range = selection_range(store, columns, side);
    bytes_text(doc, range, form, limit)
}

/// The text of the bytes `range` of `doc` in `form` (at most `limit` bytes).
///
/// # Errors
///
/// As [`selection_text`].
pub fn bytes_text(
    doc: &HexDocument,
    range: Range<u64>,
    form: CopyForm,
    limit: u64,
) -> Result<String, CopyError> {
    match text_with(doc, range, form, limit, |at, buf| Some(doc.read(at, buf))) {
        Some(result) => result,
        // `read` always answers.
        None => Ok(String::new()),
    }
}

/// [`selection_text`] from the page cache only (W13a review: the UI thread never reads a file): `None` when a page of
/// the side's bytes is not cached (nothing is read; the window copies in a job), else what `selection_text` returns.
pub fn selection_text_cached(
    doc: &HexDocument,
    store: &SegmentStore,
    columns: Range<u64>,
    side: Side,
    form: CopyForm,
    limit: u64,
) -> Option<Result<String, CopyError>> {
    let range = selection_range(store, columns, side);
    text_with(doc, range, form, limit, |at, buf| doc.read_cached(at, buf))
}

/// The text of `range` read through `read` (`None` from `read`: not available, no text).
fn text_with(
    doc: &HexDocument,
    range: Range<u64>,
    form: CopyForm,
    limit: u64,
    mut read: impl FnMut(u64, &mut [u8]) -> Option<Result<usize, HexError>>,
) -> Option<Result<String, CopyError>> {
    let len = range.end.saturating_sub(range.start);
    if len > limit {
        return Some(Err(CopyError::TooLarge { bytes: len, limit }));
    }
    let mut out = String::with_capacity(match form {
        CopyForm::Hex => (len as usize).saturating_mul(3),
        CopyForm::Chars(_) => len as usize,
    });
    let chars = match form {
        CopyForm::Chars(CharSet::None) => CharSet::Ascii,
        CopyForm::Chars(set) => set,
        CopyForm::Hex => CharSet::Ascii,
    };
    let layout = crate::rows::HexLayout {
        chars,
        ..crate::rows::HexLayout::default()
    };
    const STEP: u64 = 1 << 20;
    let mut buf = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let n = STEP.min(range.end - at);
        buf.resize(n as usize, 0);
        let got = match read(at, &mut buf)? {
            Ok(got) => got,
            Err(e) => return Some(Err(e.into())),
        };
        if got < buf.len() {
            return Some(Err(HexError::Truncated {
                spec: doc.display().to_owned(),
            }
            .into()));
        }
        for &b in &buf {
            match form {
                CopyForm::Hex => {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push(HEX[usize::from(b >> 4)]);
                    out.push(HEX[usize::from(b & 15)]);
                }
                CopyForm::Chars(_) => out.push(layout.char_of(b).unwrap_or('.')),
            }
        }
        at += n;
    }
    Some(Ok(out))
}

const HEX: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];
