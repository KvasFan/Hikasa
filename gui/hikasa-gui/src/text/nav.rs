//! Navigation of a text view (TC-050, TC-051, TC-053; M2 W11b, contract §1.11.18, D-309), toolkit-free: the steps of
//! the browse modes (`nav::step` from the focus pane's caret), the next or previous existing line, Go To
//! (`parse_goto`, `goto`), the bookmarks 0–9 of the documents, the pane moves, *nothing found* (the message panel or a
//! beep, OPT-007) and the first difference on load. Every target is core's; this module selects it, moves the caret
//! and brings it into view.

use hikasa_core::{Pair, Side};
use hikasa_text::Direction;
use hikasa_text::LineIdx;
use hikasa_text::edit::{EditableDocument, TextPos};
use hikasa_text::find::FindSides;
use hikasa_text::nav::{BrowseMode, NavContext, NavPos, NavTarget, existing_line, goto, step};

use super::edit::other as other_side;
use super::{TextRequest, TextViewModel, input};
use crate::status::Message;

/// The message of a step that found nothing (TC-051).
pub const NO_MORE: &str = "No more differences.";
/// The message of a find step that found nothing.
pub const NOT_FOUND: &str = "Not found.";

/// The context of a step: the comparison, the documents, the display filter, *Ignore Unimportant*, the find panel's
/// query (when compiled) and sides.
fn context(view: &TextViewModel) -> NavContext<'_> {
    NavContext {
        cmp: view.comparison().map(|c| &**c),
        docs: Pair::new(view.doc(Side::Left), view.doc(Side::Right)),
        view: view.filtered(),
        ignore_unimportant: view.ignore_unimportant(),
        finder: view.find.compiled().map(|f| &**f),
        sides: if view.is_edit() {
            FindSides::Left
        } else {
            view.find.sides
        },
    }
}

/// Where a step of `dir` starts: the focus pane's selection end (forward) or start (backward), its row.
pub fn from_of(view: &TextViewModel, dir: Direction) -> NavPos {
    let side = view.focus();
    let r = view.pane(side).selection().range();
    let pos = match dir {
        Direction::Forward => r.end,
        Direction::Backward => r.start,
    };
    NavPos {
        row: view
            .row_of_line(side, pos.line.0)
            .map(hikasa_text::RowIdx)
            .filter(|_| !view.is_edit()),
        side,
        pos,
    }
}

/// The target of one step of `mode` (`None`: nothing in that direction), as core computes it from the focus pane's
/// caret, with `[next_difference] wrap_around`.
pub fn target(view: &TextViewModel, mode: BrowseMode, dir: Direction) -> Option<NavTarget> {
    if view.is_edit() && !matches!(mode, BrowseMode::Edit | BrowseMode::Find) {
        return None;
    }
    if !view.loaded() {
        return None;
    }
    step(
        mode,
        &context(view),
        from_of(view, dir),
        dir,
        view.tweaks.wrap_around,
    )
}

/// One step of `mode`: the target selected (line targets with the caret at their start, a hit or an inline
/// difference with the caret at its end), its pane focused and brought into view; nothing found: the message panel or
/// a beep. True when the caret moved.
pub fn go(view: &mut TextViewModel, mode: BrowseMode, dir: Direction) -> bool {
    match target(view, mode, dir) {
        Some(t) => {
            apply(
                view,
                &t,
                matches!(mode, BrowseMode::Find | BrowseMode::Difference),
            );
            if t.wrapped && mode == BrowseMode::Find {
                view.find.note = Some(super::find::PASSED_END.to_owned());
            }
            true
        }
        None => {
            not_found(view, mode == BrowseMode::Find);
            false
        }
    }
}

/// Selects `t` in its pane (`caret_at_end`: a hit, else a line range with the caret at its start).
pub fn apply(view: &mut TextViewModel, t: &NavTarget, caret_at_end: bool) {
    let side = t.side;
    view.set_focus(side);
    let (a, b) = (t.range.start, t.range.end);
    let pane = view.pane_mut(side);
    if caret_at_end {
        pane.select(a, b);
    } else {
        pane.select(b, a);
    }
    view.caret_moved();
    let row = t
        .row
        .map(|r| r.0)
        .or_else(|| view.row_of_line(side, a.line.0));
    if let Some(row) = row {
        view.reveal_row(row);
    }
}

/// Nothing found (TC-051): with `[next_difference] show_message_panel` the message panel says so, else the window
/// beeps (W8's request for attention).
pub fn not_found(view: &mut TextViewModel, find: bool) {
    if view.tweaks.show_message_panel {
        let text = if find { NOT_FOUND } else { NO_MORE };
        view.request_window(TextRequest::Message(Message::info(text)));
    } else {
        view.request_window(TextRequest::Beep);
    }
}

/// `ExistsNext`/`ExistsPrev` (Alt+Down/Up): the next or previous row with a line of the focus side.
pub fn exists(view: &mut TextViewModel, dir: Direction) -> bool {
    let side = view.focus();
    let found = (|| {
        let cmp = view.comparison()?;
        let from = view.row_of_line(side, view.pane(side).caret.line.0)?;
        let row = existing_line(cmp, hikasa_text::RowIdx(from), side, dir)?;
        let line = cmp.row(row).and_then(|r| match side {
            Side::Right => r.right,
            _ => r.left,
        })?;
        Some((row.0, line.0))
    })();
    let Some((row, line)) = found else {
        not_found(view, false);
        return false;
    };
    let pane = view.pane_mut(side);
    pane.set_caret(TextPos::new(line, 0), false);
    pane.sticky_x = None;
    view.caret_moved();
    view.reveal_row(row);
    true
}

/// Go To's answer: physical line `line` (0: the caret's) and column of the focus pane (`goto`, clamped by core).
pub fn go_to_line(view: &mut TextViewModel, line: u32, column: Option<u32>) -> bool {
    let side = view.focus();
    let Some(doc) = view.doc(side) else {
        return false;
    };
    let line = if line == 0 {
        super::pane::physical_line(doc, view.pane(side).caret.line)
    } else {
        line
    };
    let pos = goto(doc, line, column);
    let pane = view.pane_mut(side);
    pane.set_caret(pos, false);
    pane.sticky_x = None;
    view.caret_moved();
    if let Some(row) = view.row_of_line(side, pos.line.0) {
        view.reveal_row(row);
    }
    true
}

/// The last physical line of the focus pane (Go To's range `1–<last>`).
pub fn last_line(view: &TextViewModel) -> u32 {
    view.doc(view.focus()).map_or(1, |d| {
        d.document()
            .physical_line(LineIdx(d.line_count().saturating_sub(1)))
            .max(1)
    })
}

/// `BookmarkToggle<n>`: bookmark `n` on the focus pane's caret line (on again: off).
pub fn toggle_bookmark(view: &mut TextViewModel, n: u8) {
    let side = view.focus();
    let line = view.pane(side).caret.line;
    if let Some((doc, _)) = view.doc_mut_live(side) {
        doc.toggle_bookmark(n, line);
    }
    view.bump();
}

/// `BookmarkGoTo<n>`: bookmark `n` of the focus pane, else of the other pane, which takes the focus (D-309's open
/// default). Not set anywhere: the message panel.
pub fn go_to_bookmark(view: &mut TextViewModel, n: u8) -> bool {
    let focus = view.focus();
    let found = [focus, other_side(focus)]
        .into_iter()
        .filter(|s| !view.is_edit() || *s == Side::Left)
        .find_map(|s| view.doc(s).and_then(|d| d.bookmark(n)).map(|l| (s, l)));
    let Some((side, line)) = found else {
        view.request_window(TextRequest::Message(Message::info(format!(
            "Bookmark {n} is not set."
        ))));
        return false;
    };
    view.set_focus(side);
    let pane = view.pane_mut(side);
    pane.set_caret(TextPos::new(line.0, 0), false);
    pane.sticky_x = None;
    view.caret_moved();
    if let Some(row) = view.row_of_line(side, line.0) {
        view.reveal_row(row);
    }
    true
}

/// `BookmarkClear`: every bookmark of the focus pane.
pub fn clear_bookmarks(view: &mut TextViewModel) {
    let side = view.focus();
    if let Some((doc, _)) = view.doc_mut_live(side) {
        doc.clear_bookmarks();
    }
    view.bump();
}

/// The bookmarks of `line` of `side` (the gutter's digits).
pub fn bookmarks_at(doc: &EditableDocument, line: u32) -> Vec<u8> {
    doc.bookmarks()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_some_and(|l| l.0 == line))
        .map(|(n, _)| n as u8)
        .collect()
}

/// `GoLeft`/`GoRight`/`GoNext`/`GoPrev`: `side` takes the keyboard; its caret goes to the current row's line there (a
/// gap: the next line of that side).
pub fn go_pane(view: &mut TextViewModel, side: Side) {
    if view.is_edit() || view.focus() == side {
        return;
    }
    let row = view.cursor().map(|r| r.0);
    view.set_focus(side);
    if let Some(row) = row
        && let Some(line) = view.line_at_row(side, row)
    {
        let pane = view.pane_mut(side);
        if pane.caret.line.0 != line {
            pane.set_caret(TextPos::new(line, 0), false);
            pane.sticky_x = None;
        }
    }
    view.caret_moved();
}

/// The toolbar arrows' availability per browse mode (`DiffAvailable`): one step each way, computed after an install
/// and a caret move (cached by the view).
pub fn arrows(view: &TextViewModel) -> [bool; 2] {
    let mode = view.layout.browse_mode;
    [Direction::Forward, Direction::Backward].map(|d| target(view, mode, d).is_some())
}

/// `go_to_first_difference_on_load`: after the first install, the first difference section from the start.
pub fn first_difference(view: &mut TextViewModel) {
    if view.is_edit() || !view.tweaks.first_difference_on_load {
        return;
    }
    let ctx = context(view);
    // A section starting on the first row counts (`next_section_from` the first row).
    let first = ctx
        .cmp
        .and_then(|c| {
            c.next_section_from(
                hikasa_text::RowIdx(0),
                Direction::Forward,
                ctx.ignore_unimportant,
            )
        })
        .and_then(|id| ctx.cmp?.sections().get(id.0 as usize).map(|s| s.rows.start));
    if let Some(row) = first {
        view.go_to_row(row);
    }
}

/// Runs a navigation command of the focus pane (queued behind a frozen pane's inputs); false when `id` is not one.
pub fn command(view: &mut TextViewModel, id: crate::commands::CommandId) -> bool {
    use crate::commands::CommandId as C;
    let side = view.focus();
    let ours = matches!(
        id,
        C::DiffNext
            | C::DiffPrev
            | C::EditNext
            | C::EditPrev
            | C::DiffItemNext
            | C::DiffItemPrev
            | C::ExistsNext
            | C::ExistsPrev
            | C::GoLeft
            | C::GoRight
            | C::GoNext
            | C::GoPrev
            | C::BookmarkClear
    ) || bookmark_of(id).is_some();
    if !ours {
        return false;
    }
    if view.frozen(side) {
        input::queue(view, side, input::Input::Run(id));
        return true;
    }
    let _moved = match id {
        C::DiffNext => go(view, BrowseMode::Difference, Direction::Forward),
        C::DiffPrev => go(view, BrowseMode::Difference, Direction::Backward),
        C::EditNext => go(view, BrowseMode::Edit, Direction::Forward),
        C::EditPrev => go(view, BrowseMode::Edit, Direction::Backward),
        C::DiffItemNext => go(view, view.layout.browse_mode, Direction::Forward),
        C::DiffItemPrev => go(view, view.layout.browse_mode, Direction::Backward),
        C::ExistsNext => exists(view, Direction::Forward),
        C::ExistsPrev => exists(view, Direction::Backward),
        C::GoLeft => {
            go_pane(view, Side::Left);
            true
        }
        C::GoRight => {
            go_pane(view, Side::Right);
            true
        }
        C::GoNext | C::GoPrev => {
            go_pane(view, other_side(side));
            true
        }
        C::BookmarkClear => {
            clear_bookmarks(view);
            true
        }
        _ => match bookmark_of(id) {
            Some((true, n)) => {
                toggle_bookmark(view, n);
                true
            }
            Some((false, n)) => go_to_bookmark(view, n),
            None => false,
        },
    };
    true
}

/// The bookmark command of `id`: (toggle, number).
pub fn bookmark_of(id: crate::commands::CommandId) -> Option<(bool, u8)> {
    use crate::commands::CommandId as C;
    const TOGGLE: [C; 10] = [
        C::BookmarkToggle0,
        C::BookmarkToggle1,
        C::BookmarkToggle2,
        C::BookmarkToggle3,
        C::BookmarkToggle4,
        C::BookmarkToggle5,
        C::BookmarkToggle6,
        C::BookmarkToggle7,
        C::BookmarkToggle8,
        C::BookmarkToggle9,
    ];
    const GO: [C; 10] = [
        C::BookmarkGoTo0,
        C::BookmarkGoTo1,
        C::BookmarkGoTo2,
        C::BookmarkGoTo3,
        C::BookmarkGoTo4,
        C::BookmarkGoTo5,
        C::BookmarkGoTo6,
        C::BookmarkGoTo7,
        C::BookmarkGoTo8,
        C::BookmarkGoTo9,
    ];
    if let Some(n) = TOGGLE.iter().position(|c| *c == id) {
        return Some((true, n as u8));
    }
    GO.iter().position(|c| *c == id).map(|n| (false, n as u8))
}

/// The help topic of Go To.
pub const GOTO_TOPIC: &str = "dialog-go-to";

/// Go To of a text view (`GoTo`, Ctrl+G, D-309): *Line[:column]* in the range `1–<last physical line>`; core's
/// `parse_goto` errors under the field; OK moves the focus pane's caret (`goto`, clamped by core).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoToLineDialog {
    /// The view.
    pub view: hikasa_session::ipc::ViewId,
    /// The last physical line of the focus pane.
    pub last: u32,
    /// The field.
    pub text: String,
    /// `parse_goto`'s error.
    pub error: Option<String>,
    chosen: Option<(u32, Option<u32>)>,
}

impl GoToLineDialog {
    /// The dialog of `view`, whose focus pane has `last` physical lines.
    pub fn new(view: hikasa_session::ipc::ViewId, last: u32) -> GoToLineDialog {
        GoToLineDialog {
            view,
            last,
            text: String::new(),
            error: None,
            chosen: None,
        }
    }

    /// The range the field takes (`1–<last>`).
    pub fn range_text(&self) -> String {
        format!("Line[:column] (1–{})", self.last)
    }

    /// The line and column after OK (once).
    pub fn take_chosen(&mut self) -> Option<(u32, Option<u32>)> {
        self.chosen.take()
    }
}

impl crate::dialog::DialogModel for GoToLineDialog {
    fn title(&self) -> String {
        "Go To".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        GOTO_TOPIC
    }

    fn buttons(&self) -> Vec<crate::dialog::DialogButton> {
        use crate::dialog::{ButtonRole, DialogButton};
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: !self.text.trim().is_empty(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(
        &mut self,
        role: crate::dialog::ButtonRole,
        _svc: &hikasa_session::Service,
    ) -> crate::dialog::DialogStep {
        use crate::dialog::{ButtonRole, DialogStep};
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match hikasa_text::nav::parse_goto(&self.text) {
            Ok(target) => {
                self.chosen = Some(target);
                DialogStep::Close
            }
            Err(e) => {
                self.error = Some(e);
                DialogStep::Stay
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
