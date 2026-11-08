//! The editing commands (TC-060, TC-062; D-264, contract §1.10.6): pure functions that turn a command and the
//! selection into an [`EditRequest`] and the selection after it, so the GUI computes no text (T11). Cursor
//! geometry (visual lines, pixel columns, word wrap) stays in the view.
//!
//! The selection returned for a command that inserts text is the caret after the text as the document's lines
//! stand without a characters-per-line limit; with a limit the inserted text may be re-wrapped, and the view
//! takes [`super::EditOutcome::selection`] instead.

use std::ops::Range;

use hikasa_core::side::Side;
use unicode_segmentation::UnicodeSegmentation;

use super::words::{self, Class};
use super::{EditRequest, EditableDocument, Selection, TextPos};
use crate::compare::{Direction, RowIdx, TextComparison};
use crate::document::{LineEnding, LineIdx, TextDocument};

/// A direction (word steps, edited ranges).
pub type Dir = Direction;

/// An editing command (§1.10.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditCommand {
    /// Typed text; `overwrite` (Ins) replaces as many grapheme clusters of the line as typed, never a
    /// terminator.
    Type {
        /// The text.
        text: String,
        /// Overwrite mode.
        overwrite: bool,
    },
    /// Pasted text (breaks become the insert terminator).
    Paste {
        /// The text.
        text: String,
    },
    /// Enter.
    NewLine,
    /// Tab.
    Tab,
    /// Shift+Tab.
    BackTab,
    /// Backspace.
    Backspace,
    /// Delete.
    Delete,
    /// Delete the lines of the selection.
    DeleteLine,
    /// Delete to column 0.
    DeleteToLineStart,
    /// Delete to the line end.
    DeleteToLineEnd,
    /// Delete the word at the caret (between words: the run of blanks).
    DeleteWord,
    /// Delete to the start of the word.
    DeleteToWordStart,
    /// Delete to the end of the word.
    DeleteToWordEnd,
    /// An empty line before the caret's line.
    InsertLineBefore,
    /// An empty line after the caret's line.
    InsertLineAfter,
    /// One indent unit more on each line of the selection (blank lines excepted).
    IncreaseIndent,
    /// One indent unit less on each line of the selection.
    DecreaseIndent,
}

/// The request and the selection after it for `cmd` at `sel`; `None` when the command does nothing there
/// (Backspace at the start of the document, an indent decrease without indent, …).
pub fn plan_command(
    doc: &EditableDocument,
    sel: Selection,
    cmd: &EditCommand,
) -> Option<(EditRequest, Selection)> {
    let sel = Selection {
        anchor: doc.clamp(sel.anchor),
        caret: doc.clamp(sel.caret),
    };
    let range = sel.range();
    let caret = sel.caret;
    let opts = doc.options();
    match cmd {
        EditCommand::Type { text, overwrite } => {
            if text.is_empty() {
                return None;
            }
            let mut range = range;
            if *overwrite && sel.is_empty() {
                let line = doc.line(caret.line);
                let typed = text.graphemes(true).count();
                let mut end = caret.byte as usize;
                for _ in 0..typed {
                    if end >= line.len() {
                        break;
                    }
                    end = words::next_grapheme(&line, end);
                }
                range = caret..TextPos::new(caret.line.0, end as u32);
            }
            Some(replace(doc, range, text))
        }
        EditCommand::Paste { text } => Some(replace(doc, range, text)),
        EditCommand::NewLine => {
            let mut text = doc.insert_eol().as_text().to_owned();
            if opts.auto_indent {
                let line = doc.line(range.start.line);
                let indent = words::indent_len(&line).min(range.start.byte as usize);
                text.push_str(&line[..indent]);
            }
            Some(replace(doc, range, &text))
        }
        EditCommand::Tab => {
            if range.start.line != range.end.line {
                return indent(doc, sel, true);
            }
            let text = if opts.tab_as_spaces {
                let line = doc.line(range.start.line);
                let ts = u32::from(opts.tab_size.max(1));
                let col = words::column(&line, range.start.byte as usize, opts.tab_size);
                " ".repeat((ts - col % ts) as usize)
            } else {
                "\t".to_owned()
            };
            Some(replace(doc, range, &text))
        }
        EditCommand::BackTab | EditCommand::DecreaseIndent => indent(doc, sel, false),
        EditCommand::IncreaseIndent => indent(doc, sel, true),
        EditCommand::Backspace => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let line = doc.line(caret.line);
            let b = caret.byte as usize;
            if b == 0 {
                if caret.line.0 == 0 {
                    return None;
                }
                let prev = LineIdx(caret.line.0 - 1);
                let prev_len = doc.line_len(prev);
                if doc.line_ending(prev) == LineEnding::Artificial {
                    // The piece before an artificial break: its last grapheme.
                    let text = doc.line(prev);
                    let start = words::prev_grapheme(&text, text.len()) as u32;
                    return Some(delete(
                        TextPos::new(prev.0, start)..TextPos::new(prev.0, prev_len),
                    ));
                }
                return Some(delete(TextPos::new(prev.0, prev_len)..caret));
            }
            if opts.backspace_unindents && line[..b].chars().all(words::is_blank) {
                let ts = u32::from(opts.tab_size.max(1));
                let col = words::column(&line, b, opts.tab_size);
                let target = (col - 1) / ts * ts;
                let mut start = b;
                while start > 0 && words::column(&line, start, opts.tab_size) > target {
                    start -= 1;
                }
                return Some(delete(TextPos::new(caret.line.0, start as u32)..caret));
            }
            let start = words::prev_grapheme(&line, b) as u32;
            Some(delete(TextPos::new(caret.line.0, start)..caret))
        }
        EditCommand::Delete => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let line = doc.line(caret.line);
            let b = caret.byte as usize;
            if b < line.len() {
                let end = words::next_grapheme(&line, b) as u32;
                return Some(delete(caret..TextPos::new(caret.line.0, end)));
            }
            let next = caret.line.0 + 1;
            if next >= doc.position_lines() {
                return None;
            }
            if doc.line_ending(caret.line) == LineEnding::Artificial {
                let text = doc.line(LineIdx(next));
                let end = words::next_grapheme(&text, 0) as u32;
                return Some(delete(TextPos::new(next, 0)..TextPos::new(next, end)));
            }
            Some(delete(caret..TextPos::new(next, 0)))
        }
        EditCommand::DeleteLine => {
            let n = doc.line_count();
            if n == 0 {
                return None;
            }
            let first = doc.phys_start(range.start.line.0.min(n - 1));
            let mut last = range.end.line.0;
            if range.end.byte == 0 && range.end.line > range.start.line {
                last -= 1;
            }
            let end = doc.phys_end(last.min(n - 1));
            let from = TextPos::new(first, 0);
            if end < n {
                // The lines with their terminators.
                Some((
                    delete_req(from..TextPos::new(end, 0)),
                    Selection::caret(from),
                ))
            } else if first > 0 {
                // Through the last line: the terminator before the lines goes with them.
                let prev = first - 1;
                let from = TextPos::new(prev, doc.line_len(LineIdx(prev)));
                let to = TextPos::new(n - 1, doc.line_len(LineIdx(n - 1)));
                let caret = TextPos::new(doc.phys_start(prev), 0);
                Some((delete_req(from..to), Selection::caret(caret)))
            } else {
                // Every line.
                let to = if doc.position_lines() > n {
                    TextPos::new(n, 0)
                } else {
                    TextPos::new(n - 1, doc.line_len(LineIdx(n - 1)))
                };
                Some((delete_req(from..to), Selection::caret(from)))
            }
        }
        EditCommand::DeleteToLineStart => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            (caret.byte > 0).then(|| delete(TextPos::new(caret.line.0, 0)..caret))
        }
        EditCommand::DeleteToLineEnd => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let len = doc.line_len(caret.line);
            (caret.byte < len).then(|| delete(caret..TextPos::new(caret.line.0, len)))
        }
        EditCommand::DeleteWord => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let r = word_at(doc, caret);
            (r.start != r.end).then(|| delete(r))
        }
        EditCommand::DeleteToWordStart => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let start = word_boundary(doc, caret, Dir::Backward);
            (start != caret).then(|| delete(start..caret))
        }
        EditCommand::DeleteToWordEnd => {
            if !sel.is_empty() {
                return Some(delete(range));
            }
            let end = word_boundary(doc, caret, Dir::Forward);
            (end != caret).then(|| delete(caret..end))
        }
        EditCommand::InsertLineBefore | EditCommand::InsertLineAfter => {
            let line_idx = caret.line.0;
            let text = doc.line(caret.line);
            let indent = if opts.auto_indent {
                text[..words::indent_len(&text)].to_owned()
            } else {
                String::new()
            };
            let eol = doc.insert_eol().as_text();
            if matches!(cmd, EditCommand::InsertLineBefore) {
                let at = TextPos::new(doc.phys_start(line_idx), 0);
                let new = format!("{indent}{eol}");
                let caret = TextPos::new(at.line.0, indent.len() as u32);
                Some((insert_req(at, new), Selection::caret(caret)))
            } else {
                let end = if line_idx < doc.line_count() {
                    doc.phys_end(line_idx) - 1
                } else {
                    line_idx
                };
                let at = TextPos::new(end, doc.line_len(LineIdx(end)));
                let new = format!("{eol}{indent}");
                let caret = TextPos::new(end + 1, indent.len() as u32);
                Some((insert_req(at, new), Selection::caret(caret)))
            }
        }
    }
}

/// `Replace` of `range` with `text`, the caret after the text (no CPL re-wrap assumed).
fn replace(doc: &EditableDocument, range: Range<TextPos>, text: &str) -> (EditRequest, Selection) {
    let caret = after_text(range.start, text);
    let _ = doc;
    (
        EditRequest::Replace {
            range,
            text: text.to_owned(),
            keep_eols: false,
        },
        Selection::caret(caret),
    )
}

/// The position after `text` inserted at `at` (every break makes one line).
fn after_text(at: TextPos, text: &str) -> TextPos {
    let mut line = at.line.0;
    let mut byte = at.byte;
    let mut chars = text.char_indices().peekable();
    let mut seg_start = 0usize;
    while let Some((_, c)) = chars.next() {
        if c == '\r' || c == '\n' {
            if c == '\r' && chars.peek().map(|&(_, n)| n) == Some('\n') {
                chars.next();
            }
            line += 1;
            byte = 0;
            seg_start = chars.peek().map_or(text.len(), |&(k, _)| k);
        }
    }
    byte += (text.len() - seg_start) as u32;
    TextPos::new(line, byte)
}

/// Deletion of `range`, the caret at its start.
fn delete(range: Range<TextPos>) -> (EditRequest, Selection) {
    let caret = range.start;
    (delete_req(range), Selection::caret(caret))
}

fn delete_req(range: Range<TextPos>) -> EditRequest {
    EditRequest::Replace {
        range,
        text: String::new(),
        keep_eols: false,
    }
}

fn insert_req(at: TextPos, text: String) -> EditRequest {
    EditRequest::Replace {
        range: at..at,
        text,
        keep_eols: false,
    }
}

/// One indent unit more (`increase`) or less on every line of the selection (the end line excluded when the
/// selection ends at its column 0); blank lines are not indented.
fn indent(
    doc: &EditableDocument,
    sel: Selection,
    increase: bool,
) -> Option<(EditRequest, Selection)> {
    let opts = doc.options();
    let range = sel.range();
    let first = range.start.line.0;
    let mut last = range.end.line.0;
    if range.end.byte == 0 && last > first {
        last -= 1;
    }
    let last = last.min(doc.line_count().saturating_sub(1));
    let ts = usize::from(opts.tab_size.max(1));
    let unit = if opts.tab_as_spaces {
        " ".repeat(ts)
    } else {
        "\t".to_owned()
    };
    let mut edits = Vec::new();
    // Bytes added (positive) or removed at the start of each changed line.
    let mut shifts: Vec<(u32, i64)> = Vec::new();
    for l in first..=last {
        if l >= doc.line_count() {
            break;
        }
        // Only the first piece of a physical line holds its indent.
        if l > 0 && doc.line_ending(LineIdx(l - 1)) == LineEnding::Artificial {
            continue;
        }
        let text = doc.line(LineIdx(l));
        if increase {
            if text.trim_matches(words::is_blank).is_empty() {
                continue;
            }
            let at = TextPos::new(l, 0);
            edits.push((at..at, unit.clone()));
            shifts.push((l, unit.len() as i64));
        } else {
            let bytes = text.as_bytes();
            let take = if bytes.first() == Some(&b'\t') {
                1
            } else {
                let spaces = bytes.iter().take_while(|&&b| b == b' ').count();
                if spaces < ts && bytes.get(spaces) == Some(&b'\t') {
                    spaces
                } else {
                    spaces.min(ts)
                }
            };
            if take == 0 {
                continue;
            }
            edits.push((
                TextPos::new(l, 0)..TextPos::new(l, take as u32),
                String::new(),
            ));
            shifts.push((l, -(take as i64)));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let shift = |p: TextPos| -> TextPos {
        match shifts.iter().find(|(l, _)| *l == p.line.0) {
            Some(&(_, d)) if d > 0 && p.byte > 0 => TextPos::new(p.line.0, p.byte + d as u32),
            Some(&(_, d)) if d < 0 => TextPos::new(p.line.0, (i64::from(p.byte) + d).max(0) as u32),
            _ => p,
        }
    };
    let after = Selection {
        anchor: shift(sel.anchor),
        caret: shift(sel.caret),
    };
    Some((
        EditRequest::Batch {
            edits,
            keep_eols: false,
        },
        after,
    ))
}

/// The word at `pos` (double-click; *Initialize find with the current word*): the run of word characters,
/// blanks, or one other character at `pos` (the run before `pos` when `pos` ends a word or the line).
pub fn word_at(doc: &EditableDocument, pos: TextPos) -> Range<TextPos> {
    let pos = doc.clamp(pos);
    let line = doc.line(pos.line);
    let (s, e) = words::run_at(&line, pos.byte as usize, &doc.options().word_chars);
    TextPos::new(pos.line.0, s as u32)..TextPos::new(pos.line.0, e as u32)
}

/// The next word boundary from `pos` (Ctrl+Right: past the word and the blanks after it; Ctrl+Left: back over
/// blanks and the word before). At a line end (start) it moves to the next line's start (previous line's
/// end).
pub fn word_boundary(doc: &EditableDocument, pos: TextPos, dir: Dir) -> TextPos {
    let pos = doc.clamp(pos);
    let extra = doc.options().word_chars.as_str();
    let line = doc.line(pos.line);
    let b = pos.byte as usize;
    match dir {
        Direction::Forward => {
            if b >= line.len() {
                let next = pos.line.0 + 1;
                return if next < doc.position_lines() {
                    TextPos::new(next, 0)
                } else {
                    pos
                };
            }
            let mut it = line[b..].char_indices().peekable();
            let mut end = b;
            if let Some(&(_, c)) = it.peek() {
                match words::class(c, extra) {
                    Class::Other => {
                        it.next();
                        end = b + c.len_utf8();
                    }
                    Class::Word => {
                        while let Some(&(k, c)) = it.peek() {
                            if words::class(c, extra) != Class::Word {
                                break;
                            }
                            it.next();
                            end = b + k + c.len_utf8();
                        }
                    }
                    Class::Blank => {}
                }
            }
            while let Some(&(k, c)) = it.peek() {
                if !words::is_blank(c) {
                    break;
                }
                it.next();
                end = b + k + c.len_utf8();
            }
            TextPos::new(pos.line.0, end as u32)
        }
        Direction::Backward => {
            if b == 0 {
                return if pos.line.0 > 0 {
                    let prev = LineIdx(pos.line.0 - 1);
                    TextPos::new(prev.0, doc.line_len(prev))
                } else {
                    pos
                };
            }
            let mut start = b;
            let mut chars = line[..b].chars().rev().peekable();
            while let Some(&c) = chars.peek() {
                if !words::is_blank(c) {
                    break;
                }
                start -= c.len_utf8();
                chars.next();
            }
            if let Some(&c) = chars.peek() {
                let class = words::class(c, extra);
                if class == Class::Other {
                    start -= c.len_utf8();
                } else {
                    for c in chars {
                        if words::class(c, extra) != class {
                            break;
                        }
                        start -= c.len_utf8();
                    }
                }
            }
            TextPos::new(pos.line.0, start as u32)
        }
    }
}

/// Smart Home: the first non-blank character of the line, or column 0 when already there (or the line is
/// blank).
pub fn line_home(doc: &EditableDocument, pos: TextPos) -> TextPos {
    let pos = doc.clamp(pos);
    let line = doc.line(pos.line);
    let first = words::indent_len(&line) as u32;
    if pos.byte == first || first as usize == line.len() {
        TextPos::new(pos.line.0, 0)
    } else {
        TextPos::new(pos.line.0, first)
    }
}

/// The lines of `side` in the section holding `line` (Ctrl+D), from the start of its first line to the start
/// of the line after it (the end of the document's last line when there is none); `None` outside a section or
/// when the section has no line on `side`.
pub fn select_section(cmp: &TextComparison, side: Side, line: LineIdx) -> Option<Range<TextPos>> {
    let row = cmp.row_for_line(side, line)?;
    let id = cmp.section_at(row)?;
    let section = cmp.sections().get(id.0 as usize)?;
    let lines = match side {
        Side::Right => section.right_lines.clone(),
        _ => section.left_lines.clone(),
    };
    if lines.is_empty() {
        return None;
    }
    let doc = cmp.document(side);
    Some(TextPos::new(lines.start, 0)..end_of(doc, lines.end))
}

/// The start of line `line`, or the end of the last line when `line` is past it and the last line has no
/// terminator.
fn end_of(doc: &TextDocument, line: u32) -> TextPos {
    let n = doc.line_count();
    if line >= n && n > 0 && doc.line_ending(LineIdx(n - 1)) == LineEnding::None {
        return TextPos::new(n - 1, doc.line_len(n - 1) as u32);
    }
    TextPos::new(line, 0)
}

/// The edit that copies rows to the other pane (TC-062).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyEdit {
    /// The pane the request applies to (the other side of `from`).
    pub target: Side,
    /// A `Replace` with `keep_eols: true`.
    pub request: EditRequest,
    /// The selection after it: the copied lines in the target.
    pub selection: Selection,
}

/// The edit that makes rows `rows` equal to `from`'s lines (Copy to Right/Left, Copy Line, §1.10.6): the target
/// lines those rows cover are replaced by the source lines in row order; rows without a target line insert
/// after the target line of the last row above them (or at the start). The source lines are read from the
/// comparison's documents (a live comparison keeps them equal to the panes). A terminator the copy has to add
/// (after the source's open last line, or after the target's) is `insert_eol`, the target pane's
/// [`crate::edit::EditableDocument::insert_eol`] (D-290). `None` for an empty row range.
pub fn copy_rows(
    cmp: &TextComparison,
    rows: Range<u32>,
    from: Side,
    insert_eol: LineEnding,
) -> Option<CopyEdit> {
    let rows = rows.start..rows.end.min(cmp.row_count());
    if rows.is_empty() {
        return None;
    }
    let target = if from == Side::Right {
        Side::Left
    } else {
        Side::Right
    };
    let line_of = |r: u32, side: Side| -> Option<u32> {
        let row = cmp.row(RowIdx(r))?;
        match side {
            Side::Right => row.right.map(|l| l.0),
            _ => row.left.map(|l| l.0),
        }
    };
    let src = cmp.document(from);
    let dst = cmp.document(target);
    let mut text = String::new();
    let mut last_src_eol = LineEnding::Lf;
    let mut any_src = false;
    let (mut t0, mut t1): (Option<u32>, u32) = (None, 0);
    for r in rows.clone() {
        if let Some(l) = line_of(r, from) {
            text.push_str(&src.line(LineIdx(l)));
            last_src_eol = src.line_ending(LineIdx(l));
            if last_src_eol != LineEnding::Artificial {
                text.push_str(last_src_eol.as_text());
            }
            any_src = true;
        }
        if let Some(l) = line_of(r, target) {
            t0.get_or_insert(l);
            t1 = l + 1;
        }
    }
    let n = dst.line_count();
    let dominant = match insert_eol {
        LineEnding::None | LineEnding::Artificial => LineEnding::Lf,
        e => e,
    };
    let (start, end) = match t0 {
        Some(t0) => (t0, t1),
        None => {
            let above = (0..rows.start).rev().find_map(|r| line_of(r, target));
            let at = above.map_or(0, |l| l + 1);
            (at, at)
        }
    };
    let dst_last_open = n > 0 && dst.line_ending(LineIdx(n - 1)) == LineEnding::None;
    let mut range_start = TextPos::new(start, 0);
    let mut range_end = TextPos::new(end, 0);
    if end >= n && dst_last_open {
        // Up to the end of the open last line.
        range_end = TextPos::new(n - 1, dst.line_len(n - 1) as u32);
        if start >= n {
            // Inserting after the open last line: a terminator first, the copied lines without their last one.
            range_start = range_end;
            let mut joined = String::from(dominant.as_text());
            joined.push_str(text.strip_suffix(last_src_eol.as_text()).unwrap_or(&text));
            text = joined;
        } else if last_src_eol != LineEnding::None && any_src {
            // The open last line is replaced: keep it open.
            if let Some(stripped) = text.strip_suffix(last_src_eol.as_text()) {
                text = stripped.to_owned();
            }
        }
    } else if any_src && last_src_eol == LineEnding::None && end < n {
        // The source's open last line goes before more target lines: it needs a terminator.
        text.push_str(dominant.as_text());
    }
    let lines_after = after_text(range_start, &text);
    Some(CopyEdit {
        target,
        request: EditRequest::Replace {
            range: range_start..range_end,
            text,
            keep_eols: true,
        },
        selection: Selection {
            anchor: range_start,
            caret: lines_after,
        },
    })
}
