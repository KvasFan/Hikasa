//! The tokenizer (FF-020…FF-027, §2.10; D-13, D-41, D-58): [`Token`], [`LineState`], [`LineOutcome`],
//! [`Tokenizer`], and the parallel [`tokenize_all`].
//!
//! A line is scanned left to right; at each position the grammar items are tried in list order and the first
//! match wins (FF-026). Text that no item covers is "Everything Else" (`ElementId(0)`). Delimited and Lines
//! items may carry over to the next line through [`LineState`]; a line's tokens depend only on
//! `(line, line_no, start state)` (INV-TOK-4).

mod parallel;
pub(crate) mod scan;

use serde::{Deserialize, Serialize};

pub use parallel::{LineBytes, LineSource, TokenizedLines, tokenize_all, tokenize_all_with_chunk};

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_core::error::InvariantViolation;

use crate::compile::{CompiledGrammar, ElementId, ElementTable};
use scan::{CacheSlot, Cx, ItemMatcher, char_len_at, find_close};

/// Tokenizer state carried from one line to the next (INV-TOK-4: it depends only on preceding lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LineState {
    /// Not inside a multi-line construct.
    #[default]
    Normal,
    /// Inside multi-line Delimited item `item`.
    InDelimited {
        /// Grammar item index.
        item: u16,
    },
    /// Inside a Lines item with `remaining` lines still to cover.
    InLines {
        /// Grammar item index.
        item: u16,
        /// Lines still covered after this one starts.
        remaining: u32,
    },
}

/// A token: byte range within one line (no EOL) and its element (INV-TOK-1..3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Token {
    /// Start byte (char boundary).
    pub start: u32,
    /// End byte (exclusive, char boundary, > start).
    pub end: u32,
    /// Element (`EVERYTHING_ELSE` for uncovered text).
    pub element: ElementId,
    /// Grammar item index; `None` for everything-else runs.
    pub item: Option<u16>,
}

/// Checks INV-TOK-1..3 for the tokens of one line: every token is non-empty; the tokens are sorted, contiguous and
/// cover `[0, len)` exactly; boundaries are char boundaries; "Everything Else" tokens (and only they) have no
/// item, and two of them are never adjacent.
pub fn check_tokens(line: &str, tokens: &[Token]) -> Result<(), InvariantViolation> {
    let fail = |id: &'static str, detail: String| Err(InvariantViolation { id, detail });
    let mut pos = 0u32;
    let mut previous_ee = false;
    for t in tokens {
        if t.end <= t.start {
            return fail("INV-TOK-1", format!("empty token {t:?}"));
        }
        if t.start != pos {
            return fail("INV-TOK-2", format!("token {t:?} does not start at {pos}"));
        }
        if !line.is_char_boundary(t.start as usize) || !line.is_char_boundary(t.end as usize) {
            return fail("INV-TOK-3", format!("token {t:?} splits a char"));
        }
        let ee = t.item.is_none();
        if ee != (t.element == ElementId::EVERYTHING_ELSE) {
            return fail(
                "INV-TOK-3",
                format!("token {t:?}: item and element disagree"),
            );
        }
        if ee && previous_ee {
            return fail(
                "INV-TOK-3",
                format!("adjacent everything-else tokens at {pos}"),
            );
        }
        previous_ee = ee;
        pos = t.end;
    }
    if pos as usize != line.len() {
        return fail(
            "INV-TOK-2",
            format!("tokens cover {pos} of {} bytes", line.len()),
        );
    }
    Ok(())
}

impl Token {
    /// The token's text in `line`.
    pub fn text<'l>(&self, line: &'l str) -> &'l str {
        line.get(self.start as usize..self.end as usize)
            .unwrap_or("")
    }
}

/// Per-line tokenizer result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineOutcome {
    /// State at the start of the next line.
    pub end_state: LineState,
    /// fancy-regex runtime-limit hits on this line (treated as "no match").
    pub regex_errors: u32,
}

/// A successful item match at the scan position.
#[derive(Debug, Clone, Copy)]
struct Hit {
    end: usize,
    /// The token ended because the line ended (D-13): the rest of the line is everything-else whitespace.
    eol: bool,
    next: LineState,
}

/// Appends tokens, dropping empty ones and merging adjacent everything-else runs (INV-TOK-1, INV-TOK-3).
struct Emit<'o> {
    out: &'o mut Vec<Token>,
}

impl Emit<'_> {
    #[inline]
    fn ee(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        if let Some(last) = self.out.last_mut()
            && last.item.is_none()
            && last.end as usize == start
        {
            last.end = end as u32;
            return;
        }
        self.out.push(Token {
            start: start as u32,
            end: end as u32,
            element: ElementId::EVERYTHING_ELSE,
            item: None,
        });
    }

    #[inline]
    fn item(&mut self, item: usize, element: ElementId, start: usize, end: usize) {
        if start >= end {
            return;
        }
        self.out.push(Token {
            start: start as u32,
            end: end as u32,
            element,
            item: Some(item as u16),
        });
    }
}

/// Length of the leading-whitespace prefix.
#[inline]
fn leading_ws(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Number of chars in `s` (fast for ASCII).
#[inline]
fn char_count(s: &str) -> u32 {
    let n = if s.is_ascii() {
        s.len()
    } else {
        s.chars().count()
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Single-threaded tokenizer with reusable scratch buffers.
pub struct Tokenizer<'g> {
    grammar: &'g CompiledGrammar,
    caches: Vec<CacheSlot>,
    epoch: u64,
    fold: String,
    reference: bool,
    all_items: Vec<u16>,
    /// Byte offsets of the current line's field boundaries (grammars with Columns items).
    fences: Vec<usize>,
}

impl<'g> Tokenizer<'g> {
    /// New tokenizer for `grammar`.
    pub fn new(grammar: &'g CompiledGrammar) -> Tokenizer<'g> {
        Tokenizer {
            grammar,
            caches: Vec::new(),
            epoch: 0,
            fold: String::new(),
            reference: false,
            all_items: Vec::new(),
            fences: Vec::new(),
        }
    }

    /// A tokenizer with every fast path switched off (no prefilter, no next-match cache, no accelerator, no
    /// keyword hash set, no `memchr` jumps): the plain algorithm of §2.10.2, with an anchored regex attempt at
    /// every position, used by tests to check that the fast paths never change a result.
    #[doc(hidden)]
    pub fn reference(grammar: &'g CompiledGrammar) -> Tokenizer<'g> {
        let mut t = Tokenizer::new(grammar);
        t.reference = true;
        t.all_items = (0..grammar.items.len())
            .map(|k| u16::try_from(k).unwrap_or(u16::MAX))
            .collect();
        t
    }

    /// The grammar.
    pub fn grammar(&self) -> &'g CompiledGrammar {
        self.grammar
    }

    /// Tokenizes one line (`line_no` is 0-based; needed for `or_line_one`). Clears and fills `out`.
    ///
    /// A `state` that does not fit the grammar (an item index out of range or of the wrong kind) is treated as
    /// `Normal` (T9).
    pub fn tokenize_line(
        &mut self,
        line: &str,
        line_no: u32,
        state: LineState,
        out: &mut Vec<Token>,
    ) -> LineOutcome {
        // Without a token nothing can cancel the run.
        self.run(line, line_no, state, out, None)
            .unwrap_or(LineOutcome {
                end_state: LineState::Normal,
                regex_errors: 0,
            })
    }

    /// [`Tokenizer::tokenize_line`] that also observes `cancel` inside the line: every 64 KiB of scanned text
    /// and before every regex engine call, so that a single multi-megabyte line still returns within the
    /// cancellation latency (§5.3, G4). On cancellation `out` is cleared and `Err(Cancelled)` returned.
    pub fn tokenize_line_cancellable(
        &mut self,
        line: &str,
        line_no: u32,
        state: LineState,
        out: &mut Vec<Token>,
        cancel: &CancelToken,
    ) -> Result<LineOutcome, Cancelled> {
        self.run(line, line_no, state, out, Some(cancel))
    }

    fn run(
        &mut self,
        line: &str,
        line_no: u32,
        state: LineState,
        out: &mut Vec<Token>,
        cancel: Option<&CancelToken>,
    ) -> Result<LineOutcome, Cancelled> {
        out.clear();
        let grammar = self.grammar;
        self.fences.clear();
        if !grammar.fences.is_empty() {
            fence_offsets(line, &grammar.fences, &mut self.fences);
        }
        let mut cx = Cx::new(
            line,
            &mut self.epoch,
            &mut self.caches,
            grammar.regex_slots,
            self.reference,
            &mut self.fold,
            cancel,
        );
        let mut em = Emit { out };
        let end_state = scan_line(
            grammar,
            &self.all_items,
            &self.fences,
            &mut cx,
            &mut em,
            line_no,
            state,
        );
        if cx.cancelled {
            em.out.clear();
            return Err(Cancelled);
        }
        debug_assert_eq!(check_tokens(line, em.out), Ok(()));
        Ok(LineOutcome {
            end_state,
            regex_errors: cx.errors,
        })
    }
}

/// Byte offsets of the field boundaries (char columns `cols`, sorted and unique) that lie inside `line`.
fn fence_offsets(line: &str, cols: &[u32], out: &mut Vec<usize>) {
    if line.is_ascii() {
        out.extend(
            cols.iter()
                .map(|&c| c as usize)
                .take_while(|&c| c < line.len()),
        );
        return;
    }
    let mut want = cols.iter().copied().peekable();
    for (k, (at, _)) in line.char_indices().enumerate() {
        let Some(&next) = want.peek() else {
            break;
        };
        if k == next as usize {
            out.push(at);
            want.next();
        }
    }
}

/// The per-line algorithm of §2.10.2.
///
/// Field boundaries (`fences`, the byte offsets of the Columns items' start columns): the haystack of every
/// item is the line up to the next boundary, so no token crosses one. A token that ends because its field
/// ends is end-of-line terminated for that field (D-13 trimming at the boundary); the scan goes on at the
/// boundary. The line's end state is the non-`Normal` next state of the last end-of-line-terminated token
/// (a multi-line construct continues on the next line), or `Normal`. Without boundaries this is the plain
/// algorithm: the first end-of-line-terminated token ends the line.
fn scan_line(
    g: &CompiledGrammar,
    all_items: &[u16],
    fences: &[usize],
    cx: &mut Cx<'_>,
    em: &mut Emit<'_>,
    line_no: u32,
    state: LineState,
) -> LineState {
    let full = cx.line;
    let bytes = full.as_bytes();
    let len = bytes.len();
    if g.items.is_empty() {
        // "Everything Else" only: one token for the whole line.
        em.ee(0, len);
        return LineState::Normal;
    }
    let mut next_fence = 0usize;
    let mut seg_end = len;
    if let Some(&f) = fences.first() {
        seg_end = f;
        next_fence = 1;
        cx.set_haystack(full.get(..f).unwrap_or(full));
    }
    let mut pending = LineState::Normal;
    let mut p = 0usize;

    // (a) Continuation of a multi-line construct.
    match state {
        LineState::InLines { item, remaining } => {
            let k = usize::from(item);
            if let Some(it) = g.items.get(k)
                && matches!(it.matcher, ItemMatcher::Lines { .. })
                && remaining > 0
            {
                let lead = leading_ws(cx.line);
                let t = cx.trailing().max(lead);
                em.ee(0, lead);
                em.item(k, it.element, lead, t);
                em.ee(t, seg_end);
                let next = if remaining > 1 {
                    LineState::InLines {
                        item,
                        remaining: remaining - 1,
                    }
                } else {
                    LineState::Normal
                };
                if seg_end == len {
                    return next;
                }
                pending = next;
                p = seg_end;
            }
        }
        LineState::InDelimited { item } => {
            let k = usize::from(item);
            if let Some(it) = g.items.get(k)
                && it.multiline
                && let ItemMatcher::Delimited {
                    to: Some(to),
                    escape,
                    ..
                } = &it.matcher
            {
                let lead = leading_ws(cx.line);
                em.ee(0, lead);
                match find_close(to, *escape, cx, lead) {
                    Some(end) => {
                        em.item(k, it.element, lead, end);
                        p = end;
                    }
                    None => {
                        let t = cx.trailing().max(lead);
                        em.item(k, it.element, lead, t);
                        em.ee(t, seg_end);
                        if seg_end == len {
                            return state;
                        }
                        pending = state;
                        p = seg_end;
                    }
                }
            }
        }
        LineState::Normal => {}
    }

    // (b) Left-to-right scan; the first matching item in list order wins (FF-026).
    let mut col = if g.has_columns {
        char_count(full.get(..p).unwrap_or(""))
    } else {
        0
    };
    let mut ee_start = p;
    loop {
        if p >= seg_end {
            if seg_end >= len {
                break;
            }
            // The next field.
            seg_end = fences.get(next_fence).copied().unwrap_or(len);
            next_fence += 1;
            cx.set_haystack(full.get(..seg_end).unwrap_or(full));
            continue;
        }
        cx.progress(p);
        if cx.cancelled {
            return LineState::Normal;
        }
        let b = bytes[p];
        if !cx.reference && b < 128 && p > 0 && g.prefilter.skip_mid[usize::from(b)] {
            p += 1;
            col += 1;
            continue;
        }
        let candidates = if cx.reference {
            all_items
        } else {
            g.prefilter.candidates(usize::from(b.min(128)), p == 0)
        };
        let mut found = None;
        for &k in candidates {
            if let Some(hit) = try_item(g, usize::from(k), cx, p, col, line_no) {
                found = Some((usize::from(k), hit));
                break;
            }
        }
        match found {
            Some((k, hit)) => {
                em.ee(ee_start, p);
                let element = g
                    .items
                    .get(k)
                    .map_or(ElementId::EVERYTHING_ELSE, |i| i.element);
                em.item(k, element, p, hit.end);
                if hit.eol {
                    em.ee(hit.end, seg_end);
                    if hit.next != LineState::Normal {
                        pending = hit.next;
                    }
                    if seg_end == len {
                        return pending;
                    }
                    if g.has_columns {
                        col += char_count(full.get(p..seg_end).unwrap_or(""));
                    }
                    p = seg_end;
                } else {
                    if g.has_columns {
                        col += char_count(full.get(p..hit.end).unwrap_or(""));
                    }
                    p = hit.end;
                }
                ee_start = p;
            }
            None => {
                p += char_len_at(full, p);
                col += 1;
            }
        }
    }
    em.ee(ee_start, len);
    pending
}

/// `first_match` for one item (§2.10.2 table).
#[inline]
fn try_item(
    g: &CompiledGrammar,
    k: usize,
    cx: &mut Cx<'_>,
    p: usize,
    col: u32,
    line_no: u32,
) -> Option<Hit> {
    let item = g.items.get(k)?;
    let normal = |end| Hit {
        end,
        eol: false,
        next: LineState::Normal,
    };
    match &item.matcher {
        ItemMatcher::Basic(m) => m.match_at(cx, p).map(normal),
        ItemMatcher::Delimited {
            from, to, escape, ..
        } => {
            let from_end = from.match_at(cx, p)?;
            if let Some(to) = to
                && let Some(end) = find_close(to, *escape, cx, from_end)
            {
                return Some(normal(end));
            }
            let end = cx.trailing().max(from_end);
            let next = if item.multiline {
                LineState::InDelimited { item: k as u16 }
            } else {
                LineState::Normal
            };
            Some(Hit {
                end,
                eol: true,
                next,
            })
        }
        ItemMatcher::List(list) => list.match_at(cx, p, &g.word).map(normal),
        ItemMatcher::Columns { from, to } => {
            if col != *from {
                return None;
            }
            match to {
                Some(to) => {
                    let line = cx.line;
                    let mut end = p;
                    for _ in 0..(to - from) {
                        if end >= line.len() {
                            break;
                        }
                        end += char_len_at(line, end);
                    }
                    Some(normal(end.min(line.len())))
                }
                None => {
                    let end = cx.trailing();
                    (end > p).then_some(Hit {
                        end,
                        eol: true,
                        next: LineState::Normal,
                    })
                }
            }
        }
        ItemMatcher::Lines {
            heading,
            next_lines,
            or_line_one,
        } => {
            if p != 0 {
                return None;
            }
            let started = (*or_line_one && line_no == 0)
                || heading.as_ref().is_some_and(|h| h.found_anywhere(cx));
            if !started {
                return None;
            }
            let end = cx.trailing();
            if end == 0 {
                return None;
            }
            let next = if *next_lines > 0 {
                LineState::InLines {
                    item: k as u16,
                    remaining: *next_lines,
                }
            } else {
                LineState::Normal
            };
            Some(Hit {
                end,
                eol: true,
                next,
            })
        }
    }
}

/// Escapes token text for the one-record-per-line text dump: `\` → `\\`, tab → `\t`, CR → `\r`, LF → `\n`,
/// other control characters → `\u{XX}`.
pub fn escape_token_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out
}

/// Appends the text dump of one line's tokens (the human format of `hikasa format tokens`):
/// `L<line>:<start>-<end>\t<element>\t<escaped text>\n` per token, with the 1-based line number and 0-based
/// byte offsets.
pub fn write_token_dump(
    out: &mut String,
    line_number: u32,
    line: &str,
    tokens: &[Token],
    elements: &ElementTable,
) {
    use std::fmt::Write as _;
    for t in tokens {
        let _ = writeln!(
            out,
            "L{line_number}:{}-{}\t{}\t{}",
            t.start,
            t.end,
            elements.name(t.element),
            escape_token_text(t.text(line))
        );
    }
}

#[cfg(test)]
mod tests;
