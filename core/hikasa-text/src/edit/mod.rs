//! The editor's model (TC-060, TC-061, TC-050; D-261…D-265, D-290; contract §1.10.3–§1.10.7):
//! [`EditableDocument`] — positions, edits with the line-ending rules, unbounded undo and redo, line marks,
//! bookmarks — the pure editing [`commands`] and [`Editability`].
//!
//! ```text
//! apply(request)                      one request = one undo step (§1.10.4)
//!   → changes: each replaces bytes inside a span of whole physical lines (CPL pieces re-wrapped, a lone CR
//!     joined with a following empty LF line exactly as a fresh load would see the text)
//!   → per change: rope edit (bytes), block-table splice (only the lines whose text or terminator changed),
//!     marks (unchanged lines keep theirs), bookmarks follow their lines
//!   → EditOutcome { step, delta, selection, changed }
//! ```
//!
//! Every call is in memory and costs O(edit size + log lines + blocks), so a UI thread may call it (D-105). The
//! document's text is always exactly what a fresh load of it would index (P-BLOCK, A10.1).

mod apply;
pub mod commands;
pub mod cursor;
pub mod editability;
mod pos;
mod undo;
pub(crate) mod words;

use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;

use hikasa_core::Spec;
use hikasa_formats::TextEncoding;
use hikasa_vfs::ops::ItemStamp;
use serde::{Deserialize, Serialize};

use crate::document::{LineEnding, LineIdx, LineMarks, LoadStamp, TextDocument};

pub use commands::{
    CopyEdit, Dir, EditCommand, copy_rows, line_home, plan_command, select_section, word_at,
    word_boundary,
};
pub use cursor::{
    LineSet, char_width, cluster_starts, delete_lines, doc_end, lines_text, next_pos, prev_pos,
};
pub use editability::{EditBlock, Editability};
pub use pos::{Selection, TextPos};

use undo::UndoLog;

/// The options of one pane's editor (from the pane's format, OPT-006 and D-290).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOptions {
    /// Tab size in columns (FF-003). 0 counts as 1.
    pub tab_size: u8,
    /// Tab inserts spaces.
    pub tab_as_spaces: bool,
    /// Extra word characters of the format (D-58), besides letters, digits and `_`.
    pub word_chars: String,
    /// New lines start with the leading blanks of the line they were split from (OPT-006).
    pub auto_indent: bool,
    /// Backspace inside leading blanks goes back to the previous tab stop (OPT-006).
    pub backspace_unindents: bool,
    /// The terminator of a document without a terminated line (D-290, resolved by the host).
    pub default_eol: LineEnding,
    /// The pane's characters-per-line limit (TC-003; `None` or `Some(0)`: none).
    pub chars_per_line_limit: Option<u32>,
}

impl Default for EditOptions {
    /// Tabs of 4, no spaces, no extra word characters, auto indent and backspace unindent on (OPT-006
    /// defaults), LF, no limit.
    fn default() -> EditOptions {
        EditOptions {
            tab_size: 4,
            tab_as_spaces: false,
            word_chars: String::new(),
            auto_indent: true,
            backspace_unindents: true,
            default_eol: LineEnding::Lf,
            chars_per_line_limit: None,
        }
    }
}

/// An edit (§1.10.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRequest {
    /// Replaces `range` with `text`. Line breaks in `text` (LF, CRLF, CR) become the document's insert terminator
    /// ([`EditableDocument::insert_eol`]), except with `keep_eols` (a copy from the other pane), which keeps them.
    Replace {
        /// The range (an end before its start is swapped).
        range: Range<TextPos>,
        /// The new text.
        text: String,
        /// Keep the breaks of `text` as given.
        keep_eols: bool,
    },
    /// Several replacements as one step; ranges in document order and disjoint.
    Batch {
        /// `(range, text)` pairs.
        edits: Vec<(Range<TextPos>, String)>,
        /// Keep the breaks of the texts as given.
        keep_eols: bool,
    },
    /// Sets the terminator of lines `lines` (`None` only for the last line; artificial pieces keep theirs).
    SetEols {
        /// The lines.
        lines: Range<u32>,
        /// The terminator.
        eol: LineEnding,
    },
    /// The save encoding and BOM (§1.10.12): a property of the document, undone like an edit.
    SetEncoding {
        /// Encoding.
        encoding: TextEncoding,
        /// Write a BOM.
        bom: bool,
    },
}

/// How an edit joins the undo log (§1.10.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupHint {
    /// A step of its own.
    None,
    /// Typing at the adapter's monotonic time `at_ms`: a one-character insert or delete merges with the previous
    /// typing step of the same kind within 1,000 ms, at most one word a step.
    Typing {
        /// Milliseconds of the adapter's monotonic clock (core reads no clock, T1).
        at_ms: u64,
    },
    /// Merges with the previous request of the same group id (a replace-all of the find panel).
    Group(u64),
}

/// The id of an undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UndoId(pub u64);

/// Lines `[at, at + removed)` became `[at, at + inserted)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineDelta {
    /// First line.
    pub at: u32,
    /// Lines before.
    pub removed: u32,
    /// Lines after.
    pub inserted: u32,
}

impl LineDelta {
    /// No change.
    pub const NONE: LineDelta = LineDelta {
        at: 0,
        removed: 0,
        inserted: 0,
    };

    /// True when no line changed.
    pub fn is_empty(&self) -> bool {
        self.removed == 0 && self.inserted == 0
    }
}

/// What an applied edit, undo or redo did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditOutcome {
    /// The undo step it made or extended (for undo and redo: the step undone or redone).
    pub step: UndoId,
    /// The lines that changed, as one covering delta (the live comparison's input, §1.10.10).
    pub delta: LineDelta,
    /// The selection after the edit (the caret after the inserted text; for undo, the selection before the step).
    pub selection: Selection,
    /// Lines whose text or terminator changed (new numbering).
    pub changed: Range<u32>,
    /// Every change of the step in the order applied, each as its own delta (the live comparison replays them
    /// one by one; the covering `delta` is their union).
    pub deltas: Vec<LineDelta>,
}

/// Why an edit was refused (nothing changed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    /// The pane may not be edited (§1.10.7); the first cause.
    Blocked(EditBlock),
    /// A position past a line's content, inside a char, or beyond the last line; overlapping batch ranges.
    BadRange,
    /// The document would pass `MAX_DOCUMENT_BYTES` or `MAX_LINES` (D-43).
    TooLarge,
    /// The request does not apply to this document.
    Unsupported(&'static str),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Blocked(b) => write!(f, "the document may not be edited ({})", b.as_str()),
            EditError::BadRange => f.write_str("the position is not in the document"),
            EditError::TooLarge => f.write_str("the edit would pass the document limits"),
            EditError::Unsupported(why) => write!(f, "unsupported edit: {why}"),
        }
    }
}

impl std::error::Error for EditError {}

/// The mark of a line (TC-031's two colours, §1.10.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineMark {
    /// As loaded.
    Original,
    /// Changed since the last save (or the load).
    Edited,
    /// Changed before the last save.
    SavedEdit,
}

/// Bookmarks 0–9: the line of each number.
pub type Bookmarks = [Option<LineIdx>; 10];

/// The file a save splices unchanged lines from (§1.10.12, M2 W10b): the file as it was loaded or last saved, and the
/// stamp the load (or the save) took of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpliceSource {
    /// The file.
    pub spec: Spec,
    /// Its size, time and CRC-32 when it was read or written.
    pub stamp: LoadStamp,
}

/// What one save wrote, for [`EditableDocument::saved_to`] (M2 W10b, §1.10.13): `hikasa_text::save` fills it per
/// done item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedFile {
    /// The file written (the file a link led to, for a save through a link).
    pub target: Spec,
    /// The new file's stamp: its size and time after the write, the CRC-32 of the bytes written.
    pub stamp: LoadStamp,
    /// The document generation the written content was made from.
    pub generation: u64,
    /// The encoding written.
    pub encoding: TextEncoding,
    /// A BOM was written.
    pub bom: bool,
    /// Save As: the document's location moves to `target` (title, editability); `false` for an in-place save and for
    /// a save to the view's `--savetarget`, where only the splice source becomes `target`.
    pub moves_location: bool,
    /// The backup the save made and what it holds (the replaced file's stamp): the next save replaces it without the
    /// `OverwriteBackup` question.
    pub backup: Option<(Spec, ItemStamp)>,
}

/// Where a document lives and what a save needs of its file (§1.10.12–§1.10.14, M2 W10b).
#[derive(Debug, Clone, Default)]
struct SaveState {
    /// The document's own file (title, editability, the in-place save); `None`: Save is Save As.
    location: Option<Spec>,
    /// The splice source.
    source: Option<SpliceSource>,
    /// The encoding and BOM length of the splice source's bytes.
    source_encoding: Option<(TextEncoding, u8)>,
    /// The unchanged lines' `orig` indexes point into the splice source (false after a save of an older
    /// generation, and for a converted or truncated document).
    spliceable: bool,
    /// The view's `--savetarget` (CLI-003): an in-place save writes there instead.
    save_target: Option<Spec>,
    /// Table lines of the document as loaded or last saved (the `orig` index space).
    orig_lines: u32,
    /// The artificial lines (CPL, TC-003) of the document as loaded or last saved, in order: an `orig` index starts a
    /// physical line of the source when the one before it is not in this list. Empty without artificial breaks.
    orig_artificial: Arc<[u32]>,
    /// The backup the last save made, with the replaced file's stamp.
    last_backup: Option<(Spec, ItemStamp)>,
    /// The generation `apply_save_options` left and the lines it changed.
    pre_save: Option<(u64, u64)>,
}

/// How the lines of a document moved through [`EditableDocument::set_options`] (a re-wrap of a new characters-per-line
/// limit): old positions and lines to new ones. The identity when nothing was re-wrapped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineMap {
    /// The first byte of every old table line, plus the text's end (`None`: the identity).
    old_starts: Option<Arc<[u32]>>,
}

impl LineMap {
    /// Nothing moved.
    pub fn identity() -> LineMap {
        LineMap::default()
    }

    /// True when every position stays where it was.
    pub fn is_identity(&self) -> bool {
        self.old_starts.is_none()
    }

    /// The new position of `old` (a position of the document before the re-wrap): the same byte of the text.
    pub fn pos(&self, doc: &EditableDocument, old: TextPos) -> TextPos {
        let Some(starts) = &self.old_starts else {
            return doc.clamp(old);
        };
        let line = (old.line.0 as usize).min(starts.len() - 1);
        let next = starts.get(line + 1).copied().unwrap_or(starts[line]);
        let abs = starts[line] + old.byte.min(next.saturating_sub(starts[line]));
        doc.clamp(doc.pos_of(abs))
    }

    /// The new line of old line `old` (the line holding its first byte; `old` = the old line count maps to the new
    /// one).
    pub fn line(&self, doc: &EditableDocument, old: u32) -> u32 {
        match &self.old_starts {
            None => old,
            Some(starts) if old as usize + 1 >= starts.len() => doc.line_count(),
            Some(_) => self.pos(doc, TextPos::new(old, 0)).line.0,
        }
    }

    /// The new lines of old lines `old`.
    pub fn lines(&self, doc: &EditableDocument, old: Range<u32>) -> Range<u32> {
        let start = self.line(doc, old.start);
        start..self.line(doc, old.end).max(start)
    }
}

/// One pane's editable document (§1.10.3): a plain value owned by its view.
#[derive(Debug, Clone)]
pub struct EditableDocument {
    doc: TextDocument,
    opts: EditOptions,
    editability: Editability,
    binary_allowed: bool,
    insert_eol: LineEnding,
    encoding: (TextEncoding, bool),
    generation: u64,
    saved_gen: u64,
    log: UndoLog,
    bookmarks: [Option<u32>; 10],
    /// The D-43 limits (tests lower them).
    limits: (u64, u32),
    /// Location, splice source and save target (M2 W10b).
    save: SaveState,
}

impl EditableDocument {
    /// The editor of `doc`. The insert terminator is the loaded document's dominant one (ties Lf > CrLf > Cr)
    /// when it has a terminated line, else `opts.default_eol` (D-290); the save encoding is the loaded one, with
    /// a BOM when the source had one.
    pub fn new(
        doc: Arc<TextDocument>,
        opts: EditOptions,
        editability: Editability,
    ) -> EditableDocument {
        let mut doc = Arc::try_unwrap(doc).unwrap_or_else(|shared| (*shared).clone());
        doc.normalize_wrap(opts.chars_per_line_limit);
        let eol = doc.eol();
        let insert_eol = if eol.lf + eol.crlf + eol.cr > 0 {
            eol.dominant()
        } else {
            match opts.default_eol {
                LineEnding::None | LineEnding::Artificial => LineEnding::Lf,
                other => other,
            }
        };
        let encoding = (doc.encoding().encoding, doc.encoding().bom_len > 0);
        let origin = doc.origin().spec.clone();
        let source = match (&origin, doc.load_stamp()) {
            (Some(spec), Some(stamp)) if editability.can_save_in_place => Some(SpliceSource {
                spec: spec.clone(),
                stamp,
            }),
            _ => None,
        };
        let save = SaveState {
            location: origin.filter(|_| editability.can_save_in_place),
            spliceable: source.is_some() && !doc.converted() && !doc.truncated(),
            source_encoding: source
                .as_ref()
                .map(|_| (doc.encoding().encoding, doc.encoding().bom_len)),
            source,
            save_target: None,
            orig_lines: doc.line_count(),
            orig_artificial: Arc::from(doc.artificial_lines()),
            last_backup: None,
            pre_save: None,
        };
        EditableDocument {
            doc,
            opts,
            editability,
            binary_allowed: false,
            insert_eol,
            encoding,
            generation: 0,
            saved_gen: 0,
            log: UndoLog::default(),
            bookmarks: [None; 10],
            limits: (crate::MAX_DOCUMENT_BYTES, crate::MAX_LINES),
            save,
        }
    }

    /// A snapshot of the current text: an immutable document sharing the rope and every block (O(blocks)).
    pub fn snapshot(&self) -> Arc<TextDocument> {
        Arc::new(self.doc.clone())
    }

    /// The current document (borrowed).
    pub fn document(&self) -> &TextDocument {
        &self.doc
    }

    /// The options.
    pub fn options(&self) -> &EditOptions {
        &self.opts
    }

    /// Lines (0 for an empty document).
    pub fn line_count(&self) -> u32 {
        self.doc.line_count()
    }

    /// Lines that hold positions: `line_count()`, plus one empty line after a terminated last line (or the one
    /// line of an empty document), where the caret goes after a final Enter.
    pub fn position_lines(&self) -> u32 {
        let n = self.doc.line_count();
        if n == 0 || self.doc.line_ending(LineIdx(n - 1)) != LineEnding::None {
            n + 1
        } else {
            n
        }
    }

    /// Content of `line` (`""` out of range and on the empty line after a terminated last line).
    pub fn line(&self, line: LineIdx) -> Cow<'_, str> {
        self.doc.line(line)
    }

    /// Terminator of `line`.
    pub fn line_ending(&self, line: LineIdx) -> LineEnding {
        self.doc.line_ending(line)
    }

    /// Byte length of `line`'s content.
    pub fn line_len(&self, line: LineIdx) -> u32 {
        self.doc.line_len(line.0) as u32
    }

    /// The text of `range`, terminators as stored (artificial breaks add nothing). Positions are clamped.
    pub fn text(&self, range: Range<TextPos>) -> String {
        let (a, b) = order(range);
        let (a, b) = (self.clamp(a), self.clamp(b));
        let (s, e) = (self.abs(a), self.abs(b));
        let rope = self.doc.rope();
        if s >= e {
            return String::new();
        }
        rope.byte_slice(s as usize..e as usize).to_string()
    }

    /// The insert terminator (§1.10.3).
    pub fn insert_eol(&self) -> LineEnding {
        self.insert_eol
    }

    /// The save encoding and BOM.
    pub fn encoding(&self) -> (TextEncoding, bool) {
        self.encoding
    }

    /// True unless the undo log stands at the last save point (or at the load when never saved).
    pub fn is_modified(&self) -> bool {
        self.log.is_modified()
    }

    /// +1 per applied edit, undo or redo.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `pos` in range: the line clamped to the position lines, the byte to the content, then left to a char
    /// boundary.
    pub fn clamp(&self, pos: TextPos) -> TextPos {
        let lines = self.position_lines();
        if pos.line.0 >= lines {
            let last = lines - 1;
            return TextPos::new(last, self.line_len(LineIdx(last)));
        }
        let text = self.doc.line(pos.line);
        let mut byte = (pos.byte as usize).min(text.len());
        while !text.is_char_boundary(byte) {
            byte -= 1;
        }
        TextPos::new(pos.line.0, byte as u32)
    }

    /// The editability (the Binary block is gone after [`EditableDocument::allow_binary_edits`]).
    pub fn editability(&self) -> Editability {
        let mut e = self.editability.clone();
        if self.binary_allowed {
            e.blocks.retain(|&b| b != EditBlock::Binary);
        }
        e
    }

    /// Replaces the editability (a later run clearing `Cancelled`, a session change).
    pub fn set_editability(&mut self, editability: Editability) {
        self.editability = editability;
    }

    /// Lifts the `Binary` block after the view's *contains binary data, edit anyway?* prompt (W11). Nothing else
    /// lifts it.
    pub fn allow_binary_edits(&mut self) {
        self.binary_allowed = true;
    }

    /// Applies an edit as one undo step (merged with the previous one per `group`, §1.10.4).
    pub fn apply(&mut self, req: &EditRequest, group: GroupHint) -> Result<EditOutcome, EditError> {
        if let Some(block) = self.editability().first() {
            return Err(EditError::Blocked(block));
        }
        self.apply_unchecked(req, group)
    }

    /// The mark of `line` (§1.10.5).
    pub fn mark(&self, line: LineIdx) -> LineMark {
        let LineMarks { generation, .. } = self.doc.marks(line);
        if generation == 0 {
            LineMark::Original
        } else if u64::from(generation) > self.saved_gen {
            LineMark::Edited
        } else {
            LineMark::SavedEdit
        }
    }

    /// The raw marks of `line`.
    pub fn marks(&self, line: LineIdx) -> LineMarks {
        self.doc.marks(line)
    }

    /// The generation at the last save (0 before).
    pub fn saved_generation(&self) -> u64 {
        self.saved_gen
    }

    /// Records an in-place save of the current text (§1.10.5, §1.10.13): the current generation becomes the saved
    /// one, every line's file line becomes its own index, `Edited` lines become `SavedEdit`, the undo log's position
    /// becomes the save point, and the file written (the splice source, else the location) with `stamp` becomes the
    /// next splice source. [`EditableDocument::saved_to`] is the general form a save's report applies.
    pub fn saved(&mut self, stamp: LoadStamp) {
        let target = self
            .save
            .source
            .as_ref()
            .map(|source| source.spec.clone())
            .or_else(|| self.save.location.clone());
        match target {
            Some(target) => self.saved_to(&SavedFile {
                target,
                stamp,
                generation: self.generation,
                encoding: self.encoding.0,
                bom: self.encoding.1,
                moves_location: false,
                backup: None,
            }),
            None => self.mark_saved_now(),
        }
    }

    /// Records what a save wrote (§1.10.13, M2 W10b). When `saved.generation` is the current generation the text
    /// written is the document's: it is saved as [`EditableDocument::saved`] says, and `saved.target` with its stamp
    /// becomes the splice source (the `orig` indexes point into it). When the document changed after the plan was
    /// made, the file holds an older text: the document stays modified and the next save encodes the whole text
    /// (its lines no longer point into any file). A Save As (`moves_location`) also moves the location.
    pub fn saved_to(&mut self, saved: &SavedFile) {
        let current = saved.generation == self.generation;
        if current {
            self.mark_saved_now();
        }
        self.save.source = Some(SpliceSource {
            spec: saved.target.clone(),
            stamp: saved.stamp,
        });
        let bom_len = if saved.bom {
            saved.encoding.bom().len() as u8
        } else {
            0
        };
        self.save.source_encoding = Some((saved.encoding, bom_len));
        self.save.spliceable = current;
        if saved.moves_location {
            self.save.location = Some(saved.target.clone());
            self.editability.can_save_in_place = true;
        }
        self.save.last_backup.clone_from(&saved.backup);
    }

    /// The marks, the saved generation and the save point of a save of the current text.
    fn mark_saved_now(&mut self) {
        self.saved_gen = self.generation;
        self.doc.rebase_orig();
        self.log.mark_saved();
        self.save.orig_lines = self.doc.line_count();
        self.save.orig_artificial = Arc::from(self.doc.artificial_lines());
    }

    /// The document's own file (§1.10.14): where an in-place save writes (unless a save target is set), the title,
    /// the editability. `None` for a document without one (`-`, `clipboard://`, text pairs): Save is Save As.
    pub fn location(&self) -> Option<&Spec> {
        self.save.location.as_ref()
    }

    /// True when Save writes the document's own file (a location, and `can_save_in_place`).
    pub fn can_save_in_place(&self) -> bool {
        self.save.location.is_some() && self.editability.can_save_in_place
    }

    /// Sets the location (a host that knows better, a reload). The splice source is not changed.
    pub fn set_location(&mut self, location: Option<Spec>) {
        self.editability.can_save_in_place = location.is_some();
        self.save.location = location;
    }

    /// The file a save splices unchanged lines from, with its stamp (§1.10.12): the file as loaded or last saved.
    pub fn splice_source(&self) -> Option<&SpliceSource> {
        self.save.source.as_ref()
    }

    /// The encoding and BOM length of the splice source's bytes.
    pub fn splice_encoding(&self) -> Option<(TextEncoding, u8)> {
        self.save.source_encoding
    }

    /// True when the unchanged lines' file lines point into the splice source, so a save can splice them.
    pub fn spliceable(&self) -> bool {
        self.save.spliceable && self.save.source.is_some()
    }

    /// The view's `--savetarget` (CLI-003, §1.10.14): an in-place Save writes there, the document keeps its
    /// location.
    pub fn save_target(&self) -> Option<&Spec> {
        self.save.save_target.as_ref()
    }

    /// Sets the save target (the Service, from `LiveOptions::save_target`).
    pub fn set_save_target(&mut self, target: Option<Spec>) {
        self.save.save_target = target;
    }

    /// The backup the last save made, with the stamp of the file it holds.
    pub fn last_backup(&self) -> Option<&(Spec, ItemStamp)> {
        self.save.last_backup.as_ref()
    }

    /// Lines the format's *when saving* options changed in the last step, when it is the current one
    /// (`hikasa_text::save::apply_save_options`); 0 otherwise.
    pub fn pre_save_lines(&self) -> u64 {
        match self.save.pre_save {
            Some((generation, lines)) if generation == self.generation => lines,
            _ => 0,
        }
    }

    /// Records the lines the save options changed in the step just applied.
    pub(crate) fn note_pre_save(&mut self, lines: u64) {
        self.save.pre_save = Some((self.generation, lines));
    }

    /// Table lines of the document as loaded or last saved: the index space of `orig`.
    pub(crate) fn orig_lines(&self) -> u32 {
        self.save.orig_lines
    }

    /// The artificial lines of the document as loaded or last saved.
    pub(crate) fn orig_artificial(&self) -> &[u32] {
        &self.save.orig_artificial
    }

    /// The next maximal run of `Edited` or `SavedEdit` lines strictly after `from` (`Dir::Forward`) or strictly
    /// before it (`Dir::Backward`); `None` when there is none.
    pub fn edited_ranges(&self, from: LineIdx, dir: Dir) -> Option<Range<u32>> {
        let n = self.line_count();
        let marked = |l: u32| self.doc.marks(LineIdx(l)).generation != 0;
        match dir {
            Dir::Forward => {
                // Skip the run `from` is in.
                let mut l = from.0.saturating_add(1);
                if from.0 < n && marked(from.0) {
                    while l < n && marked(l) {
                        l += 1;
                    }
                }
                let start = self.doc.table().next_marked(l, n)?;
                let mut end = start + 1;
                while end < n && marked(end) {
                    end += 1;
                }
                Some(start..end)
            }
            Dir::Backward => {
                let mut l = from.0.min(n);
                if from.0 < n && marked(from.0) {
                    while l > 0 && marked(l - 1) {
                        l -= 1;
                    }
                }
                let end = self.doc.table().prev_marked(l)? + 1;
                let mut start = end - 1;
                while start > 0 && marked(start - 1) {
                    start -= 1;
                }
                Some(start..end)
            }
        }
    }

    /// Toggles bookmark `n` (0–9) on `line`: removes it when it is on that line, else moves it there.
    pub fn toggle_bookmark(&mut self, n: u8, line: LineIdx) {
        let Some(slot) = self.bookmarks.get_mut(usize::from(n)) else {
            return;
        };
        *slot = if *slot == Some(line.0) {
            None
        } else {
            Some(line.0)
        };
    }

    /// The line of bookmark `n`.
    pub fn bookmark(&self, n: u8) -> Option<LineIdx> {
        self.bookmarks
            .get(usize::from(n))
            .copied()
            .flatten()
            .map(LineIdx)
    }

    /// Every bookmark.
    pub fn bookmarks(&self) -> Bookmarks {
        self.bookmarks.map(|b| b.map(LineIdx))
    }

    /// Removes every bookmark.
    pub fn clear_bookmarks(&mut self) {
        self.bookmarks = [None; 10];
    }

    /// True when a step can be undone.
    pub fn can_undo(&self) -> bool {
        self.log.can_undo() && self.editability().editable()
    }

    /// True when a step can be redone (and the pane is not blocked).
    pub fn can_redo(&self) -> bool {
        self.log.can_redo() && self.editability().editable()
    }

    /// Undoes the last step (§1.10.4): its inverse changes, applied as one edit, so the live comparison sees an
    /// edit. `None` when there is nothing to undo or the pane is blocked (undo changes the text like any edit;
    /// [`EditableDocument::try_undo`] gives the block).
    pub fn undo(&mut self) -> Option<EditOutcome> {
        self.try_undo().ok().flatten()
    }

    /// Redoes the last undone step; `None` when there is none or the pane is blocked.
    pub fn redo(&mut self) -> Option<EditOutcome> {
        self.try_redo().ok().flatten()
    }

    /// [`EditableDocument::undo`] with the reason of a refusal: `Err(Blocked)` when the pane may not be edited.
    pub fn try_undo(&mut self) -> Result<Option<EditOutcome>, EditError> {
        if let Some(block) = self.editability().first() {
            return Err(EditError::Blocked(block));
        }
        Ok(self.undo_unchecked())
    }

    /// [`EditableDocument::redo`] with the reason of a refusal.
    pub fn try_redo(&mut self) -> Result<Option<EditOutcome>, EditError> {
        if let Some(block) = self.editability().first() {
            return Err(EditError::Blocked(block));
        }
        Ok(self.redo_unchecked())
    }

    /// Drops the undo log (W11's *Recompare Files*, a reload); marks stay.
    pub fn clear_undo(&mut self) {
        self.log.clear();
    }

    /// Number of steps in the log (undoable plus redoable).
    pub fn undo_steps(&self) -> usize {
        self.log.len()
    }

    /// New editing options (a format change, M2 W11a §1.11.13, D-304): tab size, tab as spaces, word characters, the
    /// OPT-006 flags, the default terminator and the characters-per-line limit. No undo step is made (artificial breaks
    /// are no content) and the generation does not move.
    ///
    /// A different limit re-wraps every physical line; positions map through the re-wrap (a position keeps its byte
    /// offset in the text, so a `TextPos` of a physical line keeps its byte in that line), bookmarks follow their
    /// lines and each line keeps the highest mark its physical line had. Because the undo log names lines of the old
    /// wrapping, a re-wrap that changes the lines clears the log (W11a note 3; the document stays modified when it
    /// was), and a modified document then saves by encoding its whole text (its lines no longer point into the file
    /// as wrapped before); an unmodified one takes the new wrapping as its file's. The returned [`LineMap`] maps the
    /// view's carets, anchors and ignored lines.
    pub fn set_options(&mut self, opts: EditOptions) -> LineMap {
        let cpl = |o: &EditOptions| o.chars_per_line_limit.filter(|&c| c > 0);
        let (old, new) = (cpl(&self.opts), cpl(&opts));
        self.opts = opts;
        if old == new {
            return LineMap::identity();
        }
        let table = self.doc.table();
        let starts: Arc<[u32]> = (0..=table.len()).map(|l| table.start_of(l)).collect();
        let marks: Vec<(usize, u32)> = self
            .bookmarks
            .iter()
            .enumerate()
            .filter_map(|(n, b)| b.map(|l| (n, starts.get(l as usize).copied().unwrap_or(0))))
            .collect();
        if !self.doc.rewrap(new) {
            return LineMap::identity();
        }
        self.log.clear();
        if self.is_modified() {
            self.save.spliceable = false;
        } else {
            // The text is the file's: its lines under the new wrapping are the file's lines.
            self.doc.rebase_orig();
            self.save.orig_lines = self.doc.line_count();
            self.save.orig_artificial = Arc::from(self.doc.artificial_lines());
        }
        for (n, abs) in marks {
            self.bookmarks[n] = Some(self.pos_of(abs).line.0);
        }
        LineMap {
            old_starts: Some(starts),
        }
    }

    /// Lowers the D-43 limits (`TooLarge` tests without 4 GiB documents). **Not a stable API.**
    #[doc(hidden)]
    pub fn with_limits(mut self, max_bytes: u64, max_lines: u32) -> EditableDocument {
        self.limits = (max_bytes, max_lines);
        self
    }

    /// Absolute byte offset of a valid position.
    fn abs(&self, pos: TextPos) -> u32 {
        self.doc.table().start_of(pos.line.0) + pos.byte
    }

    /// The position of an absolute byte offset (at an artificial break: the start of the next piece).
    fn pos_of(&self, off: u32) -> TextPos {
        let table = self.doc.table();
        let line = table.line_at_byte(off);
        if line >= table.len() {
            // At the end: the empty line after a terminated last line, or the end of the last line.
            let n = table.len();
            if n == 0 {
                return TextPos::ZERO;
            }
            if table.eol(n - 1) == LineEnding::None {
                return TextPos::new(n - 1, self.line_len(LineIdx(n - 1)));
            }
            return TextPos::new(n, 0);
        }
        let start = table.start_of(line);
        let len = self.line_len(LineIdx(line));
        let byte = (off - start).min(len);
        if byte == len && table.eol(line) == LineEnding::Artificial && line + 1 < table.len() {
            return TextPos::new(line + 1, 0);
        }
        TextPos::new(line, byte)
    }

    /// Checks a position: a line holding positions, a byte within the content at a char boundary.
    fn check(&self, pos: TextPos) -> Result<(), EditError> {
        if pos.line.0 >= self.position_lines() {
            return Err(EditError::BadRange);
        }
        let text = self.doc.line(pos.line);
        if pos.byte as usize > text.len() || !text.is_char_boundary(pos.byte as usize) {
            return Err(EditError::BadRange);
        }
        Ok(())
    }
}

/// `range` with its start first.
fn order(range: Range<TextPos>) -> (TextPos, TextPos) {
    if range.start <= range.end {
        (range.start, range.end)
    } else {
        (range.end, range.start)
    }
}

/// `text` with every break (LF, CRLF, lone CR) replaced by `eol`.
fn normalize_breaks(text: &str, eol: LineEnding) -> Cow<'_, str> {
    if !text.contains(['\n', '\r']) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push_str(eol.as_text());
            }
            '\n' => out.push_str(eol.as_text()),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaks_are_normalized() {
        assert_eq!(
            normalize_breaks("a\r\nb\rc\nd", LineEnding::Lf),
            "a\nb\nc\nd"
        );
        assert_eq!(normalize_breaks("a\nb", LineEnding::CrLf), "a\r\nb");
        assert_eq!(normalize_breaks("plain", LineEnding::Cr), "plain");
    }

    #[test]
    fn documents_are_send() {
        fn check<T: Send + Sync>() {}
        check::<EditableDocument>();
    }
}
