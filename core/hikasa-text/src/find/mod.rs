//! Find and replace (TC-064; D-283; contract §1.10.18): [`FindQuery`], [`Finder`], [`Replacement`],
//! [`replace_one`], [`replace_all`], [`find_in_comparison`]; Find in Files in [`files`].
//!
//! ```text
//! Finder::new(query)   plain text: regex::escape'd; regex: as written; both through hikasa_formats::Pattern
//!                      (regex first, fancy-regex for backreferences and look-around, RegexLimits, T9); whole word
//!                      with a regex: the pattern between two boundary assertions of the format's word characters
//!                      (plain text checks the boundaries itself); a pattern that can only match a line break is
//!                      refused (a search does not cross line ends)
//! scan(document)       windows of about 1 MiB of whole physical lines, copied once from the rope; each physical
//!                      line (the pieces CPL cut, TC-003, joined: an artificial break is no terminator) is searched
//!                      alone, so ^ $ \b see the line; plain text first asks a whole-window prefilter (memmem, or
//!                      the case-folding literal), so lines without a candidate are never searched one by one
//! hits                 non-overlapping, left to right in a line; empty matches skipped (one character on); a
//!                      backtrack limit on a line = no match there + warning regex_runtime_error once per search
//! ```
//!
//! A hit's range is in [`TextPos`]: the table line and byte of its start and of its end (with CPL a hit may start
//! in one piece of a physical line and end in a later one). Every call is pure and in memory; [`Finder`] is `Sync`
//! (Find in Files searches files on a pool with one finder).

pub mod files;

use std::ops::{ControlFlow, Range};
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::error::{Classify, ErrorKind};
use hikasa_core::{Ctl, Pair, Side, Warning, WarningCode};
use hikasa_formats::{Pattern, RegexLimits, Template};
use serde::{Deserialize, Serialize};

use crate::compare::{Direction, RowIdx, TextComparison};
use crate::document::{LineEnding, LineIdx, TextDocument};
use crate::edit::words::is_word;
use crate::edit::{EditRequest, EditableDocument, TextPos};

/// What [`Finder::scan`] calls per hit: the hit, its physical line's text and its byte range there.
pub type HitVisitor<'v> = dyn FnMut(Hit, &str, Range<usize>) -> ControlFlow<()> + 'v;

/// A search direction (`hikasa_text::Direction`).
pub type Dir = Direction;

/// What to find (the find panel's fields, TC-064).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FindQuery {
    /// The text or the pattern.
    pub text: String,
    /// `text` is a regular expression (else plain text).
    pub regex: bool,
    /// Case matters (else Unicode simple case folding).
    pub match_case: bool,
    /// A hit must be a whole word (§1.10.18's boundary rule).
    pub whole_word: bool,
}

/// Why a search or a replacement is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FindError {
    /// An empty query, a pattern or replacement template that does not compile, a pattern that can only match a
    /// line break (`invalid_input`).
    #[error("{0}")]
    InvalidInput(String),
    /// The search was cancelled.
    #[error("operation cancelled")]
    Cancelled,
}

impl Classify for FindError {
    fn kind(&self) -> ErrorKind {
        match self {
            FindError::InvalidInput(_) => ErrorKind::InvalidInput,
            FindError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

impl From<hikasa_core::Cancelled> for FindError {
    fn from(_: hikasa_core::Cancelled) -> FindError {
        FindError::Cancelled
    }
}

/// The sides a comparison's search looks at (the panel's selector).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindSides {
    /// The left pane only.
    Left,
    /// The right pane only.
    Right,
    /// Both, by row, the left side first in a row.
    #[default]
    Both,
}

impl FindSides {
    /// True when `side` is searched.
    pub fn has(self, side: Side) -> bool {
        match self {
            FindSides::Left => side == Side::Left,
            FindSides::Right => side == Side::Right,
            FindSides::Both => matches!(side, Side::Left | Side::Right),
        }
    }
}

/// One hit: its range, and whether the search wrapped to reach it (the panel's *passed the end*).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hit {
    /// The matched text.
    pub range: Range<TextPos>,
    /// The search went past the end (or the start) of the document once to reach it.
    pub wrapped: bool,
}

/// The bytes of one window copied from the rope at a time (§1.10.19's *cancellable within 100 ms*: a check per
/// window).
const WINDOW: usize = 1 << 20;

/// The first window of a search: small, so a hit near the caret costs little; each next window doubles up to
/// [`WINDOW`].
const FIRST_WINDOW: usize = 16 << 10;

/// A compiled query (§1.10.18).
#[derive(Debug)]
pub struct Finder {
    query: FindQuery,
    /// The pattern searched (a whole-word regex wrapped in its boundaries; groups keep their numbers).
    pattern: Pattern,
    /// Plain text with case: the bytes searched over a whole window first.
    literal: Option<memchr::memmem::Finder<'static>>,
    /// Plain text, case-folded: the window is searched with `pattern` first (a literal holds no line break).
    folded_literal: bool,
    /// Plain text and whole word: the boundaries are checked per candidate (one candidate per start).
    check_words: bool,
    word_chars: String,
    /// Lines where the backtrack limit was hit since the last [`Finder::take_warning`].
    runtime: AtomicU64,
}

impl Clone for Finder {
    fn clone(&self) -> Finder {
        Finder {
            query: self.query.clone(),
            pattern: self.pattern.clone(),
            literal: self.literal.clone(),
            folded_literal: self.folded_literal,
            check_words: self.check_words,
            word_chars: self.word_chars.clone(),
            runtime: AtomicU64::new(0),
        }
    }
}

/// The text `refuse` gives a pattern that can only match a line break.
const NO_LINE_BREAKS: &str = "a search does not cross line ends";

impl Finder {
    /// Compiles `q`: plain text escaped, a regex as written, through [`Pattern`] with `limits`; `word_chars` are the
    /// format's extra word characters (D-58). `invalid_input` for an empty query, a pattern that does not compile, a
    /// plain text holding a line break, and a regex that can only match a line break.
    pub fn new(q: &FindQuery, word_chars: &str, limits: &RegexLimits) -> Result<Finder, FindError> {
        if q.text.is_empty() {
            return Err(FindError::InvalidInput("nothing to find".to_owned()));
        }
        let (source, check_words) = if q.regex {
            if only_line_breaks(&q.text) {
                return Err(FindError::InvalidInput(format!(
                    "pattern {:?}: {NO_LINE_BREAKS}",
                    q.text
                )));
            }
            if q.whole_word {
                // `B` = not inside a word: the character before is no word character, or the one after is not
                // (at the start: the line edge or the hit's first character; at the end: its last or the edge).
                let class = word_class(word_chars);
                let b = format!("(?:(?<!{class})|(?!{class}))");
                (format!("{b}(?:{}){b}", q.text), false)
            } else {
                (q.text.clone(), false)
            }
        } else {
            if q.text.contains(['\n', '\r']) {
                return Err(FindError::InvalidInput(format!(
                    "{:?}: {NO_LINE_BREAKS}",
                    q.text
                )));
            }
            (regex::escape(&q.text), q.whole_word)
        };
        let pattern = Pattern::compile(&source, !q.match_case, limits)
            .map_err(|e| FindError::InvalidInput(format!("pattern {:?}: {}", q.text, e.message)))?;
        let literal = (!q.regex && q.match_case)
            .then(|| memchr::memmem::Finder::new(q.text.as_bytes()).into_owned());
        Ok(Finder {
            query: q.clone(),
            pattern,
            literal,
            folded_literal: !q.regex && !q.match_case,
            check_words,
            word_chars: word_chars.to_owned(),
            runtime: AtomicU64::new(0),
        })
    }

    /// The query.
    pub fn query(&self) -> &FindQuery {
        &self.query
    }

    /// The names of the pattern's groups by index (the whole match first): what a replacement may name.
    pub fn group_names(&self) -> Vec<Option<String>> {
        if self.query.regex {
            self.pattern.group_names()
        } else {
            vec![None]
        }
    }

    /// `regex_runtime_error` once when the backtrack limit was hit on a line since the last call (those lines
    /// counted as no match), and resets the count.
    pub fn take_warning(&self) -> Option<Warning> {
        let lines = self.runtime.swap(0, Ordering::Relaxed);
        (lines > 0).then(|| Warning {
            code: WarningCode::RegexRuntimeError,
            message: format!(
                "pattern {:?}: the backtracking limit was hit on {lines} line{}; they count as no match",
                self.query.text,
                if lines == 1 { "" } else { "s" }
            ),
            spec: None,
        })
    }

    /// The first hit in `line` that starts at or after byte `from` (`None` past the end or inside a character).
    pub fn find_in_line(&self, line: &str, from: u32) -> Result<Option<Range<u32>>, FindError> {
        Ok(self
            .first_in(line, from as usize)
            .map(|r| r.start as u32..r.end as u32))
    }

    /// Every hit of `line` from byte `from`, left to right and non-overlapping, until `visit` breaks.
    pub fn hits_in_line(
        &self,
        line: &str,
        from: u32,
        mut visit: impl FnMut(Range<u32>) -> ControlFlow<()>,
    ) {
        let mut at = from as usize;
        while let Some(r) = self.first_in(line, at) {
            at = r.end;
            if visit(r.start as u32..r.end as u32).is_break() {
                return;
            }
        }
    }

    /// The first non-empty hit of `line` at or after `at`.
    fn first_in(&self, line: &str, mut at: usize) -> Option<Range<usize>> {
        loop {
            if at > line.len() || !line.is_char_boundary(at) {
                return None;
            }
            let m = match self.pattern.find_at(line, at) {
                Ok(Some(m)) => m,
                Ok(None) => return None,
                Err(_) => {
                    self.runtime.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
            };
            if m.is_empty() || (self.check_words && !self.word_bounds(line, &m)) {
                at = next_char(line, m.start)?;
                continue;
            }
            return Some(m);
        }
    }

    /// §1.10.18's whole-word rule: each end is checked only when the hit's own character there is a word
    /// character, and then the character outside it (or the line edge) must not be one.
    fn word_bounds(&self, line: &str, m: &Range<usize>) -> bool {
        let w = |c: char| is_word(c, &self.word_chars);
        let first = line[m.start..].chars().next();
        let before = line[..m.start].chars().next_back();
        let last = line[..m.end].chars().next_back();
        let after = line[m.end..].chars().next();
        let start_ok = !(first.is_some_and(w) && before.is_some_and(w));
        let end_ok = !(last.is_some_and(w) && after.is_some_and(w));
        start_ok && end_ok
    }

    /// The next hit from `from` (forward: the first that starts at or after it; backward: the last that starts
    /// before it); with `wrap`, the search continues once from the other end and the hit is `wrapped`.
    pub fn next(
        &self,
        doc: &EditableDocument,
        from: TextPos,
        dir: Dir,
        wrap: bool,
    ) -> Result<Option<Hit>, FindError> {
        self.next_in(doc.document(), doc.clamp(from), dir, wrap)
    }

    /// [`Finder::next`] on a document (Find in Files, a pane's snapshot).
    pub fn next_in(
        &self,
        doc: &TextDocument,
        from: TextPos,
        dir: Dir,
        wrap: bool,
    ) -> Result<Option<Hit>, FindError> {
        let ctl = Ctl::detached();
        let found = match dir {
            Direction::Forward => self.first_from(doc, from, None, &ctl)?,
            Direction::Backward => self.last_before(doc, from, None, &ctl)?,
        };
        if found.is_some() || !wrap {
            return Ok(found);
        }
        let again = match dir {
            Direction::Forward => self.first_from(doc, TextPos::ZERO, Some(from), &ctl)?,
            Direction::Backward => self.last_before(doc, end_of(doc), Some(from), &ctl)?,
        };
        Ok(again.map(|mut hit| {
            hit.wrapped = true;
            hit
        }))
    }

    /// Every hit of the document in order, at most `limit` (Select All matching lines); cancellable per window.
    pub fn all(
        &self,
        doc: &EditableDocument,
        limit: u64,
        ctl: &Ctl<'_>,
    ) -> Result<Vec<Hit>, FindError> {
        self.all_in(doc.document(), limit, ctl)
    }

    /// [`Finder::all`] on a document.
    pub fn all_in(
        &self,
        doc: &TextDocument,
        limit: u64,
        ctl: &Ctl<'_>,
    ) -> Result<Vec<Hit>, FindError> {
        let mut out = Vec::new();
        if limit == 0 {
            return Ok(out);
        }
        self.scan(doc, TextPos::ZERO, ctl, &mut |hit, _, _| {
            out.push(hit);
            if out.len() as u64 >= limit {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Ok(out)
    }

    /// The first hit starting at or after `from` and, with `until`, before it.
    fn first_from(
        &self,
        doc: &TextDocument,
        from: TextPos,
        until: Option<TextPos>,
        ctl: &Ctl<'_>,
    ) -> Result<Option<Hit>, FindError> {
        let mut found = None;
        self.scan(doc, from, ctl, &mut |hit, _, _| {
            if until.is_none_or(|u| hit.range.start < u) {
                found = Some(hit);
            }
            ControlFlow::Break(())
        })?;
        Ok(found)
    }

    /// The last hit starting before `before` and, with `after`, at or after it.
    fn last_before(
        &self,
        doc: &TextDocument,
        before: TextPos,
        after: Option<TextPos>,
        ctl: &Ctl<'_>,
    ) -> Result<Option<Hit>, FindError> {
        let mut found = None;
        self.scan_back(doc, before, ctl, &mut |hit| {
            if after.is_none_or(|a| hit.range.start >= a) {
                found = Some(hit);
            }
            ControlFlow::Break(())
        })?;
        Ok(found)
    }

    /// Visits every hit that starts at or after `from`, in order, with its physical line's text and its byte range
    /// there, until `visit` breaks. The engine of every forward search.
    pub fn scan(
        &self,
        doc: &TextDocument,
        from: TextPos,
        ctl: &Ctl<'_>,
        visit: &mut HitVisitor<'_>,
    ) -> Result<(), FindError> {
        let text_len = doc.rope().len_bytes();
        let from = clamp_pos(doc, from);
        let (mut ws, mut skip) = unit_offset(doc, from);
        let mut buf = String::new();
        let mut size = FIRST_WINDOW;
        while ws < text_len {
            ctl.check()?;
            let we = window_end(doc, ws, size);
            size = (size * 2).min(WINDOW);
            fill(doc, ws, we, &mut buf);
            let bytes = buf.as_bytes();
            let mut u = 0usize;
            while u < bytes.len() {
                // The prefilter: plain text jumps to the physical line of its next candidate.
                if let Some(q) = self.candidate(&buf, u + skip) {
                    let Some(q) = q else {
                        break;
                    };
                    let start = unit_start_before(bytes, u, q);
                    if start > u {
                        u = start;
                        skip = 0;
                    }
                }
                let (content_end, next) = next_unit(bytes, u);
                let text = buf.get(u..content_end).unwrap_or("");
                let mut flow = ControlFlow::Continue(());
                let mut mapper = PosMap::new(doc, ws + u);
                self.hits_in_line(text, skip as u32, |r| {
                    let r = r.start as usize..r.end as usize;
                    let hit = Hit {
                        range: mapper.pos(r.start, false)..mapper.pos(r.end, true),
                        wrapped: false,
                    };
                    flow = visit(hit, text, r);
                    flow
                });
                if flow.is_break() {
                    return Ok(());
                }
                skip = 0;
                u = next;
            }
            ws = we;
            skip = 0;
        }
        Ok(())
    }

    /// Visits every hit that starts before `before`, last first, until `visit` breaks.
    fn scan_back(
        &self,
        doc: &TextDocument,
        before: TextPos,
        ctl: &Ctl<'_>,
        visit: &mut dyn FnMut(Hit) -> ControlFlow<()>,
    ) -> Result<(), FindError> {
        let before = clamp_pos(doc, before);
        let (unit, limit) = unit_offset(doc, before);
        // The window ends after the physical line of `before`; its hits must start before `limit` there.
        let mut we = unit_end(doc, unit);
        let mut first = true;
        let mut buf = String::new();
        let mut hits: Vec<Hit> = Vec::new();
        let mut size = FIRST_WINDOW;
        while we > 0 || first {
            ctl.check()?;
            let ws = window_start(doc, we, size);
            size = (size * 2).min(WINDOW);
            fill(doc, ws, we, &mut buf);
            let bytes = buf.as_bytes();
            hits.clear();
            let mut u = 0usize;
            while u < bytes.len() {
                if let Some(q) = self.candidate(&buf, u) {
                    let Some(q) = q else {
                        break;
                    };
                    u = unit_start_before(bytes, u, q).max(u);
                }
                let (content_end, next) = next_unit(bytes, u);
                let text = buf.get(u..content_end).unwrap_or("");
                let last = first && ws + u == unit;
                let mut mapper = PosMap::new(doc, ws + u);
                self.hits_in_line(text, 0, |r| {
                    if last && r.start as usize >= limit {
                        return ControlFlow::Break(());
                    }
                    let r = r.start as usize..r.end as usize;
                    hits.push(Hit {
                        range: mapper.pos(r.start, false)..mapper.pos(r.end, true),
                        wrapped: false,
                    });
                    ControlFlow::Continue(())
                });
                u = next;
            }
            for hit in hits.drain(..).rev() {
                if visit(hit).is_break() {
                    return Ok(());
                }
            }
            first = false;
            if ws == 0 {
                break;
            }
            we = ws;
        }
        Ok(())
    }

    /// The prefilter of plain text from `at` in the window: `Some(Some(q))` the next candidate's offset, `Some(None)`
    /// none in the window, `None` no prefilter (a regex: every line is searched).
    fn candidate(&self, buf: &str, at: usize) -> Option<Option<usize>> {
        if let Some(literal) = &self.literal {
            let tail = buf.as_bytes().get(at..)?;
            return Some(literal.find(tail).map(|k| at + k));
        }
        if self.folded_literal {
            return Some(match self.pattern.find_at(buf, at) {
                Ok(m) => m.map(|m| m.start),
                Err(_) => Some(at),
            });
        }
        None
    }
}

/// True for a regex that can only match a line break: a `\n`, `\r` (or their hex forms) written outside a class,
/// or a class of line breaks only.
fn only_line_breaks(source: &str) -> bool {
    let breaks = |escape: &str| {
        matches!(
            escape.to_ascii_lowercase().as_str(),
            "n" | "r" | "x0a" | "x0d" | "x{a}" | "x{d}" | "x{0a}" | "x{0d}" | "u000a" | "u000d"
        )
    };
    // Each escape: the letter after `\` and, for `x`/`u`, its digits.
    fn escape_at(chars: &[char], i: usize) -> (String, usize) {
        let mut word = String::new();
        let Some(&c) = chars.get(i) else {
            return (word, i);
        };
        word.push(c);
        let mut j = i + 1;
        match c {
            'x' | 'u' if chars.get(j) == Some(&'{') => {
                while let Some(&d) = chars.get(j) {
                    word.push(d);
                    j += 1;
                    if d == '}' {
                        break;
                    }
                }
            }
            'x' => {
                for _ in 0..2 {
                    if let Some(&d) = chars.get(j) {
                        word.push(d);
                        j += 1;
                    }
                }
            }
            'u' => {
                for _ in 0..4 {
                    if let Some(&d) = chars.get(j) {
                        word.push(d);
                        j += 1;
                    }
                }
            }
            _ => {}
        }
        (word, j)
    }
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\n' | '\r' => return true,
            '\\' => {
                let (word, j) = escape_at(&chars, i + 1);
                if breaks(&word) {
                    return true;
                }
                i = j;
            }
            '[' => {
                // A class: refused when not negated and every member is a line break.
                let negated = chars.get(i + 1) == Some(&'^');
                let mut j = i + 1 + usize::from(negated);
                let mut members = 0;
                let mut all_breaks = true;
                let mut first = true;
                while let Some(&c) = chars.get(j) {
                    if c == ']' && !first {
                        break;
                    }
                    first = false;
                    members += 1;
                    if c == '\\' {
                        let (word, k) = escape_at(&chars, j + 1);
                        all_breaks &= breaks(&word);
                        j = k;
                    } else {
                        all_breaks &= c == '\n' || c == '\r';
                        j += 1;
                    }
                }
                if !negated && members > 0 && all_breaks {
                    return true;
                }
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    false
}

/// The regex class of word characters (D-58): letters and numbers as `char::is_alphanumeric`, `_`, the extras.
fn word_class(extra: &str) -> String {
    let mut class = String::from(r"[\p{Alphabetic}\p{N}_");
    for c in extra.chars() {
        class.push_str(&format!(r"\x{{{:x}}}", c as u32));
    }
    class.push(']');
    class
}

/// The byte after the character at `at`, `None` at the end.
fn next_char(line: &str, at: usize) -> Option<usize> {
    let c = line.get(at..)?.chars().next()?;
    Some(at + c.len_utf8())
}

/// The position after the last character of the document (after its last line's content).
fn end_of(doc: &TextDocument) -> TextPos {
    let lines = doc.line_count();
    if lines == 0 {
        return TextPos::ZERO;
    }
    let last = lines - 1;
    if doc.line_ending(LineIdx(last)) == LineEnding::None {
        TextPos::new(last, doc.line_len(last) as u32)
    } else {
        TextPos::new(lines, 0)
    }
}

/// `pos` clamped into the document (a byte past its line's end goes to the end; a line past the last goes to the
/// end of the text).
fn clamp_pos(doc: &TextDocument, pos: TextPos) -> TextPos {
    if pos.line.0 >= doc.line_count() {
        return TextPos::new(doc.line_count(), 0);
    }
    let len = doc.line_len(pos.line.0) as u32;
    TextPos {
        line: pos.line,
        byte: pos.byte.min(len),
    }
}

/// The start of a table line's content in the rope (the text's length past the last line).
fn line_start(doc: &TextDocument, line: u32) -> usize {
    doc.content_range(line)
        .map_or(doc.rope().len_bytes(), |(s, _)| s)
}

/// The physical line of `pos` (its first piece's start in the rope) and `pos`'s offset in it.
fn unit_offset(doc: &TextDocument, pos: TextPos) -> (usize, usize) {
    if pos.line.0 >= doc.line_count() {
        return (doc.rope().len_bytes(), 0);
    }
    let mut first = pos.line.0;
    if !doc.artificial_lines().is_empty() {
        while first > 0 && doc.line_ending(LineIdx(first - 1)) == LineEnding::Artificial {
            first -= 1;
        }
    }
    let start = line_start(doc, first);
    let at = line_start(doc, pos.line.0) + pos.byte as usize;
    (start, at - start)
}

/// The end (after its real terminator) of the physical line that starts at `unit`.
fn unit_end(doc: &TextDocument, unit: usize) -> usize {
    let text_len = doc.rope().len_bytes();
    if unit >= text_len {
        return text_len;
    }
    let mut line = doc.line_at_byte(unit);
    while doc.line_ending(LineIdx(line)) == LineEnding::Artificial {
        line += 1;
    }
    line_start(doc, line + 1)
}

/// The end of the window that starts at `ws` (a physical line's start): whole physical lines, about [`WINDOW`]
/// bytes.
fn window_end(doc: &TextDocument, ws: usize, size: usize) -> usize {
    let text_len = doc.rope().len_bytes();
    let target = ws.saturating_add(size);
    if target >= text_len {
        return text_len;
    }
    unit_end(doc, target)
}

/// The start of the window that ends at `we` (a physical line's end): whole physical lines, about [`WINDOW`]
/// bytes.
fn window_start(doc: &TextDocument, we: usize, size: usize) -> usize {
    let target = we.saturating_sub(size);
    if target == 0 {
        return 0;
    }
    let mut line = doc.line_at_byte(target);
    while line > 0 && doc.line_ending(LineIdx(line - 1)) == LineEnding::Artificial {
        line -= 1;
    }
    let start = line_start(doc, line);
    if start >= we {
        // A window never ends empty: step back one whole physical line.
        let mut prev = doc.line_at_byte(we.saturating_sub(1));
        while prev > 0 && doc.line_ending(LineIdx(prev - 1)) == LineEnding::Artificial {
            prev -= 1;
        }
        return line_start(doc, prev);
    }
    start
}

/// Copies the rope's bytes `[ws, we)` (whole lines: character boundaries) into `buf`.
fn fill(doc: &TextDocument, ws: usize, we: usize, buf: &mut String) {
    buf.clear();
    if let Some(slice) = doc.rope().get_byte_slice(ws..we) {
        buf.reserve(we - ws);
        for chunk in slice.chunks() {
            buf.push_str(chunk);
        }
    }
}

/// The physical line at `at` in a window: the end of its content and the start of the next one (after a LF, CRLF
/// or CR; an artificial break has no byte, so a physical line is found whole).
fn next_unit(bytes: &[u8], at: usize) -> (usize, usize) {
    match memchr::memchr2(b'\n', b'\r', bytes.get(at..).unwrap_or(&[])) {
        None => (bytes.len(), bytes.len()),
        Some(k) => {
            let e = at + k;
            if bytes[e] == b'\r' && bytes.get(e + 1) == Some(&b'\n') {
                (e, e + 2)
            } else {
                (e, e + 1)
            }
        }
    }
}

/// The start of the physical line holding offset `q` of the window, not before `floor`.
fn unit_start_before(bytes: &[u8], floor: usize, q: usize) -> usize {
    let q = q.min(bytes.len());
    match memchr::memrchr2(b'\n', b'\r', bytes.get(floor..q).unwrap_or(&[])) {
        Some(k) => floor + k + 1,
        None => floor,
    }
}

/// Maps offsets of one physical line to [`TextPos`] (table line and byte): one lookup without CPL pieces.
struct PosMap<'d> {
    doc: &'d TextDocument,
    unit: usize,
    first: Option<u32>,
}

impl<'d> PosMap<'d> {
    fn new(doc: &'d TextDocument, unit: usize) -> PosMap<'d> {
        PosMap {
            doc,
            unit,
            first: None,
        }
    }

    /// The position of offset `off`; at a boundary between two pieces a start goes to the later piece and an end
    /// stays in the earlier one.
    fn pos(&mut self, off: usize, is_end: bool) -> TextPos {
        let doc = self.doc;
        let first = *self
            .first
            .get_or_insert_with(|| doc.line_at_byte(self.unit));
        if doc.artificial_lines().is_empty() {
            return TextPos::new(first, off as u32);
        }
        let abs = self.unit + off;
        let mut line = if abs >= doc.rope().len_bytes() {
            doc.line_count().saturating_sub(1)
        } else {
            doc.line_at_byte(abs).max(first)
        };
        let mut start = line_start(doc, line);
        if is_end && abs == start && line > first {
            line -= 1;
            start = line_start(doc, line);
        }
        TextPos::new(line, abs.saturating_sub(start) as u32)
    }
}

/// What a hit is replaced with (§1.10.18): the literal text (plain), or a [`Template`] of the regex's groups
/// (`$1`, `${1}`, `${name}`, `$$`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    kind: ReplacementKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplacementKind {
    Literal(String),
    Template(Template),
}

impl Replacement {
    /// The replacement of `text` for the finder's query: literal for plain text, a template for a regex (an unknown
    /// group is `invalid_input` here).
    pub fn new(text: &str, f: &Finder) -> Result<Replacement, FindError> {
        if !f.query.regex {
            return Ok(Replacement {
                kind: ReplacementKind::Literal(text.to_owned()),
            });
        }
        let template = Template::parse(text, &f.group_names())
            .map_err(|e| FindError::InvalidInput(format!("replacement {e}")))?;
        Ok(Replacement {
            kind: match template.literal() {
                Some(literal) => ReplacementKind::Literal(literal),
                None => ReplacementKind::Template(template),
            },
        })
    }

    /// The text that replaces the hit `range` of `line` (its physical line) found by `f`.
    pub fn expand(&self, f: &Finder, line: &str, range: Range<usize>) -> Result<String, FindError> {
        match &self.kind {
            ReplacementKind::Literal(text) => Ok(text.clone()),
            ReplacementKind::Template(template) => {
                let groups = f
                    .pattern
                    .captures_at(line, range.start)
                    .map_err(|e| FindError::InvalidInput(e.to_string()))?;
                match groups {
                    Some(groups) if groups.first().and_then(Option::as_ref) == Some(&range) => {
                        Ok(template.expand(line, &groups))
                    }
                    _ => Err(FindError::InvalidInput(
                        "the text at the hit no longer matches".to_owned(),
                    )),
                }
            }
        }
    }
}

/// The physical line holding `pos`: its text and `pos`'s offset there.
fn unit_text(doc: &TextDocument, pos: TextPos) -> (String, usize) {
    let (start, off) = unit_offset(doc, pos);
    let end = unit_end(doc, start);
    let mut buf = String::new();
    fill(doc, start, end, &mut buf);
    let (content_end, _) = next_unit(buf.as_bytes(), 0);
    buf.truncate(content_end);
    (buf, off)
}

/// The edit that replaces `hit` (one step; line breaks in the replacement become the document's insert terminator,
/// §1.10.3). `invalid_input` when the text at the hit no longer matches.
pub fn replace_one(
    doc: &EditableDocument,
    f: &Finder,
    hit: &Hit,
    with: &Replacement,
) -> Result<EditRequest, FindError> {
    let d = doc.document();
    let (line, start) = unit_text(d, hit.range.start);
    let (_, end_unit_off) = unit_offset(d, hit.range.end);
    let (unit_a, _) = unit_offset(d, hit.range.start);
    let (unit_b, _) = unit_offset(d, hit.range.end);
    let end = if unit_a == unit_b {
        end_unit_off
    } else {
        usize::MAX
    };
    if end > line.len() || f.first_in(&line, start) != Some(start..end) {
        return Err(FindError::InvalidInput(
            "the text at the hit no longer matches".to_owned(),
        ));
    }
    let text = with.expand(f, &line, start..end)?;
    Ok(EditRequest::Replace {
        range: hit.range.clone(),
        text,
        keep_eols: false,
    })
}

/// Replace All (one step): every hit inside `scope` (the whole document when `None`), as one `Batch`, and their
/// count (*Replace N occurrences?*, W11); `None` when there is none. Cancellable per window.
pub fn replace_all(
    doc: &EditableDocument,
    f: &Finder,
    with: &Replacement,
    scope: Option<Range<TextPos>>,
    ctl: &Ctl<'_>,
) -> Result<(Option<EditRequest>, u64), FindError> {
    let d = doc.document();
    let from = scope.as_ref().map_or(TextPos::ZERO, |s| s.start);
    let mut edits = Vec::new();
    let mut error = None;
    f.scan(d, from, ctl, &mut |hit, line, range| {
        if let Some(s) = &scope
            && hit.range.end > s.end
        {
            return ControlFlow::Break(());
        }
        match with.expand(f, line, range) {
            Ok(text) => {
                edits.push((hit.range, text));
                ControlFlow::Continue(())
            }
            Err(e) => {
                error = Some(e);
                ControlFlow::Break(())
            }
        }
    })?;
    if let Some(e) = error {
        return Err(e);
    }
    let count = edits.len() as u64;
    Ok((
        (count > 0).then_some(EditRequest::Batch {
            edits,
            keep_eols: false,
        }),
        count,
    ))
}

/// The search of a comparison (§1.10.18): from the row `from.0` on side `from.1`, by row, the left side first in a
/// row; forward the hits of that cell count from its start, backward only the cells before it. `docs` give the panes'
/// live text (`cmp`'s rows index their lines).
pub fn find_in_comparison(
    cmp: &TextComparison,
    docs: Pair<&EditableDocument>,
    f: &Finder,
    from: (RowIdx, Side),
    sides: FindSides,
    dir: Dir,
    wrap: bool,
) -> Result<Option<(Side, Hit)>, FindError> {
    find_in_comparison_at(cmp, docs, f, (from.0, from.1, 0), sides, dir, wrap)
}

/// [`find_in_comparison`] from a caret (row, side, byte in that side's line): forward the first hit that starts at or
/// after it, backward the last that starts before it (additive: repeated Find Next walks a line hit by hit).
pub fn find_in_comparison_at(
    cmp: &TextComparison,
    docs: Pair<&EditableDocument>,
    f: &Finder,
    from: (RowIdx, Side, u32),
    sides: FindSides,
    dir: Dir,
    wrap: bool,
) -> Result<Option<(Side, Hit)>, FindError> {
    let (row, side, byte) = from;
    let side = if side == Side::Right {
        Side::Right
    } else {
        Side::Left
    };
    let rows = cmp.row_count();
    let found = cmp_step(cmp, docs, f, (row.0.min(rows), side, byte), sides, dir)?;
    if found.is_some() || !wrap {
        return Ok(found);
    }
    // Nothing on this side of the caret: the hit nearest the other end is on the far side of it.
    let restart = match dir {
        Direction::Forward => (0, Side::Left, 0),
        Direction::Backward => (rows, Side::Left, 0),
    };
    let again = cmp_step(cmp, docs, f, restart, sides, dir)?;
    Ok(again.map(|(side, mut hit)| {
        hit.wrapped = true;
        (side, hit)
    }))
}

/// The smallest line of `side` whose row is at or after `row` (the line count when none): rows grow with lines.
fn line_bound(cmp: &TextComparison, side: Side, lines: u32, row: u32) -> u32 {
    let (mut lo, mut hi) = (0u32, lines);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        match cmp.row_for_line(side, LineIdx(mid)) {
            Some(r) if r.0 >= row => hi = mid,
            _ => lo = mid + 1,
        }
    }
    lo
}

/// The line of `side` in `row`, if it has one.
fn row_line(cmp: &TextComparison, row: u32, side: Side) -> Option<u32> {
    let r = cmp.row(RowIdx(row))?;
    match side {
        Side::Right => r.right.map(|l| l.0),
        _ => r.left.map(|l| l.0),
    }
}

/// A hit's sort key in a comparison: (row, side: left 0 right 1, start).
fn key_of(cmp: &TextComparison, side: Side, hit: &Hit) -> (u32, u8, TextPos) {
    let row = cmp
        .row_for_line(side, hit.range.start.line)
        .map_or(u32::MAX, |r| r.0);
    (row, u8::from(side == Side::Right), hit.range.start)
}

/// One step of a comparison's search from the caret `(row, side, byte)`: forward the first hit at or after it in
/// (row, left before right, position) order, backward the last before it. Each side's first (last) hit in document
/// order is its first (last) in row order, since rows grow with lines.
fn cmp_step(
    cmp: &TextComparison,
    docs: Pair<&EditableDocument>,
    f: &Finder,
    caret: (u32, Side, u32),
    sides: FindSides,
    dir: Dir,
) -> Result<Option<(Side, Hit)>, FindError> {
    let (row, side, byte) = caret;
    let ctl = Ctl::detached();
    let mut best: Option<((u32, u8, TextPos), Side, Hit)> = None;
    for s in [Side::Left, Side::Right] {
        if !sides.has(s) {
            continue;
        }
        let doc = if s == Side::Left {
            docs.left
        } else {
            docs.right
        };
        let d = doc.document();
        let lines = d.line_count();
        let own = || match row_line(cmp, row, s) {
            Some(l) => TextPos::new(l, byte),
            None => TextPos::new(line_bound(cmp, s, lines, row), 0),
        };
        let found = match dir {
            Direction::Forward => {
                // The caret's side from the caret; the right side after a left caret from the caret's row; the left
                // side after a right caret from the next row.
                let start = match (s, side) {
                    (s, c) if s == c => own(),
                    (Side::Right, _) => TextPos::new(line_bound(cmp, s, lines, row), 0),
                    _ => TextPos::new(line_bound(cmp, s, lines, row + 1), 0),
                };
                f.first_from(d, start, None, &ctl)?
            }
            Direction::Backward => {
                // Before the caret on its side; the left side before a right caret up to the caret's row included;
                // the right side before a left caret up to the caret's row excluded.
                let before = match (s, side) {
                    (s, c) if s == c => own(),
                    (Side::Left, _) => TextPos::new(line_bound(cmp, s, lines, row + 1), 0),
                    _ => TextPos::new(line_bound(cmp, s, lines, row), 0),
                };
                f.last_before(d, before, None, &ctl)?
            }
        };
        if let Some(hit) = found {
            let key = key_of(cmp, s, &hit);
            let better = best.as_ref().is_none_or(|(k, _, _)| match dir {
                Direction::Forward => key < *k,
                Direction::Backward => key > *k,
            });
            if better {
                best = Some((key, s, hit));
            }
        }
    }
    Ok(best.map(|(_, s, hit)| (s, hit)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_break_patterns() {
        for p in [
            r"\n", r"\r\n", r"a\nb", r"[\n]", r"[\r\n]+", r"\x0A", r"\x{a}", "a|\\n",
        ] {
            assert!(only_line_breaks(p), "{p}");
        }
        for p in [r"\s", r"[^\n]", r"\\n", r"[a\n]", r"\w+", r"[\]]"] {
            assert!(!only_line_breaks(p), "{p}");
        }
    }

    #[test]
    fn units() {
        let b = b"ab\r\ncd\ref\ngh";
        assert_eq!(next_unit(b, 0), (2, 4));
        assert_eq!(next_unit(b, 4), (6, 7));
        assert_eq!(next_unit(b, 7), (9, 10));
        assert_eq!(next_unit(b, 10), (12, 12));
        assert_eq!(unit_start_before(b, 0, 8), 7);
        assert_eq!(unit_start_before(b, 0, 1), 0);
    }
}
