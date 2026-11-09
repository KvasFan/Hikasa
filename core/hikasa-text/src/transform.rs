//! Conversions (TC-063; D-279, contract §1.10.15): trim trailing white space, leading spaces to tabs, tabs to
//! spaces and line endings, each turned into **one** [`EditRequest`] (one undo step) and the number of lines it
//! changes (the status *Converted N lines*).
//!
//! ```text
//! transform(doc, t, lines, tab_size)
//!   → the physical lines holding `lines` (CPL pieces taken whole, TC-003)
//!   → per physical line: the normative rule of §1.10.15 on its content (or its terminator)
//!   → TransformOutcome { request: Batch of the changed runs | SetEols | None, lines: physical lines changed }
//! ```
//!
//! Pure and in memory: the view applies the request with [`EditableDocument::apply`]; `hikasa text convert`
//! (D-280) and the save options of a format (§1.10.12) call the same function, so a file converted on the
//! command line equals the same conversion in a view.
//!
//! ```
//! use std::sync::Arc;
//! use hikasa_text::edit::{EditOptions, Editability, EditableDocument, GroupHint};
//! use hikasa_text::transform::{Transform, transform};
//! use hikasa_text::TextDocument;
//!
//! let mut doc = EditableDocument::new(
//!     Arc::new(TextDocument::from_text("a  \n\tb\nc\n")),
//!     EditOptions::default(),
//!     Editability::EDITABLE,
//! );
//! let out = transform(&doc, Transform::TrimTrailing, None, 4);
//! assert_eq!(out.lines, 1);
//! doc.apply(&out.request.unwrap(), GroupHint::None).unwrap();
//! assert_eq!(doc.document().rope().to_string(), "a\n\tb\nc\n");
//! // Applied twice equals once (P-CONVERT).
//! assert_eq!(transform(&doc, Transform::TrimTrailing, None, 4).request, None);
//! ```

use std::borrow::Cow;
use std::ops::Range;

use crate::document::{LineEnding, LineIdx};
use crate::edit::{EditRequest, EditableDocument, TextPos};

/// A conversion (TC-063).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transform {
    /// Removes the trailing run of Unicode `White_Space` characters of each line (space, tab, NBSP, …).
    TrimTrailing,
    /// In the leading blanks, each run of spaces that reaches a tab stop becomes one tab.
    LeadingSpacesToTabs,
    /// Every tab becomes spaces up to the next tab stop of its display column.
    TabsToSpaces,
    /// Every terminated line's terminator becomes the given one (`Lf`, `CrLf` or `Cr`; the last line keeps *no
    /// terminator*); a conversion of every line also makes it the insert terminator.
    LineEndings(LineEnding),
}

/// What a conversion would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransformOutcome {
    /// The edit (one undo step); `None`: nothing to do.
    pub request: Option<EditRequest>,
    /// The lines it changes (physical lines: a line wrapped by the characters-per-line limit counts once), the
    /// *Converted N lines* of the status (TC-063).
    pub lines: u64,
}

impl TransformOutcome {
    const NOTHING: TransformOutcome = TransformOutcome {
        request: None,
        lines: 0,
    };
}

/// The conversion `t` of the lines `lines` of `doc` (`None`: every line; a range is a selection's lines, clipped
/// to the document, every physical line it touches converted whole). Tab stops are every `tab_size` display
/// columns (0 counts as 1), counted from the start of the physical line: a tab advances to the next stop, a wide
/// character (East Asian Wide or Fullwidth) takes 2 columns, a combining mark or a control character 0, every
/// other character 1 (the editor's column rule, [`crate::edit::commands`]).
///
/// The content conversions give one [`EditRequest::Batch`] (texts kept as given) with one edit per run of
/// consecutive changed lines, so every unchanged line keeps its marks and bytes. `LineEndings(e)` gives one
/// [`EditRequest::SetEols`]; over every terminated line it also sets [`EditableDocument::insert_eol`] to `e`,
/// even when no terminator changes (`lines` 0 then). A range conversion to `Cr` (or to `Lf`) also converts the
/// empty `Lf` line after it (the line ending with a lone CR before it) when the two would otherwise read as one
/// CRLF line, so no line is lost. `LineEndings(None)` and `LineEndings(Artificial)` do nothing.
pub fn transform(
    doc: &EditableDocument,
    t: Transform,
    lines: Option<Range<u32>>,
    tab_size: u8,
) -> TransformOutcome {
    let n = doc.line_count();
    let range = match lines {
        None => 0..n,
        Some(r) => r.start.min(n)..r.end.min(n),
    };
    if range.start >= range.end {
        // An empty document: only the insert terminator can change.
        return match t {
            Transform::LineEndings(e) if n == 0 => line_endings(doc, 0..0, e),
            _ => TransformOutcome::NOTHING,
        };
    }
    let first = doc.phys_start(range.start);
    let end = doc.phys_end(range.end - 1);
    let ts = u32::from(tab_size.max(1));
    match t {
        Transform::TrimTrailing => content(doc, first..end, trim_trailing),
        Transform::LeadingSpacesToTabs => {
            content(doc, first..end, |line| leading_spaces_to_tabs(line, ts))
        }
        Transform::TabsToSpaces => content(doc, first..end, |line| tabs_to_spaces(line, ts)),
        Transform::LineEndings(e) => line_endings(doc, first..end, e),
    }
}

/// The trailing `White_Space` run removed.
pub(crate) fn trim_trailing(line: &str) -> Option<String> {
    let trimmed = line.trim_end_matches(char::is_whitespace);
    (trimmed.len() != line.len()).then(|| trimmed.to_owned())
}

/// The display width of a character other than a tab.
fn width(c: char) -> u32 {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u32
}

/// Leading runs of spaces reaching a tab stop become tabs.
pub(crate) fn leading_spaces_to_tabs(line: &str, ts: u32) -> Option<String> {
    let blanks = line.len() - line.trim_start_matches([' ', '\t']).len();
    let lead = &line[..blanks];
    if !lead.contains(' ') {
        return None;
    }
    let mut out = String::with_capacity(line.len());
    let mut col = 0u32;
    // Spaces of the current run (from a stop or the last tab).
    let mut run = 0usize;
    for c in lead.chars() {
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', run));
            run = 0;
            col = (col / ts + 1) * ts;
            out.push('\t');
        } else {
            col += 1;
            run += 1;
            if col.is_multiple_of(ts) {
                out.push('\t');
                run = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n(' ', run));
    if out == lead {
        return None;
    }
    out.push_str(&line[blanks..]);
    Some(out)
}

/// Tabs expanded to the next stop of their display column.
fn tabs_to_spaces(line: &str, ts: u32) -> Option<String> {
    if !line.contains('\t') {
        return None;
    }
    let mut out = String::with_capacity(line.len() + 8);
    let mut col = 0u32;
    for c in line.chars() {
        if c == '\t' {
            let next = (col / ts + 1) * ts;
            out.extend(std::iter::repeat_n(' ', (next - col) as usize));
            col = next;
        } else {
            col += width(c);
            out.push(c);
        }
    }
    Some(out)
}

/// One physical line: its pieces `[first, end)`, its content and its terminator (of the last piece).
struct Phys<'a> {
    first: u32,
    end: u32,
    content: Cow<'a, str>,
    eol: LineEnding,
}

/// The physical line starting at table line `p`.
fn phys(doc: &EditableDocument, p: u32) -> Phys<'_> {
    let end = doc.phys_end(p);
    let content = if end == p + 1 {
        doc.line(LineIdx(p))
    } else {
        let mut s = String::new();
        for l in p..end {
            s.push_str(&doc.line(LineIdx(l)));
        }
        Cow::Owned(s)
    };
    Phys {
        first: p,
        end,
        content,
        eol: doc.line_ending(LineIdx(end - 1)),
    }
}

/// The position of byte `b` of a physical line's content (at a piece boundary: the end of the earlier piece).
fn pos_in(doc: &EditableDocument, line: &Phys<'_>, b: usize) -> TextPos {
    let mut rest = b as u32;
    for l in line.first..line.end {
        let len = doc.line_len(LineIdx(l));
        if rest <= len || l + 1 == line.end {
            return TextPos::new(l, rest.min(len));
        }
        rest -= len;
    }
    TextPos::new(line.first, 0)
}

/// The common prefix and suffix of `old` and `new` in bytes, at char boundaries of both (they do not overlap).
fn common_ends(old: &str, new: &str) -> (usize, usize) {
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
    (p, s)
}

/// A run of consecutive changed physical lines being built into one edit.
struct Run {
    /// Where the edit starts.
    start: TextPos,
    /// Where it ends so far (after the last changed byte of the run's last line).
    stop: TextPos,
    /// The replacement of `[start, stop)`.
    text: String,
    /// The bytes from `stop` to the next line (the last line's unchanged end and its terminator): part of the
    /// edit only when the next line changes too.
    tail: String,
}

/// A content conversion of the physical lines starting in `span`.
pub(crate) fn content(
    doc: &EditableDocument,
    span: Range<u32>,
    rule: impl Fn(&str) -> Option<String>,
) -> TransformOutcome {
    let mut edits: Vec<(Range<TextPos>, String)> = Vec::new();
    let mut run: Option<Run> = None;
    let mut changed = 0u64;
    let mut p = span.start;
    while p < span.end {
        let line = phys(doc, p);
        p = line.end;
        let Some(new) = rule(&line.content) else {
            if let Some(r) = run.take() {
                edits.push((r.start..r.stop, r.text));
            }
            continue;
        };
        changed += 1;
        let (pre, suf) = common_ends(&line.content, &new);
        let stop = pos_in(doc, &line, line.content.len() - suf);
        let tail = format!("{}{}", &new[new.len() - suf..], line.eol.as_text());
        match run.as_mut() {
            // The previous physical line changed too: one edit over both, its tail kept as it was.
            Some(r) => {
                let kept = std::mem::take(&mut r.tail);
                r.text.push_str(&kept);
                r.text.push_str(&new[..new.len() - suf]);
                r.stop = stop;
                r.tail = tail;
            }
            None => {
                run = Some(Run {
                    start: pos_in(doc, &line, pre),
                    stop,
                    text: new[pre..new.len() - suf].to_owned(),
                    tail,
                });
            }
        }
    }
    if let Some(r) = run.take() {
        edits.push((r.start..r.stop, r.text));
    }
    if edits.is_empty() {
        return TransformOutcome::NOTHING;
    }
    TransformOutcome {
        request: Some(EditRequest::Batch {
            edits,
            keep_eols: true,
        }),
        lines: changed,
    }
}

/// `LineEndings(e)` over the physical lines starting in `span` (empty: a document without lines).
fn line_endings(doc: &EditableDocument, span: Range<u32>, e: LineEnding) -> TransformOutcome {
    if !matches!(e, LineEnding::Lf | LineEnding::CrLf | LineEnding::Cr) {
        return TransformOutcome::NOTHING;
    }
    let n = doc.line_count();
    let (mut first, mut end) = (span.start, span.end);
    // A neighbour that would join a converted line into one CRLF line converts with it: an empty LF line after
    // a line becoming CR, a line ending with a lone CR before an empty line becoming LF.
    if e == LineEnding::Cr {
        while end < n
            && doc.line_len(LineIdx(end)) == 0
            && doc.line_ending(LineIdx(end)) == LineEnding::Lf
        {
            end += 1;
        }
    }
    if e == LineEnding::Lf {
        while first > 0
            && first < n
            && doc.line_len(LineIdx(first)) == 0
            && doc.line_ending(LineIdx(first)) != LineEnding::None
            && doc.line_ending(LineIdx(first - 1)) == LineEnding::Cr
        {
            first = doc.phys_start(first - 1);
        }
    }
    let whole = first == 0 && end == n;
    let mut changed = 0u64;
    let mut p = first;
    while p < end {
        let q = doc.phys_end(p);
        let eol = doc.line_ending(LineIdx(q - 1));
        if eol != LineEnding::None && eol != e {
            changed += 1;
        }
        p = q;
    }
    // The last line keeps *no terminator*.
    let stop = if end == n && n > 0 && doc.line_ending(LineIdx(n - 1)) == LineEnding::None {
        n - 1
    } else {
        end
    };
    if changed == 0 && !(whole && doc.insert_eol() != e) {
        return TransformOutcome::NOTHING;
    }
    TransformOutcome {
        request: Some(EditRequest::SetEols {
            lines: first.min(stop)..stop,
            eol: e,
        }),
        lines: changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_rules() {
        assert_eq!(trim_trailing("a \t\u{a0}"), Some("a".into()));
        assert_eq!(trim_trailing("a"), None);
        assert_eq!(leading_spaces_to_tabs("        x", 4), Some("\t\tx".into()));
        assert_eq!(leading_spaces_to_tabs("      x", 4), Some("\t  x".into()));
        assert_eq!(leading_spaces_to_tabs("  \tx", 4), None);
        assert_eq!(
            leading_spaces_to_tabs("\t    x  ", 4),
            Some("\t\tx  ".into())
        );
        assert_eq!(leading_spaces_to_tabs("x    y", 4), None);
        assert_eq!(tabs_to_spaces("a\tb", 4), Some("a   b".into()));
        assert_eq!(tabs_to_spaces("界\tb", 4), Some("界  b".into()));
        assert_eq!(
            tabs_to_spaces("e\u{301}\tb", 4),
            Some("e\u{301}   b".into())
        );
        assert_eq!(tabs_to_spaces("ab", 4), None);
    }
}
