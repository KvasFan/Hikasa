//! Keys, typed text and the IME of a focused pane (M2 W11a, contract §1.11.4–§1.11.5, D-295, D-296), toolkit-free:
//! `ui/text.rs` turns egui's events into these calls. Typed text (several characters of one event, AltGr and dead-key
//! results included) is one `EditCommand::Type`; an IME commit is one `Type`; a preedit is drawn, never stored, and
//! keys are not interpreted while it is non-empty (the IME owns them). While a pane is frozen (a save, a relive, a
//! Cut's clipboard write) every input of it — keys, clicks, drags, commands — is queued in order and replayed in
//! that order when it thaws (D-300, §1.11.35 W11a note 5).

use hikasa_core::Side;
use hikasa_text::edit::{EditCommand, TextPos, word_at};

use super::edit::{self, Origin};
use super::pane::{Move, Preedit, apply_move, line_selection};
use super::wrap::Metrics;
use super::{TextRequest, TextViewModel, index_of};
use crate::commands::CommandId;
use crate::status::Message;

/// A key the pane interprets itself (focus-bound keys, D-111 rule 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// ←
    Left,
    /// →
    Right,
    /// ↑
    Up,
    /// ↓
    Down,
    /// Home
    Home,
    /// End
    End,
    /// PgUp
    PageUp,
    /// PgDn
    PageDown,
    /// Enter
    Enter,
    /// Tab
    Tab,
    /// Backspace
    Backspace,
    /// Delete
    Delete,
    /// Esc
    Escape,
    /// Ins (overwrite mode)
    Insert,
}

/// A key press with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    /// The key.
    pub key: Key,
    /// Shift.
    pub shift: bool,
    /// Ctrl (Cmd on macOS).
    pub ctrl: bool,
}

#[cfg_attr(not(test), allow(dead_code))] // the tests' and the benchmark's presses
impl Press {
    /// A press without modifiers.
    pub fn plain(key: Key) -> Press {
        Press {
            key,
            shift: false,
            ctrl: false,
        }
    }

    /// With Shift.
    pub fn shift(key: Key) -> Press {
        Press {
            key,
            shift: true,
            ctrl: false,
        }
    }

    /// With Ctrl.
    pub fn ctrl(key: Key) -> Press {
        Press {
            key,
            shift: false,
            ctrl: true,
        }
    }
}

/// An IME event (egui's `ImeEvent`, toolkit-free). egui 0.37 sends no `Enabled`/`Disabled` (an empty preedit is the
/// composition dismissed); the model keeps them for the platforms that do.
#[allow(dead_code)] // `Enabled` and `Disabled` for the platforms that send them
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ime {
    /// Composition starts.
    Enabled,
    /// The composed text so far (empty: none).
    Preedit(String),
    /// The composition's result.
    Commit(String),
    /// Composition ends.
    Disabled,
}

/// An input of a frozen pane (D-300, §1.11.35 W11a note 5): queued as the user gave it and replayed in order when the
/// pane thaws, through the same functions; an edit is planned then, against the document of that moment.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// A key of §1.11.4/§1.11.6 with the page it moves (visual lines).
    Key(Press, u32),
    /// An editing command (typed text, an IME commit, a paste, a key's edit).
    Command(EditCommand, Origin),
    /// A press of the primary button (the position on the document shown then).
    Press {
        /// The position.
        pos: TextPos,
        /// Shift.
        shift: bool,
        /// Single, double or triple.
        clicks: Clicks,
    },
    /// A drag of the primary button to a position.
    Drag(TextPos),
    /// The primary button released.
    Release,
    /// A press in the line-number gutter.
    GutterPress {
        /// The line.
        line: u32,
        /// Shift.
        shift: bool,
    },
    /// A drag in the gutter.
    GutterDrag(u32),
    /// A command of the view on the pane: Undo, Redo, Select All, Select Section, the section steps, the copies to
    /// the other side.
    Run(CommandId),
    /// A copy button of the gutter: the section holding that position of that pane, from `from`.
    CopySection {
        /// The source pane.
        from: Side,
        /// A pane and a position of the section's first row.
        at: (Side, TextPos),
    },
    /// Copy (`false`) or Cut (`true`) of the pane's selection.
    Clip(bool),
    /// *Save with Encoding* of the header.
    Encoding(String),
    /// *Byte Order Mark* of the header.
    Bom,
    /// A line-ending conversion of the header.
    Eols(hikasa_text::LineEnding),
}

/// A queued input with the panes it acts on.
#[derive(Debug, Clone, PartialEq)]
pub struct Queued {
    /// The pane it acts on (a copy: the target).
    pub side: Side,
    /// The pane with the keyboard when it came (it is replayed from that pane).
    pub focus: Side,
    /// Per pane, the edits of [`TextViewModel::replayed`] already applied when it came (its positions are mapped
    /// through the later ones).
    epoch: [usize; 2],
    /// The input.
    pub input: Input,
}

impl Queued {
    /// True when the input acts on `side` (a copy to the other side: both panes).
    pub fn touches(&self, side: Side) -> bool {
        self.side == side
            || match &self.input {
                Input::CopySection { .. } => true,
                Input::Run(id) => is_copy(*id),
                _ => false,
            }
    }
}

/// The copies to the other side (they read one pane and write the other).
pub fn is_copy(id: CommandId) -> bool {
    matches!(
        id,
        CommandId::CopyRight
            | CommandId::CopyLeft
            | CommandId::CopySide
            | CommandId::CopyItemRight
            | CommandId::CopyItemLeft
            | CommandId::CopyItemSide
    )
}

/// How an edit moved the positions of its document (a queued position is mapped through the edits replayed before
/// it).
#[derive(Debug, Clone, PartialEq)]
pub enum PosEdit {
    /// Replacements, in document order: the old range and the end of the new text.
    Exact(Vec<(std::ops::Range<TextPos>, TextPos)>),
    /// Only the lines are known (an undo, a redo, a document with a characters-per-line limit).
    Lines(Vec<hikasa_text::edit::LineDelta>),
}

impl PosEdit {
    /// What `req` (applied with `out`) does to positions; `None` when it moves none (terminators, encoding).
    pub fn of(
        req: &hikasa_text::EditRequest,
        out: &hikasa_text::EditOutcome,
        exact: bool,
    ) -> Option<PosEdit> {
        use hikasa_text::EditRequest;
        let replaced = |range: &std::ops::Range<TextPos>, text: &str| {
            let (a, b) = if range.end < range.start {
                (range.end, range.start)
            } else {
                (range.start, range.end)
            };
            (a..b, end_of_text(a, text))
        };
        match req {
            EditRequest::SetEols { .. } | EditRequest::SetEncoding { .. } => None,
            EditRequest::Replace { range, text, .. } if exact => {
                Some(PosEdit::Exact(vec![replaced(range, text)]))
            }
            EditRequest::Batch { edits, .. } if exact => Some(PosEdit::Exact(
                edits.iter().map(|(r, t)| replaced(r, t)).collect(),
            )),
            _ => Some(PosEdit::Lines(out.deltas.clone())),
        }
    }

    /// A position after the edit (clamped by the caller).
    pub fn map(&self, pos: TextPos) -> TextPos {
        match self {
            PosEdit::Exact(edits) => {
                // From the last to the first: an earlier range keeps its coordinates while a later one is mapped.
                let mut p = pos;
                for (range, end) in edits.iter().rev() {
                    if p < range.start || (p == range.start && range.start != range.end) {
                        continue;
                    }
                    p = if p < range.end {
                        // Inside the replaced text: after the new text.
                        *end
                    } else if p.line == range.end.line {
                        TextPos::new(end.line.0, end.byte + (p.byte - range.end.byte))
                    } else {
                        let line = i64::from(p.line.0) + i64::from(end.line.0)
                            - i64::from(range.end.line.0);
                        TextPos::new(u32::try_from(line.max(0)).unwrap_or(0), p.byte)
                    };
                }
                p
            }
            PosEdit::Lines(deltas) => {
                let mut p = pos;
                for d in deltas {
                    let line = p.line.0;
                    if line < d.at {
                        continue;
                    }
                    p = if line >= d.at + d.removed {
                        TextPos::new(line - d.removed + d.inserted, p.byte)
                    } else {
                        TextPos::new(
                            d.at + (line - d.at).min(d.inserted.saturating_sub(1)),
                            p.byte,
                        )
                    };
                }
                p
            }
        }
    }
}

/// The position after `text` inserted at `at` (its breaks: LF, CRLF, CR).
fn end_of_text(at: TextPos, text: &str) -> TextPos {
    let bytes = text.as_bytes();
    let mut breaks = 0u32;
    let mut after = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                breaks += 1;
                after = i + 1;
            }
            b'\n' => {
                breaks += 1;
                after = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if breaks == 0 {
        TextPos::new(at.line.0, at.byte + text.len() as u32)
    } else {
        TextPos::new(at.line.0 + breaks, (text.len() - after) as u32)
    }
}

/// The message when the queue of a save is full.
pub const WAIT_FOR_SAVE: &str = "Wait for the save to end.";

/// Queues an input of `side` while it is frozen (≤ [`super::MAX_QUEUED`]), else refuses it with
/// [`WAIT_FOR_SAVE`]. A drag replaces the drag queued just before it (one position a frame).
pub fn queue(view: &mut TextViewModel, side: Side, input: Input) {
    if let (Input::Drag(pos), Some(last)) = (&input, view.queued.last_mut())
        && last.side == side
        && matches!(last.input, Input::Drag(_))
    {
        last.input = Input::Drag(*pos);
        return;
    }
    if view.queued.len() >= super::MAX_QUEUED {
        view.request_window(TextRequest::Message(Message::info(WAIT_FOR_SAVE)));
        return;
    }
    let epoch = [view.replayed[0].len(), view.replayed[1].len()];
    view.queued.push(Queued {
        side,
        focus: view.focus,
        epoch,
        input,
    });
    view.dirty = true;
}

/// Notes an edit of `side` for the queued positions (only while inputs of that pane wait or are replayed).
pub fn note_positions(view: &mut TextViewModel, side: Side, edit: Option<PosEdit>) {
    let k = index_of(side);
    if let Some(edit) = edit
        && (view.replaying || view.queued.iter().any(|q| q.touches(side)))
    {
        view.replayed[k].push(edit);
    }
}

/// A queued position of `side` on the document of today.
fn mapped(view: &TextViewModel, side: Side, epoch: [usize; 2], pos: TextPos) -> TextPos {
    let k = index_of(side);
    let edits = view.replayed[k].get(epoch[k]..).unwrap_or_default();
    let p = edits.iter().fold(pos, |p, e| e.map(p));
    match view.doc(side) {
        Some(doc) => super::pane::snap(doc, p),
        None => p,
    }
}

/// Replays the queued inputs whose panes thawed, in order (each frame, before the frame's own input; the window's
/// metrics measure the vertical moves). An input of a pane still held — and every later input of that pane — waits.
/// True when something was replayed.
pub fn replay(view: &mut TextViewModel, metrics: &dyn Metrics) -> bool {
    if view.queued.is_empty() || view.view.is_none() {
        return false;
    }
    let pending = std::mem::take(&mut view.queued);
    let focus = view.focus;
    let mut applied = false;
    view.replaying = true;
    for q in pending {
        let blocked = [Side::Left, Side::Right]
            .into_iter()
            .any(|s| q.touches(s) && (view.held(s) || view.queued.iter().any(|r| r.touches(s))));
        if blocked {
            view.queued.push(q);
            continue;
        }
        view.focus = q.focus;
        apply(view, q, metrics);
        applied = true;
    }
    view.replaying = false;
    if view.focus != focus {
        view.focus = focus;
    }
    for side in [Side::Left, Side::Right] {
        if !view.queued.iter().any(|q| q.touches(side)) {
            view.replayed[index_of(side)].clear();
        }
    }
    if applied {
        view.caret_moved();
    }
    applied
}

/// One queued input, through the function that takes it when its pane is not frozen.
fn apply(view: &mut TextViewModel, q: Queued, metrics: &dyn Metrics) {
    let side = q.side;
    match q.input {
        Input::Key(press, page) => {
            key_now(view, press, metrics, page);
        }
        Input::Command(cmd, origin) => {
            edit::command_on(view, side, &cmd, origin);
        }
        Input::Press { pos, shift, clicks } => {
            let pos = mapped(view, side, q.epoch, pos);
            press(view, side, pos, shift, clicks);
        }
        Input::Drag(pos) => {
            let pos = mapped(view, side, q.epoch, pos);
            drag(view, pos);
        }
        Input::Release => release(view),
        Input::GutterPress { line, shift } => {
            let line = mapped(view, side, q.epoch, TextPos::new(line, 0)).line.0;
            gutter_press(view, side, line, shift);
        }
        Input::GutterDrag(line) => {
            let line = mapped(view, side, q.epoch, TextPos::new(line, 0)).line.0;
            gutter_drag(view, line);
        }
        Input::Run(id) => match id {
            CommandId::DiffSectionNext => {
                view.go_to_section(hikasa_text::Direction::Forward);
            }
            CommandId::DiffSectionPrev => {
                view.go_to_section(hikasa_text::Direction::Backward);
            }
            id => {
                edit::command(view, id);
            }
        },
        Input::CopySection { from, at } => {
            let pos = mapped(view, at.0, q.epoch, at.1);
            if let Some(rows) = view
                .row_of_line(at.0, pos.line.0)
                .and_then(|row| edit::section_of_row(view, row))
            {
                edit::copy_rows_to(view, from, rows);
            }
        }
        Input::Clip(cut) => {
            super::clipboard::copy(view, cut);
        }
        Input::Encoding(label) => {
            super::header::save_with_encoding(view, side, &label);
        }
        Input::Bom => {
            super::header::toggle_bom(view, side);
        }
        Input::Eols(eol) => {
            super::header::set_line_endings(view, side, eol);
        }
    }
}

/// The inputs waiting for their panes.
pub fn queued(view: &TextViewModel) -> usize {
    view.queued.len()
}

/// The adapter time of typing (ms of the window's frame clock, D-297).
fn at_ms(view: &TextViewModel) -> u64 {
    (view.now.max(0.0) * 1000.0) as u64
}

/// Typed text on the focus pane (`egui::Event::Text`): one `Type` (overwrite per the view's mode).
pub fn text(view: &mut TextViewModel, text: &str) {
    let side = view.focus;
    if text.is_empty() || view.pane(side).preedit.is_some() {
        return;
    }
    // Control characters never arrive as text: Enter, Tab and the others are keys.
    if text.chars().any(char::is_control) {
        return;
    }
    let overwrite = view.layout.overwrite;
    let origin = Origin::Typing { at_ms: at_ms(view) };
    edit::command_on(
        view,
        side,
        &EditCommand::Type {
            text: text.to_owned(),
            overwrite,
        },
        origin,
    );
}

/// An IME event on the focus pane (§1.11.5): the preedit is shown only; a commit is one `Type` step.
pub fn ime(view: &mut TextViewModel, event: Ime) {
    let side = view.focus;
    match event {
        Ime::Enabled => {}
        Ime::Preedit(text) => {
            let pane = view.pane_mut(side);
            pane.preedit = (!text.is_empty()).then_some(Preedit { text });
            view.dirty = true;
        }
        Ime::Commit(text) => {
            view.pane_mut(side).preedit = None;
            view.dirty = true;
            if text.is_empty() {
                return;
            }
            let overwrite = view.layout.overwrite;
            edit::command_on(
                view,
                side,
                &EditCommand::Type { text, overwrite },
                Origin::Single,
            );
        }
        Ime::Disabled => {
            view.pane_mut(side).preedit = None;
            view.dirty = true;
        }
    }
}

/// True when the IME of the focus pane composes (keys are its own).
pub fn composing(view: &TextViewModel) -> bool {
    view.pane(view.focus).preedit.is_some()
}

/// True when the IME may be enabled over the focus pane (an editable pane, D-296: not over a blocked one).
pub fn ime_allowed(view: &TextViewModel) -> bool {
    view.doc(view.focus)
        .is_some_and(|d| d.editability().editable())
}

/// A key on the focus pane; `page` is the visual lines a page moves. True when the pane took it. A frozen pane
/// queues the keys it takes (a move, an edit, Esc); scrolling and Ins act at once (they are the view's).
pub fn key(view: &mut TextViewModel, press: Press, metrics: &dyn Metrics, page: u32) -> bool {
    if composing(view) {
        return true;
    }
    let side = view.focus;
    if view.frozen(side) {
        let view_level = matches!(
            (press.key, press.ctrl, press.shift),
            (Key::Up | Key::Down, true, _) | (Key::Insert, false, false)
        );
        if !view_level {
            if !takes(press) {
                return false;
            }
            queue(view, side, Input::Key(press, page));
            return true;
        }
    }
    key_now(view, press, metrics, page)
}

/// True for a key the pane interprets (§1.11.4, §1.11.6).
fn takes(press: Press) -> bool {
    match press.key {
        Key::Left
        | Key::Right
        | Key::Up
        | Key::Down
        | Key::Home
        | Key::End
        | Key::PageUp
        | Key::PageDown
        | Key::Escape => true,
        Key::Insert => false,
        Key::Enter | Key::Delete => !press.ctrl && !press.shift,
        Key::Tab => !press.ctrl,
        Key::Backspace => !press.ctrl,
    }
}

/// A key on the focus pane, not queued (the replay of a queued one).
fn key_now(view: &mut TextViewModel, press: Press, metrics: &dyn Metrics, page: u32) -> bool {
    let side = view.focus;
    let Some(doc) = view.doc(side).cloned() else {
        return false;
    };
    let mv = match (press.key, press.ctrl) {
        (Key::Left, false) => Some(Move::Left),
        (Key::Right, false) => Some(Move::Right),
        (Key::Left, true) => Some(Move::WordLeft),
        (Key::Right, true) => Some(Move::WordRight),
        (Key::Up, false) => Some(Move::Up),
        (Key::Down, false) => Some(Move::Down),
        (Key::Home, false) => Some(Move::Home),
        (Key::End, false) => Some(Move::End),
        (Key::Home, true) => Some(Move::DocStart),
        (Key::End, true) => Some(Move::DocEnd),
        (Key::PageUp, false) => Some(Move::PageUp(page)),
        (Key::PageDown, false) => Some(Move::PageDown(page)),
        _ => None,
    };
    if let Some(mv) = mv {
        let geom = |line: u32| view.line_geom(side, line, metrics);
        let mut pane = view.pane(side).clone();
        apply_move(&mut pane, &doc, mv, press.shift, &geom);
        *view.pane_mut(side) = pane;
        view.caret_moved();
        if press.shift {
            super::clipboard::shift_moved(view);
        }
        return true;
    }
    let origin = Origin::Typing { at_ms: at_ms(view) };
    match (press.key, press.ctrl, press.shift) {
        (Key::Up, true, _) => {
            view.scroll_rows -= 1.0;
            view.dirty = true;
        }
        (Key::Down, true, _) => {
            view.scroll_rows += 1.0;
            view.dirty = true;
        }
        (Key::Escape, ..) => {
            view.pane_mut(side).collapse();
            view.dirty = true;
        }
        (Key::Insert, false, false) => {
            view.layout.overwrite = !view.layout.overwrite;
            view.dirty = true;
        }
        (Key::Enter, false, false) => {
            edit::command_on(view, side, &EditCommand::NewLine, Origin::Single);
        }
        (Key::Tab, false, false) => {
            edit::command_on(view, side, &EditCommand::Tab, Origin::Single);
        }
        (Key::Tab, false, true) => {
            edit::command_on(view, side, &EditCommand::BackTab, Origin::Single);
        }
        (Key::Backspace, false, _) => {
            edit::command_on(view, side, &EditCommand::Backspace, origin);
        }
        (Key::Delete, false, false) => {
            edit::command_on(view, side, &EditCommand::Delete, origin);
        }
        _ => return false,
    }
    true
}

/// How many clicks a press is part of (egui's double and triple clicks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clicks {
    /// A press (or a single click).
    One,
    /// A double click: the word.
    Two,
    /// A triple click: the physical line with its terminator.
    Three,
}

/// A press of the primary button at `pos` of `side` (§1.11.4 *Pointer*): the focus goes to that pane; Shift extends
/// from the anchor; a double click selects the word (`word_at`), a triple click the line with its terminator.
pub fn press(view: &mut TextViewModel, side: Side, pos: TextPos, shift: bool, clicks: Clicks) {
    view.set_focus(side);
    if view.frozen(side) {
        queue(view, side, Input::Press { pos, shift, clicks });
        return;
    }
    let Some(doc) = view.doc(side) else {
        return;
    };
    let pos = super::pane::snap(doc, pos);
    let (anchor, caret) = match clicks {
        Clicks::One => {
            let pane = view.pane_mut(side);
            pane.set_caret(pos, shift);
            pane.sticky_x = None;
            view.caret_moved();
            return;
        }
        Clicks::Two => {
            let r = word_at(doc, pos);
            (r.start, r.end)
        }
        Clicks::Three => line_selection(doc, pos.line.0),
    };
    view.pane_mut(side).select(anchor, caret);
    view.caret_moved();
}

/// A drag of the primary button to `pos` of the focus pane: the selection extends there.
pub fn drag(view: &mut TextViewModel, pos: TextPos) {
    let side = view.focus;
    if view.frozen(side) {
        queue(view, side, Input::Drag(pos));
        return;
    }
    let Some(doc) = view.doc(side) else {
        return;
    };
    let pos = super::pane::snap(doc, pos);
    if view.pane(side).caret != pos {
        let pane = view.pane_mut(side);
        pane.set_caret(pos, true);
        pane.sticky_x = None;
        view.caret_moved();
    }
}

/// The primary button was released: a selection settles (the primary selection, D-298).
pub fn release(view: &mut TextViewModel) {
    let side = view.focus;
    if view.frozen(side) {
        queue(view, side, Input::Release);
        return;
    }
    super::clipboard::pointer_released(view);
}

/// A press in the line-number gutter of `side` on `line`: that line with its terminator is selected (Shift: from the
/// anchor's line).
pub fn gutter_press(view: &mut TextViewModel, side: Side, line: u32, shift: bool) {
    view.set_focus(side);
    if view.frozen(side) {
        queue(view, side, Input::GutterPress { line, shift });
        return;
    }
    let Some(doc) = view.doc(side) else {
        return;
    };
    let line = line.min(doc.position_lines().saturating_sub(1));
    let from = if shift {
        view.pane(side).anchor.line.0
    } else {
        line
    };
    view.gutter_from = Some(from);
    super::edit::select_lines(view, side, from.min(line)..from.max(line) + 1);
}

/// A drag in the gutter to `line`: the lines from the press's line to it.
pub fn gutter_drag(view: &mut TextViewModel, line: u32) {
    let side = view.focus;
    if view.frozen(side) {
        queue(view, side, Input::GutterDrag(line));
        return;
    }
    let Some(from) = view.gutter_from else {
        return;
    };
    let Some(doc) = view.doc(side) else {
        return;
    };
    let line = line.min(doc.position_lines().saturating_sub(1));
    super::edit::select_lines(view, side, from.min(line)..from.max(line) + 1);
    if line < from {
        // Dragging up: the caret at the top.
        let pane = view.pane_mut(side);
        std::mem::swap(&mut pane.anchor, &mut pane.caret);
    }
}

/// A middle click at `pos` of `side` (X11, Wayland): the caret goes there and the primary selection is pasted.
pub fn middle_click(view: &mut TextViewModel, side: Side, pos: TextPos) {
    press(view, side, pos, false, Clicks::One);
    view.request_window(TextRequest::PastePrimary(side));
}

/// A right click at `pos` of `side`: outside the selection the caret moves there (the menu follows, D-312).
pub fn secondary_press(view: &mut TextViewModel, side: Side, pos: TextPos) {
    let inside = view.focus == side && {
        let r = view.pane(side).selection().range();
        r.start <= pos && pos <= r.end && r.start != r.end
    };
    if !inside {
        press(view, side, pos, false, Clicks::One);
    }
}
