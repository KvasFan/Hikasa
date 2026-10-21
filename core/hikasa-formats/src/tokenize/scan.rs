//! Item matchers, the first-char prefilter data, word classes and the next-match cache (§2.10.2, §2.10.5).
//!
//! Everything here is an implementation detail of [`super::Tokenizer`]. The fast paths (prefilter, regex
//! next-match cache, the simple-pattern accelerator, keyword hash set, `memchr` jumps in closing-delimiter
//! searches) must never change a result; the tokenizer's reference mode switches them all off (one anchored
//! regex attempt per position), and property tests compare both modes, regex error counts included.

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};

use hikasa_core::cancel::CancelToken;
use hikasa_core::case::simple_lower;

use crate::compile::ElementId;
use crate::regex_engine::{Accel, FirstChars, Pattern};

/// FxHash-style hasher for keyword lookups: fast on short strings and deterministic (no random state, T1).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct WordHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl WordHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for WordHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for &chunk in chunks {
            self.add(u64::from_le_bytes(chunk));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word) ^ ((rest.len() as u64) << 56));
        }
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(u64::from(i));
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type WordSet = HashSet<Box<str>, BuildHasherDefault<WordHasher>>;

/// `is_word(c) = c.is_alphanumeric() || c == '_' || word_chars.contains(c)` (§2.10.1, D-58).
#[derive(Debug, Clone)]
pub(crate) struct WordClass {
    ascii: u128,
    extra: Vec<char>,
}

impl WordClass {
    pub(crate) fn new(word_chars: &str) -> WordClass {
        let mut ascii = 1u128 << b'_';
        for b in 0u8..128 {
            if b.is_ascii_alphanumeric() {
                ascii |= 1u128 << b;
            }
        }
        let mut extra = Vec::new();
        for c in word_chars.chars() {
            if c.is_ascii() {
                ascii |= 1u128 << (c as u32);
            } else if !extra.contains(&c) {
                extra.push(c);
            }
        }
        WordClass { ascii, extra }
    }

    #[inline]
    pub(crate) fn is_word(&self, c: char) -> bool {
        if c.is_ascii() {
            self.ascii & (1u128 << (c as u32)) != 0
        } else {
            c.is_alphanumeric() || self.extra.contains(&c)
        }
    }

    /// End of the maximal word starting at `p`.
    #[inline]
    pub(crate) fn word_end(&self, line: &str, p: usize) -> usize {
        let bytes = line.as_bytes();
        let mut q = p;
        while q < bytes.len() {
            let b = bytes[q];
            if b < 128 {
                if self.ascii & (1u128 << b) == 0 {
                    return q;
                }
                q += 1;
            } else {
                match line.get(q..).and_then(|rest| rest.chars().next()) {
                    Some(c) if self.is_word(c) => q += c.len_utf8(),
                    _ => return q,
                }
            }
        }
        q
    }

    /// True when the char before `p` is a word char.
    #[inline]
    pub(crate) fn word_before(&self, line: &str, p: usize) -> bool {
        if p == 0 {
            return false;
        }
        let b = line.as_bytes()[p - 1];
        if b < 128 {
            return self.ascii & (1u128 << b) != 0;
        }
        line.get(..p)
            .and_then(|s| s.chars().next_back())
            .is_some_and(|c| self.is_word(c))
    }

    /// True when the char at `p` is a word char.
    #[inline]
    pub(crate) fn word_at(&self, line: &str, p: usize) -> bool {
        match line.as_bytes().get(p) {
            None => false,
            Some(&b) if b < 128 => self.ascii & (1u128 << b) != 0,
            Some(_) => line
                .get(p..)
                .and_then(|s| s.chars().next())
                .is_some_and(|c| self.is_word(c)),
        }
    }
}

/// One next-match cache entry of a regex matcher for the current haystack (`epoch`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CacheSlot {
    epoch: u64,
    /// The leftmost match found by `find_at(haystack, from)`: valid for positions in `from..=start` (no match
    /// starts in `from..start`), or for every position `>= from` when `None`.
    from: usize,
    found: Option<(usize, usize)>,
    /// `found` holds a result (false until the first unanchored search of this haystack).
    valid: bool,
    /// Bytes charged to unanchored searches on this haystack (`end of match or haystack − start`).
    charged: usize,
    /// Unanchored searches are off for the rest of this haystack (their budget ran out, or one hit a
    /// fancy-regex runtime limit): every position gets an anchored attempt instead.
    anchored_only: bool,
}

/// An accelerator decision that examined at most this many bytes is used directly; a longer failed scan
/// primes the next-match cache instead, so that the next positions of the same run are answered in O(1).
const SHORT_SCAN: usize = 16;

/// Cancellation is checked whenever the scan position passes another `CANCEL_STRIDE` bytes (§5.3; the regex
/// engine calls check it too, each time).
pub(crate) const CANCEL_STRIDE: usize = 64 * 1024;

/// Per-haystack matching state: the haystack, the cache epoch, the caches, the error count and the
/// cancellation check.
///
/// The haystack is the line, or the line up to the next field boundary of a grammar with Columns items
/// (§2.10.2): items see the text before the boundary as the whole line (`$`, word boundaries and end-of-line
/// handling apply at the boundary), while `^` and look-behind still see the real start of the line.
pub(crate) struct Cx<'a> {
    pub(crate) line: &'a str,
    epoch: u64,
    /// The tokenizer's epoch counter: every haystack gets a fresh epoch, so caches never leak between lines
    /// or fields.
    epochs: &'a mut u64,
    /// Next-match caches, allocated on the first regex search (ASCII input on simple patterns never needs
    /// one, §2.10.5).
    caches: &'a mut Vec<CacheSlot>,
    slots: usize,
    pub(crate) reference: bool,
    pub(crate) errors: u32,
    trailing: Option<usize>,
    pub(crate) fold: &'a mut String,
    cancel: Option<&'a CancelToken>,
    next_check: usize,
    /// Set once a cancellation was observed: matchers then report "no match" and the scan stops.
    pub(crate) cancelled: bool,
}

impl<'a> Cx<'a> {
    pub(crate) fn new(
        line: &'a str,
        epochs: &'a mut u64,
        caches: &'a mut Vec<CacheSlot>,
        slots: usize,
        reference: bool,
        fold: &'a mut String,
        cancel: Option<&'a CancelToken>,
    ) -> Cx<'a> {
        *epochs += 1;
        Cx {
            line,
            epoch: *epochs,
            epochs,
            caches,
            slots,
            reference,
            errors: 0,
            trailing: None,
            fold,
            cancel,
            next_check: CANCEL_STRIDE,
            cancelled: false,
        }
    }

    /// Switches to another haystack (the line up to the next field boundary): fresh caches.
    pub(crate) fn set_haystack(&mut self, haystack: &'a str) {
        *self.epochs += 1;
        self.epoch = *self.epochs;
        self.line = haystack;
        self.trailing = None;
    }

    /// Start of the trailing-whitespace suffix of the haystack (computed once per haystack).
    #[inline]
    pub(crate) fn trailing(&mut self) -> usize {
        match self.trailing {
            Some(t) => t,
            None => {
                let t = self.line.trim_end().len();
                self.trailing = Some(t);
                t
            }
        }
    }

    /// Checks the cancellation token when the scan has advanced another [`CANCEL_STRIDE`] bytes.
    #[inline]
    pub(crate) fn progress(&mut self, p: usize) {
        if p >= self.next_check {
            self.next_check = p.saturating_add(CANCEL_STRIDE);
            self.check_cancel();
        }
    }

    /// Checks the cancellation token (before every regex engine call: one call may scan the whole line).
    #[inline]
    fn check_cancel(&mut self) {
        if !self.cancelled && self.cancel.is_some_and(CancelToken::is_cancelled) {
            self.cancelled = true;
        }
    }

    /// The cache slot of a regex matcher for this haystack (reset when the haystack changed).
    fn slot(&mut self, slot: usize) -> Option<&mut CacheSlot> {
        if self.caches.len() < self.slots {
            self.caches.resize(self.slots, CacheSlot::default());
        }
        let epoch = self.epoch;
        let c = self.caches.get_mut(slot)?;
        if c.epoch != epoch {
            *c = CacheSlot {
                epoch,
                ..CacheSlot::default()
            };
        }
        Some(c)
    }

    /// What the next-match cache knows about a match starting at `p`: `Some(None)` no match, `Some(Some(end))`
    /// the match end (possibly `p` for an empty match), `None` unknown.
    #[inline]
    fn cached(&self, slot: usize, p: usize) -> Option<Option<usize>> {
        let c = self.caches.get(slot)?;
        if c.epoch != self.epoch || !c.valid || c.from > p {
            return None;
        }
        match c.found {
            None => Some(None),
            Some((s, _)) if s > p => Some(None),
            Some((s, e)) if s == p => Some(Some(e)),
            Some(_) => None,
        }
    }

    fn anchored_only(&self, slot: usize) -> bool {
        self.caches
            .get(slot)
            .is_some_and(|c| c.epoch == self.epoch && c.anchored_only)
    }

    /// An anchored attempt at `p` (exact, §2.10.2); a runtime-limit hit counts as "no match" and is counted.
    fn anchored(&mut self, pattern: &Pattern, p: usize) -> Option<usize> {
        self.check_cancel();
        if self.cancelled {
            return None;
        }
        match pattern.match_anchored(self.line, p) {
            Ok(end) => end,
            Err(_) => {
                self.errors = self.errors.saturating_add(1);
                None
            }
        }
    }

    /// The regex engine's answer for a match at `p`: an unanchored `find_at` that fills the next-match cache
    /// (so each regex item scans a haystack amortised once), or an anchored attempt once unanchored searches
    /// are off for this haystack.
    ///
    /// Unanchored searches are charged the bytes they cover (`end − p`, or `len − p` when nothing is found).
    /// Matches that another item's token skips make later searches cover the same bytes again; once the charge
    /// exceeds `2 × len + 4096`, the item switches to anchored attempts, which never look past the match at
    /// hand. An unanchored search that hits a fancy-regex runtime limit is not counted (the limit was hit
    /// somewhere after `p`); the item switches to anchored attempts, which count their own hits as the
    /// reference algorithm does.
    fn engine_match(&mut self, pattern: &Pattern, slot: usize, p: usize) -> Option<usize> {
        let len = self.line.len();
        if !self.anchored_only(slot) {
            self.check_cancel();
            if self.cancelled {
                return None;
            }
            let result = pattern.find_at(self.line, p);
            let c = self.slot(slot)?;
            match result {
                Ok(found) => {
                    let found = found.map(|r| (r.start, r.end));
                    let cost = found.map_or(len, |(_, e)| e).saturating_sub(p);
                    c.charged = c.charged.saturating_add(cost);
                    c.from = p;
                    c.found = found;
                    c.valid = true;
                    if c.charged > 2 * len + 4096 {
                        c.anchored_only = true;
                    }
                    return match found {
                        Some((s, e)) if s == p => Some(e),
                        _ => None,
                    };
                }
                Err(_) => c.anchored_only = true,
            }
        }
        self.anchored(pattern, p)
    }
}

/// Case-insensitive (simple 1:1 folding) literal match at `p`.
#[inline]
pub(crate) fn literal_ci(text: &str, line: &str, p: usize) -> Option<usize> {
    let hay = line.get(p..)?;
    let hb = hay.as_bytes();
    let tb = text.as_bytes();
    if text.is_ascii() && hb.len() >= tb.len() && hb[..tb.len()].is_ascii() {
        return hb[..tb.len()]
            .eq_ignore_ascii_case(tb)
            .then_some(p + tb.len());
    }
    let mut hay_chars = hay.char_indices();
    for tc in text.chars() {
        let (_, hc) = hay_chars.next()?;
        if simple_lower(hc) != simple_lower(tc) {
            return None;
        }
    }
    Some(p + hay_chars.next().map_or(hay.len(), |(at, _)| at))
}

/// A literal or a regex.
#[derive(Debug, Clone)]
pub(crate) enum Matcher {
    /// Literal text; `ci` = simple case folding.
    Literal { text: Box<str>, ci: bool },
    /// A compiled regex with its cache slot.
    Regex { pattern: Pattern, slot: usize },
}

impl Matcher {
    /// End of a non-empty match starting exactly at `p` (INV-TOK-1: a zero-length match is no match).
    ///
    /// A regex matches at `p` when the leftmost match found by `find_at(line, p)` starts at `p` — equivalently,
    /// when an anchored match at `p` exists (§2.10.2). The reference mode asks the anchored question at every
    /// position. The fast path answers from the next-match cache when it can (O(1)), then from the exact ASCII
    /// accelerator when its decision was cheap, and otherwise from the regex engine (which refills the cache).
    #[inline]
    pub(crate) fn match_at(&self, cx: &mut Cx<'_>, p: usize) -> Option<usize> {
        match self {
            Matcher::Literal { text, ci: false } => cx
                .line
                .as_bytes()
                .get(p..)?
                .starts_with(text.as_bytes())
                .then_some(p + text.len()),
            Matcher::Literal { text, ci: true } => literal_ci(text, cx.line, p),
            Matcher::Regex { pattern, slot } => {
                if cx.cancelled {
                    return None;
                }
                if cx.reference {
                    return cx.anchored(pattern, p).filter(|&e| e > p);
                }
                if let Some(known) = cx.cached(*slot, p) {
                    return known.filter(|&e| e > p);
                }
                match pattern.accelerate(cx.line, p) {
                    Some(Accel::Decided { end: Some(end), .. }) => {
                        return (end > p).then_some(end);
                    }
                    Some(Accel::Decided {
                        end: None,
                        examined,
                    }) if examined <= SHORT_SCAN || cx.anchored_only(*slot) => return None,
                    _ => {}
                }
                cx.engine_match(pattern, *slot, p).filter(|&e| e > p)
            }
        }
    }

    /// True when the matcher matches anywhere in the line (Lines headings): one unanchored search from the
    /// start of the line; a runtime-limit hit counts as "not found" and is counted.
    pub(crate) fn found_anywhere(&self, cx: &mut Cx<'_>) -> bool {
        match self {
            Matcher::Literal { text, ci: false } => {
                memchr::memmem::find(cx.line.as_bytes(), text.as_bytes()).is_some()
            }
            Matcher::Literal { text, ci: true } => cx
                .line
                .char_indices()
                .any(|(at, _)| literal_ci(text, cx.line, at).is_some()),
            Matcher::Regex { pattern, .. } => {
                cx.check_cancel();
                if cx.cancelled {
                    return false;
                }
                match pattern.find_at(cx.line, 0) {
                    Ok(found) => found.is_some(),
                    Err(_) => {
                        cx.errors = cx.errors.saturating_add(1);
                        false
                    }
                }
            }
        }
    }

    /// First char of a literal.
    fn literal_first(&self) -> Option<char> {
        match self {
            Matcher::Literal { text, .. } => text.chars().next(),
            Matcher::Regex { .. } => None,
        }
    }

    /// ASCII byte a match must start with, when every possible match starts with that exact byte.
    fn jump_byte(&self) -> Option<u8> {
        match self {
            Matcher::Literal { text, ci } => {
                let b = *text.as_bytes().first()?;
                (b < 128 && !(*ci && b.is_ascii_alphabetic())).then_some(b)
            }
            Matcher::Regex { .. } => None,
        }
    }
}

/// Byte length of the char at `x` (1 past the end or on a continuation byte).
#[inline]
pub(crate) fn char_len_at(line: &str, x: usize) -> usize {
    match line.as_bytes().get(x) {
        Some(&b) if b < 0x80 => 1,
        Some(_) => line
            .get(x..)
            .and_then(|s| s.chars().next())
            .map_or(1, char::len_utf8),
        None => 1,
    }
}

/// `find_close` of §2.10.3: end of the closing delimiter at or after `x`, honouring the escape char and the
/// doubled-delimiter rule.
pub(crate) fn find_close(
    to: &Matcher,
    escape: Option<char>,
    cx: &mut Cx<'_>,
    mut x: usize,
) -> Option<usize> {
    let line = cx.line;
    let len = line.len();
    let to_first = to.literal_first();
    let jump = if cx.reference {
        None
    } else {
        match (to.jump_byte(), escape) {
            (Some(t), None) => Some((t, None)),
            (Some(t), Some(e)) if e.is_ascii() => Some((t, Some(e as u8))),
            _ => None,
        }
    };
    while x < len {
        cx.progress(x);
        if cx.cancelled {
            return None;
        }
        if let Some((t, e)) = jump {
            let rest = line.as_bytes().get(x..)?;
            let skip = match e {
                Some(e) => memchr::memchr2(t, e, rest),
                None => memchr::memchr(t, rest),
            }?;
            x += skip;
        }
        if let Some(e) = escape
            && line.get(x..).is_some_and(|s| s.starts_with(e))
            && x + e.len_utf8() < len
        {
            let after = x + e.len_utf8();
            let closer_not_doubled =
                to_first == Some(e) && !line.get(after..).is_some_and(|s| s.starts_with(e));
            if !closer_not_doubled {
                x = after + char_len_at(line, after);
                continue;
            }
        }
        if let Some(end) = to.match_at(cx, x) {
            return Some(end);
        }
        x += char_len_at(line, x);
    }
    None
}

/// A List item: literal word tokens in a hash set, everything else tried one by one; the longest match wins.
#[derive(Debug, Clone)]
pub(crate) struct ListMatcher {
    ci: bool,
    /// Word tokens (folded when `ci`).
    words: WordSet,
    /// Word tokens as written (reference mode tries them one by one).
    word_list: Vec<Box<str>>,
    /// Regex tokens and literal tokens that contain non-word chars, with the chars they can start with.
    others: Vec<(Matcher, FirstChars)>,
    /// The union of the `others` first chars: positions it rejects skip the `others` loop.
    others_first: FirstChars,
    /// Quick rejection before hashing: bit `b1` of `pairs[b0]` is set when some (folded) word token starts with
    /// the ASCII bytes `b0 b1`; bit `b0` of `singles` when the one-byte token `b0` exists. Tokens that start
    /// with a non-ASCII byte disable the filter for words that do.
    pairs: Box<[u128; 128]>,
    singles: u128,
}

/// The ASCII byte as a filter index, lower-cased for case-insensitive lists (`simple_lower` agrees on ASCII).
#[inline]
fn filter_byte(b: u8, ci: bool) -> Option<usize> {
    (b < 128).then(|| usize::from(if ci { b.to_ascii_lowercase() } else { b }))
}

impl ListMatcher {
    pub(crate) fn new(ci: bool) -> ListMatcher {
        ListMatcher {
            ci,
            words: WordSet::default(),
            word_list: Vec::new(),
            others: Vec::new(),
            others_first: FirstChars::default(),
            pairs: Box::new([0; 128]),
            singles: 0,
        }
    }

    /// False when no word token can equal the (folded) word `w`; true when it may (non-ASCII bytes are
    /// never filtered).
    #[inline]
    fn may_contain(&self, w: &[u8]) -> bool {
        match w {
            [] => false,
            [b0] => filter_byte(*b0, self.ci).is_none_or(|b| self.singles & (1u128 << b) != 0),
            [b0, b1, ..] => match (filter_byte(*b0, self.ci), filter_byte(*b1, self.ci)) {
                (Some(b0), Some(b1)) => self.pairs[b0] & (1u128 << b1) != 0,
                _ => true,
            },
        }
    }

    pub(crate) fn add_word(&mut self, token: &str) {
        let key: Box<str> = if self.ci {
            token.chars().map(simple_lower).collect::<String>().into()
        } else {
            token.into()
        };
        match *key.as_bytes() {
            [b0] if b0 < 128 => self.singles |= 1u128 << b0,
            [b0, b1, ..] if b0 < 128 && b1 < 128 => self.pairs[usize::from(b0)] |= 1u128 << b1,
            // A token starting with non-ASCII bytes: words starting with such bytes are never filtered.
            _ => {}
        }
        self.words.insert(key);
        self.word_list.push(token.into());
    }

    pub(crate) fn add_other(&mut self, matcher: Matcher, first: FirstChars) {
        self.others_first.union(first);
        self.others.push((matcher, first));
    }

    /// End of the longest token match at `p` that respects the word boundaries.
    #[inline]
    pub(crate) fn match_at(&self, cx: &mut Cx<'_>, p: usize, word: &WordClass) -> Option<usize> {
        let line = cx.line;
        let mut best: Option<usize> = None;
        if cx.reference {
            for token in &self.word_list {
                let end = if self.ci {
                    literal_ci(token, line, p)
                } else {
                    line.get(p..)
                        .is_some_and(|s| s.starts_with(&**token))
                        .then(|| p + token.len())
                };
                if let Some(end) = end
                    && boundaries_ok(word, line, p, end)
                    && best.is_none_or(|b| end > b)
                {
                    best = Some(end);
                }
            }
        } else if !self.words.is_empty() && word.word_at(line, p) && !word.word_before(line, p) {
            let q = word.word_end(line, p);
            let w = line.get(p..q).unwrap_or("");
            let hit = if !self.may_contain(w.as_bytes()) {
                false
            } else if self.ci {
                cx.fold.clear();
                cx.fold.extend(w.chars().map(simple_lower));
                self.words.contains(cx.fold.as_str())
            } else {
                self.words.contains(w)
            };
            if hit {
                best = Some(q);
            }
        }
        let class = line.as_bytes().get(p).copied().unwrap_or(0);
        let admits = |first: &FirstChars| {
            if class < 128 {
                first.ascii & (1u128 << class) != 0 || (p == 0 && first.at_line_start)
            } else {
                first.non_ascii || (p == 0 && first.at_line_start)
            }
        };
        if !cx.reference && !admits(&self.others_first) {
            return best;
        }
        for (other, first) in &self.others {
            if !cx.reference && !admits(first) {
                continue;
            }
            if let Some(end) = other.match_at(cx, p)
                && boundaries_ok(word, line, p, end)
                && best.is_none_or(|b| end > b)
            {
                best = Some(end);
            }
        }
        best
    }
}

/// List word boundaries (§2.10.2): a match that starts with a word char must start at a word start, one that
/// ends with a word char must end at a word end.
fn boundaries_ok(word: &WordClass, line: &str, p: usize, end: usize) -> bool {
    let Some(text) = line.get(p..end) else {
        return false;
    };
    let (Some(first), Some(last)) = (text.chars().next(), text.chars().next_back()) else {
        return false;
    };
    if word.is_word(first) && word.word_before(line, p) {
        return false;
    }
    if word.is_word(last) && word.word_at(line, end) {
        return false;
    }
    true
}

/// A compiled grammar item.
#[derive(Debug, Clone)]
pub(crate) enum ItemMatcher {
    /// FF-020.
    Basic(Matcher),
    /// FF-021.
    Delimited {
        from: Matcher,
        to: Option<Matcher>,
        escape: Option<char>,
    },
    /// FF-022.
    List(ListMatcher),
    /// FF-023: `from` is the 0-based char column; `to` the 1-based inclusive last column.
    Columns { from: u32, to: Option<u32> },
    /// FF-024.
    Lines {
        heading: Option<Matcher>,
        next_lines: u32,
        or_line_one: bool,
    },
}

/// One grammar item with its element.
#[derive(Debug, Clone)]
pub(crate) struct CompiledItem {
    pub(crate) element: ElementId,
    pub(crate) matcher: ItemMatcher,
    /// The item can leave a line in a non-`Normal` state.
    pub(crate) multiline: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_class() {
        let w = WordClass::new("-é");
        assert!(w.is_word('a') && w.is_word('_') && w.is_word('-') && w.is_word('é'));
        assert!(w.is_word('ж') && !w.is_word(' ') && !w.is_word('.'));
        assert_eq!(w.word_end("END-IF.", 0), 6);
        assert_eq!(w.word_end("aéb c", 0), 4);
        assert!(w.word_before("a-b", 2));
        assert!(!w.word_before("a b", 2));
        assert!(w.word_at("ab", 1));
        assert!(!w.word_at("ab", 2));
    }

    #[test]
    fn case_insensitive_literals() {
        assert_eq!(literal_ci("end", "x END y", 2), Some(5));
        assert_eq!(literal_ci("end", "x En", 2), None);
        // KELVIN SIGN folds to 'k' (3 bytes in the haystack, 1 in the literal).
        assert_eq!(literal_ci("k", "\u{212A}x", 0), Some(3));
        assert_eq!(literal_ci("é", "É", 0), Some(2));
    }

    #[test]
    fn hasher_is_deterministic() {
        let mut a = WordHasher::default();
        a.write(b"keyword_long_enough");
        let mut b = WordHasher::default();
        b.write(b"keyword_long_enough");
        assert_eq!(a.finish(), b.finish());
        let mut c = WordHasher::default();
        c.write(b"keyword_long_enougH");
        assert_ne!(a.finish(), c.finish());
    }
}
