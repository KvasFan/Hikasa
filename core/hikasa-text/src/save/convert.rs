//! Conversions streamed through a save (TC-063, D-280; W10b note 60): `hikasa text convert` never edits its
//! documents. The conversions run line by line inside the encoder's pass, so a file is held once (the loaded
//! document) and the converted text only one line and one encoded step (≤ 1 MiB) at a time, never as a second copy,
//! an undo log or the encoded file.
//!
//! ```text
//! the document's lines → stage per conversion (trim, tabs, line endings: §1.10.15's rules, `transform`'s ranges
//!                         and neighbours) → a lone-CR line and an empty LF line a trim made are one CRLF line, and
//!                         an emptied last line without terminator is no line, as `EditableDocument::apply` reads them
//!   → convert_counts      each conversion's count, the lines changed, the terminators (no encoding)
//!   → converted_content   the plan's pass: a line no stage touched is a `Source` piece of its bytes (the splice);
//!                         the others encoded; the bytes, the issues; `Piece::Converted` holds the recipe
//!   → ConvertStream       the execution's pass: the same pieces again, made one step at a time
//! ```
//!
//! The bytes equal the editor's: [`crate::transform::transform`] applied step by step to an
//! [`EditableDocument`], then [`super::save_content`] (P-CONVERT-STREAM in `save_roundtrip.rs` checks it).

use std::collections::VecDeque;
use std::ops::Range;
use std::sync::Arc;

use hikasa_core::progress::Ctl;
use hikasa_formats::{Encoder, TextEncoding};

use super::content::{ContentIssue, Found, Piece, ReencodeReason, SaveContent, SourceMap};
use crate::document::{EolSummary, LineEnding, LineIdx, TextDocument};
use crate::edit::EditableDocument;
use crate::error::TextError;
use crate::transform::{Transform, leading_spaces_to_tabs, tabs_to_spaces, trim_trailing};

/// Bytes of encoded text gathered into one piece.
const STEP: usize = 1 << 20;

/// Lines between two cancellation checks of a pass at planning.
const CHECK_EVERY: u32 = 1 << 16;

/// The conversions of a file (`hikasa text convert`, §1.10.15), applied in their order to the document's lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConvertSpec {
    /// The conversions, in order (the fixed order trim, tabs, line endings is the caller's).
    pub steps: Vec<Transform>,
    /// The lines each conversion takes (0-based, clipped; `None`: every line), as [`crate::transform::transform`]'s
    /// `lines`: each step counts the lines of the text the steps before it made.
    pub lines: Option<Range<u32>>,
    /// The tab size of the tab conversions.
    pub tab_size: u8,
}

/// What the conversions of a [`ConvertSpec`] do to a document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConvertCounts {
    /// Each step's [`crate::transform::TransformOutcome::lines`], in order.
    pub steps: Vec<u64>,
    /// Lines of the result some step changed (the editor's *Edited* lines).
    pub changed: u64,
    /// Lines of the result.
    pub lines: u64,
    /// Its terminators.
    pub eols: EolSummary,
}

/// One line on its way through the stages.
#[derive(Debug, Clone)]
struct Line {
    text: String,
    eol: LineEnding,
    /// Some stage changed it.
    touched: bool,
    /// Its line in the document (and the splice source).
    orig: u32,
}

/// A content conversion of the lines of `range`.
#[derive(Debug)]
struct ContentStage {
    t: Transform,
    ts: u32,
    range: Option<Range<u32>>,
    at: u32,
    count: u64,
    /// A line ending with a lone CR, held until the next shows whether the two read as one CRLF line.
    held: Option<Line>,
}

/// `LineEndings(e)` over the lines of `range`, with `transform`'s neighbours.
#[derive(Debug)]
struct EolStage {
    e: LineEnding,
    range: Option<Range<u32>>,
    at: u32,
    count: u64,
    /// To `Lf`: the lines before the range that end with a lone CR (empty but the first), held until the range's
    /// first line shows whether they convert with it.
    held: Vec<Line>,
    /// To `Cr`: the empty LF lines right after the range still convert.
    after: bool,
}

#[derive(Debug)]
enum Stage {
    Content(ContentStage),
    Eols(EolStage),
}

fn in_range(range: &Option<Range<u32>>, i: u32) -> bool {
    range.as_ref().is_none_or(|r| r.contains(&i))
}

impl ContentStage {
    fn rule(&self, line: &str) -> Option<String> {
        match self.t {
            Transform::TrimTrailing => trim_trailing(line),
            Transform::LeadingSpacesToTabs => leading_spaces_to_tabs(line, self.ts),
            Transform::TabsToSpaces => tabs_to_spaces(line, self.ts),
            Transform::LineEndings(_) => None,
        }
    }

    fn push(&mut self, mut line: Line, out: &mut Vec<Line>) {
        let i = self.at;
        self.at += 1;
        if in_range(&self.range, i)
            && let Some(new) = self.rule(&line.text)
        {
            line.text = new;
            line.touched = true;
            self.count += 1;
        }
        if line.text.is_empty() && line.eol == LineEnding::None {
            // The last line, emptied: a document has no empty last line without a terminator.
            out.extend(self.held.take());
            return;
        }
        if let Some(prev) = self.held.take() {
            if line.text.is_empty() && line.eol == LineEnding::Lf {
                // "x\r" and "\n": one CRLF line, as a load (and the editor) reads them.
                out.push(Line {
                    eol: LineEnding::CrLf,
                    touched: true,
                    ..prev
                });
                return;
            }
            out.push(prev);
        }
        if line.eol == LineEnding::Cr {
            self.held = Some(line);
        } else {
            out.push(line);
        }
    }

    fn finish(&mut self, out: &mut Vec<Line>) {
        out.extend(self.held.take());
    }
}

impl EolStage {
    fn convert(&mut self, line: &mut Line) {
        if line.eol != LineEnding::None && line.eol != self.e {
            line.eol = self.e;
            line.touched = true;
            self.count += 1;
        }
    }

    fn push(&mut self, mut line: Line, out: &mut Vec<Line>) {
        let i = self.at;
        self.at += 1;
        let (start, end) = self
            .range
            .as_ref()
            .map_or((0, u32::MAX), |r| (r.start, r.end));
        if start >= end {
            out.push(line);
            return;
        }
        if i < start {
            if self.e == LineEnding::Lf && line.eol == LineEnding::Cr {
                if !line.text.is_empty() {
                    out.append(&mut self.held);
                }
                self.held.push(line);
            } else {
                out.append(&mut self.held);
                out.push(line);
            }
            return;
        }
        if i == start && !self.held.is_empty() {
            let mut held = std::mem::take(&mut self.held);
            if line.text.is_empty() && line.eol != LineEnding::None {
                for before in &mut held {
                    self.convert(before);
                }
            }
            out.append(&mut held);
        }
        if i < end {
            self.convert(&mut line);
        } else if self.after && self.e == LineEnding::Cr {
            if line.text.is_empty() && line.eol == LineEnding::Lf {
                self.convert(&mut line);
            } else {
                self.after = false;
            }
        }
        out.push(line);
    }

    fn finish(&mut self, out: &mut Vec<Line>) {
        out.append(&mut self.held);
    }
}

impl Stage {
    fn new(t: Transform, lines: &Option<Range<u32>>, tab_size: u8) -> Stage {
        match t {
            Transform::LineEndings(e) => Stage::Eols(EolStage {
                // `None` and `Artificial` convert nothing.
                e,
                range: if matches!(e, LineEnding::Lf | LineEnding::CrLf | LineEnding::Cr) {
                    lines.clone()
                } else {
                    Some(0..0)
                },
                at: 0,
                count: 0,
                held: Vec::new(),
                after: true,
            }),
            t => Stage::Content(ContentStage {
                t,
                ts: u32::from(tab_size.max(1)),
                range: lines.clone(),
                at: 0,
                count: 0,
                held: None,
            }),
        }
    }

    fn push(&mut self, line: Line, out: &mut Vec<Line>) {
        match self {
            Stage::Content(stage) => stage.push(line, out),
            Stage::Eols(stage) => stage.push(line, out),
        }
    }

    fn finish(&mut self, out: &mut Vec<Line>) {
        match self {
            Stage::Content(stage) => stage.finish(out),
            Stage::Eols(stage) => stage.finish(out),
        }
    }

    fn count(&self) -> u64 {
        match self {
            Stage::Content(stage) => stage.count,
            Stage::Eols(stage) => stage.count,
        }
    }
}

/// The lines of a document through the stages, one at a time.
struct Lines {
    doc: Arc<TextDocument>,
    next: u32,
    stages: Vec<Stage>,
    ready: VecDeque<Line>,
    finished: bool,
    /// Scratch of the stages.
    a: Vec<Line>,
    b: Vec<Line>,
}

impl Lines {
    fn new(doc: Arc<TextDocument>, spec: &ConvertSpec) -> Lines {
        let stages = spec
            .steps
            .iter()
            .map(|&t| Stage::new(t, &spec.lines, spec.tab_size))
            .collect();
        Lines {
            doc,
            next: 0,
            stages,
            ready: VecDeque::new(),
            finished: false,
            a: Vec::new(),
            b: Vec::new(),
        }
    }

    /// Runs `a` through the stages from `from` on (finishing them after the input ended), into `ready`.
    fn run(&mut self, from: usize, finish: bool) {
        for k in from..self.stages.len() {
            let Some(stage) = self.stages.get_mut(k) else {
                break;
            };
            for line in self.a.drain(..) {
                stage.push(line, &mut self.b);
            }
            if finish {
                stage.finish(&mut self.b);
            }
            std::mem::swap(&mut self.a, &mut self.b);
        }
        self.ready.extend(self.a.drain(..));
    }

    fn next_line(&mut self) -> Option<Line> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Some(line);
            }
            if self.finished {
                return None;
            }
            if self.next < self.doc.line_count() {
                let i = self.next;
                self.next += 1;
                self.a.push(Line {
                    text: self.doc.line(LineIdx(i)).into_owned(),
                    eol: self.doc.line_ending(LineIdx(i)),
                    touched: false,
                    orig: i,
                });
                self.run(0, false);
            } else {
                self.finished = true;
                self.run(0, true);
            }
        }
    }

    fn counts(&self) -> Vec<u64> {
        self.stages.iter().map(Stage::count).collect()
    }
}

fn count_eol(eols: &mut EolSummary, eol: LineEnding) {
    match eol {
        LineEnding::Lf => eols.lf += 1,
        LineEnding::CrLf => eols.crlf += 1,
        LineEnding::Cr => eols.cr += 1,
        LineEnding::None => eols.none += 1,
        LineEnding::Artificial => {}
    }
}

/// The counts of `spec` on `doc` (its current lines): one pass of the stages, nothing encoded. Cancellable every
/// 65,536 lines.
pub fn convert_counts(
    doc: &EditableDocument,
    spec: &ConvertSpec,
    ctl: &Ctl<'_>,
) -> Result<ConvertCounts, TextError> {
    let mut lines = Lines::new(doc.snapshot(), spec);
    let mut counts = ConvertCounts::default();
    while let Some(line) = lines.next_line() {
        if lines.next.is_multiple_of(CHECK_EVERY) {
            ctl.check()?;
        }
        counts.lines += 1;
        counts.changed += u64::from(line.touched);
        count_eol(&mut counts.eols, line.eol);
    }
    counts.steps = lines.counts();
    Ok(counts)
}

/// The recipe of a converted file's pieces after the BOM ([`Piece::Converted`]): the document, the conversions, and
/// either the splice source's lines (unchanged lines spliced) or a whole encode, then the Ctrl-Z tail.
pub struct ConvertStream {
    doc: Arc<TextDocument>,
    spec: ConvertSpec,
    encoding: TextEncoding,
    /// Splice: the source's lines; `None`: every line encoded through one encoder.
    map: Option<Arc<SourceMap>>,
    tail: Option<Range<u64>>,
    lossy: bool,
}

impl std::fmt::Debug for ConvertStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvertStream")
            .field("spec", &self.spec)
            .field("encoding", &self.encoding)
            .field("splice", &self.map.is_some())
            .field("tail", &self.tail)
            .finish()
    }
}

impl PartialEq for ConvertStream {
    fn eq(&self, other: &ConvertStream) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for ConvertStream {}

impl ConvertStream {
    /// True when the pieces read the splice source.
    pub(crate) fn reads_source(&self) -> bool {
        self.map.is_some() || self.tail.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// The pieces, made one at a time (as the save streams them).
    pub fn pieces(self: &Arc<ConvertStream>) -> ConvertPieces {
        ConvertPieces {
            stream: Arc::clone(self),
            lines: Lines::new(Arc::clone(&self.doc), &self.spec),
            encoder: Encoder::new(self.encoding),
            text: String::new(),
            source: None,
            ready: VecDeque::new(),
            next_orig: 0,
            at: 0,
            done: false,
            found: Found::default(),
            encoded_lines: 0,
            eols: EolSummary::default(),
        }
    }
}

/// The pieces of a [`ConvertStream`], in order: `Source` ranges (adjacent lines merged) and `Encoded` steps of at
/// most about 1 MiB.
pub struct ConvertPieces {
    stream: Arc<ConvertStream>,
    lines: Lines,
    encoder: Encoder,
    /// Text not yet encoded.
    text: String,
    /// A run of spliced lines not yet given out.
    source: Option<Range<u64>>,
    ready: VecDeque<Piece>,
    next_orig: u32,
    /// The table line of the result.
    at: u32,
    done: bool,
    found: Found,
    encoded_lines: u32,
    eols: EolSummary,
}

impl ConvertPieces {
    fn flush_source(&mut self) {
        if let Some(range) = self.source.take()
            && range.end > range.start
        {
            self.ready.push_back(Piece::Source {
                offset: range.start,
                len: range.end - range.start,
            });
        }
    }

    /// Encodes the gathered text: a splice encodes each run with an encoder of its own (as `save_content`), a whole
    /// encode keeps one encoder (a stateful encoding's state crosses steps), `last` closing it.
    fn flush_text(&mut self, last: bool) {
        let splice = self.stream.map.is_some();
        if self.text.is_empty() && (splice || !last) {
            return;
        }
        let mut bytes = Vec::with_capacity(self.text.len() + 16);
        self.found.replaced += if splice {
            Encoder::new(self.stream.encoding).encode(&self.text, true, &mut bytes)
        } else {
            self.encoder.encode(&self.text, last, &mut bytes)
        };
        self.text.clear();
        if !bytes.is_empty() {
            self.ready.push_back(Piece::Encoded(Arc::from(bytes)));
        }
    }

    fn take(&mut self, line: Line) {
        count_eol(&mut self.eols, line.eol);
        let at = self.at;
        self.at += 1;
        let spliced = match &self.stream.map {
            Some(map) if !line.touched && line.orig >= self.next_orig => map.line_bytes(line.orig),
            _ => None,
        };
        if let Some(range) = spliced {
            self.flush_text(false);
            self.next_orig = line.orig + 1;
            match &mut self.source {
                Some(run) if run.end == range.start => run.end = range.end,
                _ => {
                    self.flush_source();
                    self.source = Some(range);
                }
            }
            return;
        }
        self.flush_source();
        let lossy = self
            .found
            .look_text(&line.text, at, self.stream.encoding, self.stream.lossy);
        if lossy {
            self.found.lossy_line();
        }
        self.encoded_lines += 1;
        self.text.push_str(&line.text);
        self.text.push_str(line.eol.as_text());
        if self.text.len() >= STEP {
            self.flush_text(false);
        }
    }

    /// The next piece.
    fn next_piece(&mut self) -> Option<Piece> {
        loop {
            if let Some(piece) = self.ready.pop_front() {
                return Some(piece);
            }
            if self.done {
                return None;
            }
            match self.lines.next_line() {
                Some(line) => self.take(line),
                None => {
                    self.done = true;
                    self.flush_text(true);
                    self.flush_source();
                    if let Some(tail) = self.stream.tail.clone()
                        && tail.end > tail.start
                    {
                        self.ready.push_back(Piece::Source {
                            offset: tail.start,
                            len: tail.end - tail.start,
                        });
                    }
                }
            }
        }
    }
}

impl Iterator for ConvertPieces {
    type Item = Piece;

    fn next(&mut self) -> Option<Piece> {
        self.next_piece()
    }
}

/// What one pass of a [`ConvertStream`] measured.
struct Measured {
    bytes: u64,
    spliced: u64,
    encoded_lines: u32,
    eols: EolSummary,
    found: Found,
}

fn measure(stream: &Arc<ConvertStream>, ctl: &Ctl<'_>) -> Result<Measured, TextError> {
    let mut pieces = stream.pieces();
    let (mut bytes, mut spliced) = (0u64, 0u64);
    let mut checked = 0u32;
    while let Some(piece) = pieces.next_piece() {
        if pieces.at / CHECK_EVERY != checked {
            checked = pieces.at / CHECK_EVERY;
            ctl.check()?;
        }
        match piece {
            Piece::Source { len, .. } => {
                bytes += len;
                spliced += len;
            }
            Piece::Encoded(encoded) => bytes += encoded.len() as u64,
            Piece::Bom | Piece::Converted(_) => {}
        }
    }
    Ok(Measured {
        bytes,
        spliced,
        encoded_lines: pieces.encoded_lines,
        eols: pieces.eols,
        found: pieces.found,
    })
}

/// The content a save of `doc` converted by `spec` writes (W10b note 60): what [`super::save_content`] writes for the
/// document the conversions would make in an editor, made by one pass of the stages and the encoder (the pieces are
/// made again when the save streams them). `doc` is the document as loaded (its encoding and BOM set to the ones
/// written); `map` the splice source's lines when it was read and held its stamp.
pub fn converted_content(
    doc: &EditableDocument,
    map: Option<SourceMap>,
    spec: &ConvertSpec,
    reencode: Option<ReencodeReason>,
    ctl: &Ctl<'_>,
) -> Result<SaveContent, TextError> {
    let (encoding, bom) = doc.encoding();
    let physical = doc.orig_lines() - doc.orig_artificial().len() as u32;
    let reason = if let Some(reason) = reencode {
        Some(reason)
    } else if !doc.spliceable() || doc.line_count() != doc.orig_lines() {
        Some(ReencodeReason::NoSource)
    } else if doc
        .splice_encoding()
        .is_some_and(|(enc, _)| enc != encoding)
    {
        Some(ReencodeReason::EncodingChanged)
    } else if map.as_ref().is_none_or(|map| map.lines() != physical) {
        Some(ReencodeReason::NoSource)
    } else {
        None
    };
    let map = map.map(Arc::new);
    let snapshot = doc.snapshot();
    let lossy = snapshot.malformed() > 0;
    let bom_len = if bom { encoding.bom().len() as u64 } else { 0 };
    let make = |map: Option<Arc<SourceMap>>, tail: Option<Range<u64>>| {
        Arc::new(ConvertStream {
            doc: Arc::clone(&snapshot),
            spec: spec.clone(),
            encoding,
            map,
            tail,
            lossy,
        })
    };
    let content = |stream: Arc<ConvertStream>, measured: Measured, reason| {
        let mut pieces = Vec::with_capacity(2);
        if bom_len > 0 {
            pieces.push(Piece::Bom);
        }
        pieces.push(Piece::Converted(stream));
        let mut issues = measured.found.issues();
        if let Some(reason) = reason {
            issues.push(ContentIssue::Reencoded { reason });
        }
        SaveContent {
            pieces,
            bytes: bom_len + measured.bytes,
            encoding,
            bom: bom_len > 0,
            eols: measured.eols,
            spliced_bytes: measured.spliced,
            encoded_lines: measured.encoded_lines,
            issues,
        }
    };
    if reason.is_none()
        && let Some(map) = &map
    {
        let stream = make(Some(Arc::clone(map)), Some(map.tail()));
        let measured = measure(&stream, ctl)?;
        if measured.encoded_lines == 0 || !is_stateful(encoding) {
            return Ok(content(stream, measured, None));
        }
    }
    let reason = reason.unwrap_or(if is_stateful(encoding) {
        ReencodeReason::Stateful
    } else {
        ReencodeReason::NoSource
    });
    let tail = match (reason, &map) {
        (ReencodeReason::EncodingChanged | ReencodeReason::Stateful, Some(map)) => Some(map.tail()),
        _ => None,
    };
    let stream = make(None, tail);
    let measured = measure(&stream, ctl)?;
    Ok(content(stream, measured, Some(reason)))
}

/// The converted text of `doc`, line by line with its terminator (an External save conversion's input).
pub(crate) fn for_each_line(
    doc: &EditableDocument,
    spec: &ConvertSpec,
    mut f: impl FnMut(&str, &str),
) {
    let mut lines = Lines::new(doc.snapshot(), spec);
    while let Some(line) = lines.next_line() {
        f(&line.text, line.eol.as_text());
    }
}

/// True for an encoding whose shift state may cross lines.
fn is_stateful(encoding: TextEncoding) -> bool {
    encoding.name() == "ISO-2022-JP"
}
