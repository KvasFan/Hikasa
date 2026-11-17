//! Keys, typed text and the IME of a focused pane (M2 W11a, contract §1.11.4–§1.11.5, D-295, D-296), toolkit-free:
//! `ui/text.rs` turns egui's events into these calls. Typed text (several characters of one event, AltGr and dead-key
//! results included) is one `EditCommand::Type`; an IME commit is one `Type`; a preedit is drawn, never stored, and
//! keys are not interpreted while it is non-empty (the IME owns them). While a save of the pane runs, its edits are
//! queued (D-300).

use hikasa_core::Side;
use hikasa_text::edit::{EditCommand, TextPos, word_at};

use super::edit::{self, Origin};
use super::pane::{Move, Preedit, apply_move, line_selection};
use super::wrap::Metrics;
use super::{PendingEdit, TextRequest, TextViewModel};
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
#[cfg_attr(not(test), allow(dead_code))]
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

/// An input waiting for a save to end (D-300).
#[derive(Debug, Clone, PartialEq)]
pub enum Queued {
    /// An edit.
    Edit(PendingEdit),
    /// An editing command of a pane, planned when it is applied (at the caret of that moment).
    Command(Side, EditCommand, Origin),
    /// Undo of a pane.
    Undo(Side),
    /// Redo of a pane.
    Redo(Side),
}

/// The message when the queue of a save is full.
pub const WAIT_FOR_SAVE: &str = "Wait for the save to end.";

/// Queues an input during a save (≤ [`super::MAX_QUEUED`]), else refuses it with [`WAIT_FOR_SAVE`].
pub fn queue(view: &mut TextViewModel, input: Queued) {
    if view.queued.len() >= super::MAX_QUEUED {
        view.request_window(TextRequest::Message(Message::info(WAIT_FOR_SAVE)));
        return;
    }
    view.queued.push(input);
}

/// Applies the queued inputs of `side` in order (the save ended).
pub fn replay(view: &mut TextViewModel, side: Side) {
    let queued = std::mem::take(&mut view.queued);
    let (mine, rest): (Vec<Queued>, Vec<Queued>) = queued.into_iter().partition(|q| match q {
        Queued::Edit(p) => p.side == side,
        Queued::Command(s, ..) | Queued::Undo(s) | Queued::Redo(s) => *s == side,
    });
    view.queued = rest;
    for q in mine {
        match q {
            Queued::Edit(p) => {
                edit::edit(view, p.side, p.request, p.origin, p.selection);
            }
            Queued::Command(s, cmd, origin) => {
                edit::command_on(view, s, &cmd, origin);
            }
            Queued::Undo(s) => {
                let _ = edit::undo(view, s);
            }
            Queued::Redo(s) => {
                let _ = edit::redo(view, s);
            }
        }
    }
}

/// The inputs waiting for a save.
#[cfg_attr(not(test), allow(dead_code))]
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

/// A key on the focus pane; `page` is the visual lines a page moves. True when the pane took it.
pub fn key(view: &mut TextViewModel, press: Press, metrics: &dyn Metrics, page: u32) -> bool {
    if composing(view) {
        return true;
    }
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
    let Some(doc) = view.doc(side) else {
        return;
    };
    let pos = doc.clamp(pos);
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
    let Some(doc) = view.doc(side) else {
        return;
    };
    let pos = doc.clamp(pos);
    if view.pane(side).caret != pos {
        let pane = view.pane_mut(side);
        pane.set_caret(pos, true);
        pane.sticky_x = None;
        view.caret_moved();
    }
}

/// The primary button was released: a selection settles (the primary selection, D-298).
pub fn release(view: &mut TextViewModel) {
    super::clipboard::pointer_released(view);
}

/// A press in the line-number gutter of `side` on `line`: that line with its terminator is selected (Shift: from the
/// anchor's line).
pub fn gutter_press(view: &mut TextViewModel, side: Side, line: u32, shift: bool) {
    view.set_focus(side);
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
