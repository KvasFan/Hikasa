//! The status bar items of a Text Compare view (M2 W11a, contract §1.11.12, D-303), toolkit-free: the caret's line
//! and column (`nav::column_of_tabs`), `INS`/`OVR`, the caret row's state, the element under the caret (FF-027), the
//! focus pane's encoding and line ending, its editability, the times and *Recomparing…*. They are recomputed only when
//! the caret, the documents' generations, an install or the mode change (a counter in the tests), never per frame.

use std::cell::RefCell;

use hikasa_core::Side;
use hikasa_text::nav::column_of_tabs;
use hikasa_text::{CharClass, Importance, LineIdx, RowIdx, RowState};

use super::{TextViewModel, header, verdict_text};
use crate::status::StatusItems;

/// What the items depend on.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    focus: Side,
    caret: (u32, u32),
    docs: (u64, u64),
    installs: u64,
    generation: u64,
    overwrite: bool,
    recomparing: bool,
    saving: bool,
}

/// The cache of the items.
#[derive(Debug, Default)]
pub struct Cache {
    inner: RefCell<Option<(Key, StatusItems)>>,
    /// Builds so far (tests: per caret move, never per frame).
    pub builds: std::cell::Cell<u64>,
}

/// The status bar items (cached by their inputs).
pub fn items(view: &TextViewModel) -> StatusItems {
    let focus = view.focus();
    let caret = view.pane(focus).caret;
    let key = Key {
        focus,
        caret: (caret.line.0, caret.byte),
        docs: (
            view.doc(Side::Left).map_or(0, |d| d.generation()),
            view.doc(Side::Right).map_or(0, |d| d.generation()),
        ),
        installs: view.installs(),
        generation: view.generation(),
        overwrite: view.display().overwrite,
        recomparing: view.recompare_state().showing(view.now()),
        saving: view.saving[0] || view.saving[1],
    };
    if let Some((k, items)) = &*view.status_bar.inner.borrow()
        && *k == key
    {
        return items.clone();
    }
    let items = build(view, focus);
    view.status_bar.builds.set(view.status_bar.builds.get() + 1);
    *view.status_bar.inner.borrow_mut() = Some((key, items.clone()));
    items
}

/// The row state's text of `row`.
fn row_text(view: &TextViewModel, row: u32) -> &'static str {
    let Some(live) = view.live_view() else {
        return "";
    };
    if live.live.is_pending(RowIdx(row)) {
        return "Pending";
    }
    let Some(r) = live.live.current().row(RowIdx(row)) else {
        return "";
    };
    if r.ignored {
        return "Ignored";
    }
    match (r.state, r.importance) {
        (RowState::Same, _) => "Same",
        (RowState::LeftOnly, _) => "Left Orphan",
        (RowState::RightOnly, _) => "Right Orphan",
        (RowState::Changed, Importance::Important) => "Important Difference",
        (RowState::Changed, _) => "Unimportant Difference",
    }
}

/// The element under the caret (FF-027): the token of the comparison's spans covering the caret's byte.
fn element_text(view: &TextViewModel, side: Side, line: u32, byte: u32) -> String {
    let (Some(cmp), Some(doc)) = (view.comparison(), view.doc(side)) else {
        return String::new();
    };
    if line >= doc.line_count() {
        return "Everything Else".to_owned();
    }
    // Never tokenize the document on the UI thread: until the start states are prepared, the element is not known.
    if !cmp.spans_prepared(side) {
        return String::new();
    }
    let spans = cmp.spans(side, LineIdx(line));
    let span = spans
        .iter()
        .find(|s| s.range.start <= byte && byte < s.range.end)
        .or_else(|| spans.iter().rfind(|s| s.range.end == byte));
    match span {
        Some(s) if s.class == CharClass::Element => cmp
            .format_of(side)
            .grammar()
            .elements()
            .name(s.element)
            .to_owned(),
        _ => "Everything Else".to_owned(),
    }
}

fn build(view: &TextViewModel, side: Side) -> StatusItems {
    let mut right = Vec::new();
    let left = match view.comparison() {
        Some(cmp) => verdict_text(cmp.verdict(view.ignore_unimportant()).kind()).to_owned(),
        None => view.status().state,
    };
    if let Some(doc) = view.doc(side) {
        let caret = doc.clamp(view.pane(side).caret);
        let line_text = if caret.line.0 < doc.position_lines() {
            doc.line(caret.line)
        } else {
            std::borrow::Cow::Borrowed("")
        };
        let (col, display) = column_of_tabs(&line_text, caret.byte, doc.options().tab_size);
        let ln = super::pane::physical_line(doc, caret.line);
        right.push(if col == display {
            format!("Ln {ln}, Col {col}")
        } else {
            format!("Ln {ln}, Col {col} ({display})")
        });
        right.push(
            if view.display().overwrite {
                "OVR"
            } else {
                "INS"
            }
            .to_owned(),
        );
        if let Some(row) = view.row_of_line(side, caret.line.0) {
            let state = row_text(view, row);
            if !state.is_empty() {
                right.push(state.to_owned());
            }
        }
        let element = element_text(view, side, caret.line.0, caret.byte);
        if !element.is_empty() {
            right.push(element);
        }
        right.push(header::encoding_text(doc));
        right.push(header::eol_text(doc).to_owned());
        let status = doc.editability().status_text();
        if !status.is_empty() {
            right.push(status.to_owned());
        }
        if doc.is_modified() {
            right.push("Modified".to_owned());
        }
    }
    if let Some(t) = view.timings_text() {
        right.push(t);
    }
    if view.recompare_state().showing(view.now()) {
        right.push("Recomparing…".to_owned());
    }
    if super::save::running(view) {
        right.push("Saving…".to_owned());
    }
    StatusItems { left, right }
}
