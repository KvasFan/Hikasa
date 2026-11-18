//! The clipboard of the text panes (M2 W11a, contract §1.11.7, D-298), toolkit-free: what Copy and Cut write (the
//! selected text with its terminators as stored; a line set's lines; without a selection the caret's line with its
//! terminator), what Paste applies (`EditCommand::Paste`, breaks becoming the pane's insert terminator), and the
//! primary selection (X11, Wayland): written when a selection settles (the button released, or 300 ms after the last
//! Shift+move), capped at 1 MiB. The window's provider does the reads and writes on its jobs (`app/text.rs`).

use hikasa_core::Side;
use hikasa_text::edit::{
    EditCommand, EditRequest, EditableDocument, delete_lines, lines_text, plan_command,
};

use super::edit::{self, Origin};
use super::pane::line_selection;
use super::{TextRequest, TextViewModel, index_of};

/// The primary selection is written this long after the last Shift+move.
pub const PRIMARY_SETTLE_S: f64 = 0.300;
/// The most text the primary selection takes.
pub const PRIMARY_MAX_BYTES: usize = 1 << 20;
/// The message of a paste without text.
pub const NO_TEXT: &str = "The clipboard holds no text.";

/// When the primary selection is written next.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PrimaryState {
    /// The frame time it settles at; `None`: nothing waits.
    pub settle_at: Option<f64>,
    /// Writes asked so far (tests).
    pub written: u64,
}

impl PrimaryState {
    /// When the view wants a frame for it.
    pub fn due_at(&self) -> Option<f64> {
        self.settle_at
    }
}

/// The text Copy takes from the focus pane: the selection (a line set: its lines), else the caret's line with its
/// terminator (D-298's open default). `None` before the documents exist.
pub fn copy_text(view: &TextViewModel) -> Option<String> {
    let side = view.focus();
    let doc = view.doc(side)?;
    let pane = view.pane(side);
    if let Some(lines) = pane.lines.as_ref().filter(|l| !l.is_empty()) {
        return Some(lines_text(doc, lines));
    }
    if pane.anchor != pane.caret {
        return Some(doc.text(pane.selection().range()));
    }
    let (a, b) = line_selection(doc, pane.caret.line.0);
    Some(doc.text(a..b))
}

/// A Cut waiting for its clipboard write: the edit that deletes exactly the text written, planned on the document
/// of that moment (the pane is frozen until the write lands, so the document stays that one).
#[derive(Debug, Clone, PartialEq)]
pub struct Cutting {
    /// The deletion.
    pub request: EditRequest,
    /// The document's generation when it was planned.
    pub generation: u64,
    /// The clipboard holds the text, but a relive held the documents when it landed: the deletion is applied when
    /// the relive has landed ([`relive_landed`]).
    pub written: bool,
}

/// The message of a Cut whose document changed before its clipboard write landed (the text stays).
pub const CUT_CHANGED: &str = "The text changed before the clipboard was written: nothing was cut.";

/// The message of a Cut whose clipboard write failed (the text stays).
pub const CUT_KEPT: &str = "The clipboard could not be written: nothing was cut.";

/// Copy (`cut` false) or Cut of the focus pane (D-298): the text goes to the window's clipboard (a job,
/// [`TextRequest::WriteClipboard`]); a Cut deletes that exact text only once the clipboard holds it ([`written`]),
/// the pane frozen meanwhile. A frozen pane queues the command (its selection is the one of its turn). True when
/// something was written or queued.
pub fn copy(view: &mut TextViewModel, cut: bool) -> bool {
    let side = view.focus();
    if view.frozen(side) {
        super::input::queue(view, side, super::input::Input::Clip(cut));
        return true;
    }
    let Some(text) = copy_text(view) else {
        return false;
    };
    if cut {
        let Some(request) = cut_request(view) else {
            return false;
        };
        let generation = view.doc(side).map_or(0, EditableDocument::generation);
        view.cutting[index_of(side)] = Some(Cutting {
            request,
            generation,
            written: false,
        });
        view.dirty = true;
    }
    view.request_window(TextRequest::WriteClipboard { side, text, cut });
    true
}

/// The deletion of a Cut of the focus pane: the selection (a line set: its lines), else the caret's line.
fn cut_request(view: &TextViewModel) -> Option<EditRequest> {
    let side = view.focus();
    let doc = view.doc(side)?;
    let pane = view.pane(side);
    if let Some(lines) = pane.lines.as_ref().filter(|l| !l.is_empty()) {
        return delete_lines(doc, lines);
    }
    if pane.anchor != pane.caret {
        return plan_command(doc, pane.selection(), &EditCommand::Delete).map(|(req, _)| req);
    }
    let (a, b) = line_selection(doc, pane.caret.line.0);
    (a != b).then_some(EditRequest::Replace {
        range: a..b,
        text: String::new(),
        keep_eols: false,
    })
}

/// The clipboard write of a Copy or Cut of `side` ended (`ok`: the clipboard holds the text). A Cut deletes its
/// text now, as one step; a failed write keeps it (a message says so). The pane thaws: its queued inputs follow.
pub fn written(view: &mut TextViewModel, side: Side, cut: bool, ok: bool) {
    if !cut {
        return;
    }
    let Some(mut pending) = view.cutting[index_of(side)].take() else {
        return;
    };
    view.dirty = true;
    if !ok {
        view.request_window(TextRequest::Message(crate::status::Message::info(CUT_KEPT)));
        return;
    }
    if view.relive.is_some() {
        // A relive compares copies of the documents and installs that comparison: an edit now would not be in it
        // (W13c review finding 15, a race of P-LIVE-VIEW). The deletion waits for the landing; the pane stays frozen.
        pending.written = true;
        view.cutting[index_of(side)] = Some(pending);
        return;
    }
    apply_cut(view, side, pending);
}

/// A relive landed or ended: the Cuts whose clipboard write landed meanwhile delete their text now, unless the
/// relive moved that side's lines (`reshaped`: a re-wrap, a reopened file), where the planned range names other text.
pub fn relive_landed(view: &mut TextViewModel, reshaped: [bool; 2]) {
    for side in [Side::Left, Side::Right] {
        let k = index_of(side);
        if !view.cutting[k].as_ref().is_some_and(|c| c.written) {
            continue;
        }
        let Some(pending) = view.cutting[k].take() else {
            continue;
        };
        view.dirty = true;
        if reshaped[k] {
            view.request_window(TextRequest::Message(crate::status::Message::info(
                CUT_CHANGED,
            )));
        } else {
            apply_cut(view, side, pending);
        }
    }
}

/// Deletes the text of a Cut whose clipboard write landed, when the document is still the one it was planned on.
fn apply_cut(view: &mut TextViewModel, side: Side, pending: Cutting) {
    if view.doc(side).map(EditableDocument::generation) != Some(pending.generation) {
        // The document changed under the write (a reload, a relive): its range names other text now.
        view.request_window(TextRequest::Message(crate::status::Message::info(
            CUT_CHANGED,
        )));
        return;
    }
    edit::edit(view, side, pending.request, Origin::Single, None);
}

/// Paste of `text` into `side` at its caret (replacing the selection). True when applied.
pub fn paste(view: &mut TextViewModel, side: Side, text: &str) -> bool {
    if text.is_empty() {
        view.request_window(TextRequest::Message(crate::status::Message::info(NO_TEXT)));
        return false;
    }
    edit::command_on(
        view,
        side,
        &EditCommand::Paste {
            text: text.to_owned(),
        },
        Origin::Single,
    )
}

/// The text of a clipboard holding files: their paths, one a line (D-298).
pub fn paths_text(paths: &[std::path::PathBuf]) -> String {
    let mut out = String::new();
    for p in paths {
        out.push_str(&p.to_string_lossy());
        out.push('\n');
    }
    out
}

/// The caret or the selection changed: an emptied selection writes nothing.
pub fn selection_changed(view: &mut TextViewModel) {
    if !view.pane(view.focus()).has_selection() {
        view.primary.settle_at = None;
    }
}

/// A Shift+move: the selection settles [`PRIMARY_SETTLE_S`] after the last one.
pub fn shift_moved(view: &mut TextViewModel) {
    if view.pane(view.focus()).has_selection() {
        view.primary.settle_at = Some(view.now + PRIMARY_SETTLE_S);
    }
}

/// The pointer button was released after a drag or a click that selected: the selection settles now.
pub fn pointer_released(view: &mut TextViewModel) {
    if view.pane(view.focus()).has_selection() {
        view.primary.settle_at = Some(view.now);
    }
}

/// Writes the primary selection when it settled (each frame).
pub fn tick(view: &mut TextViewModel) {
    let Some(at) = view.primary.settle_at else {
        return;
    };
    if view.now + 1e-9 < at {
        return;
    }
    view.primary.settle_at = None;
    if !view.pane(view.focus()).has_selection() {
        return;
    }
    let Some(mut text) = copy_text(view) else {
        return;
    };
    if text.len() > PRIMARY_MAX_BYTES {
        let mut cut = PRIMARY_MAX_BYTES;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    view.primary.written += 1;
    view.request_window(TextRequest::WritePrimary(text));
}
