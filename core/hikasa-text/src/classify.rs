//! Importance classification and line keys (§2.13; TC-012, TC-014, TC-020…TC-022; D-04, D-13, D-14, D-53):
//! [`CharClass`], [`ClassifiedSpan`], the compiled per-format rule set, the general key builder, the identity
//! fast path, trivial lines and the streamed matches of the unimportant patterns.
//!
//! ```text
//! class(char at x):  inside an unimportant pattern match → UnimportantPattern
//!                    inside a grammar token e ≠ EE         → Element(e)
//!                    whitespace in the leading prefix      → LeadingWhitespace   (an all-blank line is all leading)
//!                    whitespace in the trailing suffix     → TrailingWhitespace
//!                    other whitespace                      → EmbeddedWhitespace
//!                    otherwise                             → Other
//! key(line): the important chars in order, case-insensitive ones folded (hikasa_core::case::simple_lower);
//!            SEP(e1, e2) where dropped chars separate two word chars of different tokens, one of them a
//!            grammar token (D-14: `int x` ≠ `intx`, `a + b` = `a+b`, EE text `a b` = `ab`);
//!            ART after an artificial break (always); EOL kind when line endings are compared.
//! encoding:  text = UTF-8 · SEP = 0xFF varint(e1) varint(e2) · EOL = 0xFE kind (1 LF, 2 CRLF, 3 CR, 4 None) ·
//!            ART = 0xFD. Bytes 0xF8..=0xFF never occur in UTF-8, so keys parse uniquely (INV-KEY-1).
//! identity:  key = raw line ++ end markers (D-53); the general key of a line with nothing dropped or folded is
//!            the same bytes, whatever the tokenizer state (INV-KEY-3)
//! trivial:   the key has no text (identity: the line is empty)                              (INV-KEY-2)
//! ```

use std::ops::Range;
use std::sync::Arc;

use hikasa_core::cancel::CancelToken;
use hikasa_core::case::simple_lower;
use hikasa_formats::{CompiledFormat, ElementId, EngineKind, Pattern, RegexLimits, Token};
use serde::{Deserialize, Serialize};

use crate::document::LineEnding;
use crate::error::TextError;
use crate::rules::TextRules;

/// Char classes (§2.13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharClass {
    /// Inside a match of an "unimportant text" pattern (TC-020): never important.
    UnimportantPattern,
    /// Inside a grammar token (the span's `element` says which).
    Element,
    /// Whitespace of the leading-whitespace prefix (an all-whitespace line is all leading).
    LeadingWhitespace,
    /// Whitespace between other text.
    EmbeddedWhitespace,
    /// Whitespace of the trailing-whitespace suffix.
    TrailingWhitespace,
    /// Any other text not covered by a grammar token ("everything else").
    Other,
}

impl CharClass {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            CharClass::UnimportantPattern => "unimportant_pattern",
            CharClass::Element => "element",
            CharClass::LeadingWhitespace => "leading_whitespace",
            CharClass::EmbeddedWhitespace => "embedded_whitespace",
            CharClass::TrailingWhitespace => "trailing_whitespace",
            CharClass::Other => "other",
        }
    }
}

/// A coloured span of a line (syntax element + importance), for views.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClassifiedSpan {
    /// Byte range.
    pub range: Range<u32>,
    /// Element.
    pub element: ElementId,
    /// Char class used for importance.
    pub class: CharClass,
    /// Importance under the rules.
    pub important: bool,
}

/// Marker tags of the key encoding (§2.13.2 as amended in wave 3). They are bytes that never occur in UTF-8
/// text (0xF8..=0xFF are invalid in UTF-8), so every key parses uniquely into text and markers (INV-KEY-1).
///
/// `SEP` = 0xFF varint(element before) varint(element after): a token boundary at dropped chars (D-14).
pub(crate) const TAG_SEP: u8 = 0xFF;
/// `EOL` = 0xFE kind (1 LF, 2 CRLF, 3 CR, 4 None): the line ending, only when endings are compared (TC-022).
pub(crate) const TAG_EOL: u8 = 0xFE;
/// `ART` = 0xFD: the line ends at an artificial break of the characters-per-line limit (TC-003). Always
/// written, so a wrapped line never equals a line ended by a real line break.
pub(crate) const TAG_ART: u8 = 0xFD;
/// Every byte at or above this value is a marker tag; text bytes are below it.
pub(crate) const FIRST_TAG: u8 = 0xF8;

/// Haystack bytes between two cancellation checks inside one line (§5.3: every 64 KiB of scanned text).
pub(crate) const LINE_CHECK_BYTES: usize = 64 * 1024;

/// The EOL kind byte of the `EOL` marker: 1 LF, 2 CRLF, 3 CR, 4 None (`Artificial` never gets an `EOL` marker;
/// it gets `ART`).
pub(crate) const fn eol_code(eol: LineEnding) -> u8 {
    match eol {
        LineEnding::Lf => 1,
        LineEnding::CrLf => 2,
        LineEnding::Cr => 3,
        LineEnding::None => 4,
        LineEnding::Artificial => 0,
    }
}

/// Appends the end-of-line markers of a key: `ART` for an artificial break (always), else `EOL kind` when line
/// endings are compared.
#[inline]
pub(crate) fn push_eol_markers(out: &mut Vec<u8>, eol: LineEnding, compare_eol: bool) {
    if eol == LineEnding::Artificial {
        out.push(TAG_ART);
    } else if compare_eol {
        out.push(TAG_EOL);
        out.push(eol_code(eol));
    }
}

/// `is_word(c)` of the tokenizer (§2.10.1, D-58): alphanumeric, `_`, or one of the format's `word_chars`.
#[inline]
pub(crate) fn is_word(c: char, word_chars: &[char]) -> bool {
    c.is_alphanumeric() || c == '_' || word_chars.contains(&c)
}

/// Compiles the unimportant-text patterns of `rules` (format-independent, shared by every comparison of a
/// session).
pub(crate) fn compile_patterns(
    rules: &TextRules,
    limits: &RegexLimits,
) -> Result<Arc<[Pattern]>, TextError> {
    rules
        .importance
        .unimportant
        .iter()
        .enumerate()
        .map(|(k, p)| {
            p.compile(limits).map_err(|message| {
                TextError::Rules(format!("importance.unimportant[{k}]: {message}"))
            })
        })
        .collect()
}

/// A span of one line with everything the key builder and the views need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) element: ElementId,
    pub(crate) class: CharClass,
    pub(crate) important: bool,
    pub(crate) case_sensitive: bool,
}

/// The merged matches of the unimportant patterns on one line, produced lazily in increasing order: each
/// pattern is searched left to right as non-overlapping leftmost-first matches, and overlapping or touching
/// matches (of any patterns) are merged. Memory is O(patterns) however many matches a line has (§5.7, T9).
///
/// The cancellation token is checked before every regex engine call (§5.3); once it fires, the cursor reports
/// no further match and [`PatternCursor::cancelled`] is true: the caller's key or spans are then partial and are
/// discarded by the caller, which checks the token afterwards. Runtime-limit hits (fancy-regex) end that
/// pattern's matches on the line and are counted.
pub(crate) struct PatternCursor<'a> {
    patterns: &'a [Pattern],
    line: &'a str,
    cancel: &'a CancelToken,
    /// Per pattern: its next match, if any.
    next: Vec<Option<(usize, usize)>>,
    /// Per pattern: where its next search starts (`line.len() + 1` = exhausted).
    pos: Vec<usize>,
    /// The first merged range not yet passed.
    current: Option<(usize, usize)>,
    errors: u32,
    cancelled: bool,
}

impl<'a> PatternCursor<'a> {
    /// A cursor over `line` (no allocation when there are no patterns).
    pub(crate) fn new(patterns: &'a [Pattern], line: &'a str, cancel: &'a CancelToken) -> Self {
        let mut cursor = PatternCursor {
            patterns,
            line,
            cancel,
            next: Vec::new(),
            pos: Vec::new(),
            current: None,
            errors: 0,
            cancelled: false,
        };
        if !patterns.is_empty() {
            cursor.next = vec![None; patterns.len()];
            cursor.pos = vec![0; patterns.len()];
            for k in 0..patterns.len() {
                cursor.advance(k);
            }
            cursor.current = cursor.merge_next();
        }
        cursor
    }

    /// Loads the next match of pattern `k` into `next[k]` (`None` when it has no more matches).
    fn advance(&mut self, k: usize) {
        self.next[k] = None;
        let line = self.line;
        let Some(pattern) = self.patterns.get(k) else {
            return;
        };
        let mut pos = self.pos[k];
        while pos <= line.len() {
            if self.cancelled || self.cancel.is_cancelled() {
                self.cancelled = true;
                break;
            }
            match pattern.find_at(line, pos) {
                Ok(Some(m)) if m.start < m.end => {
                    self.next[k] = Some((m.start, m.end));
                    self.pos[k] = m.end;
                    return;
                }
                Ok(Some(m)) => {
                    // A zero-length match marks nothing; continue after the next char.
                    match line.get(m.end..).and_then(|rest| rest.chars().next()) {
                        Some(c) => pos = m.end + c.len_utf8(),
                        None => break,
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    self.errors = self.errors.saturating_add(1);
                    break;
                }
            }
        }
        self.pos[k] = line.len() + 1;
    }

    /// The next merged range: the leftmost next match, extended while some pattern's next match starts at or
    /// before its end.
    fn merge_next(&mut self) -> Option<(usize, usize)> {
        let k = (0..self.next.len())
            .filter(|&k| self.next[k].is_some())
            .min_by_key(|&k| self.next[k])?;
        let (start, mut end) = self.next[k]?;
        self.advance(k);
        while let Some(j) =
            (0..self.next.len()).find(|&j| self.next[j].is_some_and(|(s, _)| s <= end))
        {
            if let Some((_, e)) = self.next[j] {
                end = end.max(e);
            }
            self.advance(j);
        }
        Some((start, end))
    }

    /// The first merged range whose end is after byte `p` (it may start after `p`); ranges ending at or before
    /// `p` are passed for good, so `p` must not decrease between calls.
    #[inline]
    pub(crate) fn at(&mut self, p: usize) -> Option<(usize, usize)> {
        while let Some((_, end)) = self.current {
            if end > p {
                break;
            }
            self.current = self.merge_next();
        }
        self.current
    }

    /// True when byte `p` lies inside a match (same monotonicity rule as [`PatternCursor::at`]).
    #[inline]
    pub(crate) fn contains(&mut self, p: usize) -> bool {
        self.at(p).is_some_and(|(start, _)| start <= p)
    }

    /// Runs through every remaining match (the runtime-limit pass).
    pub(crate) fn finish(&mut self) {
        while self.current.is_some() {
            self.current = self.merge_next();
        }
    }

    /// Runtime-limit hits so far.
    pub(crate) fn errors(&self) -> u32 {
        self.errors
    }

    /// True once the token fired (the matches are incomplete).
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled
    }
}

/// The rules compiled for one format: per-element importance and case flags, the class flags, the unimportant
/// patterns and the format's extra word chars.
#[derive(Debug, Clone)]
pub(crate) struct RuleSet {
    patterns: Arc<[Pattern]>,
    elem_important: Vec<bool>,
    elem_case: Vec<bool>,
    word_chars: Box<[char]>,
    leading: bool,
    embedded: bool,
    trailing: bool,
    other: bool,
    other_case: bool,
    /// Line endings are part of the key (TC-022).
    pub(crate) compare_eol: bool,
    /// `rules.is_identity(format)`: every char of every line is important and case-sensitive.
    pub(crate) all_important: bool,
}

impl RuleSet {
    /// Compiles `rules` for `format` with the already compiled `patterns`.
    pub(crate) fn new(
        rules: &TextRules,
        format: &CompiledFormat,
        patterns: Arc<[Pattern]>,
    ) -> RuleSet {
        let table = format.grammar().elements();
        let ids = (0..table.len()).map(ElementId);
        let elem_important = ids
            .clone()
            .map(|id| rules.element_important(format, id))
            .collect();
        let elem_case = ids.map(|id| table.case_sensitive(id)).collect();
        let i = &rules.importance;
        RuleSet {
            patterns,
            elem_important,
            elem_case,
            word_chars: format.text().word_chars.chars().collect(),
            leading: i.leading_whitespace,
            embedded: i.embedded_whitespace,
            trailing: i.trailing_whitespace,
            other: i.everything_else,
            other_case: i.everything_else_case_sensitive,
            compare_eol: i.compare_line_endings,
            all_important: rules.is_identity(format),
        }
    }

    /// True when some unimportant pattern runs on fancy-regex, the only engine with a runtime limit.
    pub(crate) fn has_fancy_patterns(&self) -> bool {
        self.patterns
            .iter()
            .any(|p| p.engine() == EngineKind::Fancy)
    }

    /// The pattern cursor of `line`.
    pub(crate) fn patterns<'a>(
        &'a self,
        line: &'a str,
        cancel: &'a CancelToken,
    ) -> PatternCursor<'a> {
        PatternCursor::new(&self.patterns, line, cancel)
    }

    #[inline]
    fn element_important(&self, id: ElementId) -> bool {
        self.elem_important
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(true)
    }

    #[inline]
    fn element_case(&self, id: ElementId) -> bool {
        self.elem_case
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(true)
    }

    /// The union of the matches of every unimportant pattern on `line`, as a list (tests only: production code
    /// streams them with [`PatternCursor`]).
    #[cfg(test)]
    pub(crate) fn pattern_ranges(&self, line: &str, errors: &mut u32) -> Vec<(usize, usize)> {
        let never = CancelToken::never();
        let mut cursor = self.patterns(line, &never);
        let mut out = Vec::new();
        let mut p = 0;
        while let Some((s, e)) = cursor.at(p) {
            out.push((s, e));
            p = e;
        }
        *errors += cursor.errors();
        out
    }

    /// Importance of a whitespace char at byte `p`.
    #[inline]
    fn ws_important(&self, p: usize, lead_end: usize, trail_start: usize) -> bool {
        if p < lead_end {
            self.leading
        } else if p >= trail_start {
            self.trailing
        } else {
            self.embedded
        }
    }

    /// Class of a whitespace char at byte `p`.
    #[inline]
    fn ws_class(p: usize, lead_end: usize, trail_start: usize) -> CharClass {
        if p < lead_end {
            CharClass::LeadingWhitespace
        } else if p >= trail_start {
            CharClass::TrailingWhitespace
        } else {
            CharClass::EmbeddedWhitespace
        }
    }

    /// Appends the general key of `line` (§2.13.2 as amended in wave 3) to `out` and returns the regex
    /// runtime-limit hits of the unimportant patterns.
    ///
    /// ```text
    /// key = the important chars of the line, each folded when its class is case-insensitive, in order;
    ///       plus SEP(e1, e2) between two kept chars c1 (token t1, element e1) and c2 (token t2, element e2)
    ///       when chars between them were dropped, t1 ≠ t2, e1 or e2 is a grammar element (not EE), and c1 and
    ///       c2 are both word chars (D-14: `int x` ≠ `intx`; `a + b` = `a+b`; EE text merges, `a b` = `ab`);
    ///       then ART for an artificial break, else EOL kind when endings are compared.
    /// ```
    ///
    /// When nothing of a line is dropped or folded (every rule important and case-sensitive), the key is the raw
    /// line plus its end markers, which is exactly the identity key (D-53, INV-KEY-3), whatever the tokenizer's
    /// start state. `cancel` is checked every 64 KiB of the line and before every regex call; on cancellation the
    /// key is partial (the caller discards it).
    pub(crate) fn write_key(
        &self,
        line: &str,
        tokens: &[Token],
        eol: LineEnding,
        out: &mut Vec<u8>,
        cancel: &CancelToken,
    ) -> u32 {
        let mut pats = self.patterns(line, cancel);
        let (lead_end, trail_start) = ws_bounds(line);
        let uniform_ee = self.leading == self.embedded
            && self.embedded == self.trailing
            && self.trailing == self.other;
        let mut w = KeyWriter {
            out,
            word_chars: &self.word_chars,
            last: None,
            dropped: false,
        };
        let mut scanned = 0usize;
        for (ti, t) in tokens.iter().enumerate() {
            let (s, e) = (t.start as usize, t.end as usize);
            let Some(text) = line.get(s..e) else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            scanned += text.len();
            if scanned >= LINE_CHECK_BYTES {
                scanned = 0;
                if cancel.is_cancelled() {
                    return pats.errors();
                }
            }
            let overlaps = pats.at(s).is_some_and(|(start, _)| start < e);
            if t.element != ElementId::EVERYTHING_ELSE {
                if !self.element_important(t.element) {
                    w.drop_chars();
                    continue;
                }
                let cs = self.element_case(t.element);
                if !overlaps {
                    if !w.keep_str(ti, t.element, text, cs, cancel) {
                        return pats.errors();
                    }
                    continue;
                }
                let mut cur = s;
                while cur < e && !pats.cancelled() {
                    match pats.at(cur) {
                        Some((ps, pe)) if ps <= cur => {
                            w.drop_chars();
                            cur = pe.min(e);
                        }
                        Some((ps, _)) if ps < e => {
                            w.keep_str(ti, t.element, line.get(cur..ps).unwrap_or(""), cs, cancel);
                            cur = ps;
                        }
                        _ => {
                            w.keep_str(ti, t.element, line.get(cur..e).unwrap_or(""), cs, cancel);
                            cur = e;
                        }
                    }
                }
            } else if !overlaps && uniform_ee {
                if self.other {
                    if !w.keep_str(ti, t.element, text, self.other_case, cancel) {
                        return pats.errors();
                    }
                } else {
                    w.drop_chars();
                }
            } else {
                for (off, c) in text.char_indices() {
                    if off % LINE_CHECK_BYTES == LINE_CHECK_BYTES - 1 && cancel.is_cancelled() {
                        return pats.errors();
                    }
                    let p = s + off;
                    if pats.contains(p) {
                        w.drop_chars();
                        continue;
                    }
                    let (important, cs) = if c.is_whitespace() {
                        (self.ws_important(p, lead_end, trail_start), true)
                    } else {
                        (self.other, self.other_case)
                    };
                    if important {
                        w.keep_char(ti, t.element, c, cs);
                    } else {
                        w.drop_chars();
                    }
                }
            }
        }
        push_eol_markers(w.out, eol, self.compare_eol);
        pats.errors()
    }

    /// The identity key (D-53): the raw line plus its end markers (`ART` for an artificial break, else
    /// `EOL kind` when line endings are compared). Equal to [`RuleSet::write_key`] whenever nothing is dropped or
    /// folded.
    #[inline]
    pub(crate) fn write_identity_key(&self, line: &str, eol: LineEnding, out: &mut Vec<u8>) {
        out.extend_from_slice(line.as_bytes());
        push_eol_markers(out, eol, self.compare_eol);
    }

    /// The end markers of an identity key whose text was written piecewise.
    #[inline]
    pub(crate) fn push_identity_markers(&self, eol: LineEnding, out: &mut Vec<u8>) {
        push_eol_markers(out, eol, self.compare_eol);
    }

    /// Classifies every char of `line` (§2.13.1) into maximal spans of equal (token, class, importance, case).
    /// Runtime-limit hits of the patterns are added to `errors`. The pattern matches are streamed (memory
    /// O(spans)); callers bound the line length (`TextComparison::spans` classifies at most the first
    /// [`crate::compare::SPAN_LIMIT_BYTES`] of a line).
    pub(crate) fn classify(
        &self,
        line: &str,
        tokens: &[Token],
        cancel: &CancelToken,
        errors: &mut u32,
    ) -> Vec<Span> {
        let mut pats = self.patterns(line, cancel);
        let (lead_end, trail_start) = ws_bounds(line);
        let mut out: Vec<Span> = Vec::with_capacity(tokens.len() + 2);
        for t in tokens {
            let (s, e) = (t.start as usize, t.end as usize);
            let Some(text) = line.get(s..e) else {
                continue;
            };
            let mut current: Option<Span> = None;
            for (off, c) in text.char_indices() {
                let p = s + off;
                let in_pattern = pats.contains(p);
                let (class, important, case_sensitive) = if in_pattern {
                    (CharClass::UnimportantPattern, false, true)
                } else if t.element != ElementId::EVERYTHING_ELSE {
                    (
                        CharClass::Element,
                        self.element_important(t.element),
                        self.element_case(t.element),
                    )
                } else if c.is_whitespace() {
                    (
                        RuleSet::ws_class(p, lead_end, trail_start),
                        self.ws_important(p, lead_end, trail_start),
                        true,
                    )
                } else {
                    (CharClass::Other, self.other, self.other_case)
                };
                let end = (p + c.len_utf8()) as u32;
                match current.as_mut() {
                    Some(span)
                        if span.class == class
                            && span.important == important
                            && span.case_sensitive == case_sensitive =>
                    {
                        span.end = end;
                    }
                    _ => {
                        if let Some(done) = current.take() {
                            out.push(done);
                        }
                        current = Some(Span {
                            start: p as u32,
                            end,
                            element: t.element,
                            class,
                            important,
                            case_sensitive,
                        });
                    }
                }
            }
            if let Some(done) = current {
                out.push(done);
            }
        }
        *errors = errors.saturating_add(pats.errors());
        out
    }
}

/// The writer of one general key: the kept chars, and a `SEP` where a dropped stretch separates two word chars
/// of different tokens, one of them a grammar token.
struct KeyWriter<'o, 'w> {
    out: &'o mut Vec<u8>,
    word_chars: &'w [char],
    /// The last kept char: its token index, its element and whether it is a word char.
    last: Option<(usize, ElementId, bool)>,
    /// Some char was dropped after the last kept char.
    dropped: bool,
}

impl KeyWriter<'_, '_> {
    /// Notes dropped chars (relevant only after a kept char: dropped chars at the start never separate).
    #[inline]
    fn drop_chars(&mut self) {
        if self.last.is_some() {
            self.dropped = true;
        }
    }

    /// Writes `SEP` before a kept char `first` of token `token` (element `element`) when the rule says so.
    #[inline]
    fn boundary(&mut self, token: usize, element: ElementId, first: char) {
        if !self.dropped {
            return;
        }
        self.dropped = false;
        if let Some((t, e, word)) = self.last
            && t != token
            && (e != ElementId::EVERYTHING_ELSE || element != ElementId::EVERYTHING_ELSE)
            && word
            && is_word(first, self.word_chars)
        {
            self.out.push(TAG_SEP);
            push_varint(self.out, u64::from(e.0));
            push_varint(self.out, u64::from(element.0));
        }
    }

    /// Keeps `text` (folded when `!case_sensitive`); false when `cancel` fired while a long text was copied.
    fn keep_str(
        &mut self,
        token: usize,
        element: ElementId,
        text: &str,
        case_sensitive: bool,
        cancel: &CancelToken,
    ) -> bool {
        let (Some(first), Some(last)) = (text.chars().next(), text.chars().next_back()) else {
            return true;
        };
        self.boundary(token, element, first);
        if text.len() <= LINE_CHECK_BYTES {
            append(self.out, text, case_sensitive);
        } else {
            let mut rest = text;
            while !rest.is_empty() {
                if cancel.is_cancelled() {
                    return false;
                }
                let mut cut = rest.len().min(LINE_CHECK_BYTES);
                while !rest.is_char_boundary(cut) {
                    cut += 1;
                }
                let (piece, tail) = rest.split_at(cut);
                append(self.out, piece, case_sensitive);
                rest = tail;
            }
        }
        self.last = Some((token, element, is_word(last, self.word_chars)));
        true
    }

    /// Keeps one char.
    #[inline]
    fn keep_char(&mut self, token: usize, element: ElementId, c: char, case_sensitive: bool) {
        self.boundary(token, element, c);
        push_char(self.out, c, case_sensitive);
        self.last = Some((token, element, is_word(c, self.word_chars)));
    }
}

/// `(end of the leading-whitespace prefix, start of the trailing-whitespace suffix)`; an all-whitespace line is
/// all leading (its trailing suffix is empty).
#[inline]
pub(crate) fn ws_bounds(line: &str) -> (usize, usize) {
    let lead_end = line.len() - line.trim_start().len();
    if lead_end == line.len() {
        (lead_end, line.len())
    } else {
        (lead_end, line.trim_end().len())
    }
}

/// Appends a LEB128 varint (7 bits per byte, least significant first).
fn push_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads a LEB128 varint at `*at`, advancing it; `None` when truncated.
fn read_varint(key: &[u8], at: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *key.get(*at)?;
        *at += 1;
        if shift < 64 {
            v |= u64::from(byte & 0x7F) << shift;
        }
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
    }
}

/// Position after the marker that starts at `at` (`key[at] >= FIRST_TAG`).
fn skip_marker(key: &[u8], at: usize) -> usize {
    match key.get(at) {
        Some(&TAG_SEP) => {
            let mut p = at + 1;
            let _ = read_varint(key, &mut p);
            let _ = read_varint(key, &mut p);
            p.min(key.len())
        }
        Some(&TAG_EOL) => (at + 2).min(key.len()),
        _ => at + 1,
    }
}

/// Appends `text`, folded with the simple 1:1 lower-case mapping when `!case_sensitive`.
#[inline]
fn append(out: &mut Vec<u8>, text: &str, case_sensitive: bool) {
    if case_sensitive {
        out.extend_from_slice(text.as_bytes());
    } else if text.is_ascii() {
        out.extend(text.bytes().map(|b| b.to_ascii_lowercase()));
    } else {
        for c in text.chars() {
            push_char(out, simple_lower(c), true);
        }
    }
}

/// Appends one char, folded when `!case_sensitive`.
#[inline]
fn push_char(out: &mut Vec<u8>, c: char, case_sensitive: bool) {
    let c = if case_sensitive { c } else { simple_lower(c) };
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

/// How a side's keys were built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyShape {
    /// Identity keys (raw lines plus end markers), else general keys.
    pub(crate) identity: bool,
    /// Line endings are part of the key.
    pub(crate) compare_eol: bool,
}

impl KeyShape {
    /// INV-KEY-2: the key has no text (identity path: the line is empty). Text always comes first in a key that
    /// has some, and a `SEP` needs text before it, so this is "the key is empty or starts with a marker".
    #[inline]
    pub(crate) fn is_trivial(self, key: &[u8]) -> bool {
        key.first().is_none_or(|&b| b >= FIRST_TAG)
    }

    /// The first `max_chars` chars of a line's important text (the key without its markers; the raw line on the
    /// identity path) into `out`. O(max_chars): closeness profiles are built from it (§2.15.2).
    pub(crate) fn sample(self, key: &[u8], max_chars: usize, out: &mut String) {
        out.clear();
        let mut at = 0usize;
        let mut chars = 0usize;
        while at < key.len() && chars < max_chars {
            let b = key[at];
            if b >= FIRST_TAG {
                at = skip_marker(key, at);
                continue;
            }
            let len = match b {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF7 => 4,
                _ => 1,
            };
            let end = (at + len).min(key.len());
            match std::str::from_utf8(&key[at..end]) {
                Ok(c) => out.push_str(c),
                Err(_) => out.push('\u{FFFD}'),
            }
            at = end;
            chars += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{TextPattern, UnimportantItem};
    use hikasa_formats::{FormatId, FormatRegistry, LineState, Tokenizer};

    fn rules_with(items: &str) -> TextRules {
        let mut rules = TextRules::default();
        rules
            .importance
            .apply_unimportant(&UnimportantItem::parse_list(items).unwrap());
        rules
    }

    fn key_in(
        rules: &TextRules,
        format: &CompiledFormat,
        line: &str,
        state: LineState,
        eol: LineEnding,
    ) -> Vec<u8> {
        let patterns = compile_patterns(rules, &RegexLimits::default()).unwrap();
        let set = RuleSet::new(rules, format, patterns);
        let mut tokens = Vec::new();
        Tokenizer::new(format.grammar()).tokenize_line(line, 0, state, &mut tokens);
        let mut out = Vec::new();
        set.write_key(line, &tokens, eol, &mut out, &CancelToken::never());
        out
    }

    fn key(rules: &TextRules, format: &CompiledFormat, line: &str) -> Vec<u8> {
        key_in(rules, format, line, LineState::Normal, LineEnding::Lf)
    }

    fn c_cpp() -> Arc<CompiledFormat> {
        FormatRegistry::builtin()
            .unwrap()
            .compiled(&FormatId::new("c-cpp").unwrap())
            .unwrap()
    }

    const SHAPE: KeyShape = KeyShape {
        identity: false,
        compare_eol: false,
    };

    #[test]
    fn varints_round_trip() {
        for v in [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX,
        ] {
            let mut buf = Vec::new();
            push_varint(&mut buf, v);
            let mut at = 0;
            assert_eq!(read_varint(&buf, &mut at), Some(v));
            assert_eq!(at, buf.len());
        }
        assert_eq!(read_varint(&[0x80], &mut 0), None);
    }

    /// Nothing dropped or folded: the general key is the raw line plus its end markers, i.e. the identity key,
    /// whatever the tokenizer's start state (INV-KEY-3).
    #[test]
    fn all_important_keys_are_the_raw_line_in_every_state() {
        let registry = FormatRegistry::builtin().unwrap();
        let rust = registry.compiled(&FormatId::new("rust").unwrap()).unwrap();
        let rules = TextRules::default();
        assert!(rules.is_identity(&rust));
        let set = RuleSet::new(
            &rules,
            &rust,
            compile_patterns(&rules, &RegexLimits::default()).unwrap(),
        );
        let line = "let x = 1; /* c */ \"s\" // tail";
        // The line as code, and the same line inside a block comment (another start state).
        let mut tokens = Vec::new();
        let mut tok = Tokenizer::new(rust.grammar());
        let inside = tok
            .tokenize_line("/* open", 0, LineState::Normal, &mut tokens)
            .end_state;
        assert_ne!(inside, LineState::Normal);
        let mut identity = Vec::new();
        set.write_identity_key(line, LineEnding::Lf, &mut identity);
        assert_eq!(identity, line.as_bytes());
        for state in [LineState::Normal, inside] {
            assert_eq!(key_in(&rules, &rust, line, state, LineEnding::Lf), identity);
        }
        // A never-matching unimportant pattern changes nothing either.
        let mut never = TextRules::default();
        never
            .importance
            .unimportant
            .push(TextPattern::plain("§never§"));
        for state in [LineState::Normal, inside] {
            assert_eq!(key_in(&never, &rust, line, state, LineEnding::Lf), identity);
        }
    }

    /// INV-KEY-1: crafted collisions do not collide (text bytes never look like markers).
    #[test]
    fn crafted_collisions_differ() {
        let c = c_cpp();
        let ws_off = rules_with("whitespace");
        // `int x` vs `intx`: a dropped space between two word chars of different tokens (D-14).
        assert_ne!(key(&ws_off, &c, "int x;"), key(&ws_off, &c, "intx;"));
        assert_eq!(key(&ws_off, &c, "int  x;"), key(&ws_off, &c, "int x;"));
        // Operators need no separator: `a + b` = `a+b` with whitespace unimportant.
        assert_eq!(key(&ws_off, &c, "x = a + b;"), key(&ws_off, &c, "x=a+b;"));
        assert_eq!(
            key(&ws_off, &c, "return (x);"),
            key(&ws_off, &c, "return(x);")
        );
        // Text that contains control chars is still text.
        let ee = CompiledFormat::everything_else();
        let plain = TextRules::default();
        assert_ne!(key(&plain, &ee, "\u{1}\u{1}a"), key(&plain, &ee, "a"));
        // Different texts never give equal keys when nothing is dropped.
        assert_ne!(key(&plain, &c, "ab \"c\""), key(&plain, &c, "a \"bc\""));
        // The separator carries both elements.
        let k = key(&ws_off, &c, "int x;");
        let sep = k.iter().position(|&b| b == TAG_SEP).unwrap();
        assert_eq!(&k[..sep], b"int");
        assert_eq!(k.last(), Some(&b';'));
    }

    #[test]
    fn d14_text_merges_across_removed_whitespace() {
        let ee = CompiledFormat::everything_else();
        let ws_off = rules_with("whitespace");
        assert_eq!(key(&ws_off, &ee, "a b"), key(&ws_off, &ee, "ab"));
        assert_eq!(key(&ws_off, &ee, "  a\tb  "), key(&ws_off, &ee, "ab"));
        // Trailing only.
        let trail_off = rules_with("trailing-ws");
        assert_eq!(key(&trail_off, &ee, "a b  "), key(&trail_off, &ee, "a b"));
        assert_ne!(key(&trail_off, &ee, "a  b"), key(&trail_off, &ee, "a b"));
        // An all-blank line is leading whitespace.
        let lead_off = rules_with("leading-ws");
        assert!(key(&lead_off, &ee, "   ").is_empty());
        assert!(SHAPE.is_trivial(&key(&lead_off, &ee, "   ")));
        assert!(!SHAPE.is_trivial(&key(&lead_off, &ee, "  x")));
    }

    #[test]
    fn patterns_classes_and_spans() {
        let ee = CompiledFormat::everything_else();
        let mut rules = TextRules::default();
        rules
            .importance
            .unimportant
            .push(TextPattern::regex(r"\$Id:[^$]*\$"));
        rules.importance.unimportant.push(TextPattern {
            pattern: "TODO".into(),
            regex: false,
            match_case: false,
        });
        assert!(key(&rules, &ee, "$Id: 12 $").is_empty());
        assert_eq!(key(&rules, &ee, "a todo b"), key(&rules, &ee, "a ToDo b"));
        let patterns = compile_patterns(&rules, &RegexLimits::default()).unwrap();
        let set = RuleSet::new(&rules, &ee, patterns);
        let mut errors = 0;
        assert_eq!(
            set.pattern_ranges("x todo $Id: 1 $ TODO", &mut errors),
            [(2, 6), (7, 15), (16, 20)]
        );
        let line = "  ab todo  ";
        let tokens = [Token {
            start: 0,
            end: line.len() as u32,
            element: ElementId::EVERYTHING_ELSE,
            item: None,
        }];
        let spans: Vec<(Range<u32>, CharClass, bool)> = set
            .classify(line, &tokens, &CancelToken::never(), &mut errors)
            .into_iter()
            .map(|s| (s.start..s.end, s.class, s.important))
            .collect();
        assert_eq!(
            spans,
            [
                (0..2, CharClass::LeadingWhitespace, true),
                (2..4, CharClass::Other, true),
                (4..5, CharClass::EmbeddedWhitespace, true),
                (5..9, CharClass::UnimportantPattern, false),
                (9..11, CharClass::TrailingWhitespace, true),
            ]
        );
        assert_eq!(errors, 0);
    }

    /// Overlapping and touching matches of several patterns merge; the streamed union equals the sorted union.
    #[test]
    fn pattern_matches_merge_like_a_sorted_union() {
        let ee = CompiledFormat::everything_else();
        let mut rules = TextRules::default();
        for p in ["ab", "bc", "cd", "x", "a"] {
            rules.importance.unimportant.push(TextPattern::plain(p));
        }
        let set = RuleSet::new(
            &rules,
            &ee,
            compile_patterns(&rules, &RegexLimits::default()).unwrap(),
        );
        let mut errors = 0;
        // "abcd": ab(0,2) bc(1,3) cd(2,4) → (0,4); "x x": (5,6) (7,8); "aaa": a, a, a touching → (9,12).
        assert_eq!(
            set.pattern_ranges("abcd x x aaa", &mut errors),
            [(0, 4), (5, 6), (7, 8), (9, 12)]
        );
        // Zero-length matches mark nothing.
        let mut zero = TextRules::default();
        zero.importance.unimportant.push(TextPattern::regex("y*"));
        let set = RuleSet::new(
            &zero,
            &ee,
            compile_patterns(&zero, &RegexLimits::default()).unwrap(),
        );
        assert_eq!(set.pattern_ranges("ab yy c", &mut errors), [(3, 5)]);
    }

    /// The cursor stops at once when the token fires and keeps nothing in memory per match.
    #[test]
    fn a_cancelled_cursor_reports_no_more_matches() {
        let ee = CompiledFormat::everything_else();
        let mut rules = TextRules::default();
        rules.importance.unimportant.push(TextPattern::plain("x"));
        let set = RuleSet::new(
            &rules,
            &ee,
            compile_patterns(&rules, &RegexLimits::default()).unwrap(),
        );
        let line = "yx".repeat(1000);
        let cancel = CancelToken::new();
        let mut cursor = set.patterns(&line, &cancel);
        assert_eq!(cursor.at(0), Some((1, 2)));
        cancel.cancel();
        // The match found before the cancellation is still reported; no regex call runs afterwards.
        assert_eq!(cursor.at(2), Some((3, 4)));
        assert!(cursor.cancelled());
        assert_eq!(cursor.at(4), None);
    }

    #[test]
    fn case_folding_of_everything_else() {
        let ee = CompiledFormat::everything_else();
        let case_off = rules_with("case");
        assert_eq!(
            key(&case_off, &ee, "Hello ÄÖ"),
            key(&case_off, &ee, "hello äö")
        );
        assert_ne!(
            key(&TextRules::default(), &ee, "Hello"),
            key(&TextRules::default(), &ee, "hello")
        );
    }

    #[test]
    fn end_markers_and_identity_keys() {
        let ee = CompiledFormat::everything_else();
        let tokens = [Token {
            start: 0,
            end: 1,
            element: ElementId::EVERYTHING_ELSE,
            item: None,
        }];
        let mut rules = TextRules::default();
        rules.importance.compare_line_endings = true;
        let set = RuleSet::new(
            &rules,
            &ee,
            compile_patterns(&rules, &RegexLimits::default()).unwrap(),
        );
        let general = |eol| {
            let mut out = Vec::new();
            set.write_key("x", &tokens, eol, &mut out, &CancelToken::never());
            out
        };
        let identity = |eol| {
            let mut out = Vec::new();
            set.write_identity_key("x", eol, &mut out);
            out
        };
        for (eol, expected) in [
            (LineEnding::Lf, vec![b'x', TAG_EOL, 1]),
            (LineEnding::CrLf, vec![b'x', TAG_EOL, 2]),
            (LineEnding::Cr, vec![b'x', TAG_EOL, 3]),
            (LineEnding::None, vec![b'x', TAG_EOL, 4]),
            (LineEnding::Artificial, vec![b'x', TAG_ART]),
        ] {
            assert_eq!(general(eol), expected, "{eol:?}");
            assert_eq!(identity(eol), expected, "{eol:?}");
        }
        // Without compared endings, only an artificial break is marked (TC-003).
        let plain = RuleSet::new(
            &TextRules::default(),
            &ee,
            compile_patterns(&TextRules::default(), &RegexLimits::default()).unwrap(),
        );
        let mut lf = Vec::new();
        plain.write_identity_key("x", LineEnding::Lf, &mut lf);
        let mut art = Vec::new();
        plain.write_identity_key("x", LineEnding::Artificial, &mut art);
        assert_eq!(lf, b"x");
        assert_eq!(art, [b'x', TAG_ART]);
        let shape = KeyShape {
            identity: true,
            compare_eol: true,
        };
        assert!(shape.is_trivial(&[TAG_EOL, 4]));
        assert!(shape.is_trivial(&[TAG_ART]));
        assert!(shape.is_trivial(&[]));
        assert!(!shape.is_trivial(&art));
        let mut sample = String::new();
        shape.sample(&art, 10, &mut sample);
        assert_eq!(sample, "x");
    }

    #[test]
    fn samples_skip_markers_and_stop_early() {
        let c = c_cpp();
        let ws_off = rules_with("whitespace");
        let k = key(&ws_off, &c, "int é x;");
        let mut s = String::new();
        SHAPE.sample(&k, 100, &mut s);
        assert_eq!(s, "intéx;");
        SHAPE.sample(&k, 4, &mut s);
        assert_eq!(s, "inté");
        SHAPE.sample(&[], 4, &mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn ws_bounds_table() {
        assert_eq!(ws_bounds(""), (0, 0));
        assert_eq!(ws_bounds("   "), (3, 3));
        assert_eq!(ws_bounds("  a b  "), (2, 5));
        assert_eq!(ws_bounds("a"), (0, 1));
    }

    #[test]
    fn char_class_names() {
        for class in [
            CharClass::UnimportantPattern,
            CharClass::Element,
            CharClass::LeadingWhitespace,
            CharClass::EmbeddedWhitespace,
            CharClass::TrailingWhitespace,
            CharClass::Other,
        ] {
            assert_eq!(
                serde_json::to_string(&class).unwrap(),
                format!("\"{}\"", class.as_str())
            );
        }
    }
}
