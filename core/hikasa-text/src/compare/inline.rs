//! Classified inline differences (TC-014, §2.16): [`ClassifiedInline`] and [`InlineRange`].
//!
//! `hikasa_diff::inline_diff` gives the differing byte ranges of both lines. They are classified per row:
//!
//! 0. On a `Changed`/`Unimportant` row (equal keys: only unimportant chars differ) every range is
//!    **unimportant** (TC-014; [`all_unimportant`]).
//! 1. Ranges correspond by their position in the common text (removing every range leaves equal text on both
//!    sides, INV-INLINE-2, so the left and right ranges between the same two common chars belong to one hunk).
//!    A pair whose texts are equal after case folding, with every char in a case-insensitive class, is a
//!    case-only change inside a case-insensitive element: **unimportant**.
//! 2. Otherwise the ranges are split by the importance of their chars (the classification of
//!    `split_ranges_by_class`, computed by walking the spans instead of asking per byte).
//! 3. Finally the ranges are widened to grapheme-cluster boundaries, so that a combining mark is never split,
//!    and adjacent ranges of equal importance are merged. The boundaries are found locally around each range
//!    end (`GraphemeCursor`), never for the whole line.

use std::ops::Range;

use hikasa_core::case::eq_folded;
use hikasa_diff::InlineDiff;
use serde::{Deserialize, Serialize};
use unicode_segmentation::GraphemeCursor;

use crate::classify::{CharClass, Span};

/// One inline range, classified (TC-014).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InlineRange {
    /// Byte range in the line text.
    pub range: Range<u32>,
    /// Important (red) or unimportant (blue).
    pub important: bool,
}

/// Classified inline differences of a two-sided row.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClassifiedInline {
    /// Left ranges.
    pub left: Vec<InlineRange>,
    /// Right ranges.
    pub right: Vec<InlineRange>,
    /// The line exceeded the inline limit.
    pub whole_line: bool,
}

/// The spans of one side, for class lookups (`None` = every char important and case-sensitive).
pub(crate) type SideSpans<'s> = Option<&'s [Span]>;

/// Step 0: every range of `diff` unimportant (a `Changed`/`Unimportant` row), widened and merged (step 3).
pub(crate) fn all_unimportant(a: &str, b: &str, diff: &InlineDiff) -> ClassifiedInline {
    let blue = |ranges: &[Range<u32>]| -> Vec<InlineRange> {
        ranges
            .iter()
            .map(|r| InlineRange {
                range: r.clone(),
                important: false,
            })
            .collect()
    };
    ClassifiedInline {
        left: widen_and_merge(a, blue(&diff.left)),
        right: widen_and_merge(b, blue(&diff.right)),
        whole_line: diff.whole_line,
    }
}

/// Classifies `diff` of lines `a` and `b` (steps 1–3 above).
pub(crate) fn classify<'l>(
    a: &'l str,
    b: &'l str,
    diff: &InlineDiff,
    left: SideSpans<'_>,
    right: SideSpans<'_>,
) -> ClassifiedInline {
    let mut out_l: Vec<InlineRange> = Vec::with_capacity(diff.left.len());
    let mut out_r: Vec<InlineRange> = Vec::with_capacity(diff.right.len());
    if left.is_none() && right.is_none() {
        out_l.extend(diff.left.iter().map(|r| InlineRange {
            range: r.clone(),
            important: true,
        }));
        out_r.extend(diff.right.iter().map(|r| InlineRange {
            range: r.clone(),
            important: true,
        }));
    } else {
        // Step 1: pair the ranges by their offset in the common text.
        let lk = common_offsets(&diff.left);
        let rk = common_offsets(&diff.right);
        let (mut i, mut j) = (0usize, 0usize);
        while i < lk.len() || j < rk.len() {
            let key_l = lk.get(i).map(|&(k, _)| k);
            let key_r = rk.get(j).map(|&(k, _)| k);
            let (lr, rr) = match (key_l, key_r) {
                (Some(x), Some(y)) if x == y => {
                    i += 1;
                    j += 1;
                    (Some(lk[i - 1].1.clone()), Some(rk[j - 1].1.clone()))
                }
                (Some(x), Some(y)) if x < y => {
                    i += 1;
                    (Some(lk[i - 1].1.clone()), None)
                }
                (Some(_), None) => {
                    i += 1;
                    (Some(lk[i - 1].1.clone()), None)
                }
                _ => {
                    j += 1;
                    (None, Some(rk[j - 1].1.clone()))
                }
            };
            let text = |line: &'l str, r: &Option<Range<u32>>| -> &'l str {
                r.as_ref()
                    .and_then(|r| line.get(r.start as usize..r.end as usize))
                    .unwrap_or("")
            };
            let (lt, rt) = (text(a, &lr), text(b, &rr));
            // The class test first: it walks the (bounded) spans and fails at once on long-line tails.
            let case_only = !lt.is_empty()
                && !rt.is_empty()
                && all_case_insensitive(left, &lr)
                && all_case_insensitive(right, &rr)
                && eq_folded(lt, rt);
            if case_only {
                if let Some(r) = lr {
                    out_l.push(InlineRange {
                        range: r,
                        important: false,
                    });
                }
                if let Some(r) = rr {
                    out_r.push(InlineRange {
                        range: r,
                        important: false,
                    });
                }
            } else {
                // Step 2: split by importance class.
                if let Some(r) = lr {
                    push_split(&mut out_l, r, left);
                }
                if let Some(r) = rr {
                    push_split(&mut out_r, r, right);
                }
            }
        }
    }
    // Step 3: grapheme boundaries, then merge.
    ClassifiedInline {
        left: widen_and_merge(a, out_l),
        right: widen_and_merge(b, out_r),
        whole_line: diff.whole_line,
    }
}

/// `(offset in the common text, range)` of every range of one side.
fn common_offsets(ranges: &[Range<u32>]) -> Vec<(u32, Range<u32>)> {
    let mut removed = 0u32;
    ranges
        .iter()
        .map(|r| {
            let key = r.start - removed;
            removed += r.end - r.start;
            (key, r.clone())
        })
        .collect()
}

/// The span containing byte `at` (spans are sorted and cover the line).
fn span_at(spans: &[Span], at: u32) -> Option<&Span> {
    let k = spans.partition_point(|s| s.end <= at);
    spans.get(k).filter(|s| s.start <= at)
}

/// Every char of `range` lies in a case-insensitive class (an element without case sensitivity, or
/// everything-else text compared without case). No range: true.
fn all_case_insensitive(spans: SideSpans<'_>, range: &Option<Range<u32>>) -> bool {
    let Some(range) = range else {
        return true;
    };
    let Some(spans) = spans else {
        // Every char is case-sensitive.
        return range.start >= range.end;
    };
    let mut at = range.start;
    while at < range.end {
        let Some(span) = span_at(spans, at) else {
            return false;
        };
        if span.case_sensitive || !matches!(span.class, CharClass::Element | CharClass::Other) {
            return false;
        }
        at = span.end;
    }
    true
}

/// Splits `range` by the importance of its chars and appends the pieces: the result of
/// `split_ranges_by_class` with class = the importance of the span containing each byte (a byte outside every
/// span is important), computed by walking the spans that overlap the range (O(spans), not O(bytes)).
fn push_split(out: &mut Vec<InlineRange>, range: Range<u32>, spans: SideSpans<'_>) {
    let Some(spans) = spans else {
        out.push(InlineRange {
            range,
            important: true,
        });
        return;
    };
    let mut piece: Option<InlineRange> = None;
    let mut push = |out: &mut Vec<InlineRange>, r: Range<u32>, important: bool| {
        if r.start >= r.end {
            return;
        }
        match piece.as_mut() {
            Some(p) if p.important == important && p.range.end == r.start => p.range.end = r.end,
            _ => {
                if let Some(done) = piece.take() {
                    out.push(done);
                }
                piece = Some(InlineRange {
                    range: r,
                    important,
                });
            }
        }
    };
    let mut at = range.start;
    let mut k = spans.partition_point(|s| s.end <= at);
    while at < range.end {
        match spans.get(k) {
            Some(s) if s.start <= at => {
                let end = s.end.min(range.end);
                push(out, at..end, s.important);
                at = end;
                k += 1;
            }
            Some(s) => {
                // A gap before the next span: important.
                let end = s.start.min(range.end);
                push(out, at..end, true);
                at = end;
            }
            None => {
                push(out, at..range.end, true);
                at = range.end;
            }
        }
    }
    if let Some(done) = piece {
        out.push(done);
    }
}

/// The largest grapheme-cluster boundary of `line` at or before `at` (extended clusters).
fn boundary_at_or_before(line: &str, at: usize) -> usize {
    let mut cursor = GraphemeCursor::new(at, line.len(), true);
    if cursor.is_boundary(line, 0).unwrap_or(true) {
        return at;
    }
    cursor.prev_boundary(line, 0).ok().flatten().unwrap_or(0)
}

/// The smallest grapheme-cluster boundary of `line` at or after `at` (extended clusters).
fn boundary_at_or_after(line: &str, at: usize) -> usize {
    let mut cursor = GraphemeCursor::new(at, line.len(), true);
    if cursor.is_boundary(line, 0).unwrap_or(true) {
        return at;
    }
    cursor
        .next_boundary(line, 0)
        .ok()
        .flatten()
        .unwrap_or(line.len())
}

/// Widens every range to grapheme-cluster boundaries of `line` and merges overlapping ranges (important wins)
/// and adjacent ranges of equal importance. The result is sorted, disjoint and non-empty (INV-INLINE-1).
fn widen_and_merge(line: &str, ranges: Vec<InlineRange>) -> Vec<InlineRange> {
    if ranges.is_empty() {
        return ranges;
    }
    // ASCII lines need no widening (a line holds no CR or LF, the only ASCII multi-char cluster); longer lines
    // skip the O(n) test and ask the cursor, which is local.
    let ascii = line.len() <= 65_536 && line.is_ascii();
    let widen = |r: Range<u32>| -> Range<u32> {
        if ascii {
            return r;
        }
        let start = boundary_at_or_before(line, r.start as usize) as u32;
        let end = boundary_at_or_after(line, r.end as usize) as u32;
        start..end.max(start)
    };
    let mut out: Vec<InlineRange> = Vec::with_capacity(ranges.len());
    for r in ranges {
        let range = widen(r.range);
        if range.start >= range.end {
            continue;
        }
        match out.last_mut() {
            Some(last) if range.start < last.range.end => {
                last.range.end = last.range.end.max(range.end);
                last.important |= r.important;
            }
            Some(last) if range.start == last.range.end && last.important == r.important => {
                last.range.end = range.end;
            }
            _ => out.push(InlineRange {
                range,
                important: r.important,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_diff::{InlineOptions, inline_diff};
    use hikasa_formats::ElementId;

    fn span(start: u32, end: u32, class: CharClass, important: bool, cs: bool) -> Span {
        Span {
            start,
            end,
            element: ElementId(1),
            class,
            important,
            case_sensitive: cs,
        }
    }

    #[test]
    fn identity_ranges_are_important() {
        let d = inline_diff("let x = 10;", "let y = 12;", &InlineOptions::DEFAULT);
        let c = classify("let x = 10;", "let y = 12;", &d, None, None);
        assert_eq!(c.left.len(), 2);
        assert!(c.left.iter().chain(&c.right).all(|r| r.important));
    }

    #[test]
    fn case_only_change_in_a_case_insensitive_keyword_is_unimportant() {
        let (a, b) = ("SELECT a", "select a");
        let d = inline_diff(a, b, &InlineOptions::DEFAULT);
        let spans = [
            span(0, 6, CharClass::Element, true, false),
            span(6, 7, CharClass::EmbeddedWhitespace, true, true),
            span(7, 8, CharClass::Other, true, true),
        ];
        let c = classify(a, b, &d, Some(&spans), Some(&spans));
        assert!(!c.left.is_empty());
        assert!(c.left.iter().chain(&c.right).all(|r| !r.important), "{c:?}");
        // The same change in a case-sensitive element is important.
        let cs = [
            span(0, 6, CharClass::Element, true, true),
            span(6, 7, CharClass::EmbeddedWhitespace, true, true),
            span(7, 8, CharClass::Other, true, true),
        ];
        let c = classify(a, b, &d, Some(&cs), Some(&cs));
        assert!(c.left.iter().chain(&c.right).all(|r| r.important));
    }

    #[test]
    fn unimportant_comment_change_is_blue() {
        let (a, b) = ("x = 1; // old", "x = 2; // new");
        let d = inline_diff(a, b, &InlineOptions::DEFAULT);
        let spans = [
            span(0, 7, CharClass::Other, true, true),
            span(7, 13, CharClass::Element, false, true),
        ];
        let c = classify(a, b, &d, Some(&spans), Some(&spans));
        let imp: Vec<(Range<u32>, bool)> = c
            .left
            .iter()
            .map(|r| (r.range.clone(), r.important))
            .collect();
        assert_eq!(imp[0], (4..5, true));
        assert!(imp[1..].iter().all(|(r, i)| !i && r.start >= 7), "{imp:?}");
    }

    #[test]
    fn grapheme_widening() {
        // "e" + combining acute vs "e" + combining grave: the diff is on the mark only.
        let (a, b) = ("xe\u{301}y", "xe\u{300}y");
        let d = inline_diff(a, b, &InlineOptions::DEFAULT);
        let c = classify(a, b, &d, None, None);
        assert_eq!(
            c.left,
            [InlineRange {
                range: 1..4,
                important: true
            }]
        );
        assert_eq!(
            c.right,
            [InlineRange {
                range: 1..4,
                important: true
            }]
        );
    }

    /// The span walk gives exactly `split_ranges_by_class` with the per-byte importance (the reference), gaps
    /// between spans included.
    #[test]
    #[allow(clippy::single_range_in_vec_init)] // one range per call is intended
    fn span_walk_equals_the_per_byte_split() {
        let spans = [
            span(0, 3, CharClass::Other, true, true),
            span(3, 5, CharClass::Element, false, true),
            span(5, 6, CharClass::Element, false, false),
            span(8, 12, CharClass::Other, true, true),
            span(12, 14, CharClass::TrailingWhitespace, false, true),
        ];
        let class_of = |at: u32| u8::from(span_at(&spans, at).is_none_or(|s| s.important));
        for start in 0..16u32 {
            for end in start..17u32 {
                let mut got = Vec::new();
                push_split(&mut got, start..end, Some(&spans));
                let want: Vec<InlineRange> =
                    hikasa_diff::split_ranges_by_class(&[start..end], &class_of)
                        .into_iter()
                        .map(|(range, c)| InlineRange {
                            range,
                            important: c == 1,
                        })
                        .collect();
                assert_eq!(got, want, "{start}..{end}");
            }
        }
    }

    /// Local grapheme boundaries equal the boundaries of the whole line.
    #[test]
    fn local_boundaries_equal_the_full_segmentation() {
        use unicode_segmentation::UnicodeSegmentation;
        let line = "ae\u{301}\u{302}x🇩🇪🇫🇷y\u{1F469}\u{200D}\u{1F4BB}z";
        let mut all: Vec<usize> = line.grapheme_indices(true).map(|(at, _)| at).collect();
        all.push(line.len());
        for at in (0..=line.len()).filter(|&p| line.is_char_boundary(p)) {
            let before = *all.iter().rev().find(|&&b| b <= at).unwrap();
            let after = *all.iter().find(|&&b| b >= at).unwrap();
            assert_eq!(boundary_at_or_before(line, at), before, "{at}");
            assert_eq!(boundary_at_or_after(line, at), after, "{at}");
        }
    }

    #[test]
    fn unimportant_rows_are_all_blue() {
        let (a, b) = ("Hello World", "helloworld");
        let d = inline_diff(a, b, &InlineOptions::DEFAULT);
        let c = all_unimportant(a, b, &d);
        assert!(!c.left.is_empty());
        assert!(c.left.iter().chain(&c.right).all(|r| !r.important));
    }

    #[test]
    fn offsets_pair_hunks() {
        assert_eq!(common_offsets(&[2..4, 6..7]), [(2, 2..4), (4, 6..7)]);
    }
}
