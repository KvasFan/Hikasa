//! The clipboard of the text panes (M2 W11a, contract §1.11.7, D-298), toolkit-free: what Copy and Cut write (the
//! selected text with its terminators as stored; a line set's lines; without a selection the caret's line with its
//! terminator), what Paste applies (`EditCommand::Paste`, breaks becoming the pane's insert terminator), and the
//! primary selection (X11, Wayland): written when a selection settles (the button released, or 300 ms after the last
//! Shift+move), capped at 1 MiB. The window's provider does the reads and writes on its jobs (`app/text.rs`).

use hikasa_core::Side;
use hikasa_text::edit::{EditCommand, EditRequest, lines_text};

use super::edit::{self, Origin};
use super::pane::line_selection;
use super::{TextRequest, TextViewModel};

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

/// Cut after its Copy: the selection, else the caret's line, deleted as one step. True when applied.
pub fn cut(view: &mut TextViewModel) -> bool {
    let side = view.focus();
    let Some(doc) = view.doc(side) else {
        return false;
    };
    let pane = view.pane(side);
    if pane.has_selection() {
        return edit::command_on(view, side, &EditCommand::Delete, Origin::Single);
    }
    let (a, b) = line_selection(doc, pane.caret.line.0);
    if a == b {
        return false;
    }
    edit::edit(
        view,
        side,
        EditRequest::Replace {
            range: a..b,
            text: String::new(),
            keep_eols: false,
        },
        Origin::Single,
        None,
    )
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
