//! The text document model (TC-001…TC-003, §1.5.2): [`TextDocument`], [`LineIdx`], [`LineEnding`],
//! [`EolSummary`], [`EolStyle`], [`DocOrigin`], [`LoadOptions`] and [`MAX_LINES`].
//!
//! A document is immutable: a `ropey::Rope` holds the decoded text (for GUI rendering) and Hikasa's own line
//! table indexes it. Ropey's line indexing is not used, because its default features also break lines at the
//! Unicode separators VT, FF, NEL, LS and PS; Hikasa lines end only at CR, LF and CRLF (INV-DOC-1).

pub(crate) mod blocks;
pub(crate) mod lines;
mod load;

use std::borrow::Cow;
use std::fmt;

use hikasa_core::cancel::CancelToken;
use hikasa_core::spec::Spec;
use hikasa_core::time::FileTime;
use hikasa_formats::{BinaryVerdict, DecodeOptions, DetectedEncoding, LineSource};
use serde::{Deserialize, Serialize};

pub use lines::MAX_LINES;
pub use load::{LoadStamp, load_document};
pub(crate) use load::{document_from_bytes, external_unsupported};

use lines::LineTable;

/// 0-based line index within one document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LineIdx(pub u32);

/// Line terminator of one line (TC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// "\n".
    Lf,
    /// "\r\n".
    CrLf,
    /// "\r" not followed by "\n".
    Cr,
    /// Last line without a terminator.
    None,
    /// Artificial break from the characters-per-line limit (removed on save; TC-003).
    Artificial,
}

impl LineEnding {
    /// The terminator's text (`""` for `None` and `Artificial`).
    pub const fn as_text(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
            LineEnding::None | LineEnding::Artificial => "",
        }
    }

    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "lf",
            LineEnding::CrLf => "cr_lf",
            LineEnding::Cr => "cr",
            LineEnding::None => "none",
            LineEnding::Artificial => "artificial",
        }
    }
}

/// Counts of terminators (`Artificial` breaks are not counted).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EolSummary {
    /// Lines ending in LF.
    pub lf: u64,
    /// Lines ending in CRLF.
    pub crlf: u64,
    /// Lines ending in a lone CR.
    pub cr: u64,
    /// Lines without a terminator (0 or 1: only the last line can lack one).
    pub none: u64,
}

/// Summarised style ("MIX" when inconsistent, TC-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EolStyle {
    /// Every terminated line ends in LF.
    Lf,
    /// Every terminated line ends in CRLF.
    CrLf,
    /// Every terminated line ends in CR.
    Cr,
    /// Several kinds.
    Mixed,
    /// No terminated line.
    None,
}

impl EolStyle {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            EolStyle::Lf => "lf",
            EolStyle::CrLf => "cr_lf",
            EolStyle::Cr => "cr",
            EolStyle::Mixed => "mixed",
            EolStyle::None => "none",
        }
    }
}

impl EolSummary {
    /// The style: the single kind used by all terminated lines; `Mixed` if there are several kinds; `None` if
    /// there are no terminated lines.
    pub fn style(&self) -> EolStyle {
        match (self.lf > 0, self.crlf > 0, self.cr > 0) {
            (false, false, false) => EolStyle::None,
            (true, false, false) => EolStyle::Lf,
            (false, true, false) => EolStyle::CrLf,
            (false, false, true) => EolStyle::Cr,
            _ => EolStyle::Mixed,
        }
    }

    /// The most frequent kind (ties: Lf > CrLf > Cr), used by the Sorted conversion. `Lf` when no line is
    /// terminated.
    pub fn dominant(&self) -> LineEnding {
        if self.lf >= self.crlf && self.lf >= self.cr {
            LineEnding::Lf
        } else if self.crlf >= self.cr {
            LineEnding::CrLf
        } else {
            LineEnding::Cr
        }
    }
}

/// Where a document came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocOrigin {
    /// Spec (None for in-memory test documents).
    pub spec: Option<Spec>,
    /// Modification time of the source.
    pub mtime: Option<FileTime>,
    /// Source size in bytes (before conversion).
    pub size: u64,
}

impl DocOrigin {
    /// The canonical spec text, or `<memory>` for in-memory documents (error messages and warnings).
    pub fn display(&self) -> String {
        self.spec
            .as_ref()
            .map_or_else(|| "<memory>".to_owned(), Spec::canonical)
    }
}

/// Largest source a document may have: 4 GiB − 1 bytes (line tables store `u32` byte offsets; D-43). The
/// default and hard cap of [`LoadOptions::max_bytes`].
pub const MAX_DOCUMENT_BYTES: u64 = u32::MAX as u64;

/// Document load options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadOptions {
    /// Encoding detection and decoding options (incl. the per-pane override, TC-001).
    pub decode: DecodeOptions,
    /// Overrides the format's characters-per-line limit (`Some(0)` disables the limit).
    pub chars_per_line_limit: Option<u32>,
    /// Maximum source size. Default and hard cap: 4 GiB − 1 (line tables store u32 byte offsets).
    pub max_bytes: u64,
}

impl Default for LoadOptions {
    /// Default decoding (UTF-8 locale), the format's characters-per-line limit, [`MAX_DOCUMENT_BYTES`].
    fn default() -> LoadOptions {
        LoadOptions {
            decode: DecodeOptions::default(),
            chars_per_line_limit: None,
            max_bytes: MAX_DOCUMENT_BYTES,
        }
    }
}

impl LoadOptions {
    /// `max_bytes` clamped to the hard cap [`MAX_DOCUMENT_BYTES`].
    pub fn effective_max_bytes(&self) -> u64 {
        self.max_bytes.min(MAX_DOCUMENT_BYTES)
    }
}

/// An immutable decoded text document (TC-001). `Send + Sync`; shared via `Arc`.
/// Storage: a `ropey::Rope` plus Hikasa's own line table. Ropey's built-in line indexing is NOT used
/// because its default features also break lines at Unicode separators (VT, FF, NEL, LS, PS).
/// Hikasa lines end only at CR, LF and CRLF (INV-DOC-1).
#[derive(Clone)]
pub struct TextDocument {
    rope: ropey::Rope,
    lines: LineTable,
    encoding: DetectedEncoding,
    malformed: u64,
    eol: EolSummary,
    binary: BinaryVerdict,
    converted: bool,
    truncated: bool,
    origin: DocOrigin,
    /// What [`load_document`] saw of the file (M2 W10b).
    stamp: Option<LoadStamp>,
    /// The format's `ctrl_z_eof` at the build: a save writes the tail after the Ctrl-Z back (M2 W10b, §1.10.12).
    ctrl_z_eof: bool,
}

impl fmt::Debug for TextDocument {
    /// A summary (never the text).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextDocument")
            .field("origin", &self.origin)
            .field("lines", &self.lines.len())
            .field("bytes", &self.rope.len_bytes())
            .field("encoding", &self.encoding)
            .field("malformed", &self.malformed)
            .field("eol", &self.eol)
            .field("binary", &self.binary)
            .field("converted", &self.converted)
            .field("truncated", &self.truncated)
            .finish()
    }
}

impl TextDocument {
    /// Number of lines (`""` has 0 lines; `"\n"` has 1).
    pub fn line_count(&self) -> u32 {
        self.lines.len()
    }

    /// Line content without its terminator (borrowed when the rope chunk is contiguous). Out-of-range indices
    /// give `""`.
    pub fn line(&self, idx: LineIdx) -> Cow<'_, str> {
        match self.lines.content(idx.0) {
            Some((s, e)) => rope_range(&self.rope, s, e),
            None => Cow::Borrowed(""),
        }
    }

    /// Terminator of `idx` (`None` out of range).
    pub fn line_ending(&self, idx: LineIdx) -> LineEnding {
        self.lines.eol(idx.0)
    }

    /// 1-based physical line number in the source (differs from idx+1 only with CPL wrapping; 0 out of range).
    pub fn physical_line(&self, idx: LineIdx) -> u32 {
        self.lines.physical(idx.0)
    }

    /// Encoding used.
    pub fn encoding(&self) -> &DetectedEncoding {
        &self.encoding
    }

    /// Malformed byte sequences replaced while decoding (0 = lossless).
    pub fn malformed(&self) -> u64 {
        self.malformed
    }

    /// Terminator counts.
    pub fn eol(&self) -> &EolSummary {
        &self.eol
    }

    /// Binary verdict from the raw bytes (TC-003).
    pub fn binary(&self) -> BinaryVerdict {
        self.binary
    }

    /// True when a conversion produced the text (FF-031: read-only).
    pub fn converted(&self) -> bool {
        self.converted
    }

    /// True when [`TextDocument::from_text`] cut text beyond the D-43 limits (the document holds a prefix of the
    /// text it was given; comparisons warn with `lossy_decode`). Always false for [`TextDocument::from_bytes`]
    /// and [`load_document`], which reject such input. Additive (wave 3).
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Marks the document as cut (tests of the truncation warning: real cuts need more than 4 GiB of text).
    #[cfg(test)]
    pub(crate) fn mark_truncated(mut self) -> TextDocument {
        self.truncated = true;
        self
    }

    /// Origin.
    pub fn origin(&self) -> &DocOrigin {
        &self.origin
    }

    /// The stamp [`load_document`] took of the file while it read the bytes (size, time, CRC-32); `None` for a
    /// document built from bytes or text (M2 W10b, §1.10.13).
    pub fn load_stamp(&self) -> Option<LoadStamp> {
        self.stamp
    }

    /// The format's `ctrl_z_eof` this document was built with: an ASCII-compatible text ended at the first Ctrl-Z,
    /// and a save writes the bytes from there back unchanged (M2 W10b, §1.10.12).
    pub fn ctrl_z_eof(&self) -> bool {
        self.ctrl_z_eof
    }

    /// The table lines whose terminator is artificial (the CPL breaks, TC-003), in order (M2 W10b).
    pub(crate) fn artificial_lines(&self) -> &[u32] {
        self.lines.artificial_lines()
    }

    /// The rope (GUI rendering).
    pub fn rope(&self) -> &ropey::Rope {
        &self.rope
    }

    /// Byte range of line `idx`'s content in the rope (`None` out of range). Additive (wave 3): GUIs map rope
    /// positions to lines with it.
    pub fn line_byte_range(&self, idx: LineIdx) -> Option<std::ops::Range<usize>> {
        self.lines.content(idx.0).map(|(s, e)| s..e)
    }

    /// A sequential reader of lines `first..` (each line is borrowed from the rope when contiguous, else
    /// copied into a reused buffer): bulk passes walk the rope's chunks once instead of descending the tree
    /// per line.
    pub(crate) fn cursor(&self) -> LineCursor<'_> {
        LineCursor {
            doc: self,
            chunks: None,
            chunk: "",
            chunk_start: 0,
            buf: String::new(),
        }
    }

    /// The table line holding rope byte `off` (its content or terminator); the line count at or past the end.
    pub(crate) fn line_at_byte(&self, off: usize) -> u32 {
        self.lines
            .line_at_byte(u32::try_from(off).unwrap_or(u32::MAX))
    }

    /// Byte content range of a line, for crate-internal bulk passes.
    pub(crate) fn content_range(&self, idx: u32) -> Option<(usize, usize)> {
        self.lines.content(idx)
    }

    /// Byte length of the content of line `idx` (0 out of range).
    pub(crate) fn line_len(&self, idx: u32) -> usize {
        self.lines
            .content(idx)
            .map_or(0, |(s, e)| e.saturating_sub(s))
    }

    /// The rope chunks of line `idx`'s content, in order, without copying (long lines are compared and copied
    /// piecewise, with cancellation checks between pieces).
    pub(crate) fn line_chunks(&self, idx: u32) -> impl Iterator<Item = &str> + '_ {
        self.lines
            .content(idx)
            .filter(|(s, e)| s < e)
            .and_then(|(s, e)| self.rope.get_byte_slice(s..e))
            .map(|slice| slice.chunks())
            .into_iter()
            .flatten()
    }
}

/// The edit marks of one line (§1.10.5): what [`TextDocument::marks`] returns. Additive (W10a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineMarks {
    /// Generation of the edit that last changed the line's content or terminator (0 = as loaded).
    pub generation: u32,
    /// The line's index in the file as last saved or loaded (`None` = it has none: inserted or changed).
    pub orig: Option<LineIdx>,
}

impl TextDocument {
    /// True when the document is a snapshot of an edited document (an `EditableDocument` changed it at least
    /// once). Additive (W10a, §1.10.2).
    pub fn edited(&self) -> bool {
        self.lines.edited()
    }

    /// The edit marks of `line` (§1.10.5): generation 0 and the line's own index for a document as loaded;
    /// `orig: None` out of range. Additive (W10a).
    pub fn marks(&self, line: LineIdx) -> LineMarks {
        if line.0 >= self.line_count() {
            return LineMarks {
                generation: 0,
                orig: None,
            };
        }
        let (generation, orig) = self.lines.marks(line.0);
        LineMarks {
            generation,
            orig: (orig != blocks::NO_ORIG).then_some(LineIdx(orig)),
        }
    }

    /// Checks the block line table against a fresh build of the document's text with the characters-per-line
    /// limit `cpl` (P-BLOCK, A10.1): equal starts, terminators, physical lines and artificial breaks, block sizes
    /// within 128…1,024 lines (the last may be shorter), prefix sums, `uniform` exactly when it may be, and the
    /// EOL summary. **Not a stable API** (tests).
    #[doc(hidden)]
    pub fn check_line_table(&self, cpl: Option<u32>) -> Result<(), String> {
        self.lines.check_structure()?;
        let text = self.rope.to_string();
        let (built, eol) = LineTable::build(text.as_bytes(), cpl, "check", &CancelToken::never())
            .map_err(|e| e.to_string())?;
        if built.len() != self.lines.len() {
            return Err(format!(
                "{} lines, a fresh build has {}",
                self.lines.len(),
                built.len()
            ));
        }
        for line in 0..built.len() {
            if built.content(line) != self.lines.content(line)
                || built.eol(line) != self.lines.eol(line)
                || built.physical(line) != self.lines.physical(line)
            {
                return Err(format!(
                    "line {line}: {:?} {:?} {} != built {:?} {:?} {}",
                    self.lines.content(line),
                    self.lines.eol(line),
                    self.lines.physical(line),
                    built.content(line),
                    built.eol(line),
                    built.physical(line)
                ));
            }
        }
        if eol != self.eol {
            return Err(format!("EOL summary {:?} != built {eol:?}", self.eol));
        }
        Ok(())
    }

    /// Makes the artificial breaks those of the characters-per-line limit `cpl` (§1.10.3: an editor wraps every
    /// edited physical line at its own limit, so a document loaded with another limit, or without one, is
    /// re-wrapped once when it opens; else an undo would not restore the loaded table). A document already wrapped
    /// at `cpl` is left as it is (marks kept); the check is O(lines) and re-wrapping O(text). True when the table
    /// changed.
    pub(crate) fn normalize_wrap(&mut self, cpl: Option<u32>) -> bool {
        let t = &self.lines;
        let no_artificial = t.artificial_count() == 0;
        let fits =
            |c: u32| (0..t.len()).all(|l| t.content(l).is_none_or(|(s, e)| e - s <= c as usize));
        if no_artificial && cpl.is_none_or(fits) {
            return false;
        }
        let text = self.rope.to_string();
        let Ok((built, eol)) =
            LineTable::build(text.as_bytes(), cpl, "edit", &CancelToken::never())
        else {
            return false;
        };
        let same = built.len() == t.len()
            && (0..built.len()).all(|l| {
                built.content(l) == t.content(l)
                    && built.eol(l) == t.eol(l)
                    && built.physical(l) == t.physical(l)
            });
        if same {
            return false;
        }
        self.lines = built;
        self.eol = eol;
        true
    }

    /// The block layout: lines per block, whether line → block is a division, and an identity per block (equal
    /// identities are shared blocks). **Not a stable API** (tests of D-260).
    #[doc(hidden)]
    pub fn block_layout(&self) -> (Vec<u32>, bool, Vec<usize>) {
        let t = &self.lines;
        (
            (0..t.block_count()).map(|b| t.block_lines(b)).collect(),
            t.uniform(),
            (0..t.block_count())
                .map(|b| t.block_ptr(b) as usize)
                .collect(),
        )
    }

    /// Heap bytes of the line table (blocks, headers, prefix sums). **Not a stable API** (memory tests).
    #[doc(hidden)]
    pub fn line_table_bytes(&self) -> usize {
        self.lines.heap_bytes()
    }

    /// The line table (crate-internal: the editor and the live comparison).
    pub(crate) fn table(&self) -> &LineTable {
        &self.lines
    }

    /// Replaces `remove` bytes at byte `at_byte` of the text with `insert`, and lines `[at, at + removed)` of the
    /// table with `recs` (which describe exactly the lines of the new text there). The EOL summary follows.
    pub(crate) fn edit_raw(
        &mut self,
        at_byte: usize,
        remove: usize,
        insert: &str,
        at: u32,
        removed: u32,
        recs: &[blocks::LineRec],
    ) {
        for line in at..at + removed {
            self.eol.uncount(self.lines.eol(line));
        }
        for r in recs {
            self.eol.count(r.eol);
        }
        if remove > 0 {
            let from = self.rope.byte_to_char(at_byte);
            let to = self.rope.byte_to_char(at_byte + remove);
            self.rope.remove(from..to);
        }
        if !insert.is_empty() {
            let at_char = self.rope.byte_to_char(at_byte);
            self.rope.insert(at_char, insert);
        }
        self.lines.splice(at, removed, recs);
        debug_assert_eq!(self.rope.len_bytes(), self.lines.text_len() as usize);
    }

    /// Sets every line's file line to its own index (a save).
    pub(crate) fn rebase_orig(&mut self) {
        self.lines.rebase_orig();
    }
}

impl LineSource for TextDocument {
    fn line_count(&self) -> u32 {
        self.lines.len()
    }

    fn line(&self, idx: u32) -> Cow<'_, str> {
        TextDocument::line(self, LineIdx(idx))
    }
}

/// `rope[s..e]`, borrowed when it lies inside one chunk. Offsets come from the line table and are char
/// boundaries; anything out of range gives `""` (never a panic).
fn rope_range(rope: &ropey::Rope, s: usize, e: usize) -> Cow<'_, str> {
    if s >= e {
        return Cow::Borrowed("");
    }
    let Some((chunk, chunk_start, _, _)) = rope.get_chunk_at_byte(s) else {
        return Cow::Borrowed("");
    };
    let (ls, le) = (s - chunk_start, e - chunk_start);
    if le <= chunk.len() {
        return Cow::Borrowed(chunk.get(ls..le).unwrap_or(""));
    }
    let mut out = String::with_capacity(e - s);
    if let Some((chunks, mut at, _, _)) = rope.get_chunks_at_byte(s) {
        for piece in chunks {
            let end = at + piece.len();
            let from = s.max(at) - at;
            let to = e.min(end) - at;
            if from < to {
                out.push_str(piece.get(from..to).unwrap_or(""));
            }
            at = end;
            if at >= e {
                break;
            }
        }
    }
    Cow::Owned(out)
}

/// Sequential line reader over the rope's chunks (see [`TextDocument::cursor`]).
pub(crate) struct LineCursor<'d> {
    doc: &'d TextDocument,
    chunks: Option<ropey::iter::Chunks<'d>>,
    chunk: &'d str,
    chunk_start: usize,
    buf: String,
}

impl<'d> LineCursor<'d> {
    /// The content of line `idx`. Calls with increasing line indices walk the chunks forward; a call that goes
    /// backwards repositions the cursor (O(log n)).
    pub(crate) fn line(&mut self, idx: u32) -> &str {
        let Some((s, e)) = self.doc.lines.content(idx) else {
            return "";
        };
        self.range(s, e)
    }

    fn range(&mut self, s: usize, e: usize) -> &str {
        if s >= e {
            return "";
        }
        let chunk_end = self.chunk_start + self.chunk.len();
        if self.chunks.is_none() || s < self.chunk_start || s >= chunk_end {
            // (Re)position on the chunk that contains `s`, unless `s` is in the next chunks.
            if !self.advance_to(s) {
                return "";
            }
        }
        let chunk_end = self.chunk_start + self.chunk.len();
        if e <= chunk_end {
            let (ls, le) = (s - self.chunk_start, e - self.chunk_start);
            return self.chunk.get(ls..le).unwrap_or("");
        }
        // The line spans chunks: copy it.
        self.buf.clear();
        self.buf
            .push_str(self.chunk.get(s - self.chunk_start..).unwrap_or(""));
        while self.chunk_start + self.chunk.len() < e {
            let Some(next) = self.chunks.as_mut().and_then(Iterator::next) else {
                break;
            };
            self.chunk_start += self.chunk.len();
            self.chunk = next;
            let take = (e - self.chunk_start).min(next.len());
            self.buf.push_str(next.get(..take).unwrap_or(""));
        }
        &self.buf
    }

    /// Moves to the chunk containing byte `s`: forward through the iterator when `s` is ahead and close,
    /// else by a fresh tree descent. False when `s` is out of range.
    fn advance_to(&mut self, s: usize) -> bool {
        if let Some(chunks) = self.chunks.as_mut()
            && s >= self.chunk_start
        {
            // Walk forward at most a few chunks before falling back to a descent.
            for _ in 0..8 {
                if s < self.chunk_start + self.chunk.len() {
                    return true;
                }
                match chunks.next() {
                    Some(next) => {
                        self.chunk_start += self.chunk.len();
                        self.chunk = next;
                    }
                    None => break,
                }
            }
            if s < self.chunk_start + self.chunk.len() {
                return true;
            }
        }
        let Some((mut chunks, start, _, _)) = self.doc.rope.get_chunks_at_byte(s) else {
            return false;
        };
        let Some(first) = chunks.next() else {
            return false;
        };
        self.chunk = first;
        self.chunk_start = start;
        self.chunks = Some(chunks);
        s < self.chunk_start + self.chunk.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<TextDocument>();
    }

    #[test]
    fn cursor_matches_random_access_across_chunks() {
        // Long enough for many rope chunks; lines of varied lengths with multi-byte chars.
        let mut text = String::new();
        for k in 0..3000 {
            text.push_str(&"é".repeat(k % 37));
            text.push_str(&format!("line {k}"));
            text.push_str(if k % 3 == 0 { "\r\n" } else { "\n" });
        }
        let doc = TextDocument::from_text(&text);
        let mut cursor = doc.cursor();
        for k in 0..doc.line_count() {
            let a = doc.line(LineIdx(k)).into_owned();
            assert_eq!(cursor.line(k), a, "line {k}");
        }
        // Backwards and repeated positions reposition.
        assert_eq!(cursor.line(5), doc.line(LineIdx(5)));
        assert_eq!(cursor.line(5), doc.line(LineIdx(5)));
        assert_eq!(cursor.line(2999), doc.line(LineIdx(2999)));
        assert_eq!(cursor.line(1), doc.line(LineIdx(1)));
        assert_eq!(cursor.line(99_999), "");
        assert_eq!(doc.line(LineIdx(99_999)), "");
        assert_eq!(doc.line_ending(LineIdx(99_999)), LineEnding::None);
    }

    #[test]
    fn lines_spanning_many_chunks() {
        let mut text = String::new();
        for k in 0..40 {
            text.push_str(&format!("{k}:"));
            text.push_str(&"ü".repeat(500 * (k % 7)));
            text.push('\n');
        }
        let doc = TextDocument::from_text(&text);
        let expected: Vec<&str> = text.lines().collect();
        let mut cursor = doc.cursor();
        for k in 0..doc.line_count() {
            assert_eq!(doc.line(LineIdx(k)), expected[k as usize], "line {k}");
            assert_eq!(cursor.line(k), expected[k as usize], "cursor line {k}");
        }
        for k in (0..doc.line_count()).rev() {
            assert_eq!(cursor.line(k), expected[k as usize], "backwards {k}");
        }
    }

    #[test]
    fn eol_names() {
        for (eol, text) in [
            (LineEnding::Lf, "\n"),
            (LineEnding::CrLf, "\r\n"),
            (LineEnding::Cr, "\r"),
            (LineEnding::None, ""),
            (LineEnding::Artificial, ""),
        ] {
            assert_eq!(eol.as_text(), text);
            assert_eq!(
                serde_json::to_string(&eol).unwrap(),
                format!("\"{}\"", eol.as_str())
            );
        }
        for style in [
            EolStyle::Lf,
            EolStyle::CrLf,
            EolStyle::Cr,
            EolStyle::Mixed,
            EolStyle::None,
        ] {
            assert_eq!(
                serde_json::to_string(&style).unwrap(),
                format!("\"{}\"", style.as_str())
            );
        }
    }
}
