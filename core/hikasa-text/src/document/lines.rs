//! Hikasa's own line table (§1.5.2, INV-DOC-1/2): lines end only at CR, LF and CRLF (`memchr`), never at the
//! Unicode separators that ropey's default line indexing also breaks at; the characters-per-line limit
//! (TC-003) splits long lines with artificial breaks; the D-43 line-count guard runs before anything is
//! allocated (counting the artificial pieces too), and the scan checks cancellation every 1 MiB (§5.3).

use hikasa_core::cancel::CancelToken;

use crate::document::{EolSummary, LineEnding};
use crate::error::TextError;

/// Bytes scanned between two cancellation checks while a document is built (§5.3: every 1 MiB).
pub(crate) const BUILD_STEP: usize = 1 << 20;

/// Most lines a document may have (D-43): `hikasa_diff::MAX_SEQUENCE_LEN`.
pub const MAX_LINES: u32 = hikasa_diff::MAX_SEQUENCE_LEN;

/// The line table of one decoded text: 4 bytes of start offset and 1 byte of terminator kind per line, plus
/// the indices of artificially broken lines (only when the characters-per-line limit wrapped something).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LineTable {
    /// Start byte of every line in the decoded text.
    starts: Vec<u32>,
    /// Terminator of every line.
    eols: Vec<LineEnding>,
    /// Length of the decoded text in bytes.
    text_len: u32,
    /// Sorted indices of the lines whose terminator is `Artificial` (for `physical_line`).
    artificial: Vec<u32>,
}

/// Byte length of a terminator in the text (artificial breaks have none).
#[inline]
pub(crate) fn eol_len(eol: LineEnding) -> u32 {
    match eol {
        LineEnding::Lf | LineEnding::Cr => 1,
        LineEnding::CrLf => 2,
        LineEnding::None | LineEnding::Artificial => 0,
    }
}

/// The D-43 line-count guard: more than [`MAX_LINES`] lines → `TooManyLines`.
pub(crate) fn check_line_count(lines: u64, spec: &str) -> Result<(), TextError> {
    if lines > u64::from(MAX_LINES) {
        Err(TextError::TooManyLines {
            spec: spec.to_owned(),
            lines,
            limit: u64::from(MAX_LINES),
        })
    } else {
        Ok(())
    }
}

/// Text longer than 4 GiB − 1 bytes cannot be indexed with `u32` offsets (D-43).
pub(crate) fn check_text_len(len: usize, spec: &str) -> Result<u32, TextError> {
    u32::try_from(len).map_err(|_| TextError::TooLarge {
        spec: spec.to_owned(),
        limit: u64::from(u32::MAX),
    })
}

impl LineTable {
    /// Splits the UTF-8 text `text` at CR, LF and CRLF, then applies the characters-per-line limit `cpl` (`None`
    /// or `Some(0)`: no limit). Fails with `TooLarge` for text over 4 GiB − 1 bytes (or when the table cannot be
    /// allocated) and with `TooManyLines` beyond [`MAX_LINES`] lines, artificial pieces included (D-43), before
    /// the table is allocated; `spec` names the document in those errors. Cancellation is checked every 1 MiB.
    pub(crate) fn build(
        text: &[u8],
        cpl: Option<u32>,
        spec: &str,
        cancel: &CancelToken,
    ) -> Result<(LineTable, EolSummary), TextError> {
        let text_len = check_text_len(text.len(), spec)?;
        let bytes = text;
        let limit = cpl.unwrap_or(0) as usize;
        // The D-43 guard, before any allocation: terminators bound the real lines (CRLF counts twice, so the
        // exact count is taken before failing); with a limit, the pieces of every wrapped line are counted.
        let estimate = if limit > 0 {
            let total = count_pieces(bytes, limit, cancel)?;
            check_line_count(total, spec)?;
            total as usize
        } else {
            let lines = count_lines(bytes, cancel)?;
            check_line_count(lines, spec)?;
            lines as usize
        };
        let too_large = || TextError::TooLarge {
            spec: spec.to_owned(),
            limit: u64::from(u32::MAX),
        };
        let mut table = LineTable {
            starts: Vec::new(),
            eols: Vec::new(),
            text_len,
            artificial: Vec::new(),
        };
        table
            .starts
            .try_reserve_exact(estimate)
            .map_err(|_| too_large())?;
        table
            .eols
            .try_reserve_exact(estimate)
            .map_err(|_| too_large())?;
        let mut summary = EolSummary::default();
        let mut start = 0usize;
        let mut at = 0usize;
        let mut next_check = BUILD_STEP;
        while let Some(off) = memchr::memchr2(b'\n', b'\r', &bytes[at..]) {
            let p = at + off;
            let (eol, next) = if bytes[p] == b'\n' {
                (LineEnding::Lf, p + 1)
            } else if bytes.get(p + 1) == Some(&b'\n') {
                (LineEnding::CrLf, p + 2)
            } else {
                (LineEnding::Cr, p + 1)
            };
            table.push_line(bytes, start, p, eol, limit);
            summary.count(eol);
            start = next;
            at = next;
            if at >= next_check {
                if cancel.is_cancelled() {
                    crate::util::release_later(table);
                    return Err(TextError::Cancelled);
                }
                next_check = at + BUILD_STEP;
            }
        }
        if start < bytes.len() {
            table.push_line(bytes, start, bytes.len(), LineEnding::None, limit);
            summary.count(LineEnding::None);
        }
        check_line_count(table.starts.len() as u64, spec)?;
        if table.starts.capacity() > table.starts.len() + table.starts.len() / 4 + 16 {
            table.starts.shrink_to_fit();
            table.eols.shrink_to_fit();
        }
        Ok((table, summary))
    }

    /// Appends the line `text[start..end]` with terminator `eol`, wrapped at `limit` chars (0: no limit).
    fn push_line(&mut self, text: &[u8], start: usize, end: usize, eol: LineEnding, limit: usize) {
        // A line of at most `limit` bytes has at most `limit` chars.
        if limit > 0 && end - start > limit {
            let mut piece_start = start;
            let mut chars = 0usize;
            for (p, &b) in text.iter().enumerate().take(end).skip(start) {
                if is_continuation(b) {
                    continue;
                }
                if chars == limit {
                    self.push_raw(piece_start, LineEnding::Artificial);
                    piece_start = p;
                    chars = 0;
                }
                chars += 1;
            }
            self.push_raw(piece_start, eol);
        } else {
            self.push_raw(start, eol);
        }
    }

    fn push_raw(&mut self, start: usize, eol: LineEnding) {
        if eol == LineEnding::Artificial {
            self.artificial.push(self.starts.len() as u32);
        }
        // `start ≤ text_len ≤ u32::MAX` was checked in `build`.
        self.starts.push(start as u32);
        self.eols.push(eol);
    }

    /// Number of lines.
    #[inline]
    pub(crate) fn len(&self) -> u32 {
        self.starts.len() as u32
    }

    /// Byte range of the content of line `idx` (without its terminator); `None` out of range.
    #[inline]
    pub(crate) fn content(&self, idx: u32) -> Option<(usize, usize)> {
        let i = idx as usize;
        let start = *self.starts.get(i)?;
        let next = self.starts.get(i + 1).copied().unwrap_or(self.text_len);
        let eol = self.eols.get(i).copied().unwrap_or(LineEnding::None);
        Some((start as usize, (next - eol_len(eol)) as usize))
    }

    /// Terminator of line `idx` (`None` out of range).
    #[inline]
    pub(crate) fn eol(&self, idx: u32) -> LineEnding {
        self.eols
            .get(idx as usize)
            .copied()
            .unwrap_or(LineEnding::None)
    }

    /// 1-based physical line number of `idx` (0 out of range).
    pub(crate) fn physical(&self, idx: u32) -> u32 {
        if idx >= self.len() {
            return 0;
        }
        let artificial_before = self.artificial.partition_point(|&a| a < idx) as u32;
        idx + 1 - artificial_before
    }

    /// Bytes used by the table (memory accounting in tests).
    #[cfg(test)]
    pub(crate) fn heap_bytes(&self) -> usize {
        self.starts.capacity() * 4 + self.eols.capacity() + self.artificial.capacity() * 4
    }
}

/// A UTF-8 continuation byte (not the first byte of a char).
#[inline]
fn is_continuation(b: u8) -> bool {
    b & 0xC0 == 0x80
}

/// Number of table entries of `bytes` with a characters-per-line limit of `limit` (> 0): every line counts
/// `max(1, ⌈chars / limit⌉)` (the D-43 guard of wrapped text). Cancellation is checked every 1 MiB.
fn count_pieces(bytes: &[u8], limit: usize, cancel: &CancelToken) -> Result<u64, TextError> {
    let mut total = 0u64;
    let mut chars = 0usize;
    let mut in_line = false;
    let mut p = 0usize;
    let mut next_check = 0usize;
    while p < bytes.len() {
        if p >= next_check {
            cancel.check()?;
            next_check = p + BUILD_STEP;
        }
        let b = bytes[p];
        if b == b'\n' || b == b'\r' {
            total += (chars.div_ceil(limit)).max(1) as u64;
            chars = 0;
            in_line = false;
            p += if b == b'\r' && bytes.get(p + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
            continue;
        }
        in_line = true;
        if !is_continuation(b) {
            chars += 1;
        }
        p += 1;
    }
    if in_line {
        total += (chars.div_ceil(limit)).max(1) as u64;
    }
    Ok(total)
}

/// Exact number of lines of `bytes` (CRLF counts once; a final unterminated line counts), counted in 1 MiB
/// steps with cancellation checks (the D-43 guard runs before the table is allocated, §5.3).
fn count_lines(bytes: &[u8], cancel: &CancelToken) -> Result<u64, TextError> {
    let mut lines = 0u64;
    let mut after_last = 0usize;
    for (k, block) in bytes.chunks(BUILD_STEP).enumerate() {
        if k > 0 {
            cancel.check()?;
        }
        let base = k * BUILD_STEP;
        for off in memchr::memchr2_iter(b'\n', b'\r', block) {
            let p = base + off;
            after_last = p + 1;
            if bytes[p] == b'\n' && p > 0 && bytes[p - 1] == b'\r' {
                continue;
            }
            lines += 1;
        }
    }
    if after_last < bytes.len() {
        lines += 1;
    }
    Ok(lines)
}

impl EolSummary {
    /// Counts one terminator (`Artificial` breaks are not counted).
    pub(crate) fn count(&mut self, eol: LineEnding) {
        match eol {
            LineEnding::Lf => self.lf += 1,
            LineEnding::CrLf => self.crlf += 1,
            LineEnding::Cr => self.cr += 1,
            LineEnding::None => self.none += 1,
            LineEnding::Artificial => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::EolStyle;

    /// D-43 before wrapping: the pieces of a wrapped line count against the line limit before anything is
    /// allocated (tested on the counter, not on a 2 GiB buffer).
    #[test]
    fn wrapped_pieces_are_counted_before_allocation() {
        let never = CancelToken::never();
        assert_eq!(
            count_pieces(b"abcdefg\nxy\n\n", 3, &never).unwrap(),
            3 + 1 + 1
        );
        assert_eq!(count_pieces("éèêëa".as_bytes(), 2, &never).unwrap(), 3);
        assert_eq!(count_pieces(b"a\r\nb\rc", 1, &never).unwrap(), 3);
        assert_eq!(count_pieces(b"", 1, &never).unwrap(), 0);
        for (text, limit) in [("abcdefg\nxy\néèêëa\n", 3u32), ("x\r\n\ryy", 1), ("abc", 3)] {
            let (table, _) = LineTable::build(text.as_bytes(), Some(limit), "t", &never).unwrap();
            assert_eq!(
                u64::from(table.len()),
                count_pieces(text.as_bytes(), limit as usize, &never).unwrap(),
                "{text:?}"
            );
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            LineTable::build(b"a\nb", None, "t", &cancel),
            Ok(_) | Err(TextError::Cancelled)
        ));
        assert!(matches!(
            count_pieces(b"abc", 1, &cancel),
            Err(TextError::Cancelled)
        ));
    }

    fn split(text: &str) -> Vec<(String, LineEnding)> {
        let (table, _) =
            LineTable::build(text.as_bytes(), None, "t", &CancelToken::never()).unwrap();
        (0..table.len())
            .map(|i| {
                let (s, e) = table.content(i).unwrap();
                (text[s..e].to_owned(), table.eol(i))
            })
            .collect()
    }

    #[test]
    fn splitting() {
        assert!(split("").is_empty());
        assert_eq!(split("\n"), [("".to_owned(), LineEnding::Lf)]);
        assert_eq!(split("a"), [("a".to_owned(), LineEnding::None)]);
        assert_eq!(
            split("a\r\nb\rc\n"),
            [
                ("a".to_owned(), LineEnding::CrLf),
                ("b".to_owned(), LineEnding::Cr),
                ("c".to_owned(), LineEnding::Lf)
            ]
        );
        assert_eq!(
            split("\r\r\n"),
            [
                ("".to_owned(), LineEnding::Cr),
                ("".to_owned(), LineEnding::CrLf)
            ]
        );
        assert_eq!(
            split("x\ny"),
            [
                ("x".to_owned(), LineEnding::Lf),
                ("y".to_owned(), LineEnding::None)
            ]
        );
        // Unicode separators are not line breaks (INV-DOC-1).
        assert_eq!(
            split("a\u{2028}b\u{85}c\u{0B}d\u{0C}e"),
            [(
                "a\u{2028}b\u{85}c\u{0B}d\u{0C}e".to_owned(),
                LineEnding::None
            )]
        );
    }

    #[test]
    fn line_counts_match_the_exact_count() {
        for text in [
            "",
            "\n",
            "a",
            "a\r\nb\rc\n",
            "\r\r\n",
            "x\ny",
            "\r",
            "\r\n\r\n",
            "a\rb",
        ] {
            let (table, _) =
                LineTable::build(text.as_bytes(), None, "t", &CancelToken::never()).unwrap();
            assert_eq!(
                u64::from(table.len()),
                count_lines(text.as_bytes(), &CancelToken::never()).unwrap(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn eol_summary_and_style() {
        let (_, s) =
            LineTable::build("a\nb\nc".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(
            s,
            EolSummary {
                lf: 2,
                crlf: 0,
                cr: 0,
                none: 1
            }
        );
        assert_eq!(s.style(), EolStyle::Lf);
        assert_eq!(s.dominant(), LineEnding::Lf);
        let (_, s) =
            LineTable::build("a\r\nb\nc\r\n".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.style(), EolStyle::Mixed);
        assert_eq!(s.dominant(), LineEnding::CrLf);
        let (_, s) = LineTable::build("abc".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.style(), EolStyle::None);
        let (_, s) = LineTable::build("".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.style(), EolStyle::None);
        let (_, s) =
            LineTable::build("a\rb\r".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.style(), EolStyle::Cr);
        // Ties: Lf > CrLf > Cr.
        let (_, s) =
            LineTable::build("a\nb\r\nc\r".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.dominant(), LineEnding::Lf);
        let (_, s) =
            LineTable::build("b\r\nc\r".as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert_eq!(s.dominant(), LineEnding::CrLf);
    }

    #[test]
    fn chars_per_line_wrapping() {
        let text = "abcdefg\nxy\néèêëa\n";
        let (table, summary) =
            LineTable::build(text.as_bytes(), Some(3), "t", &CancelToken::never()).unwrap();
        let lines: Vec<(&str, LineEnding, u32)> = (0..table.len())
            .map(|i| {
                let (s, e) = table.content(i).unwrap();
                (&text[s..e], table.eol(i), table.physical(i))
            })
            .collect();
        assert_eq!(
            lines,
            [
                ("abc", LineEnding::Artificial, 1),
                ("def", LineEnding::Artificial, 1),
                ("g", LineEnding::Lf, 1),
                ("xy", LineEnding::Lf, 2),
                ("éèê", LineEnding::Artificial, 3),
                ("ëa", LineEnding::Lf, 3),
            ]
        );
        // Artificial breaks are not counted.
        assert_eq!(summary.lf, 3);
        // A limit of 0 disables wrapping; an exact fit is not wrapped.
        let (table, _) =
            LineTable::build("abc\n".as_bytes(), Some(0), "t", &CancelToken::never()).unwrap();
        assert_eq!(table.len(), 1);
        let (table, _) =
            LineTable::build("abc".as_bytes(), Some(3), "t", &CancelToken::never()).unwrap();
        assert_eq!(table.len(), 1);
        assert_eq!(table.physical(0), 1);
        assert_eq!(table.physical(7), 0);
    }

    #[test]
    fn the_line_count_guard() {
        assert!(check_line_count(u64::from(MAX_LINES), "x").is_ok());
        let err = check_line_count(u64::from(MAX_LINES) + 1, "x").unwrap_err();
        assert!(matches!(
            err,
            TextError::TooManyLines { lines, limit, .. }
                if lines == u64::from(MAX_LINES) + 1 && limit == u64::from(MAX_LINES)
        ));
        assert_eq!(MAX_LINES, i32::MAX as u32 - 2);
        assert!(check_text_len(u32::MAX as usize, "x").is_ok());
        #[cfg(target_pointer_width = "64")]
        assert!(matches!(
            check_text_len(u32::MAX as usize + 1, "x"),
            Err(TextError::TooLarge { .. })
        ));
    }

    #[test]
    fn at_most_one_unterminated_line_and_it_is_last() {
        for text in ["a\nb", "a\n", "", "x", "\r\n\r"] {
            let (table, _) =
                LineTable::build(text.as_bytes(), Some(1), "t", &CancelToken::never()).unwrap();
            let nones: Vec<u32> = (0..table.len())
                .filter(|&i| table.eol(i) == LineEnding::None)
                .collect();
            assert!(nones.len() <= 1);
            if let Some(&i) = nones.first() {
                assert_eq!(i + 1, table.len());
            }
        }
    }

    #[test]
    fn table_memory_is_five_bytes_per_line() {
        let text = "a\n".repeat(10_000);
        let (table, _) =
            LineTable::build(text.as_bytes(), None, "t", &CancelToken::never()).unwrap();
        assert!(
            table.heap_bytes() <= 10_001 * 5 + 64,
            "{}",
            table.heap_bytes()
        );
    }
}
