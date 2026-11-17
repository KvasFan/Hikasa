//! The one place that applies edits (M2 W11a, contract §1.11.6, D-297; architecture check 7: `EditableDocument::apply`,
//! `undo` and `redo` are named here only): every request goes `apply → note_edit → recompare.on_edit → caret →
//! dirty`; the editing commands of the catalogue map to core's `plan_command`, `copy_rows`, `select_section` and
//! `edit::cursor`; blocked panes show their block, a `Binary` pane asks once (D-297).

use std::ops::Range;

use hikasa_core::Side;
use hikasa_text::RowIdx;
use hikasa_text::edit::{
    EditCommand, EditError, EditOutcome, EditRequest, GroupHint, Selection, TextPos, copy_rows,
    delete_lines, doc_end, plan_command, select_section,
};

use super::{PendingEdit, TextRequest, TextViewModel, header, index_of, input, only_binary};
use crate::commands::CommandId;
use crate::status::Message;

/// How an edit joins the undo log (D-262): typing coalesces by the adapter's clock, a group by its id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Typed text, Backspace, Delete at this adapter time (ms of the window's monotonic clock).
    Typing {
        /// Milliseconds.
        at_ms: u64,
    },
    /// A group (Replace All of the find panel, W11b).
    #[allow(dead_code)] // Replace All's group (W11b)
    Group(u64),
    /// A step of its own.
    Single,
}

impl Origin {
    /// The core's hint.
    pub fn hint(self) -> GroupHint {
        match self {
            Origin::Typing { at_ms } => GroupHint::Typing { at_ms },
            Origin::Group(id) => GroupHint::Group(id),
            Origin::Single => GroupHint::None,
        }
    }
}

/// Applies `req` to the pane of `side` (§1.11.6): `apply → note_edit → recompare.on_edit → caret/selection set →
/// dirty`. `caret` is the selection after it (`None`: the outcome's).
pub fn run(
    view: &mut TextViewModel,
    side: Side,
    req: EditRequest,
    origin: Origin,
    caret: Option<Selection>,
) -> Result<EditOutcome, EditError> {
    let now = view.now;
    let Some(live) = view.view.as_mut() else {
        return Err(EditError::Unsupported("the comparison is not ready"));
    };
    let doc = match side {
        Side::Right => &mut live.docs.right,
        _ => &mut live.docs.left,
    };
    let out = doc.apply(&req, origin.hint())?;
    live.live.note_edit(side, doc, &out);
    let after = caret.unwrap_or(out.selection);
    view.recompare.on_edit(now);
    finish(view, side, after);
    Ok(out)
}

/// Undoes the last step of `side` (the caret goes to the step's selection before it).
pub fn undo(view: &mut TextViewModel, side: Side) -> Result<Option<EditOutcome>, EditError> {
    step(view, side, true)
}

/// Redoes the last undone step of `side`.
pub fn redo(view: &mut TextViewModel, side: Side) -> Result<Option<EditOutcome>, EditError> {
    step(view, side, false)
}

fn step(
    view: &mut TextViewModel,
    side: Side,
    back: bool,
) -> Result<Option<EditOutcome>, EditError> {
    let now = view.now;
    let Some(live) = view.view.as_mut() else {
        return Ok(None);
    };
    let doc = match side {
        Side::Right => &mut live.docs.right,
        _ => &mut live.docs.left,
    };
    let out = if back {
        doc.try_undo()?
    } else {
        doc.try_redo()?
    };
    let Some(out) = out else {
        return Ok(None);
    };
    live.live.note_edit(side, doc, &out);
    view.recompare.on_edit(now);
    finish(view, side, out.selection);
    Ok(Some(out))
}

/// The common end of an edit: the pane's selection, the slice rebuilt, the caret revealed.
fn finish(view: &mut TextViewModel, side: Side, after: Selection) {
    let clamped = view.doc(side).map_or((after.anchor, after.caret), |d| {
        (d.clamp(after.anchor), d.clamp(after.caret))
    });
    let pane = view.pane_mut(side);
    pane.anchor = clamped.0;
    pane.caret = clamped.1;
    pane.lines = None;
    pane.sticky_x = None;
    pane.preedit = None;
    view.rebuild_keep();
    if side == view.focus {
        view.caret_moved();
    }
}

/// The front door of every edit of the view (§1.11.6, §1.11.9 rule 6): a pane being saved queues it (≤ 1,024, then
/// *Wait for the save to end.*), a blocked pane shows its block, a `Binary` pane asks once; else [`run`]. True when
/// the edit was applied.
pub fn edit(
    view: &mut TextViewModel,
    side: Side,
    request: EditRequest,
    origin: Origin,
    caret: Option<Selection>,
) -> bool {
    let k = index_of(side);
    if view.frozen(side) {
        input::queue(
            view,
            input::Queued::Edit(PendingEdit {
                side,
                request,
                origin,
                selection: caret,
            }),
        );
        return false;
    }
    let Some(doc) = view.doc(side) else {
        return false;
    };
    if !doc.editability().editable() {
        if only_binary(doc) && view.binary_answer[k].is_none() {
            view.pending_binary = Some(PendingEdit {
                side,
                request,
                origin,
                selection: caret,
            });
            view.request_window(TextRequest::AskBinary(side));
            return false;
        }
        blocked_message(view, side);
        return false;
    }
    match run(view, side, request, origin, caret) {
        Ok(_) => true,
        Err(EditError::Blocked(_)) => {
            blocked_message(view, side);
            false
        }
        Err(EditError::TooLarge) => {
            view.request_window(TextRequest::Message(Message::error(
                "The edit would make the document larger than Hikasa can hold.",
            )));
            false
        }
        Err(e) => {
            view.request_window(TextRequest::Message(Message::error(e.to_string())));
            false
        }
    }
}

/// The message panel line of a blocked pane: `<status text>: <the blocks, joined>` (D-297; info kind, no notice).
pub fn blocked_message(view: &mut TextViewModel, side: Side) {
    let Some(doc) = view.doc(side) else {
        return;
    };
    let text = header::block_message(&doc.editability());
    view.request_window(TextRequest::Message(Message::info(text)));
}

/// The format's *when saving* options of `side` applied as one undo step (D-272), noted by the live comparison like
/// an edit. True when something changed.
pub fn apply_save_options(view: &mut TextViewModel, side: Side) -> bool {
    let now = view.now;
    let Some(live) = view.view.as_mut() else {
        return false;
    };
    let opts = live.save_options(side);
    let doc = match side {
        Side::Right => &mut live.docs.right,
        _ => &mut live.docs.left,
    };
    match hikasa_text::save::apply_save_options(doc, opts) {
        Ok(Some(pre)) => {
            live.live.note_edit(side, doc, &pre.outcome);
            view.recompare.on_edit(now);
            let caret = view.pane(side).selection();
            finish(view, side, caret);
            true
        }
        _ => false,
    }
}

/// The answer to the binary question of `side` (D-297): Yes lifts the block of that pane only and applies the
/// waiting edit; No is remembered until the view reloads.
pub fn answer_binary(view: &mut TextViewModel, side: Side, yes: bool) {
    let k = index_of(side);
    view.binary_answer[k] = Some(yes);
    let pending = view.pending_binary.take();
    if yes {
        if let Some(live) = view.view.as_mut() {
            match side {
                Side::Right => live.docs.right.allow_binary_edits(),
                _ => live.docs.left.allow_binary_edits(),
            }
        }
        view.rebuild_keep();
        if let Some(p) = pending.filter(|p| p.side == side) {
            edit(view, side, p.request, p.origin, p.selection);
        }
    }
}

/// A command of the editor on the focus pane (`EditCommand`); a line-set selection takes Delete and Backspace as
/// `delete_lines`. True when something was applied.
pub fn command_on(view: &mut TextViewModel, side: Side, cmd: &EditCommand, origin: Origin) -> bool {
    if view.frozen(side) {
        // Planned when it is applied, at the caret of that moment (D-300's order).
        input::queue(view, input::Queued::Command(side, cmd.clone(), origin));
        return false;
    }
    let Some(doc) = view.doc(side) else {
        return false;
    };
    let pane = view.pane(side);
    if let Some(lines) = pane.lines.clone()
        && matches!(
            cmd,
            EditCommand::Delete | EditCommand::Backspace | EditCommand::DeleteLine
        )
    {
        let Some(req) = delete_lines(doc, &lines) else {
            return false;
        };
        return edit(view, side, req, Origin::Single, None);
    }
    let Some((req, after)) = plan_command(doc, pane.selection(), cmd) else {
        return false;
    };
    // The plan's selection is exact without a characters-per-line limit; with one the outcome's is.
    let exact = doc.options().chars_per_line_limit.is_none_or(|c| c == 0);
    let keeps = matches!(
        cmd,
        EditCommand::IncreaseIndent | EditCommand::DecreaseIndent | EditCommand::BackTab
    ) || (matches!(cmd, EditCommand::Tab) && after.anchor != after.caret);
    let caret = (exact && keeps).then_some(after);
    edit(view, side, req, origin, caret)
}

/// The editing commands of the catalogue (§1.11.6, §1.11.8; `Copy`, `Cut`, `Paste`, saves: the window's). False for an
/// id this module does not run.
pub fn command(view: &mut TextViewModel, id: CommandId) -> bool {
    let side = view.focus;
    let cmd = match id {
        CommandId::Delete => Some(EditCommand::Delete),
        CommandId::DeleteItem => Some(EditCommand::DeleteLine),
        CommandId::DeleteItemBegin => Some(EditCommand::DeleteToLineStart),
        CommandId::DeleteItemEnd => Some(EditCommand::DeleteToLineEnd),
        CommandId::DeleteWord => Some(EditCommand::DeleteWord),
        CommandId::DeleteWordBegin => Some(EditCommand::DeleteToWordStart),
        CommandId::DeleteWordEnd => Some(EditCommand::DeleteToWordEnd),
        CommandId::InsertItemBefore => Some(EditCommand::InsertLineBefore),
        CommandId::InsertItemAfter => Some(EditCommand::InsertLineAfter),
        CommandId::IncreaseIndent => Some(EditCommand::IncreaseIndent),
        CommandId::DecreaseIndent => Some(EditCommand::DecreaseIndent),
        _ => None,
    };
    if let Some(cmd) = cmd {
        command_on(view, side, &cmd, Origin::Single);
        return true;
    }
    match id {
        CommandId::Undo | CommandId::Redo => {
            if view.frozen(side) {
                let q = if id == CommandId::Undo {
                    input::Queued::Undo(side)
                } else {
                    input::Queued::Redo(side)
                };
                input::queue(view, q);
                return true;
            }
            let result = if id == CommandId::Undo {
                undo(view, side)
            } else {
                redo(view, side)
            };
            if let Err(EditError::Blocked(_)) = result {
                blocked_message(view, side);
            }
        }
        CommandId::SelectAll => {
            if let Some(doc) = view.doc(side) {
                let end = doc_end(doc);
                view.pane_mut(side).select(TextPos::ZERO, end);
                view.caret_moved();
            }
        }
        CommandId::SelectSection => {
            let Some(cmp) = view.comparison() else {
                return true;
            };
            let line = view.pane(side).caret.line;
            if let Some(range) = select_section(cmp, side, line) {
                view.pane_mut(side).select(range.start, range.end);
                view.caret_moved();
            }
        }
        CommandId::CopyRight => copy_selection(view, Side::Left),
        CommandId::CopyLeft => copy_selection(view, Side::Right),
        CommandId::CopySide => copy_selection(view, side),
        CommandId::CopyItemRight => copy_row(view, Side::Left),
        CommandId::CopyItemLeft => copy_row(view, Side::Right),
        CommandId::CopyItemSide => copy_row(view, side),
        _ => return false,
    }
    true
}

/// The other side.
pub fn other(side: Side) -> Side {
    if side == Side::Right {
        Side::Left
    } else {
        Side::Right
    }
}

/// The rows the focus pane's selection covers (a selection ending at a line's start leaves that line out), else
/// `None`.
pub fn selected_rows(view: &TextViewModel) -> Option<Range<u32>> {
    let side = view.focus;
    let pane = view.pane(side);
    let (first, last) = if let Some(lines) = pane.lines.as_ref().filter(|l| !l.is_empty()) {
        let ranges = lines.0.ranges();
        (ranges.first()?.start, ranges.last()?.end - 1)
    } else if pane.anchor != pane.caret {
        let r = pane.selection().range();
        let last = if r.end.byte == 0 && r.end.line > r.start.line {
            r.end.line.0 - 1
        } else {
            r.end.line.0
        };
        (r.start.line.0, last)
    } else {
        return None;
    };
    let a = view.row_of_line(side, first)?;
    let b = view.row_of_line(side, last)?;
    Some(a.min(b)..a.max(b) + 1)
}

/// The rows of the section holding the cursor row.
pub fn current_section(view: &TextViewModel) -> Option<Range<u32>> {
    let cmp = view.comparison()?;
    let row = view.cursor()?;
    let id = cmp.section_at(row)?;
    cmp.sections().get(id.0 as usize).map(|s| s.rows.clone())
}

/// Copy to Right/Left (§1.11.8): the selected rows, else the current section, from `from` to the other pane.
fn copy_selection(view: &mut TextViewModel, from: Side) {
    let rows = selected_rows(view).or_else(|| current_section(view));
    if let Some(rows) = rows {
        copy_rows_to(view, from, rows);
    }
}

/// Copy Line to Right/Left: the cursor row.
fn copy_row(view: &mut TextViewModel, from: Side) {
    if let Some(row) = view.cursor() {
        copy_rows_to(view, from, row.0..row.0 + 1);
    }
}

/// `copy_rows` of `rows` from `from` applied to the other pane as one step (`Origin::Single`): the source pane's
/// caret stays, the target's goes to the copied lines' start. True when applied.
pub fn copy_rows_to(view: &mut TextViewModel, from: Side, rows: Range<u32>) -> bool {
    let target = other(from);
    let (Some(cmp), Some(doc)) = (view.comparison(), view.doc(target)) else {
        return false;
    };
    let Some(copy) = copy_rows(cmp, rows, from, doc.insert_eol()) else {
        return false;
    };
    let caret = Selection::caret(copy.selection.anchor);
    edit(view, target, copy.request, Origin::Single, Some(caret))
}

/// The line set of `lines` of the focus pane (a gutter drag).
pub fn select_lines(view: &mut TextViewModel, side: Side, lines: Range<u32>) {
    let Some(doc) = view.doc(side) else {
        return;
    };
    let last = lines.end.saturating_sub(1).max(lines.start);
    let start = TextPos::new(lines.start, 0);
    let end = if last + 1 < doc.position_lines() {
        TextPos::new(last + 1, 0)
    } else {
        doc_end(doc)
    };
    let pane = view.pane_mut(side);
    pane.select(start, end);
    view.caret_moved();
}

/// The first row of every difference section among `rows` (the gutter's copy buttons, D-299): (row, section rows).
pub fn sections_starting_in(view: &TextViewModel, rows: Range<u32>) -> Vec<(u32, Range<u32>)> {
    let Some(cmp) = view.comparison() else {
        return Vec::new();
    };
    let sections = cmp.sections();
    let k = sections.partition_point(|s| s.rows.start < rows.start);
    sections[k..]
        .iter()
        .take_while(|s| s.rows.start < rows.end)
        .filter(|s| {
            cmp.row(RowIdx(s.rows.start))
                .is_some_and(|r| r.state != hikasa_text::RowState::Same && !r.ignored)
        })
        .map(|s| (s.rows.start, s.rows.clone()))
        .collect()
}
