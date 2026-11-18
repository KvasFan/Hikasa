//! Caret steps and line sets (TC-060; M2 W11a, contract §1.11.4, D-295): pure helpers the editor's views call so that
//! a grapheme step, a document end, the text of a set of lines and the edit that deletes them are text rules of the
//! core, never of a view (T11, architecture check 7).
//!
//! | Call | What it gives |
//! |---|---|
//! | [`next_pos`], [`prev_pos`] | one grapheme cluster on or back; a line end steps to the next line's start (an artificial break of a characters-per-line limit, TC-003, is no character: the step goes past the next piece's first cluster) |
//! | [`doc_end`] | the last position of the document |
//! | [`cluster_starts`], [`char_width`] | a line's grapheme cluster boundaries and a character's display width (a view's word wrap and its test metrics) |
//! | [`LineSet`], [`lines_text`] | the text of a set of lines, each whole physical line with its stored terminator (Copy of a line-set selection, D-307) |
//! | [`delete_lines`] | one `Batch` deleting those lines with their terminators (the last line takes the terminator before it, as `DeleteLine`) |

use std::ops::Range;

use super::{EditRequest, EditableDocument, TextPos, words};
use crate::compare::RangeSet;
use crate::document::{LineEnding, LineIdx};

/// The position one grapheme cluster after `p` (`p` clamped first); a line end goes to the next line's start, the
/// document's end stays where it is.
pub fn next_pos(doc: &EditableDocument, p: TextPos) -> TextPos {
    let p = doc.clamp(p);
    let line = doc.line(p.line);
    let len = line.len();
    if (p.byte as usize) < len {
        return TextPos::new(
            p.line.0,
            words::next_grapheme(&line, p.byte as usize) as u32,
        );
    }
    let next = p.line.0 + 1;
    if next >= doc.position_lines() {
        return p;
    }
    if doc.line_ending(p.line) == LineEnding::Artificial {
        // The end of a piece and the start of the next one are the same byte: step over its first cluster.
        let text = doc.line(LineIdx(next));
        let byte = if text.is_empty() {
            0
        } else {
            words::next_grapheme(&text, 0)
        };
        return TextPos::new(next, byte as u32);
    }
    TextPos::new(next, 0)
}

/// The position one grapheme cluster before `p` (`p` clamped first); a line start goes to the previous line's end,
/// the document's start stays where it is.
pub fn prev_pos(doc: &EditableDocument, p: TextPos) -> TextPos {
    let p = doc.clamp(p);
    if p.byte > 0 {
        let line = doc.line(p.line);
        let byte = words::prev_grapheme(&line, p.byte as usize) as u32;
        if byte == 0
            && p.line.0 > 0
            && doc.line_ending(LineIdx(p.line.0 - 1)) == LineEnding::Artificial
        {
            // A piece's start is the end of the piece before it: one place, written as that end.
            let prev = LineIdx(p.line.0 - 1);
            return TextPos::new(prev.0, doc.line_len(prev));
        }
        return TextPos::new(p.line.0, byte);
    }
    if p.line.0 == 0 {
        return p;
    }
    let prev = LineIdx(p.line.0 - 1);
    let len = doc.line_len(prev);
    if doc.line_ending(prev) == LineEnding::Artificial && len > 0 {
        let text = doc.line(prev);
        return TextPos::new(prev.0, words::prev_grapheme(&text, len as usize) as u32);
    }
    TextPos::new(prev.0, len)
}

/// The last position of the document: the end of its last position line (the empty line after a terminated last
/// line, `(0, 0)` for an empty document).
pub fn doc_end(doc: &EditableDocument) -> TextPos {
    let last = doc.position_lines().saturating_sub(1);
    TextPos::new(last, doc.line_len(LineIdx(last)))
}

/// The grapheme cluster boundaries of `line`: the byte offset of every cluster's start, then `line.len()` (a view's
/// word wrap breaks only there, D-295; `[0]` for an empty line).
pub fn cluster_starts(line: &str) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(line.len() + 1);
    let mut at = 0usize;
    while at < line.len() {
        out.push(at as u32);
        at = words::next_grapheme(line, at);
    }
    out.push(line.len() as u32);
    out
}

/// The display width of `c` in columns, as the status bar's display column counts it (`nav::column_of_tabs`): wide
/// characters 2, combining marks and other zero-width characters 0, others 1. A tab counts 1 here (its width
/// depends on its column).
pub fn char_width(c: char) -> u32 {
    if c == '\t' {
        return 1;
    }
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u32
}

/// A set of whole lines (0-based table lines): the line-set selection of *Select All* in the find panel (D-307), the
/// lines a gutter drag selects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineSet(pub RangeSet);

impl LineSet {
    /// The set of `ranges` (merged and sorted).
    pub fn from_ranges(ranges: Vec<Range<u32>>) -> LineSet {
        LineSet(RangeSet::from_ranges(ranges))
    }

    /// True when the set holds no line.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The ranges of `lines` widened to whole physical lines, clipped to the document's lines, merged.
fn physical_ranges(doc: &EditableDocument, lines: &LineSet) -> Vec<Range<u32>> {
    let n = doc.line_count();
    let mut out: Vec<Range<u32>> = Vec::new();
    for r in lines.0.ranges() {
        let (a, b) = (r.start.min(n), r.end.min(n));
        if a >= b {
            continue;
        }
        let range = doc.phys_start(a)..doc.phys_end(b - 1);
        match out.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => out.push(range),
        }
    }
    out
}

/// The text of `lines`: each whole physical line with its stored terminator, in document order (a last line without
/// terminator adds none).
pub fn lines_text(doc: &EditableDocument, lines: &LineSet) -> String {
    let end = doc_end(doc);
    let mut out = String::new();
    for r in physical_ranges(doc, lines) {
        let to = if r.end < doc.position_lines() {
            TextPos::new(r.end, 0)
        } else {
            end
        };
        out.push_str(&doc.text(TextPos::new(r.start, 0)..to));
    }
    out
}

/// The edit that deletes `lines` with their terminators, as one `Batch` (one undo step); a run through the last
/// line takes the terminator before it instead (as `EditCommand::DeleteLine`). `None` when the set names no line of
/// the document.
pub fn delete_lines(doc: &EditableDocument, lines: &LineSet) -> Option<EditRequest> {
    let ranges = physical_ranges(doc, lines);
    if ranges.is_empty() {
        return None;
    }
    let n = doc.line_count();
    let end = doc_end(doc);
    let mut edits: Vec<(Range<TextPos>, String)> = Vec::with_capacity(ranges.len());
    for r in ranges {
        let range = if r.end < n {
            TextPos::new(r.start, 0)..TextPos::new(r.end, 0)
        } else if r.start > 0 {
            // Through the last line: the terminator before the run goes with it.
            let prev = r.start - 1;
            let from = TextPos::new(prev, doc.line_len(LineIdx(prev)));
            let from = match edits.last() {
                // The previous run already ends at that line's start: do not overlap it.
                Some((last, _)) if from < last.end => last.end,
                _ => from,
            };
            from..end
        } else {
            TextPos::ZERO..end
        };
        edits.push((range, String::new()));
    }
    Some(EditRequest::Batch {
        edits,
        keep_eols: true,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::document::TextDocument;
    use crate::edit::{EditOptions, Editability, GroupHint};

    fn doc(text: &str) -> EditableDocument {
        EditableDocument::new(
            Arc::new(TextDocument::from_text(text)),
            EditOptions::default(),
            Editability::default(),
        )
    }

    #[test]
    fn steps_cross_lines_and_clusters() {
        let d = doc("ae\u{301}x\r\nb");
        assert_eq!(next_pos(&d, TextPos::new(0, 1)), TextPos::new(0, 4));
        assert_eq!(prev_pos(&d, TextPos::new(0, 4)), TextPos::new(0, 1));
        assert_eq!(next_pos(&d, TextPos::new(0, 5)), TextPos::new(1, 0));
        assert_eq!(prev_pos(&d, TextPos::new(1, 0)), TextPos::new(0, 5));
        assert_eq!(next_pos(&d, TextPos::new(1, 1)), TextPos::new(1, 1));
        assert_eq!(prev_pos(&d, TextPos::ZERO), TextPos::ZERO);
        assert_eq!(doc_end(&d), TextPos::new(1, 1));
        assert_eq!(doc_end(&doc("a\n")), TextPos::new(1, 0));
        assert_eq!(doc_end(&doc("")), TextPos::ZERO);
        assert_eq!(cluster_starts("ae\u{301}x"), vec![0, 1, 4, 5]);
        assert_eq!(cluster_starts(""), vec![0]);
        assert_eq!(
            (char_width('a'), char_width('中'), char_width('\u{301}')),
            (1, 2, 0)
        );
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a one-range line set is meant as such
    fn line_sets_copy_and_delete_whole_lines() {
        let mut d = doc("a\r\nb\nc\rd");
        let set = LineSet::from_ranges(vec![0..1, 2..4]);
        assert_eq!(lines_text(&d, &set), "a\r\nc\rd");
        let req = delete_lines(&d, &set).expect("a request");
        d.apply(&req, GroupHint::None).expect("applied");
        assert_eq!(d.document().rope().to_string(), "b");
        assert!(delete_lines(&d, &LineSet::from_ranges(vec![5..9])).is_none());
    }
}
