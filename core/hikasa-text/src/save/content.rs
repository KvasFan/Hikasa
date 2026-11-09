//! Save content (TC-001, TC-003, TC-005; D-272, §1.10.12): what a save writes, described before anything is
//! written. Every physical line that is unchanged since the load (or the last save) is a [`Piece::Source`] of its
//! bytes in the splice source, so it comes out byte for byte as it was, whatever the encoding, BOM, line endings,
//! malformed bytes or stateful encoding; only edited and new lines are encoded ([`Piece::Encoded`]).
//!
//! ```text
//! SourceMap::scan(splice source)       one pass: physical line starts (CR, LF, CRLF by code unit, after the BOM,
//!                                      up to a Ctrl-Z cut), the CRC-32 and the length
//! save_content(doc, map, opts)
//!   → whole re-encode when the encoding changed, there is no (valid) source, the source changed, the line
//!     counts disagree, `orig` is not increasing, or an edited ISO-2022-JP document
//!   → else per physical line of the document (pieces joined across artificial breaks):
//!       its pieces keep `orig` o, o+1, … and those are exactly one physical line of the source → Source
//!       otherwise → its text and terminator, encoded (artificial breaks add nothing)
//!   → Bom first when the document writes one; the Ctrl-Z tail last; adjacent Source pieces merged
//! ```

use std::io::{self, Read};
use std::ops::Range;
use std::sync::Arc;

use hikasa_core::progress::{Ctl, Phase, Progress, Unit};
use hikasa_formats::{Encoder, TextEncoding, unmappable_in};
use hikasa_vfs::crc32_update;
use serde::{Deserialize, Serialize};

use crate::document::{EolSummary, LineIdx};
use crate::edit::EditableDocument;
use crate::error::TextError;

/// How many unmappable characters a [`ContentIssue::Unmappable`] lists.
pub const UNMAPPABLE_LISTED: usize = 20;

/// Bytes of text encoded into one [`Piece::Encoded`] of a whole re-encode.
const ENCODED_PIECE: usize = 1 << 20;

/// Bytes read per step when the splice source is scanned (§1.10.13).
pub(crate) const SCAN_STEP: usize = 1 << 20;

/// One piece of the file a save writes, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// The encoding's BOM ([`TextEncoding::bom`]).
    Bom,
    /// Bytes of the splice source, verbatim.
    Source {
        /// The first byte.
        offset: u64,
        /// The length.
        len: u64,
    },
    /// Edited lines with their terminators, encoded.
    Encoded(Arc<[u8]>),
}

/// Why a save encodes the whole text (§1.10.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReencodeReason {
    /// The document's encoding is not the source's (`SetEncoding`, `text convert --encoding`).
    EncodingChanged,
    /// No source to splice from: stdin, the clipboard, a text pair, a new file, a converted document, a source that
    /// cannot be read or whose lines do not match the document's.
    NoSource,
    /// The source changed on disk since it was loaded or saved, and the save writes over it (or elsewhere).
    SourceChanged,
    /// An edited document in a stateful encoding (ISO-2022-JP): its shift state may cross lines.
    Stateful,
}

impl ReencodeReason {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReencodeReason::EncodingChanged => "encoding_changed",
            ReencodeReason::NoSource => "no_source",
            ReencodeReason::SourceChanged => "source_changed",
            ReencodeReason::Stateful => "stateful",
        }
    }
}

/// A character the encoding cannot write: where it is (the table line, the 1-based character column in it) and what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmappableChar {
    /// The line (a table line: a piece of a wrapped physical line counts as one).
    pub line: LineIdx,
    /// The 1-based character column in the line.
    pub column: u32,
    /// The character.
    pub char: char,
}

/// What a save's content needs to be told about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentIssue {
    /// Encoded lines hold characters the encoding cannot write: written as `?` (never numeric references) when the
    /// save goes ahead.
    Unmappable {
        /// How many.
        count: u64,
        /// The first [`UNMAPPABLE_LISTED`].
        first: Vec<UnmappableChar>,
    },
    /// Encoded lines hold U+FFFD in a document whose decode replaced malformed bytes: those bytes are lost.
    LossyRewritten {
        /// Physical lines.
        lines: u64,
    },
    /// The whole text is encoded.
    Reencoded {
        /// Why.
        reason: ReencodeReason,
    },
}

/// What a save writes (§1.10.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveContent {
    /// The pieces, in order.
    pub pieces: Vec<Piece>,
    /// Bytes written.
    pub bytes: u64,
    /// The encoding written.
    pub encoding: TextEncoding,
    /// A BOM is written.
    pub bom: bool,
    /// The terminators written (artificial breaks write none).
    pub eols: EolSummary,
    /// Bytes spliced from the source (the Ctrl-Z tail included).
    pub spliced_bytes: u64,
    /// Physical lines encoded.
    pub encoded_lines: u32,
    /// What the save must be told about.
    pub issues: Vec<ContentIssue>,
}

impl SaveContent {
    /// The reason of a whole re-encode, if the content is one.
    pub fn reencoded(&self) -> Option<ReencodeReason> {
        self.issues.iter().find_map(|issue| match issue {
            ContentIssue::Reencoded { reason } => Some(*reason),
            _ => None,
        })
    }

    /// True when some piece reads the splice source.
    pub fn reads_source(&self) -> bool {
        self.pieces
            .iter()
            .any(|piece| matches!(piece, Piece::Source { .. }))
    }
}

/// Options of [`save_content`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContentOptions {
    /// Encode the whole text for this reason (a source that changed on disk: `SourceChanged`).
    pub reencode: Option<ReencodeReason>,
}

/// The physical lines of a splice source (§1.10.12): the start of each, where the lines end (the Ctrl-Z cut or the
/// end), the length and the CRC-32 of every byte. Built by one scan for terminators, exact for every supported
/// encoding: CR and LF never occur inside a multi-byte sequence of an ASCII-compatible encoding, and UTF-16 and UTF-32
/// are scanned by code unit. 4 bytes a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMap {
    starts: Vec<u32>,
    body_end: u64,
    len: u64,
    crc32: u32,
}

/// The code unit of an encoding's line scan.
#[derive(Debug, Clone, Copy)]
enum CodeUnit {
    Byte,
    Utf16 { big: bool },
    Utf32 { big: bool },
}

impl CodeUnit {
    fn of(encoding: TextEncoding) -> CodeUnit {
        match encoding {
            TextEncoding::Utf32Le => CodeUnit::Utf32 { big: false },
            TextEncoding::Utf32Be => CodeUnit::Utf32 { big: true },
            e if e == TextEncoding::UTF_16LE => CodeUnit::Utf16 { big: false },
            e if e == TextEncoding::UTF_16BE => CodeUnit::Utf16 { big: true },
            _ => CodeUnit::Byte,
        }
    }

    fn width(self) -> usize {
        match self {
            CodeUnit::Byte => 1,
            CodeUnit::Utf16 { .. } => 2,
            CodeUnit::Utf32 { .. } => 4,
        }
    }

    fn value(self, bytes: &[u8]) -> u32 {
        match (self, bytes) {
            (CodeUnit::Byte, [b, ..]) => u32::from(*b),
            (CodeUnit::Utf16 { big: true }, [a, b, ..]) => u32::from(u16::from_be_bytes([*a, *b])),
            (CodeUnit::Utf16 { big: false }, [a, b, ..]) => u32::from(u16::from_le_bytes([*a, *b])),
            (CodeUnit::Utf32 { big: true }, [a, b, c, d, ..]) => {
                u32::from_be_bytes([*a, *b, *c, *d])
            }
            (CodeUnit::Utf32 { big: false }, [a, b, c, d, ..]) => {
                u32::from_le_bytes([*a, *b, *c, *d])
            }
            _ => u32::MAX,
        }
    }
}

/// The state of a line scan.
struct Scan {
    starts: Vec<u32>,
    /// The start of the line being read.
    line_start: u64,
    /// The unit before was a CR: the line ends after it, or after an LF that follows.
    cr: bool,
    /// The lines end here (a Ctrl-Z).
    cut: Option<u64>,
    fits: bool,
}

impl Scan {
    fn push(&mut self, start: u64) {
        match u32::try_from(start) {
            Ok(start) => self.starts.push(start),
            Err(_) => self.fits = false,
        }
    }

    /// One unit of value `v` at `at`, `width` bytes long.
    fn unit(&mut self, v: u32, at: u64, width: u64) {
        if self.cr {
            self.cr = false;
            if v == 0x0A {
                let start = self.line_start;
                self.push(start);
                self.line_start = at + width;
                return;
            }
            let start = self.line_start;
            self.push(start);
            self.line_start = at;
        }
        if v == 0x0D {
            self.cr = true;
        } else if v == 0x0A {
            let start = self.line_start;
            self.push(start);
            self.line_start = at + width;
        }
    }

    /// The end of the lines at `end`.
    fn finish(&mut self, end: u64) {
        if self.cr {
            self.cr = false;
            let start = self.line_start;
            self.push(start);
            self.line_start = end;
        }
        if self.line_start < end {
            let start = self.line_start;
            self.push(start);
        }
    }
}

impl SourceMap {
    /// Scans `reader` (the whole splice source) for its physical lines: the bytes from `bom_len` on, in `encoding`'s
    /// code units, up to the first Ctrl-Z when `ctrl_z` and the encoding is ASCII-compatible (the rest is the tail).
    /// Every byte is read once, in [`SCAN_STEP`] steps, with a cancellation check and `reading` progress per step.
    /// `Ok(None)` for a source of 4 GiB or more (offsets are `u32`, as the document's).
    pub fn scan(
        reader: &mut dyn Read,
        encoding: TextEncoding,
        bom_len: u8,
        ctrl_z: bool,
        ctl: &Ctl<'_>,
    ) -> Result<Option<SourceMap>, ScanError> {
        let unit = CodeUnit::of(encoding);
        let width = unit.width();
        let ctrl_z = ctrl_z && encoding.is_ascii_compatible();
        let mut scan = Scan {
            starts: Vec::new(),
            line_start: u64::from(bom_len),
            cr: false,
            cut: None,
            fits: true,
        };
        let mut buffer = vec![0u8; SCAN_STEP];
        let mut crc = 0u32;
        let mut pos = 0u64;
        // The bytes of a unit split by a step's end.
        let mut carry: Vec<u8> = Vec::with_capacity(4);
        loop {
            ctl.check().map_err(|_| ScanError::Cancelled)?;
            let n = fill(reader, &mut buffer)?;
            if n == 0 {
                break;
            }
            let chunk = buffer.get(..n).unwrap_or_default();
            crc = crc32_update(crc, chunk);
            let chunk_at = pos;
            pos += n as u64;
            ctl.progress(Phase::Reading, || Progress {
                phase: Phase::Reading,
                done: pos,
                total: None,
                unit: Unit::Bytes,
                current: None,
            });
            if scan.cut.is_some() {
                continue;
            }
            // The part of the chunk that holds lines: after the BOM, before a Ctrl-Z.
            let skip = usize::try_from(u64::from(bom_len).saturating_sub(chunk_at))
                .unwrap_or(usize::MAX)
                .min(chunk.len());
            let mut body = chunk.get(skip..).unwrap_or_default();
            if ctrl_z && let Some(at) = memchr::memchr(0x1A, body) {
                scan.cut = Some(chunk_at + (skip + at) as u64);
                body = body.get(..at).unwrap_or_default();
            }
            let body_at = chunk_at + skip as u64;
            match unit {
                CodeUnit::Byte => {
                    let mut last = 0usize;
                    for i in memchr::memchr2_iter(b'\r', b'\n', body) {
                        // A pending CR followed by something other than these: the line ended after it.
                        if scan.cr && i > last {
                            let start = scan.line_start;
                            scan.push(start);
                            scan.line_start = body_at + last as u64;
                            scan.cr = false;
                        }
                        let v = u32::from(body.get(i).copied().unwrap_or(0));
                        scan.unit(v, body_at + i as u64, 1);
                        last = i + 1;
                    }
                    if scan.cr && last < body.len() {
                        let start = scan.line_start;
                        scan.push(start);
                        scan.line_start = body_at + last as u64;
                        scan.cr = false;
                    }
                }
                CodeUnit::Utf16 { .. } | CodeUnit::Utf32 { .. } => {
                    // The units of the body (the BOM is whole units, so units start at `bom_len`).
                    let mut at = body_at - carry.len() as u64;
                    let mut rest = body;
                    if !carry.is_empty() {
                        let need = width - carry.len();
                        let take = need.min(rest.len());
                        carry.extend_from_slice(rest.get(..take).unwrap_or_default());
                        rest = rest.get(take..).unwrap_or_default();
                        if carry.len() < width {
                            continue;
                        }
                        let v = unit.value(&carry);
                        scan.unit(v, at, width as u64);
                        at += width as u64;
                        carry.clear();
                    }
                    let (units, tail) = rest.split_at(rest.len() - rest.len() % width);
                    for (k, bytes) in units.chunks_exact(width).enumerate() {
                        scan.unit(unit.value(bytes), at + (k * width) as u64, width as u64);
                    }
                    carry.extend_from_slice(tail);
                }
            }
        }
        let body_end = scan.cut.unwrap_or(pos).max(u64::from(bom_len).min(pos));
        scan.finish(body_end);
        if !scan.fits || pos > u64::from(u32::MAX) {
            return Ok(None);
        }
        Ok(Some(SourceMap {
            starts: scan.starts,
            body_end,
            len: pos,
            crc32: crc,
        }))
    }

    /// Physical lines.
    pub fn lines(&self) -> u32 {
        self.starts.len() as u32
    }

    /// The source's length.
    pub fn len(&self) -> u64 {
        self.len
    }

    /// True for an empty source.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The CRC-32 of every byte.
    pub fn crc32(&self) -> u32 {
        self.crc32
    }

    /// The bytes of physical line `k`, its terminator included.
    pub fn line_bytes(&self, k: u32) -> Option<Range<u64>> {
        let start = u64::from(*self.starts.get(k as usize)?);
        let end = self
            .starts
            .get(k as usize + 1)
            .map_or(self.body_end, |&next| u64::from(next));
        Some(start..end)
    }

    /// The bytes after the lines: from a Ctrl-Z that ended the text to the end (empty without one).
    pub fn tail(&self) -> Range<u64> {
        self.body_end..self.len
    }
}

/// Why a scan stopped.
#[derive(Debug)]
pub enum ScanError {
    /// Cancelled.
    Cancelled,
    /// A read failed.
    Io(io::Error),
}

impl From<io::Error> for ScanError {
    fn from(error: io::Error) -> ScanError {
        ScanError::Io(error)
    }
}

/// Fills `buffer` from `reader` until it is full or the reader ends.
fn fill(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buffer.len() {
        match reader.read(buffer.get_mut(n..).unwrap_or_default()) {
            Ok(0) => break,
            Ok(got) => n += got,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Builds the pieces: merges adjacent `Source` pieces, collects encoded text into runs.
struct Builder {
    pieces: Vec<Piece>,
    bytes: u64,
    spliced: u64,
    /// Encoded bytes not yet made a piece.
    pending: Vec<u8>,
}

impl Builder {
    fn new() -> Builder {
        Builder {
            pieces: Vec::new(),
            bytes: 0,
            spliced: 0,
            pending: Vec::new(),
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let bytes: Arc<[u8]> = Arc::from(std::mem::take(&mut self.pending));
            self.pieces.push(Piece::Encoded(bytes));
        }
    }

    fn source(&mut self, range: Range<u64>) {
        let len = range.end.saturating_sub(range.start);
        if len == 0 {
            return;
        }
        self.flush();
        self.bytes += len;
        self.spliced += len;
        if let Some(Piece::Source { offset, len: last }) = self.pieces.last_mut()
            && *offset + *last == range.start
        {
            *last += len;
            return;
        }
        self.pieces.push(Piece::Source {
            offset: range.start,
            len,
        });
    }

    fn encoded(&mut self, bytes: &[u8]) {
        self.bytes += bytes.len() as u64;
        self.pending.extend_from_slice(bytes);
        if self.pending.len() >= ENCODED_PIECE {
            self.flush();
        }
    }
}

/// What the encoded lines showed.
#[derive(Default)]
struct Found {
    unmappable: u64,
    listed: Vec<UnmappableChar>,
    lossy: u64,
}

impl Found {
    /// Looks at one physical line's content (pieces `first..end` of `doc`) before it is encoded.
    fn look(
        &mut self,
        doc: &EditableDocument,
        encoding: TextEncoding,
        first: u32,
        end: u32,
        lossy: bool,
    ) {
        let mut replaced = false;
        for l in first..end {
            let text = doc.line(LineIdx(l));
            replaced |= lossy && text.contains('\u{FFFD}');
            let found = unmappable_in(&text, encoding);
            self.unmappable += found.len() as u64;
            for (index, c) in found {
                if self.listed.len() >= UNMAPPABLE_LISTED {
                    break;
                }
                self.listed.push(UnmappableChar {
                    line: LineIdx(l),
                    column: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    char: c,
                });
            }
        }
        if replaced {
            self.lossy += 1;
        }
    }

    fn issues(self) -> Vec<ContentIssue> {
        let mut issues = Vec::new();
        if self.unmappable > 0 {
            issues.push(ContentIssue::Unmappable {
                count: self.unmappable,
                first: self.listed,
            });
        }
        if self.lossy > 0 {
            issues.push(ContentIssue::LossyRewritten { lines: self.lossy });
        }
        issues
    }
}

/// The text of the physical line `first..end` with its terminator (artificial breaks add nothing).
fn physical_text(doc: &EditableDocument, first: u32, end: u32, out: &mut String) {
    for l in first..end {
        out.push_str(&doc.line(LineIdx(l)));
    }
    if end > first {
        out.push_str(doc.line_ending(LineIdx(end - 1)).as_text());
    }
}

/// The content a save of `doc` writes (§1.10.12). `source` is the splice source's [`SourceMap`] when it was read and
/// its stamp held; without it (or with [`ContentOptions::reencode`]) the whole text is encoded. Pure and in memory:
/// O(document lines) for the walk, plus the encoding of the edited lines (of every line for a whole re-encode).
pub fn save_content(
    doc: &EditableDocument,
    source: Option<&SourceMap>,
    opts: &ContentOptions,
) -> SaveContent {
    let (encoding, bom) = doc.encoding();
    let physical = doc.orig_lines() - doc.orig_artificial().len() as u32;
    let reason = if let Some(reason) = opts.reencode {
        Some(reason)
    } else if !doc.spliceable() {
        Some(ReencodeReason::NoSource)
    } else if doc
        .splice_encoding()
        .is_some_and(|(enc, _)| enc != encoding)
    {
        Some(ReencodeReason::EncodingChanged)
    } else if source.is_none_or(|map| map.lines() != physical) {
        Some(ReencodeReason::NoSource)
    } else {
        None
    };
    let lossy = doc.document().malformed() > 0;
    if let (None, Some(map)) = (reason, source)
        && let Some(content) = spliced(doc, map, encoding, bom, lossy)
    {
        return content;
    }
    let reason = reason.unwrap_or_else(|| {
        if is_stateful(encoding) {
            ReencodeReason::Stateful
        } else {
            ReencodeReason::NoSource
        }
    });
    // The tail after a Ctrl-Z of a source that is still the one loaded.
    let tail = match (reason, source) {
        (ReencodeReason::EncodingChanged | ReencodeReason::Stateful, Some(map)) => Some(map.tail()),
        _ => None,
    };
    whole(doc, encoding, bom, lossy, reason, tail)
}

/// True for an encoding whose shift state may cross lines.
fn is_stateful(encoding: TextEncoding) -> bool {
    encoding.name() == "ISO-2022-JP"
}

/// The splice of `doc` against `map`; `None` when it cannot be made (a non-increasing `orig`, an edited stateful
/// document): the caller encodes the whole text.
fn spliced(
    doc: &EditableDocument,
    map: &SourceMap,
    encoding: TextEncoding,
    bom: bool,
    lossy: bool,
) -> Option<SaveContent> {
    let n = doc.line_count();
    let art = doc.orig_artificial();
    let orig_lines = doc.orig_lines();
    let mut out = Builder::new();
    if bom && !encoding.bom().is_empty() {
        out.bytes += encoding.bom().len() as u64;
        out.pieces.push(Piece::Bom);
    }
    let mut found = Found::default();
    let mut encoded_lines = 0u32;
    // Text of consecutive encoded physical lines, encoded together.
    let mut run = String::new();
    let mut scratch = Vec::new();
    let flush_run = |run: &mut String, out: &mut Builder, scratch: &mut Vec<u8>| {
        if run.is_empty() {
            return;
        }
        scratch.clear();
        Encoder::new(encoding).encode(run, true, scratch);
        out.encoded(scratch);
        run.clear();
    };
    // Artificial lines of the source below the current `orig` (the rank of `orig` among physical starts).
    let mut art_at = 0usize;
    let mut next_orig = 0u32;
    let mut p = 0u32;
    while p < n {
        let end = doc.phys_end(p);
        let first_orig = doc.marks(LineIdx(p)).orig.map(|o| o.0);
        let mut splice = None;
        if let Some(o) = first_orig {
            if o < next_orig || o >= orig_lines {
                // `orig` must increase along the document: a splice never seeks back.
                return None;
            }
            let k = end - p;
            let consecutive =
                (1..k).all(|i| doc.marks(LineIdx(p + i)).orig.map(|x| x.0) == Some(o + i));
            while art.get(art_at).is_some_and(|&a| a < o) {
                art_at += 1;
            }
            // `o` starts a physical line of the source, and `o .. o+k-1` are exactly its pieces.
            let starts = o == 0 || art_at == 0 || art.get(art_at - 1) != Some(&(o - 1));
            let pieces = (0..k - 1).all(|i| art.get(art_at + i as usize) == Some(&(o + i)))
                && art.get(art_at + (k - 1) as usize) != Some(&(o + k - 1));
            if consecutive && starts && pieces {
                let physical = o - art_at as u32;
                splice = map.line_bytes(physical);
                next_orig = o + k;
            }
        }
        match splice {
            Some(range) => {
                flush_run(&mut run, &mut out, &mut scratch);
                out.source(range);
            }
            None => {
                found.look(doc, encoding, p, end, lossy);
                physical_text(doc, p, end, &mut run);
                encoded_lines += 1;
                if run.len() >= ENCODED_PIECE {
                    flush_run(&mut run, &mut out, &mut scratch);
                }
            }
        }
        p = end;
    }
    flush_run(&mut run, &mut out, &mut scratch);
    if encoded_lines > 0 && is_stateful(encoding) {
        return None;
    }
    out.source(map.tail());
    out.flush();
    Some(SaveContent {
        pieces: out.pieces,
        bytes: out.bytes,
        encoding,
        bom,
        eols: *doc.document().eol(),
        spliced_bytes: out.spliced,
        encoded_lines,
        issues: found.issues(),
    })
}

/// The whole text of `doc`, encoded through one encoder (a stateful encoding's state crosses lines), then `tail`.
fn whole(
    doc: &EditableDocument,
    encoding: TextEncoding,
    bom: bool,
    lossy: bool,
    reason: ReencodeReason,
    tail: Option<Range<u64>>,
) -> SaveContent {
    let n = doc.line_count();
    let mut out = Builder::new();
    if bom && !encoding.bom().is_empty() {
        out.bytes += encoding.bom().len() as u64;
        out.pieces.push(Piece::Bom);
    }
    let mut found = Found::default();
    let mut encoder = Encoder::new(encoding);
    let mut text = String::new();
    let mut scratch = Vec::new();
    let mut encoded_lines = 0u32;
    let mut p = 0u32;
    while p < n {
        let end = doc.phys_end(p);
        found.look(doc, encoding, p, end, lossy);
        physical_text(doc, p, end, &mut text);
        encoded_lines += 1;
        p = end;
        let last = p >= n;
        if text.len() >= ENCODED_PIECE || last {
            scratch.clear();
            encoder.encode(&text, last, &mut scratch);
            out.encoded(&scratch);
            text.clear();
        }
    }
    if let Some(tail) = tail {
        out.source(tail);
    }
    out.flush();
    let mut issues = found.issues();
    issues.push(ContentIssue::Reencoded { reason });
    SaveContent {
        pieces: out.pieces,
        bytes: out.bytes,
        encoding,
        bom,
        eols: *doc.document().eol(),
        spliced_bytes: out.spliced,
        encoded_lines,
        issues,
    }
}

impl From<ScanError> for TextError {
    fn from(error: ScanError) -> TextError {
        match error {
            ScanError::Cancelled => TextError::Cancelled,
            ScanError::Io(e) => TextError::Load {
                side: hikasa_core::Side::Left,
                source: hikasa_vfs::VfsError::from_io("<source>", hikasa_vfs::VfsOp::Read, e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn scan(bytes: &[u8], encoding: TextEncoding, bom: u8, ctrl_z: bool) -> SourceMap {
        SourceMap::scan(
            &mut Cursor::new(bytes),
            encoding,
            bom,
            ctrl_z,
            &Ctl::detached(),
        )
        .unwrap()
        .unwrap()
    }

    fn lines(map: &SourceMap, bytes: &[u8]) -> Vec<Vec<u8>> {
        (0..map.lines())
            .map(|k| {
                let r = map.line_bytes(k).unwrap();
                bytes[r.start as usize..r.end as usize].to_vec()
            })
            .collect()
    }

    #[test]
    fn the_scan_finds_every_terminator() {
        let bytes = b"a\r\nb\rc\n\n\r\rd";
        let map = scan(bytes, TextEncoding::UTF_8, 0, false);
        let got = lines(&map, bytes);
        let want: Vec<&[u8]> = vec![b"a\r\n", b"b\r", b"c\n", b"\n", b"\r", b"\r", b"d"];
        assert_eq!(got, want);
        assert_eq!(map.crc32(), hikasa_vfs::crc32_bytes(bytes));
        assert_eq!(map.tail(), 11..11);
        let map = scan(b"", TextEncoding::UTF_8, 0, false);
        assert_eq!(map.lines(), 0);
        // A BOM, a Ctrl-Z tail.
        let bytes = b"\xEF\xBB\xBFx\ny\x1Ajunk\n";
        let map = scan(bytes, TextEncoding::UTF_8, 3, true);
        assert_eq!(lines(&map, bytes), vec![b"x\n".to_vec(), b"y".to_vec()]);
        assert_eq!(map.tail(), 6..12);
        // UTF-16: by code unit (0x0A00 is no LF in LE).
        let text = "a\r\n\u{0A0D}\n";
        let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let map = scan(&bytes, TextEncoding::UTF_16LE, 0, true);
        assert_eq!(map.lines(), 2);
        assert_eq!(map.line_bytes(1), Some(6..10));
    }

    #[test]
    fn a_crlf_split_by_a_step_is_one_terminator() {
        for split in [SCAN_STEP - 1, SCAN_STEP] {
            let mut bytes = vec![b'x'; split];
            bytes.extend_from_slice(b"\r\nz\r");
            let map = scan(&bytes, TextEncoding::UTF_8, 0, false);
            assert_eq!(map.lines(), 2, "split {split}");
            assert_eq!(map.line_bytes(1), Some(split as u64 + 2..split as u64 + 4));
            let wide: Vec<u8> = bytes.iter().flat_map(|&b| [0, b]).collect();
            let map = scan(&wide, TextEncoding::UTF_16BE, 0, false);
            assert_eq!(map.lines(), 2, "split {split} wide");
        }
    }
}
