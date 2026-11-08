//! Applying edits (§1.10.3): every request becomes a list of [`Change`]s, each replacing bytes inside a span of
//! whole physical lines; a change applied returns its inverse, so undo and redo are changes too.

use hikasa_core::cancel::CancelToken;

use super::undo::{Step, StepKind};
use super::{
    EditError, EditOutcome, EditRequest, EditableDocument, GroupHint, LineDelta, Selection,
    TextPos, normalize_breaks, order,
};
use crate::document::blocks::{LineRec, NO_ORIG};
use crate::document::lines::LineTable;
use crate::document::{LineEnding, LineIdx, MAX_DOCUMENT_BYTES, MAX_LINES};

/// One reversible replacement: bytes `[byte, byte + remove)` of the text of lines `[at, at + count)` (their
/// contents and terminators) become `insert`. `marks`, when given, are the marks of every line the change
/// creates (an undo or redo restoring them); else lines whose content and terminator did not change keep
/// theirs and the others take the change's generation and no file line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Change {
    pub(crate) at: u32,
    pub(crate) count: u32,
    pub(crate) byte: u32,
    pub(crate) remove: u32,
    pub(crate) insert: String,
    pub(crate) marks: Option<Vec<(u32, u32)>>,
    /// Bookmarks the change moved: (number, line before the change).
    pub(crate) displaced: Vec<(u8, u32)>,
    /// Generation at which the change was applied (decides whether its inverse restores marks, §1.10.5).
    pub(crate) generation: u64,
}

/// A change applied.
pub(crate) struct Applied {
    /// Its inverse (applying it restores the text, marks and bookmarks).
    pub(crate) inverse: Change,
    /// The lines whose content or terminator changed.
    pub(crate) delta: LineDelta,
    /// Absolute byte offset of the end of the inserted bytes.
    pub(crate) end: u32,
    /// Bytes added to the text (negative: removed).
    pub(crate) growth: i64,
    /// Absolute byte offset of the start of the replaced bytes.
    pub(crate) start: u32,
}

impl EditableDocument {
    /// [`EditableDocument::apply`] without the editability check (undo and redo of a step made before a block
    /// appeared are refused by `apply`'s caller, not here).
    pub(crate) fn apply_unchecked(
        &mut self,
        req: &EditRequest,
        group: GroupHint,
    ) -> Result<EditOutcome, EditError> {
        match req {
            EditRequest::Replace {
                range,
                text,
                keep_eols,
            } => {
                let (a, b) = order(range.clone());
                self.check(a)?;
                self.check(b)?;
                let insert = if *keep_eols {
                    std::borrow::Cow::Borrowed(text.as_str())
                } else {
                    normalize_breaks(text, self.insert_eol)
                };
                let kind = typing_kind(self, a, b, &insert, group);
                self.check_limits(self.abs(b) - self.abs(a), &insert)?;
                let before = Selection {
                    anchor: a,
                    caret: b,
                };
                let pending = vec![Pending::Range(a, b, insert.into_owned())];
                self.run_step(pending, before, kind, None, None)
            }
            EditRequest::Batch { edits, keep_eols } => {
                let mut prev: Option<TextPos> = None;
                let mut changes = Vec::with_capacity(edits.len());
                for (range, text) in edits {
                    let (a, b) = order(range.clone());
                    self.check(a)?;
                    self.check(b)?;
                    if prev.is_some_and(|p| a < p) {
                        return Err(EditError::BadRange);
                    }
                    prev = Some(b);
                    let insert = if *keep_eols {
                        std::borrow::Cow::Borrowed(text.as_str())
                    } else {
                        normalize_breaks(text, self.insert_eol)
                    };
                    changes.push((a, b, insert.into_owned()));
                }
                let removed: u32 = changes
                    .iter()
                    .map(|(a, b, _)| self.abs(*b) - self.abs(*a))
                    .sum();
                let joined: String = changes.iter().map(|(_, _, t)| t.as_str()).collect();
                self.check_limits(removed, &joined)?;
                let before = match (changes.first(), changes.last()) {
                    (Some(f), Some(l)) => Selection {
                        anchor: f.0,
                        caret: l.1,
                    },
                    _ => return Err(EditError::Unsupported("an empty batch")),
                };
                // Applied last first: an earlier range's positions stay valid after a later one changed.
                let pending = changes
                    .into_iter()
                    .rev()
                    .map(|(a, b, t)| Pending::Range(a, b, t))
                    .collect();
                let kind = group_kind(group);
                self.run_step(pending, before, kind, None, None)
            }
            EditRequest::SetEols { lines, eol } => {
                let n = self.line_count();
                if lines.start > lines.end || lines.end > n {
                    return Err(EditError::BadRange);
                }
                if matches!(eol, LineEnding::Artificial)
                    || (*eol == LineEnding::None && lines.end < n && !lines.is_empty())
                {
                    return Err(EditError::BadRange);
                }
                if lines.is_empty() {
                    return Err(EditError::Unsupported("no lines"));
                }
                let change = self.eol_change(lines.clone(), *eol);
                self.check_limits(change.remove, &change.insert)?;
                let whole = lines.start == 0 && lines.end == n && *eol != LineEnding::None;
                let new_eol = whole.then_some(*eol);
                let pos = TextPos::new(lines.start, 0);
                self.run_step(
                    vec![Pending::Ready(change)],
                    Selection::caret(pos),
                    group_kind(group),
                    new_eol,
                    None,
                )
            }
            EditRequest::SetEncoding { encoding, bom } => {
                if self.doc.converted() {
                    return Err(EditError::Unsupported(
                        "the save program of the format decides the bytes",
                    ));
                }
                let pos = Selection::caret(TextPos::ZERO);
                self.run_step(
                    Vec::new(),
                    pos,
                    group_kind(group),
                    None,
                    Some((*encoding, *bom)),
                )
            }
        }
    }

    /// Refuses an edit that removes `removed` bytes and inserts `insert` when the result would pass the D-43
    /// limits (bytes exactly; lines counted by the breaks inserted, an upper bound).
    fn check_limits(&self, removed: u32, insert: &str) -> Result<(), EditError> {
        let bytes =
            i64::from(self.doc.table().text_len()) + insert.len() as i64 - i64::from(removed);
        let lines = i64::from(self.line_count()) + insert.matches(['\n', '\r']).count() as i64;
        let (max_bytes, max_lines) = self.limits;
        if bytes > max_bytes.min(MAX_DOCUMENT_BYTES) as i64
            || lines > i64::from(max_lines.min(MAX_LINES))
        {
            return Err(EditError::TooLarge);
        }
        Ok(())
    }

    /// First line of the physical line holding `line` (CPL pieces, TC-003).
    pub(crate) fn phys_start(&self, line: u32) -> u32 {
        let t = self.doc.table();
        let mut l = line.min(t.len());
        while l > 0 && t.eol(l - 1) == LineEnding::Artificial {
            l -= 1;
        }
        l
    }

    /// One past the last piece of the physical line holding `line`.
    pub(crate) fn phys_end(&self, line: u32) -> u32 {
        let t = self.doc.table();
        let n = t.len();
        let mut l = line;
        while l < n && t.eol(l) == LineEnding::Artificial {
            l += 1;
        }
        (l + 1).min(n).max(line.min(n))
    }

    /// The change that replaces `[a, b)` with `insert` (positions checked).
    fn change_for(&self, a: TextPos, b: TextPos, insert: &str) -> Change {
        let n = self.line_count();
        let at = self.phys_start(a.line.0.min(n));
        let end = if b.line.0 >= n {
            n
        } else {
            self.phys_end(b.line.0)
        };
        let span_start = self.doc.table().start_of(at);
        let (sa, sb) = (self.abs(a), self.abs(b));
        Change {
            at,
            count: end - at,
            byte: sa - span_start,
            remove: sb - sa,
            insert: insert.to_owned(),
            marks: None,
            displaced: Vec::new(),
            generation: 0,
        }
    }

    /// The change that sets the terminator of `lines` to `eol` (artificial pieces keep theirs).
    fn eol_change(&self, lines: std::ops::Range<u32>, eol: LineEnding) -> Change {
        let at = self.phys_start(lines.start);
        let end = self.phys_end(lines.end - 1);
        let span_start = self.doc.table().start_of(at);
        let span_end = self.doc.table().start_of(end);
        let old = self.span_text(span_start, span_end);
        let mut new = String::with_capacity(old.len() + 8);
        for line in at..end {
            new.push_str(&self.doc.line(LineIdx(line)));
            let current = self.doc.line_ending(LineIdx(line));
            let kept = current == LineEnding::Artificial || !lines.contains(&line);
            new.push_str(if kept {
                current.as_text()
            } else {
                eol.as_text()
            });
        }
        minimal_change(at, end - at, &old, &new)
    }

    /// The byte at absolute offset `off`.
    fn byte_at(&self, off: u32) -> Option<u8> {
        self.doc.rope().get_byte(off as usize)
    }

    /// The text of absolute bytes `[s, e)`.
    pub(crate) fn span_text(&self, s: u32, e: u32) -> String {
        if s >= e {
            return String::new();
        }
        self.doc
            .rope()
            .byte_slice(s as usize..e as usize)
            .to_string()
    }

    /// Applies one change at generation `generation` (restoring its stored marks when `restore`). The span is
    /// widened first when a lone CR would meet a following LF (a fresh load would join them into CRLF).
    pub(crate) fn apply_change(
        &mut self,
        mut ch: Change,
        generation: u64,
        restore: bool,
    ) -> Applied {
        let cpl = self.opts.chars_per_line_limit;
        loop {
            let t = self.doc.table();
            let span_start = t.start_of(ch.at);
            let span_end = t.start_of(ch.at + ch.count);
            // Front: the previous line ends with a lone CR and the new span text starts with LF.
            let new_starts_with_lf = if ch.byte == 0 && !ch.insert.is_empty() {
                ch.insert.starts_with('\n')
            } else if ch.byte == 0 {
                // Nothing inserted at the start: the first kept byte after the removal.
                let after = span_start + ch.remove;
                after < span_end && self.byte_at(after) == Some(b'\n')
            } else {
                false
            };
            if ch.at > 0 && new_starts_with_lf && t.eol(ch.at - 1) == LineEnding::Cr {
                let prev = self.phys_start(ch.at - 1);
                let added = span_start - t.start_of(prev);
                ch.count += ch.at - prev;
                ch.at = prev;
                ch.byte += added;
                continue;
            }
            // Back: the new span text ends with a lone CR and the next line is an empty LF line.
            let end_in_span = ch.byte + ch.remove == span_end - span_start;
            let ends_with_cr = if end_in_span {
                if ch.insert.is_empty() {
                    ch.byte > 0 && self.byte_at(span_start + ch.byte - 1) == Some(b'\r')
                } else {
                    ch.insert.ends_with('\r')
                }
            } else {
                span_end > span_start && self.byte_at(span_end - 1) == Some(b'\r')
            };
            let next = ch.at + ch.count;
            if ends_with_cr
                && next < t.len()
                && t.eol(next) == LineEnding::Lf
                && self.doc.line_len(next) == 0
            {
                ch.count += 1;
                continue;
            }
            break;
        }
        let t = self.doc.table();
        let span_start = t.start_of(ch.at);
        let span_end = t.start_of(ch.at + ch.count);
        let old = self.span_text(span_start, span_end);
        let (b0, b1) = (ch.byte as usize, (ch.byte + ch.remove) as usize);
        let mut new = String::with_capacity(old.len() - (b1 - b0) + ch.insert.len());
        new.push_str(&old[..b0]);
        new.push_str(&ch.insert);
        new.push_str(&old[b1..]);
        let built = LineTable::build(new.as_bytes(), cpl, "edit", &CancelToken::never())
            .map(|(table, _)| table)
            .unwrap_or_default();
        let old_recs: Vec<LineRec> = (ch.at..ch.at + ch.count).map(|l| t.rec(l)).collect();
        let new_n = built.len() as usize;
        let old_line = |i: usize| -> &str {
            t.content(ch.at + i as u32).map_or("", |(s, e)| {
                &old[s - span_start as usize..e - span_start as usize]
            })
        };
        let new_line =
            |j: usize| -> &str { built.content(j as u32).map_or("", |(s, e)| &new[s..e]) };
        // Lines whose content and terminator are unchanged at the start and the end of the span.
        let same = |i: usize, j: usize| {
            old_recs[i].eol == built.eol(j as u32) && old_line(i) == new_line(j)
        };
        let mut prefix = 0usize;
        while prefix < old_recs.len() && prefix < new_n && same(prefix, prefix) {
            prefix += 1;
        }
        let mut suffix = 0usize;
        while suffix < old_recs.len() - prefix
            && suffix < new_n - prefix
            && same(old_recs.len() - 1 - suffix, new_n - 1 - suffix)
        {
            suffix += 1;
        }
        let g = u32::try_from(generation).unwrap_or(u32::MAX);
        let restored = if restore { ch.marks.take() } else { None };
        let restored = restored.filter(|m| m.len() == new_n);
        let mut recs: Vec<LineRec> = Vec::with_capacity(new_n);
        for j in 0..new_n {
            let r = built.rec(j as u32);
            let (generation, orig) = match &restored {
                Some(m) => m[j],
                None if j < prefix => (old_recs[j].generation, old_recs[j].orig),
                None if j >= new_n - suffix => {
                    let i = old_recs.len() - (new_n - j);
                    (old_recs[i].generation, old_recs[i].orig)
                }
                None => (g, NO_ORIG),
            };
            recs.push(LineRec {
                generation,
                orig,
                ..r
            });
        }
        let inverse_marks: Vec<(u32, u32)> =
            old_recs.iter().map(|r| (r.generation, r.orig)).collect();
        let removed_text = old[b0..b1].to_owned();
        // The table splice covers the changed lines only, unless marks are restored over the whole span.
        let (lo, hi_old, hi_new) = if restored.is_some() {
            (0, old_recs.len(), new_n)
        } else {
            (prefix, old_recs.len() - suffix, new_n - suffix)
        };
        let delta = LineDelta {
            at: ch.at + prefix as u32,
            removed: (old_recs.len() - suffix - prefix) as u32,
            inserted: (new_n - suffix - prefix) as u32,
        };
        let at_byte = span_start + ch.byte;
        self.doc.edit_raw(
            at_byte as usize,
            ch.remove as usize,
            &ch.insert,
            ch.at + lo as u32,
            (hi_old - lo) as u32,
            &recs[lo..hi_new],
        );
        let displaced = self.follow_bookmarks(delta);
        // Undoing restores the bookmarks this change moved.
        for &(n, line) in &ch.displaced {
            if let Some(slot) = self.bookmarks.get_mut(usize::from(n)) {
                *slot = Some(line);
            }
        }
        let growth = ch.insert.len() as i64 - i64::from(ch.remove);
        Applied {
            inverse: Change {
                at: ch.at,
                count: new_n as u32,
                byte: ch.byte,
                remove: ch.insert.len() as u32,
                insert: removed_text,
                marks: Some(inverse_marks),
                displaced,
                generation,
            },
            delta,
            end: at_byte + ch.insert.len() as u32,
            growth,
            start: at_byte,
        }
    }

    /// Moves the bookmarks through `delta`; returns the ones that were on a removed line (moved to the start of
    /// the removal), with their old lines.
    fn follow_bookmarks(&mut self, delta: LineDelta) -> Vec<(u8, u32)> {
        let n = self.line_count();
        let mut displaced = Vec::new();
        for (k, slot) in self.bookmarks.iter_mut().enumerate() {
            let Some(line) = *slot else { continue };
            if line >= delta.at + delta.removed {
                *slot = Some(line + delta.inserted - delta.removed);
            } else if line >= delta.at && line - delta.at >= delta.inserted {
                displaced.push((k as u8, line));
                *slot = (n > 0).then(|| delta.at.min(n - 1));
            }
        }
        displaced
    }

    /// Applies `changes` (in order) as one step, or merges them into the previous step per `group`.
    fn run_step(
        &mut self,
        changes: Vec<Pending>,
        before: Selection,
        kind: StepKind,
        new_insert_eol: Option<LineEnding>,
        new_encoding: Option<(hikasa_formats::TextEncoding, bool)>,
    ) -> Result<EditOutcome, EditError> {
        self.generation += 1;
        let generation = self.generation;
        let mut inverses = Vec::with_capacity(changes.len());
        let mut deltas = Vec::with_capacity(changes.len());
        let mut caret_abs: Option<u32> = None;
        for pending in changes {
            let ch = match pending {
                Pending::Ready(ch) => ch,
                // Built just before it is applied: the later ranges of a batch are already applied.
                Pending::Range(a, b, insert) => self.change_for(a, b, &insert),
            };
            let applied = self.apply_change(ch, generation, false);
            // An earlier change (applied later, in a batch) moves the caret of a later one.
            if let Some(c) = caret_abs.as_mut()
                && applied.start <= *c
            {
                *c = (i64::from(*c) + applied.growth) as u32;
            }
            if caret_abs.is_none() {
                caret_abs = Some(applied.end);
            }
            inverses.push(applied.inverse);
            deltas.push(applied.delta);
        }
        let old_insert_eol = new_insert_eol.map(|e| std::mem::replace(&mut self.insert_eol, e));
        let old_encoding = new_encoding.map(|e| std::mem::replace(&mut self.encoding, e));
        let selection = match caret_abs {
            Some(off) => Selection::caret(self.pos_of(off)),
            None => before,
        };
        let step = Step {
            id: super::UndoId(0),
            inverses,
            before,
            after: selection,
            kind,
            insert_eol: old_insert_eol.map(|old| (old, self.insert_eol)),
            encoding: old_encoding.map(|old| (old, self.encoding)),
        };
        let id = self.log.push(step, self.doc.line_count());
        Ok(outcome(id, &deltas, selection))
    }

    /// Undoes the last step: its inverse changes, applied as an edit (the live comparison sees it as one). The
    /// editability is checked by [`EditableDocument::try_undo`].
    pub(crate) fn undo_unchecked(&mut self) -> Option<EditOutcome> {
        let mut step = self.log.take_undo()?;
        self.generation += 1;
        let generation = self.generation;
        let saved = self.saved_gen;
        let mut redo = Vec::with_capacity(step.inverses.len());
        let mut deltas = Vec::with_capacity(step.inverses.len());
        for ch in std::mem::take(&mut step.inverses).into_iter().rev() {
            let restore = ch.generation > saved;
            let applied = self.apply_change(ch, generation, restore);
            redo.push(applied.inverse);
            deltas.push(applied.delta);
        }
        redo.reverse();
        step.inverses = redo;
        if let Some((old, _)) = step.insert_eol {
            self.insert_eol = old;
        }
        if let Some((old, _)) = step.encoding {
            self.encoding = old;
        }
        let selection = step.before;
        let id = step.id;
        self.log.put_undone(step);
        Some(outcome(id, &deltas, selection))
    }

    /// Redoes the last undone step (the editability is checked by [`EditableDocument::try_redo`]).
    pub(crate) fn redo_unchecked(&mut self) -> Option<EditOutcome> {
        let mut step = self.log.take_redo()?;
        self.generation += 1;
        let generation = self.generation;
        let saved = self.saved_gen;
        let mut undo = Vec::with_capacity(step.inverses.len());
        let mut deltas = Vec::with_capacity(step.inverses.len());
        for ch in std::mem::take(&mut step.inverses) {
            let restore = ch.generation > saved;
            let applied = self.apply_change(ch, generation, restore);
            undo.push(applied.inverse);
            deltas.push(applied.delta);
        }
        step.inverses = undo;
        if let Some((_, new)) = step.insert_eol {
            self.insert_eol = new;
        }
        if let Some((_, new)) = step.encoding {
            self.encoding = new;
        }
        let selection = step.after;
        let id = step.id;
        self.log.put_redone(step);
        Some(outcome(id, &deltas, selection))
    }
}

/// A change of a step, or a range still to turn into one when its turn comes.
enum Pending {
    Ready(Change),
    Range(TextPos, TextPos, String),
}

/// The change from `old` to `new` (texts of the same span), trimmed to the differing bytes at char boundaries.
fn minimal_change(at: u32, count: u32, old: &str, new: &str) -> Change {
    let mut p = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(x, y)| x == y)
        .count();
    while !old.is_char_boundary(p) || !new.is_char_boundary(p) {
        p -= 1;
    }
    let max_s = (old.len() - p).min(new.len() - p);
    let mut s = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_s)
        .take_while(|(x, y)| x == y)
        .count();
    while !old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s) {
        s -= 1;
    }
    Change {
        at,
        count,
        byte: p as u32,
        remove: (old.len() - s - p) as u32,
        insert: new[p..new.len() - s].to_owned(),
        marks: None,
        displaced: Vec::new(),
        generation: 0,
    }
}

/// The outcome of a step: the covering delta of its changes (each in the numbering after the previous ones):
/// lines before `at` and after the covered region are unchanged.
fn outcome(id: super::UndoId, deltas: &[LineDelta], selection: Selection) -> EditOutcome {
    // The covered region [lo, hi) in the current numbering and its length in the original numbering.
    let mut covered: Option<(u32, u32, u32)> = None;
    for d in deltas.iter().filter(|d| !d.is_empty()) {
        covered = Some(match covered {
            None => (d.at, d.at + d.inserted, d.removed),
            Some((lo, hi, orig)) => {
                let (u_lo, u_hi) = (lo.min(d.at), hi.max(d.at + d.removed));
                let orig = orig + (u_hi - u_lo) - (hi - lo);
                (u_lo, u_hi + d.inserted - d.removed, orig)
            }
        });
    }
    let delta = covered.map_or(LineDelta::NONE, |(lo, hi, orig)| LineDelta {
        at: lo,
        removed: orig,
        inserted: hi - lo,
    });
    EditOutcome {
        step: id,
        delta,
        selection,
        changed: delta.at..delta.at + delta.inserted,
        deltas: deltas.to_vec(),
    }
}

/// The typing kind of a `Replace` for coalescing: a one-character insert at an empty range, or a
/// one-character delete with empty text.
fn typing_kind(
    doc: &EditableDocument,
    a: TextPos,
    b: TextPos,
    insert: &str,
    group: GroupHint,
) -> StepKind {
    let GroupHint::Typing { at_ms } = group else {
        return group_kind(group);
    };
    let one_char = |s: &str| {
        let mut chars = s.chars();
        chars.next().is_some() && chars.next().is_none()
    };
    if a == b && one_char(insert) && !insert.contains(['\n', '\r']) {
        let c = insert.chars().next().unwrap_or(' ');
        let before = doc.line(a.line)[..a.byte as usize].chars().next_back();
        let word_start = super::words::is_word(c, &doc.opts.word_chars)
            && !before.is_some_and(|p| super::words::is_word(p, &doc.opts.word_chars));
        return StepKind::Typing {
            insert: true,
            at_ms,
            start: a,
            end: b,
            word_start,
        };
    }
    if insert.is_empty() && a.line == b.line && one_char(&doc.text(a..b)) {
        return StepKind::Typing {
            insert: false,
            at_ms,
            start: a,
            end: b,
            word_start: false,
        };
    }
    StepKind::Plain
}

/// The step kind of a request outside typing.
fn group_kind(group: GroupHint) -> StepKind {
    match group {
        GroupHint::Group(id) => StepKind::Group(id),
        GroupHint::None | GroupHint::Typing { .. } => StepKind::Plain,
    }
}
