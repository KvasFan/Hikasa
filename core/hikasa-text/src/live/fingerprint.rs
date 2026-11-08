//! Step 1 of the incremental recompare (§1.10.11): the dirty lines of each side, their old and new keys (with
//! the tokenizer re-converged past them), and a view of every line's fingerprint and key in the new numbering.
//!
//! ```text
//! LineMap     the deltas since the last exact result merged into dirty spans (old range → new range); every
//!             other line maps by the shift of the spans before it
//! re-keying   the new and old lines of every span keyed from the start state the old document had there; with
//!             a multi-line grammar the span grows line by line until the start states agree again
//! SideView    fp(line) / key(line) / weight(line) of a new line: a dirty line's own, a clean line's old one
//!             (its fingerprint kept, its key rebuilt from the old document and the nearest checkpoint)
//! ```

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;

use hikasa_core::cancel::{CancelToken, Cancelled};
use hikasa_formats::{CompiledFormat, LineState, Token, Tokenizer};

use crate::classify::{KeyShape, RuleSet};
use crate::compare::key_fingerprint;
use crate::compare::keys::line_key;
use crate::document::{LineIdx, TextDocument};
use crate::edit::LineDelta;

/// One dirty span: old lines `old` became new lines `new`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) old: Range<u32>,
    pub(crate) new: Range<u32>,
}

/// The dirty spans of one side, sorted and disjoint (in both numberings).
#[derive(Debug, Clone, Default)]
pub(crate) struct LineMap {
    pub(crate) spans: Vec<Span>,
}

impl LineMap {
    /// The map of `deltas` (applied in order, each in the numbering after the previous ones).
    pub(crate) fn from_deltas(deltas: impl Iterator<Item = LineDelta>) -> LineMap {
        let mut spans: Vec<Span> = Vec::new();
        for d in deltas {
            if d.removed == 0 && d.inserted == 0 {
                continue;
            }
            let (a, b) = (d.at, d.at + d.removed);
            // Spans overlapping or touching [a, b] (current numbering).
            let i = spans.partition_point(|s| s.new.end < a);
            let j = spans.partition_point(|s| s.new.start <= b);
            let offset_before = |k: usize, spans: &[Span]| -> i64 {
                spans[..k]
                    .iter()
                    .map(|s| {
                        i64::from(s.new.end - s.new.start) - i64::from(s.old.end - s.old.start)
                    })
                    .sum()
            };
            let (ns, os) = if i < j && spans[i].new.start <= a {
                (spans[i].new.start, spans[i].old.start)
            } else {
                (a, (i64::from(a) - offset_before(i, &spans)) as u32)
            };
            let (ne, oe) = if i < j && spans[j - 1].new.end >= b {
                (spans[j - 1].new.end, spans[j - 1].old.end)
            } else {
                (b, (i64::from(b) - offset_before(j, &spans)) as u32)
            };
            let shift = i64::from(d.inserted) - i64::from(d.removed);
            let merged = Span {
                old: os..oe,
                new: ns..(i64::from(ne) + shift) as u32,
            };
            for s in &mut spans[j..] {
                s.new =
                    (i64::from(s.new.start) + shift) as u32..(i64::from(s.new.end) + shift) as u32;
            }
            spans.splice(i..j, [merged]);
        }
        LineMap { spans }
    }

    /// The old line of clean new line `x` (`None` for a dirty line).
    #[cfg(test)]
    fn old_of(&self, x: u32) -> Option<u32> {
        let k = self.spans.partition_point(|s| s.new.end <= x);
        if self.spans.get(k).is_some_and(|s| s.new.start <= x) {
            return None;
        }
        let shift: i64 = self.spans[..k]
            .iter()
            .map(|s| i64::from(s.new.end - s.new.start) - i64::from(s.old.end - s.old.start))
            .sum();
        Some((i64::from(x) - shift) as u32)
    }

    /// The new line of clean old line `x` (`None` for a dirty line).
    #[cfg(test)]
    fn new_of(&self, x: u32) -> Option<u32> {
        let k = self.spans.partition_point(|s| s.old.end <= x);
        if self.spans.get(k).is_some_and(|s| s.old.start <= x) {
            return None;
        }
        Some((i64::from(x) + self.shift_before_old(k)) as u32)
    }

    /// The new position of old boundary `x` (between lines); a boundary inside a span maps to its new start.
    #[cfg(test)]
    fn pos(&self, x: u32) -> u32 {
        let k = self.spans.partition_point(|s| s.old.end <= x);
        if let Some(s) = self.spans.get(k)
            && s.old.start < x
        {
            return s.new.start;
        }
        (i64::from(x) + self.shift_before_old(k)) as u32
    }

    #[cfg(test)]
    fn shift_before_old(&self, k: usize) -> i64 {
        self.spans[..k]
            .iter()
            .map(|s| i64::from(s.new.end - s.new.start) - i64::from(s.old.end - s.old.start))
            .sum()
    }

    /// Lines in spans (the larger of old and new per span).
    pub(crate) fn dirty_lines(&self) -> u64 {
        self.spans
            .iter()
            .map(|s| u64::from((s.old.end - s.old.start).max(s.new.end - s.new.start)))
            .sum()
    }
}

/// The keyed lines of one dirty span.
#[derive(Debug, Clone, Default)]
pub(crate) struct Keyed {
    pub(crate) keys: Vec<Vec<u8>>,
    pub(crate) fps: Vec<u16>,
    pub(crate) weights: Vec<u8>,
    /// Start state of each line.
    pub(crate) states: Vec<LineState>,
    /// Regex runtime-limit hits of the lines (tokenizer, patterns, and weights when they count).
    pub(crate) errors: u64,
    /// State after the last line.
    pub(crate) end: LineState,
}

/// A re-keyed span: old lines `old` (keyed from the old document) became new lines `new`.
#[derive(Debug, Clone)]
pub(crate) struct Rekeyed {
    pub(crate) old: Range<u32>,
    pub(crate) new: Range<u32>,
    pub(crate) old_keys: Keyed,
    pub(crate) new_keys: Keyed,
}

/// What keying one side needs.
pub(crate) struct Keyer<'a> {
    pub(crate) set: &'a RuleSet,
    pub(crate) shape: KeyShape,
    pub(crate) format: &'a CompiledFormat,
    /// True when weights count (Standard): in the key comparison and in the regex totals.
    pub(crate) weights: bool,
    /// True when the tokenizer carries state from line to line.
    pub(crate) multiline: bool,
    pub(crate) cancel: &'a CancelToken,
}

impl Keyer<'_> {
    /// Keys lines `lines` of `doc` from `state`, appending to `out`.
    fn key_lines(
        &self,
        doc: &TextDocument,
        lines: Range<u32>,
        state: LineState,
        out: &mut Keyed,
    ) -> Result<LineState, Cancelled> {
        let mut tokenizer = Tokenizer::new(self.format.grammar());
        let mut tokens: Vec<Token> = Vec::new();
        let mut key = Vec::new();
        let mut state = state;
        for line in lines {
            let text = doc.line(LineIdx(line));
            let eol = doc.line_ending(LineIdx(line));
            out.states.push(state);
            let (end, errors) = line_key(
                self.set,
                self.shape,
                &text,
                line,
                eol,
                state,
                &mut tokenizer,
                &mut tokens,
                &mut key,
                self.cancel,
            )?;
            out.errors += errors;
            let (w, we) = if self.weights && self.format.has_line_weights() {
                self.format.line_weight_with_errors(&text)
            } else {
                (0, 0)
            };
            out.errors += u64::from(we);
            out.weights.push(w);
            out.fps
                .push(key_fingerprint(&key, self.shape, self.cancel)?);
            out.keys.push(std::mem::take(&mut key));
            state = if self.multiline {
                end
            } else {
                LineState::Normal
            };
        }
        out.end = state;
        Ok(state)
    }

    /// The start state of old line `x`: tokenizing from the nearest checkpoint at or before it.
    pub(crate) fn old_state(
        &self,
        doc: &TextDocument,
        checkpoints: &[(u32, LineState)],
        x: u32,
    ) -> Result<LineState, Cancelled> {
        if !self.multiline || x == 0 {
            return Ok(LineState::Normal);
        }
        let k = checkpoints.partition_point(|c| c.0 <= x);
        let (from, state) = if k == 0 {
            (0, LineState::Normal)
        } else {
            checkpoints[k - 1]
        };
        let mut scratch = Keyed::default();
        self.key_lines(doc, from..x, state, &mut scratch)
    }

    /// Re-keys the spans of `map` (old lines from `old`, new lines from `new`): each span is keyed on both sides
    /// from the old start state, then grown line by line while the end states differ (a multi-line grammar).
    pub(crate) fn rekey(
        &self,
        map: &LineMap,
        old: &TextDocument,
        new: &TextDocument,
        checkpoints: &[(u32, LineState)],
    ) -> Result<Vec<Rekeyed>, Cancelled> {
        let mut out: Vec<Rekeyed> = Vec::new();
        let (old_len, new_len) = (old.line_count(), new.line_count());
        let mut k = 0;
        while k < map.spans.len() {
            let span = &map.spans[k];
            let mut r = Rekeyed {
                old: span.old.clone(),
                new: span.new.clone(),
                old_keys: Keyed::default(),
                new_keys: Keyed::default(),
            };
            let state = self.old_state(old, checkpoints, span.old.start)?;
            let mut so = self.key_lines(old, span.old.clone(), state, &mut r.old_keys)?;
            let mut sn = self.key_lines(new, span.new.clone(), state, &mut r.new_keys)?;
            k += 1;
            // Grow while the states disagree; a later span reached is absorbed.
            while so != sn {
                if let Some(next) = map.spans.get(k)
                    && next.new.start == r.new.end
                {
                    so = self.key_lines(old, next.old.clone(), so, &mut r.old_keys)?;
                    sn = self.key_lines(new, next.new.clone(), sn, &mut r.new_keys)?;
                    r.old.end = next.old.end;
                    r.new.end = next.new.end;
                    k += 1;
                    continue;
                }
                if r.old.end >= old_len || r.new.end >= new_len {
                    break;
                }
                so = self.key_lines(old, r.old.end..r.old.end + 1, so, &mut r.old_keys)?;
                sn = self.key_lines(new, r.new.end..r.new.end + 1, sn, &mut r.new_keys)?;
                r.old.end += 1;
                r.new.end += 1;
            }
            out.push(r);
        }
        Ok(out)
    }
}

/// Every line of one side in the new numbering: fingerprints, keys and weights (§1.10.11 step 1).
pub(crate) struct SideView<'a> {
    pub(crate) spans: &'a [Rekeyed],
    /// New ranges of the alignment-dirty parts of the spans (keys or weights changed).
    adirty: Vec<Range<u32>>,
    pub(crate) old: &'a TextDocument,
    pub(crate) old_fps: &'a [u16],
    pub(crate) checkpoints: &'a [(u32, LineState)],
    pub(crate) keyer: &'a Keyer<'a>,
    cache: RefCell<HashMap<u32, (Vec<u8>, u8)>>,
    /// Prefix sums of the spans' line-count changes (`spans + 1` entries).
    shifts: Vec<i64>,
}

impl<'a> SideView<'a> {
    pub(crate) fn new(
        spans: &'a [Rekeyed],
        old: &'a TextDocument,
        old_fps: &'a [u16],
        checkpoints: &'a [(u32, LineState)],
        keyer: &'a Keyer<'a>,
        adirty: Vec<Range<u32>>,
    ) -> SideView<'a> {
        let mut shifts = Vec::with_capacity(spans.len() + 1);
        let mut at = 0i64;
        shifts.push(0);
        for s in spans {
            at += i64::from(s.new.end - s.new.start) - i64::from(s.old.end - s.old.start);
            shifts.push(at);
        }
        SideView {
            shifts,
            spans,
            adirty,
            old,
            old_fps,
            checkpoints,
            keyer,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The re-keyed span holding new line `x`, and `x`'s index in it.
    fn span(&self, x: u32) -> Option<(&Rekeyed, usize)> {
        let k = self.spans.partition_point(|s| s.new.end <= x);
        let s = self.spans.get(k)?;
        (s.new.start <= x).then(|| (s, (x - s.new.start) as usize))
    }

    /// True when new line `x` is in an alignment-dirty part of a span.
    fn adirty_at(&self, x: u32) -> bool {
        let k = self.adirty.partition_point(|r| r.end <= x);
        self.adirty.get(k).is_some_and(|r| r.start <= x)
    }

    /// The shift of the lines after the first `k` spans.
    #[inline]
    fn shift(&self, k: usize) -> i64 {
        self.shifts[k]
    }

    /// Calls `f(line, fp)` for every new line of `lines`, in order: clean runs straight from the old
    /// fingerprints, re-keyed lines from their spans.
    pub(crate) fn for_each_fp(&self, lines: Range<u32>, mut f: impl FnMut(u32, u16)) {
        let mut x = lines.start;
        let mut k = self.spans.partition_point(|s| s.new.end <= x);
        while x < lines.end {
            match self.spans.get(k) {
                Some(s) if s.new.start <= x => {
                    let end = s.new.end.min(lines.end);
                    for y in x..end {
                        f(y, s.new_keys.fps[(y - s.new.start) as usize]);
                    }
                    x = end;
                    k += 1;
                }
                next => {
                    let end = next.map_or(lines.end, |s| s.new.start.min(lines.end));
                    let o = (i64::from(x) - self.shift(k)) as usize;
                    let len = (end - x) as usize;
                    if let Some(slice) = self.old_fps.get(o..o + len) {
                        for (i, &fp) in slice.iter().enumerate() {
                            f(x + i as u32, fp);
                        }
                    }
                    x = end;
                }
            }
        }
    }

    /// The old line of new line `x` when it is alignment-clean: outside the spans by their shift, inside a span
    /// of unchanged length by its offset (its key and weight are the old line's).
    pub(crate) fn old_of(&self, x: u32) -> Option<u32> {
        let k = self.spans.partition_point(|s| s.new.end <= x);
        if let Some(s) = self.spans.get(k)
            && s.new.start <= x
        {
            let equal = s.new.end - s.new.start == s.old.end - s.old.start;
            return (equal && !self.adirty_at(x)).then(|| s.old.start + (x - s.new.start));
        }
        Some((i64::from(x) - self.shift(k)) as u32)
    }

    /// The new line of old line `x` when it is alignment-clean (the inverse of [`SideView::old_of`]).
    pub(crate) fn new_of(&self, x: u32) -> Option<u32> {
        let k = self.spans.partition_point(|s| s.old.end <= x);
        if let Some(s) = self.spans.get(k)
            && s.old.start <= x
        {
            let equal = s.new.end - s.new.start == s.old.end - s.old.start;
            let y = s.new.start + (x - s.old.start);
            return (equal && !self.adirty_at(y)).then_some(y);
        }
        Some((i64::from(x) + self.shift(k)) as u32)
    }

    /// [`SideView::new_of`] of ascending old lines `xs`, walking the spans once.
    pub(crate) fn new_of_sorted(&self, xs: impl Iterator<Item = u32>) -> Vec<Option<u32>> {
        let mut k = 0usize;
        xs.map(|x| {
            while k < self.spans.len() && self.spans[k].old.end <= x {
                k += 1;
            }
            match self.spans.get(k) {
                Some(s) if s.old.start <= x => {
                    let equal = s.new.end - s.new.start == s.old.end - s.old.start;
                    let y = s.new.start + (x - s.old.start);
                    (equal && !self.adirty_at(y)).then_some(y)
                }
                _ => Some((i64::from(x) + self.shift(k)) as u32),
            }
        })
        .collect()
    }

    /// The new position of old boundary `x` (inside a span of unchanged length by its offset, inside another
    /// span its new start).
    pub(crate) fn pos(&self, x: u32) -> u32 {
        let k = self.spans.partition_point(|s| s.old.end <= x);
        if let Some(s) = self.spans.get(k)
            && s.old.start < x
        {
            if s.new.end - s.new.start == s.old.end - s.old.start {
                return s.new.start + (x - s.old.start);
            }
            return s.new.start;
        }
        (i64::from(x) + self.shift(k)) as u32
    }

    /// True when new line `x` was re-keyed (its text, terminator or start state changed).
    pub(crate) fn rekeyed(&self, x: u32) -> bool {
        self.span(x).is_some()
    }

    /// The fingerprint of new line `x` (0 = trivial).
    pub(crate) fn fp(&self, x: u32) -> u16 {
        match self.span(x) {
            Some((s, i)) => s.new_keys.fps[i],
            None => {
                let k = self.spans.partition_point(|s| s.new.end <= x);
                let o = (i64::from(x) - self.shift(k)) as u32;
                self.old_fps.get(o as usize).copied().unwrap_or(0)
            }
        }
    }

    /// The key and weight of new line `x`.
    pub(crate) fn key(&self, x: u32) -> Result<(Cow<'_, [u8]>, u8), Cancelled> {
        if let Some((s, i)) = self.span(x) {
            return Ok((
                Cow::Owned(s.new_keys.keys[i].clone()),
                s.new_keys.weights[i],
            ));
        }
        let k = self.spans.partition_point(|s| s.new.end <= x);
        let o = (i64::from(x) - self.shift(k)) as u32;
        if let Some((k, w)) = self.cache.borrow().get(&o) {
            return Ok((Cow::Owned(k.clone()), *w));
        }
        let state = self.keyer.old_state(self.old, self.checkpoints, o)?;
        let mut keyed = Keyed::default();
        self.keyer
            .key_lines_pub(self.old, o..o + 1, state, &mut keyed)?;
        let key = keyed.keys.pop().unwrap_or_default();
        let w = keyed.weights.pop().unwrap_or(0);
        self.cache.borrow_mut().insert(o, (key.clone(), w));
        Ok((Cow::Owned(key), w))
    }

    /// The keys and weights of new lines `lines` (clean runs keyed sequentially from the old document).
    pub(crate) fn keys_range(
        &self,
        lines: Range<u32>,
    ) -> Result<(Vec<Vec<u8>>, Vec<u8>), Cancelled> {
        let mut keys = Vec::with_capacity((lines.end - lines.start) as usize);
        let mut weights = Vec::with_capacity(keys.capacity());
        let mut x = lines.start;
        while x < lines.end {
            if let Some((s, i)) = self.span(x) {
                let take = ((s.new.end - x) as usize).min((lines.end - x) as usize);
                keys.extend(s.new_keys.keys[i..i + take].iter().cloned());
                weights.extend_from_slice(&s.new_keys.weights[i..i + take]);
                x += take as u32;
                continue;
            }
            // A clean run up to the next span.
            let k = self.spans.partition_point(|s| s.new.end <= x);
            let end = self
                .spans
                .get(k)
                .map_or(lines.end, |s| s.new.start.min(lines.end));
            let o = (i64::from(x) - self.shift(k)) as u32;
            let state = self.keyer.old_state(self.old, self.checkpoints, o)?;
            let mut keyed = Keyed::default();
            self.keyer
                .key_lines_pub(self.old, o..o + (end - x), state, &mut keyed)?;
            keys.extend(keyed.keys);
            weights.extend(keyed.weights);
            x = end;
        }
        Ok((keys, weights))
    }
}

impl Keyer<'_> {
    /// [`Keyer::key_lines`] for the views.
    pub(crate) fn key_lines_pub(
        &self,
        doc: &TextDocument,
        lines: Range<u32>,
        state: LineState,
        out: &mut Keyed,
    ) -> Result<LineState, Cancelled> {
        self.key_lines(doc, lines, state, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(at: u32, removed: u32, inserted: u32) -> LineDelta {
        LineDelta {
            at,
            removed,
            inserted,
        }
    }

    #[test]
    fn deltas_merge_into_spans() {
        let map = LineMap::from_deltas([d(10, 1, 1), d(20, 0, 3), d(11, 2, 0)].into_iter());
        assert_eq!(
            map.spans,
            [
                Span {
                    old: 10..13,
                    new: 10..11
                },
                Span {
                    old: 20..20,
                    new: 18..21
                },
            ]
        );
        assert_eq!(map.old_of(5), Some(5));
        assert_eq!(map.old_of(10), None);
        assert_eq!(map.old_of(11), Some(13));
        assert_eq!(map.old_of(21), Some(20));
        assert_eq!(map.new_of(13), Some(11));
        assert_eq!(map.new_of(12), None);
        // A boundary at an insertion point maps after the inserted lines.
        assert_eq!(map.pos(20), 21);
        assert_eq!(map.pos(25), 26);
        // Touching spans merge.
        let map = LineMap::from_deltas([d(5, 1, 1), d(6, 1, 1)].into_iter());
        assert_eq!(
            map.spans,
            [Span {
                old: 5..7,
                new: 5..7
            }]
        );
    }
}
