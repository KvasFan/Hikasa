//! Properties P-SIM-1, P-INLINE-1..4 and P-INTERN-1 (§6.2.3).

mod common;

use std::ops::Range;

use common::config;
use hikasa_diff::{
    BigramProfile, InlineDiff, InlineOptions, TokenId, bigram_similarity, inline_diff, intern_pair,
};
use proptest::prelude::*;

/// Short strings over an alphabet with words, whitespace, punctuation and multi-byte chars.
fn text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(vec![
            "a", "b", "c", "ab", "x1", "_", " ", "  ", "\t", ".", ",", "(", ")", "é", "日本", "🙂",
            "Ω", "-",
        ]),
        0..24,
    )
    .prop_map(|parts| parts.concat())
}

/// A pair of strings: independent, identical, or the second an edit of the first.
fn text_pair() -> impl Strategy<Value = (String, String)> {
    (text(), text(), 0u8..3, text(), any::<prop::sample::Index>()).prop_map(
        |(a, b, mode, insert, at)| match mode {
            0 => (a, b),
            1 => (a.clone(), a),
            _ => {
                let boundaries: Vec<usize> =
                    a.char_indices().map(|(i, _)| i).chain([a.len()]).collect();
                let cut = boundaries[at.index(boundaries.len())];
                let edited = format!("{}{}{}", &a[..cut], insert, &a[cut..]);
                (a, edited)
            }
        },
    )
}

fn inline_options() -> impl Strategy<Value = InlineOptions> {
    prop_oneof![
        Just(InlineOptions::default()),
        (0u32..40, 0u32..30).prop_map(|(max_line_chars, refine_max_chars)| InlineOptions {
            max_line_chars,
            refine_max_chars,
        }),
    ]
}

/// The text left after removing `ranges`.
fn without(text: &str, ranges: &[Range<u32>]) -> String {
    let mut out = String::new();
    let mut at = 0usize;
    for range in ranges {
        out.push_str(&text[at..range.start as usize]);
        at = range.end as usize;
    }
    out.push_str(&text[at..]);
    out
}

/// INV-INLINE-1 for one side.
fn well_formed(text: &str, ranges: &[Range<u32>]) -> Result<(), TestCaseError> {
    let mut previous_end: Option<u32> = None;
    for range in ranges {
        prop_assert!(range.start < range.end, "empty range {:?}", range);
        prop_assert!(range.end as usize <= text.len());
        prop_assert!(text.is_char_boundary(range.start as usize));
        prop_assert!(text.is_char_boundary(range.end as usize));
        if let Some(end) = previous_end {
            // Sorted and disjoint; adjacent ranges are merged, so a gap always separates two ranges.
            prop_assert!(range.start > end, "{:?} after end {}", range, end);
        }
        previous_end = Some(range.end);
    }
    Ok(())
}

/// Sorted multiset of bigram codes, computed independently of `BigramProfile`.
fn bigrams(text: &str) -> Vec<u64> {
    let chars: Vec<char> = text.chars().collect();
    let mut grams: Vec<u64> = match chars.as_slice() {
        [] => Vec::new(),
        [c] => vec![(u64::from(*c) << 32) | 0xFFFF_FFFF],
        _ => chars
            .windows(2)
            .map(|w| (u64::from(w[0]) << 32) | u64::from(w[1]))
            .collect(),
    };
    grams.sort_unstable();
    grams
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_text.txt"))]

    /// P-SIM-1: bigram similarity is symmetric, lies in [0, 1000], and is 1000 iff the multisets are equal.
    #[test]
    fn p_sim_1_bigram_similarity((a, b) in text_pair()) {
        let (pa, pb) = (BigramProfile::new(&a), BigramProfile::new(&b));
        let ab = bigram_similarity(&pa, &pb);
        prop_assert_eq!(ab, bigram_similarity(&pb, &pa));
        prop_assert!(ab <= 1000);
        prop_assert_eq!(ab == 1000, bigrams(&a) == bigrams(&b));
        prop_assert_eq!(pa.char_count() as usize, a.chars().count());
        prop_assert_eq!(pa.bigram_count(), bigrams(&a).len());
    }

    /// P-INLINE-1..4: ranges are sorted, disjoint, non-empty and on char boundaries (1); removing them leaves
    /// an equal common text (2); `a == b` gives no ranges (3); the output is deterministic (4).
    #[test]
    fn p_inline_1_4((a, b) in text_pair(), opts in inline_options()) {
        let d = inline_diff(&a, &b, &opts);
        well_formed(&a, &d.left)?;
        well_formed(&b, &d.right)?;
        prop_assert_eq!(without(&a, &d.left), without(&b, &d.right));
        if a == b {
            prop_assert_eq!(&d, &InlineDiff::default());
        } else {
            prop_assert!(!d.left.is_empty() || !d.right.is_empty());
        }
        let max = opts.max_line_chars as usize;
        prop_assert_eq!(
            d.whole_line,
            a != b && (a.chars().count() > max || b.chars().count() > max)
        );
        prop_assert_eq!(inline_diff(&a, &b, &opts), d);
    }

    /// P-INTERN-1: interned ids are dense and in first-seen order (left then right); equal keys get equal ids.
    #[test]
    fn p_intern_1_ids_are_dense_and_first_seen(
        left in proptest::collection::vec(proptest::collection::vec(0u8..4, 0..3), 0..40),
        right in proptest::collection::vec(proptest::collection::vec(0u8..4, 0..3), 0..40),
    ) {
        let pair = intern_pair(left.iter().map(Vec::as_slice), right.iter().map(Vec::as_slice)).unwrap();
        let keys: Vec<&Vec<u8>> = left.iter().chain(&right).collect();
        let ids: Vec<TokenId> = pair.left.iter().chain(&pair.right).copied().collect();
        prop_assert_eq!(ids.len(), keys.len());
        let mut next = 0u32;
        for (k, id) in ids.iter().enumerate() {
            if id.0 == next {
                prop_assert!(!keys[..k].contains(&keys[k]), "a repeated key got a new id");
                next += 1;
            } else {
                prop_assert!(id.0 < next, "ids must be dense and in first-seen order");
            }
            for m in 0..k {
                prop_assert_eq!(keys[m] == keys[k], ids[m] == ids[k]);
            }
        }
        prop_assert_eq!(pair.distinct, next);
    }
}
