//! The undo log (TC-060; D-262, contract §1.10.4): unbounded, per document; a step stores the inverse changes
//! (the removed text, the marks and bookmarks the edit displaced) and the selections, never a copy of the
//! document; typing coalesces by the adapter's clock.

use hikasa_formats::TextEncoding;

use super::apply::Change;
use super::{LineEnding, Selection, TextPos, UndoId};

/// Typing within this many milliseconds of the previous keystroke merges into its step.
pub(crate) const TYPING_MERGE_MS: u64 = 1_000;

/// How a step may merge with the next request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// Never merges.
    Plain,
    /// A one-character insert (`insert`) or delete of `[start, end)` at `at_ms`; `word_start` when the inserted
    /// character is a word character after a non-word character.
    Typing {
        insert: bool,
        at_ms: u64,
        start: TextPos,
        end: TextPos,
        word_start: bool,
    },
    /// A request of group `id`.
    Group(u64),
}

/// One undo step.
#[derive(Debug, Clone)]
pub(crate) struct Step {
    pub(crate) id: UndoId,
    /// The inverse of every change, in the order the changes were applied.
    pub(crate) inverses: Vec<Change>,
    pub(crate) before: Selection,
    pub(crate) after: Selection,
    pub(crate) kind: StepKind,
    /// (before, after) of the insert terminator, when the step changed it.
    pub(crate) insert_eol: Option<(LineEnding, LineEnding)>,
    /// (before, after) of the save encoding and BOM, when the step changed them.
    pub(crate) encoding: Option<((TextEncoding, bool), (TextEncoding, bool))>,
}

/// The log: steps `[0, pos)` can be undone, `[pos, len)` redone.
#[derive(Debug, Clone)]
pub(crate) struct UndoLog {
    steps: Vec<Option<Step>>,
    pos: usize,
    /// The position of the last save (the load: 0); `None` once that position is gone (a new edit after undoing
    /// past it).
    save_point: Option<usize>,
    next_id: u64,
}

impl Default for UndoLog {
    fn default() -> UndoLog {
        UndoLog {
            steps: Vec::new(),
            pos: 0,
            save_point: Some(0),
            next_id: 1,
        }
    }
}

impl UndoLog {
    /// Adds a step, merging it into the previous one when its kind allows (§1.10.4); returns the step's id.
    pub(crate) fn push(&mut self, step: Step, _lines: u32) -> UndoId {
        if self.pos < self.steps.len() {
            self.steps.truncate(self.pos);
            if self.save_point.is_some_and(|p| p > self.pos) {
                self.save_point = None;
            }
        }
        if self.save_point != Some(self.pos)
            && let Some(Some(last)) = self.steps.last_mut()
            && merges(last, &step)
        {
            last.inverses.extend(step.inverses);
            last.after = step.after;
            last.kind = step.kind;
            if let Some((before, after)) = step.insert_eol {
                let before = last.insert_eol.map_or(before, |(b, _)| b);
                last.insert_eol = Some((before, after));
            }
            if let Some((before, after)) = step.encoding {
                let before = last.encoding.map_or(before, |(b, _)| b);
                last.encoding = Some((before, after));
            }
            return last.id;
        }
        let id = UndoId(self.next_id);
        self.next_id += 1;
        self.steps.push(Some(Step { id, ..step }));
        self.pos = self.steps.len();
        id
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.pos > 0
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.pos < self.steps.len()
    }

    pub(crate) fn len(&self) -> usize {
        self.steps.len()
    }

    /// Takes the step to undo (put it back with [`UndoLog::put_undone`]).
    pub(crate) fn take_undo(&mut self) -> Option<Step> {
        if self.pos == 0 {
            return None;
        }
        self.pos -= 1;
        self.steps[self.pos].take()
    }

    /// Puts back an undone step (now redoable).
    pub(crate) fn put_undone(&mut self, step: Step) {
        self.steps[self.pos] = Some(step);
    }

    /// Takes the step to redo (put it back with [`UndoLog::put_redone`]).
    pub(crate) fn take_redo(&mut self) -> Option<Step> {
        if self.pos >= self.steps.len() {
            return None;
        }
        self.steps[self.pos].take()
    }

    /// Puts back a redone step (now undoable).
    pub(crate) fn put_redone(&mut self, step: Step) {
        self.steps[self.pos] = Some(step);
        self.pos += 1;
    }

    /// True unless the log stands at the save point.
    pub(crate) fn is_modified(&self) -> bool {
        self.save_point != Some(self.pos)
    }

    /// The current position becomes the save point.
    pub(crate) fn mark_saved(&mut self) {
        self.save_point = Some(self.pos);
    }

    /// Drops every step; the document counts as unmodified only if it stood at the save point.
    pub(crate) fn clear(&mut self) {
        let at_save = !self.is_modified();
        self.steps.clear();
        self.pos = 0;
        self.save_point = at_save.then_some(0);
    }
}

/// True when `next` merges into `last` (§1.10.4): typing of the same kind within 1,000 ms where the last step's
/// caret is, not starting a new word; or requests of the same group.
fn merges(last: &Step, next: &Step) -> bool {
    match (last.kind, next.kind) {
        (
            StepKind::Typing {
                insert: was_insert,
                at_ms: was,
                ..
            },
            StepKind::Typing {
                insert,
                at_ms,
                start,
                end,
                word_start,
            },
        ) => {
            let caret = last.after.caret;
            let adjacent = if insert {
                start == caret
            } else {
                end == caret || start == caret
            };
            was_insert == insert
                && at_ms >= was
                && at_ms - was < TYPING_MERGE_MS
                && adjacent
                && !(insert && word_start)
        }
        (StepKind::Group(a), StepKind::Group(b)) => a == b,
        _ => false,
    }
}
