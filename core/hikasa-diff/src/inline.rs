//! Inline (within-line) differences (§2.16, TC-014): [`InlineOptions`], [`InlineDiff`] and [`inline_diff`].
//!
//! ```text
//! inline_diff(a, b):
//!   a == b → {}
//!   strip the common char prefix and suffix
//!   either side longer than max_line_chars → whole_line = true, ranges = the two middles
//!   segment each middle: maximal word run | maximal whitespace run | single other char
//!   intern the segments locally (dense ids); imara Myers over the segment ids; postprocess (no heuristic)
//!   each hunk with both sides non-empty and ≤ refine_max_chars chars in total is refined per char (chars
//!   interned locally, never raw code points); other hunks contribute their segment byte ranges
//!   per side: merge adjacent ranges
//! ```
//!
//! INV-INLINE-1: ranges are sorted, disjoint, non-empty and on char boundaries. INV-INLINE-2: removing the ranges
//! leaves an equal "common" text on both sides. INV-INLINE-3: `a == b` gives no ranges. INV-INLINE-4: the output
//! is deterministic.

use std::collections::HashMap;
use std::ops::Range;

use imara_diff::{Algorithm as ImaraAlgorithm, Diff, Hunk, NoSliderHeuristic, Token};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

/// Inline diff options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InlineOptions {
    /// Lines longer than this (chars, either side) are reported as one whole-line change. Default 10_000.
    pub max_line_chars: u32,
    /// Changed segment hunks up to this many chars (both sides together) are refined per char. Default 512.
    pub refine_max_chars: u32,
}

impl InlineOptions {
    /// The defaults: 10,000 chars per line, refinement of hunks up to 512 chars.
    pub const DEFAULT: InlineOptions = InlineOptions {
        max_line_chars: 10_000,
        refine_max_chars: 512,
    };
}

impl Default for InlineOptions {
    /// [`InlineOptions::DEFAULT`].
    fn default() -> InlineOptions {
        InlineOptions::DEFAULT
    }
}

/// Differing byte ranges within a line pair (INV-INLINE-1..4).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InlineDiff {
    /// Sorted, disjoint, non-empty byte ranges on char boundaries of the left text.
    pub left: Vec<Range<u32>>,
    /// Same for the right text.
    pub right: Vec<Range<u32>>,
    /// True when the line exceeded `max_line_chars` (ranges then cover the whole differing middle).
    pub whole_line: bool,
}

/// Character/segment-level difference of two strings (§2.16). Pure and deterministic.
///
/// ```
/// use hikasa_diff::{InlineOptions, inline_diff};
///
/// let d = inline_diff("let x = 10;", "let y = 12;", &InlineOptions::default());
/// assert_eq!(d.left, [4..5, 9..10]);
/// assert_eq!(d.right, [4..5, 9..10]);
/// assert!(!d.whole_line);
/// ```
pub fn inline_diff(left: &str, right: &str, opts: &InlineOptions) -> InlineDiff {
    let mut out = InlineDiff::default();
    if left == right {
        return out;
    }
    // Common char prefix and suffix (equal chars have equal UTF-8 lengths, so the byte counts agree).
    let prefix: usize = left
        .chars()
        .zip(right.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let suffix: usize = left[prefix..]
        .chars()
        .rev()
        .zip(right[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let left_mid = prefix..left.len() - suffix;
    let right_mid = prefix..right.len() - suffix;
    let max = opts.max_line_chars as usize;
    if left.chars().count() > max || right.chars().count() > max {
        out.whole_line = true;
        push_range(&mut out.left, &left_mid);
        push_range(&mut out.right, &right_mid);
        return out;
    }
    let mut left_segments = SmallVec::<[Range<usize>; 32]>::new();
    let mut right_segments = SmallVec::<[Range<usize>; 32]>::new();
    segment(left, left_mid, &mut left_segments);
    segment(right, right_mid, &mut right_segments);
    let mut ids: HashMap<&str, u32> = HashMap::new();
    let left_tokens = intern(left, &left_segments, &mut ids);
    let right_tokens = intern(right, &right_segments, &mut ids);
    let k = ids.len() as u32;
    for hunk in myers_hunks(&left_tokens, &right_tokens, k) {
        let l = segment_bytes(&left_segments, &hunk.before);
        let r = segment_bytes(&right_segments, &hunk.after);
        let refine = !l.is_empty()
            && !r.is_empty()
            && left[l.clone()].chars().count() + right[r.clone()].chars().count()
                <= opts.refine_max_chars as usize;
        if refine {
            refine_chars(left, l, right, r, &mut out);
        } else {
            push_range(&mut out.left, &l);
            push_range(&mut out.right, &r);
        }
    }
    out
}

/// Character class of the segmentation.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class_of(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Splits `text[range]` into maximal word runs, maximal whitespace runs and single other chars (byte ranges
/// into `text`).
fn segment(text: &str, range: Range<usize>, out: &mut SmallVec<[Range<usize>; 32]>) {
    let base = range.start;
    let mut chars = text[range].char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let class = class_of(c);
        let mut end = start + c.len_utf8();
        if class != Class::Other {
            while let Some(&(at, d)) = chars.peek() {
                if class_of(d) != class {
                    break;
                }
                end = at + d.len_utf8();
                chars.next();
            }
        }
        out.push(base + start..base + end);
    }
}

/// Local, dense ids of the segments (first-occurrence order).
fn intern<'t>(
    text: &'t str,
    segments: &[Range<usize>],
    ids: &mut HashMap<&'t str, u32>,
) -> Vec<Token> {
    segments
        .iter()
        .map(|range| {
            let next = ids.len() as u32;
            Token(*ids.entry(&text[range.clone()]).or_insert(next))
        })
        .collect()
}

/// Hunks of imara Myers over dense local tokens `< k`, postprocessed without a slider heuristic.
fn myers_hunks(before: &[Token], after: &[Token], k: u32) -> Vec<Hunk> {
    debug_assert!(
        before.iter().chain(after).all(|t| t.0 < k),
        "INV-DENSE-1: inline tokens must be dense"
    );
    let mut diff = Diff::default();
    diff.compute_with(ImaraAlgorithm::Myers, before, after, k);
    diff.postprocess_with(before, after, NoSliderHeuristic);
    diff.hunks().collect()
}

/// Byte range covered by the segments `hunk` (empty when the hunk side is empty).
fn segment_bytes(segments: &[Range<usize>], hunk: &Range<u32>) -> Range<usize> {
    match (
        segments.get(hunk.start as usize),
        hunk.end
            .checked_sub(1)
            .and_then(|last| segments.get(last as usize)),
    ) {
        (Some(first), Some(last)) if hunk.start < hunk.end => first.start..last.end,
        _ => 0..0,
    }
}

/// Char-level refinement of one hunk: char ids are interned locally, Myers runs over them, and the differing
/// char ranges are added to `out`.
fn refine_chars(left: &str, l: Range<usize>, right: &str, r: Range<usize>, out: &mut InlineDiff) {
    let mut ids: SmallVec<[(char, u32); 64]> = SmallVec::new();
    let mut id_of = |c: char| match ids.iter().find(|(d, _)| *d == c) {
        Some(&(_, id)) => Token(id),
        None => {
            let id = ids.len() as u32;
            ids.push((c, id));
            Token(id)
        }
    };
    let left_chars: SmallVec<[(usize, char); 64]> = left[l.clone()]
        .char_indices()
        .map(|(at, c)| (l.start + at, c))
        .collect();
    let right_chars: SmallVec<[(usize, char); 64]> = right[r.clone()]
        .char_indices()
        .map(|(at, c)| (r.start + at, c))
        .collect();
    let before: Vec<Token> = left_chars.iter().map(|&(_, c)| id_of(c)).collect();
    let after: Vec<Token> = right_chars.iter().map(|&(_, c)| id_of(c)).collect();
    let k = ids.len() as u32;
    for hunk in myers_hunks(&before, &after, k) {
        push_range(&mut out.left, &char_bytes(&left_chars, &hunk.before, l.end));
        push_range(
            &mut out.right,
            &char_bytes(&right_chars, &hunk.after, r.end),
        );
    }
}

/// Byte range of the chars `hunk` (`end` is the byte end of the refined part).
fn char_bytes(chars: &[(usize, char)], hunk: &Range<u32>, end: usize) -> Range<usize> {
    if hunk.start >= hunk.end {
        return 0..0;
    }
    let start = chars.get(hunk.start as usize).map_or(end, |&(at, _)| at);
    let stop = chars.get(hunk.end as usize).map_or(end, |&(at, _)| at);
    start..stop
}

/// Appends a non-empty range, merging it with the previous one when adjacent.
fn push_range(out: &mut Vec<Range<u32>>, range: &Range<usize>) {
    if range.start >= range.end {
        return;
    }
    let start = to_u32(range.start);
    let end = to_u32(range.end);
    match out.last_mut() {
        Some(last) if last.end == start => last.end = end,
        _ => out.push(start..end),
    }
}

/// Byte offsets are `u32` (documents are below 4 GiB, D-43); larger values saturate.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)] // one-range expectations are intended
mod tests {
    use super::*;

    fn diff(a: &str, b: &str) -> InlineDiff {
        inline_diff(a, b, &InlineOptions::default())
    }

    #[test]
    fn identical_lines_have_no_ranges() {
        assert_eq!(diff("same", "same"), InlineDiff::default());
        assert_eq!(diff("", ""), InlineDiff::default());
    }

    #[test]
    fn insertions_and_deletions() {
        let d = diff("abc", "abXc");
        assert_eq!(
            (d.left.as_slice(), d.right.as_slice()),
            (&[][..], &[2..3][..])
        );
        let d = diff("foo bar", "foo");
        assert_eq!(d.left, [3..7]);
        assert!(d.right.is_empty());
        let d = diff("", "x");
        assert_eq!(d.right, [0..1]);
    }

    #[test]
    fn multibyte_chars_stay_whole() {
        let d = diff("naïve café", "naive cafe");
        assert_eq!(d.left, [2..4, 10..12]);
        assert_eq!(d.right, [2..3, 9..10]);
        let d = diff("日本語", "日本人");
        assert_eq!(d.left, [6..9]);
        assert_eq!(d.right, [6..9]);
    }

    #[test]
    fn long_lines_are_whole_line_changes() {
        let opts = InlineOptions {
            max_line_chars: 5,
            refine_max_chars: 512,
        };
        let d = inline_diff("abcdefgh", "abcXefgh", &opts);
        assert!(d.whole_line);
        assert_eq!(d.left, [3..4]);
        assert_eq!(d.right, [3..4]);
        let d = inline_diff("abcdefgh", "abcdefghij", &opts);
        assert!(d.whole_line);
        assert!(d.left.is_empty());
        assert_eq!(d.right, [8..10]);
    }

    #[test]
    fn unrefined_hunks_use_segment_ranges() {
        let opts = InlineOptions {
            max_line_chars: 10_000,
            refine_max_chars: 2,
        };
        // The middles are "alpha, gamma" and "beta, delto": two changed words, each hunk too long to refine.
        let d = inline_diff("f(alpha, gamma)", "f(beta, delto)", &opts);
        assert_eq!(d.left, [2..7, 9..14]);
        assert_eq!(d.right, [2..6, 8..13]);
        // With refinement, only the differing chars remain ("ha" vs "ine" after the common "alp").
        let d = diff("call(alpha, beta)", "call(alpine, beta)");
        assert_eq!(d.left, [8..10]);
        assert_eq!(d.right, [8..11]);
    }

    #[test]
    fn segment_classes() {
        let mut out = SmallVec::new();
        segment("ab_1  +-x", 0..9, &mut out);
        assert_eq!(out.as_slice(), [0..4, 4..6, 6..7, 7..8, 8..9]);
    }
}
