//! A10.7 (D-266): P-ANCHOR — random sequences and anchor sets: INV-ALIGN-1…7 and 9 hold, the result equals the
//! concatenation of `align` per segment, isolated lines are in Delete/Insert runs and never paired by closeness;
//! crossing, overlapping and out-of-range pairs are dropped with `anchor_dropped`; no anchors = `align` bit for bit
//! on P-ALIGN's generators.

mod common;

use common::*;
use hikasa_core::CancelToken;
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Anchor, AnchorSide, DiffWarningCode, Run, RunKind, align,
    align_anchored, align_traced, normalize_anchors,
};
use proptest::prelude::*;

fn anchor_strategy() -> impl Strategy<Value = Anchor> {
    prop_oneof![
        3 => (0u32..210, 1u32..6, 0u32..210, 1u32..6).prop_map(|(l, ll, r, rl)| Anchor::Pair {
            left: l..l + ll,
            right: r..r + rl,
        }),
        1 => (0u32..210, 0u32..6).prop_map(|(l, r)| Anchor::Pair { left: l..l + 1, right: r..r + 1 }),
        2 => (any::<bool>(), 0u32..210, 1u32..4).prop_map(|(left, s, len)| Anchor::Isolate {
            side: if left { AnchorSide::Left } else { AnchorSide::Right },
            lines: s..s + len,
        }),
    ]
}

/// The runs of `a` inside `left × right`, shifted to the segment's origin.
fn runs_in(runs: &[Run], left: &std::ops::Range<u32>, right: &std::ops::Range<u32>) -> Vec<Run> {
    runs.iter()
        .filter(|r| {
            r.left.start >= left.start
                && r.left.end <= left.end
                && r.right.start >= right.start
                && r.right.end <= right.end
                && !(r.left.is_empty() && r.right.is_empty())
        })
        .filter(|r| !(r.left.start == left.end && r.right.start == right.end))
        .map(|r| Run {
            kind: r.kind,
            left: r.left.start - left.start..r.left.end - left.start,
            right: r.right.start - right.start..r.right.end - right.start,
        })
        .collect()
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/anchors.txt"))]

    /// P-ANCHOR (A10.7).
    #[test]
    fn p_anchor_segments_isolation_and_invariants(
        case in case_strategy(),
        opts in options_strategy(),
        anchors in proptest::collection::vec(anchor_strategy(), 0..6),
    ) {
        let input = case.input();
        let scorer = case.scorer();
        let never = CancelToken::never();
        let a = align_anchored(&input, &opts, &anchors, false, Some(&scorer), None, &never).unwrap();
        let check = a.check_invariants_anchored(&input, &opts, &anchors);
        prop_assert!(check.is_ok(), "{:?}", check);
        prop_assert_eq!(a.apply(&case.left, &case.right), case.right.clone());
        let (n, m) = (case.left.len() as u32, case.right.len() as u32);
        let (kept, dropped) = normalize_anchors(&anchors, n, m);
        prop_assert_eq!(kept.len() + dropped.len(), anchors.len());
        prop_assert!(dropped.iter().all(|w| w.code == DiffWarningCode::AnchorDropped));
        let has_iso = kept.iter().any(|k| matches!(k, Anchor::Isolate { .. }));
        // Without isolated lines, every segment is exactly `align` of its lines (with the segment's own budgets).
        if !has_iso {
            let mut bounds = Vec::new();
            let (mut l, mut r) = (0u32, 0u32);
            for k in &kept {
                if let Anchor::Pair { left, right } = k {
                    bounds.push((l..left.start, r..right.start, false));
                    bounds.push((left.clone(), right.clone(), true));
                    (l, r) = (left.end, right.end);
                }
            }
            bounds.push((l..n, r..m, false));
            for (ls, rs, pair) in bounds {
                if ls.is_empty() && rs.is_empty() {
                    continue;
                }
                let got = runs_in(a.runs(), &ls, &rs);
                if pair && ls.len() == 1 && rs.len() == 1 {
                    prop_assert_eq!(got.len(), if case.left[ls.start as usize] == case.right[rs.start as usize] || !opts.never_align_differences { 1 } else { 2 });
                    continue;
                }
                let sub = AlignInput {
                    left: &case.left[ls.start as usize..ls.end as usize],
                    right: &case.right[rs.start as usize..rs.end as usize],
                    left_weights: if case.left_weights.is_empty() { &[] } else { &case.left_weights[ls.start as usize..ls.end as usize] },
                    right_weights: if case.right_weights.is_empty() { &[] } else { &case.right_weights[rs.start as usize..rs.end as usize] },
                    trivial: &case.trivial,
                    distinct: case.alphabet,
                };
                let (dl, dr) = (ls.start, rs.start);
                let shifted = |i: u32, j: u32| scorer_at(&case, dl + i, dr + j);
                let want = align(&sub, &opts, Some(&shifted), None, &never).unwrap();
                prop_assert_eq!(got.as_slice(), want.runs(), "segment {:?} {:?}", ls, rs);
            }
        }
        // Isolated lines are never paired, by closeness or position.
        for k in &kept {
            if let Anchor::Isolate { side, lines } = k {
                for run in a.runs() {
                    let r = if *side == AnchorSide::Left { &run.left } else { &run.right };
                    if r.start < lines.end && lines.start < r.end {
                        prop_assert!(matches!(run.kind, RunKind::Delete | RunKind::Insert));
                    }
                }
            }
        }
        // The traced alignment gives the same runs.
        let (t, _) = align_traced(&input, &opts, &anchors, false, Some(&scorer), None, &never).unwrap();
        prop_assert_eq!(t.runs(), a.runs());
    }

    /// No anchors: `align` bit for bit (A10.7).
    #[test]
    fn no_anchors_is_align(case in case_strategy(), opts in options_strategy()) {
        let scorer = case.scorer();
        let never = CancelToken::never();
        let a = align_anchored(&case.input(), &opts, &[], false, Some(&scorer), None, &never).unwrap();
        let b = align(&case.input(), &opts, Some(&scorer), None, &never).unwrap();
        prop_assert_eq!(a, b);
    }
}

/// The case's similarity of left line `l` and right line `r` (whole-sequence indices).
fn scorer_at(case: &Case, l: u32, r: u32) -> u16 {
    use hikasa_diff::PairScorer;
    case.scorer().similarity(l, r)
}

#[test]
fn crossing_overlapping_and_out_of_range_pairs_are_dropped() {
    let pair =
        |l: std::ops::Range<u32>, r: std::ops::Range<u32>| Anchor::Pair { left: l, right: r };
    let (kept, dropped) = normalize_anchors(
        &[
            pair(5..6, 5..6),
            pair(2..3, 7..8),  // crosses the first (right not after it, once sorted)
            pair(5..7, 9..10), // overlaps the kept 5..6
            pair(8..9, 20..21), // beyond the right side
            pair(9..9, 1..2),  // empty
            Anchor::Isolate {
                side: AnchorSide::Left,
                lines: 9..30,
            },
            Anchor::Isolate {
                side: AnchorSide::Right,
                lines: 12..13,
            },
        ],
        10,
        10,
    );
    // Sorted by left start: 2..3 is kept first, so 5..6 → 5..6 crosses it; 5..7 → 9..10 comes after it.
    assert_eq!(
        kept,
        [
            pair(2..3, 7..8),
            pair(5..7, 9..10),
            Anchor::Isolate {
                side: AnchorSide::Left,
                lines: 9..10
            }
        ]
    );
    let messages: Vec<&str> = dropped.iter().map(|w| w.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "anchor `align left 9 with right 21` dropped: it leaves the lines",
            "anchor `align left 10 (empty) with right 2` dropped: a range is empty",
            "anchor `isolate right 13` dropped: it leaves the lines",
            "anchor `align left 6 with right 6` dropped: it overlaps or crosses an earlier anchor",
        ]
    );
}

#[test]
fn a_pair_of_single_lines_is_one_row() {
    let case = build_case(
        4,
        &[0, 1, 2, 3],
        &[0, 3, 2, 1],
        vec![false; 4],
        false,
        &[],
        vec![0; 64],
    );
    let opts = AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Histogram)
    };
    let a = align_anchored(
        &case.input(),
        &opts,
        &[Anchor::Pair {
            left: 1..2,
            right: 3..4,
        }],
        false,
        None,
        None,
        &CancelToken::never(),
    )
    .unwrap();
    let kinds: Vec<(RunKind, std::ops::Range<u32>, std::ops::Range<u32>)> = a
        .runs()
        .iter()
        .map(|r| (r.kind, r.left.clone(), r.right.clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            (RunKind::Equal, 0..1, 0..1),
            (RunKind::Insert, 1..1, 1..3),
            (RunKind::Equal, 1..2, 3..4),
            (RunKind::Delete, 2..4, 4..4),
        ]
    );
}
