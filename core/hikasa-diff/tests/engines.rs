//! Behaviour of the engines that §2.14–§2.15 states explicitly, beyond the properties and goldens.

mod common;

use common::*;
use hikasa_core::{CancelToken, WarningCode};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, DiffError, Run, RunKind, RunSink, TokenId,
    TrivialSet, align, intern_pair,
};

/// Interned line input built from string slices; blank lines are trivial.
struct Lines {
    left: Vec<TokenId>,
    right: Vec<TokenId>,
    trivial: TrivialSet,
    distinct: u32,
}

impl Lines {
    fn new(left: &[&str], right: &[&str]) -> Lines {
        let keys = intern_pair(
            left.iter().map(|l| l.as_bytes()),
            right.iter().map(|l| l.as_bytes()),
        )
        .unwrap();
        let mut trivial = TrivialSet::with_capacity(keys.distinct);
        for (line, id) in left
            .iter()
            .zip(&keys.left)
            .chain(right.iter().zip(&keys.right))
        {
            if line.trim().is_empty() {
                trivial.insert(*id);
            }
        }
        Lines {
            left: keys.left,
            right: keys.right,
            trivial,
            distinct: keys.distinct,
        }
    }

    fn input<'a>(&'a self, left_weights: &'a [u8], right_weights: &'a [u8]) -> AlignInput<'a> {
        AlignInput {
            left: &self.left,
            right: &self.right,
            left_weights,
            right_weights,
            trivial: &self.trivial,
            distinct: self.distinct,
        }
    }

    fn align(&self, opts: &AlignOptions) -> hikasa_diff::Alignment {
        let input = self.input(&[], &[]);
        let alignment = align(&input, opts, None, None, &CancelToken::never()).unwrap();
        alignment.check_invariants(&input, opts).unwrap();
        alignment
    }
}

fn run(kind: RunKind, left: std::ops::Range<u32>, right: std::ops::Range<u32>) -> Run {
    Run { kind, left, right }
}

fn standard() -> AlignOptions {
    AlignOptions::with_algorithm(Algorithm::Standard)
}

/// §2.14.5: a weighted line (priority 4, score 17) wins over up to 16 crossing unweighted unique lines.
#[test]
fn a_weighted_line_beats_sixteen_crossing_unweighted_lines() {
    let body: Vec<String> = (0..16).map(|k| format!("unique line {k}")).collect();
    let mut left: Vec<&str> = vec!["HEADER"];
    left.extend(body.iter().map(String::as_str));
    let mut right: Vec<&str> = body.iter().map(String::as_str).collect();
    right.push("HEADER");
    // The last lines differ, so no common prefix or suffix hides the choice.
    left.push("end left");
    right.push("end right");
    let lines = Lines::new(&left, &right);
    let mut left_weights = vec![0u8; left.len()];
    left_weights[0] = 4;
    let mut right_weights = vec![0u8; right.len()];
    right_weights[16] = 4;
    let opts = AlignOptions {
        closeness: None,
        ..standard()
    };
    let input = lines.input(&left_weights, &right_weights);
    let weighted = align(&input, &opts, None, None, &CancelToken::never()).unwrap();
    assert_eq!(
        pairs_of(&weighted, RunKind::Equal),
        [(0, 16)],
        "the header is the only anchor"
    );
    // Without weights the 16 lines win.
    let unweighted = lines.align(&opts);
    assert_eq!(equal_pairs(&unweighted), 16);
    // With 18 crossing unweighted lines (total 18 > 17) the unweighted chain wins.
    let body: Vec<String> = (0..18).map(|k| format!("unique line {k}")).collect();
    let mut left: Vec<&str> = vec!["HEADER"];
    left.extend(body.iter().map(String::as_str));
    left.push("end left");
    let mut right: Vec<&str> = body.iter().map(String::as_str).collect();
    right.push("HEADER");
    right.push("end right");
    let lines = Lines::new(&left, &right);
    let mut left_weights = vec![0u8; left.len()];
    left_weights[0] = 4;
    let mut right_weights = vec![0u8; right.len()];
    right_weights[18] = 4;
    let input = lines.input(&left_weights, &right_weights);
    let alignment = align(&input, &opts, None, None, &CancelToken::never()).unwrap();
    assert_eq!(equal_pairs(&alignment), 18);
}

/// Weights above 4 count as 4 (totality): the same result as weight 4.
#[test]
fn weights_above_four_count_as_four() {
    let lines = Lines::new(&["H", "a", "b", "x"], &["a", "b", "H", "y"]);
    let opts = AlignOptions {
        closeness: None,
        ..standard()
    };
    let at = |w: u8| {
        let (lw, rw) = ([w, 0, 0, 0], [0, 0, w, 0]);
        align(
            &lines.input(&lw, &rw),
            &opts,
            None,
            None,
            &CancelToken::never(),
        )
        .unwrap()
    };
    assert_eq!(at(4), at(200));
    assert_eq!(pairs_of(&at(4), RunKind::Equal), [(0, 2)]);
}

/// Stage B scores each maximal block from its first eligible line: a block whose start is a matching blank line
/// is still found (the deviation recorded in `algo/standard.rs`), so the alignment reaches the LCS here.
#[test]
fn stage_b_finds_blocks_behind_a_trivial_match() {
    let left = ["", "x", "y", "end1"];
    let right = ["x", "y", "", "x", "y", "end2"];
    let lines = Lines::new(&left, &right);
    let alignment = lines.align(&standard());
    assert_eq!(
        alignment.runs(),
        [
            run(RunKind::Insert, 0..0, 0..2),
            run(RunKind::Equal, 0..3, 2..5),
            // No scorer: the differing last lines are not similar enough to pair.
            run(RunKind::Delete, 3..4, 5..5),
            run(RunKind::Insert, 4..4, 5..6),
        ]
    );
    assert_eq!(equal_pairs(&alignment), lcs_len(&lines.left, &lines.right));
}

/// Stage C: when only trivial keys match, blocks of trivial lines are used.
#[test]
fn stage_c_aligns_trivial_blocks() {
    let left = ["a", "", "", "b"];
    let right = ["c", "", "", "d"];
    let lines = Lines::new(&left, &right);
    let alignment = lines.align(&AlignOptions {
        closeness: None,
        ..standard()
    });
    assert_eq!(
        alignment.runs(),
        [
            run(RunKind::Changed, 0..1, 0..1),
            run(RunKind::Equal, 1..3, 1..3),
            run(RunKind::Changed, 3..4, 3..4),
        ]
    );
}

/// Skew: with `S = 0` a shifted block is outside the window and the range is paired by position; with the
/// default skew it is aligned.
#[test]
fn the_skew_window_limits_anchors() {
    let left = ["a", "b", "c", "d"];
    let right = ["new", "a", "b", "c", "d", "tail"];
    let lines = Lines::new(&left, &right);
    let narrow = lines.align(&AlignOptions {
        skew_tolerance: 0,
        closeness: None,
        ..standard()
    });
    assert_eq!(equal_pairs(&narrow), 0);
    let wide = lines.align(&AlignOptions {
        closeness: None,
        ..standard()
    });
    assert_eq!(equal_pairs(&wide), 4);
    let unlimited = lines.align(&AlignOptions {
        skew_tolerance: u32::MAX,
        closeness: None,
        ..standard()
    });
    assert_eq!(unlimited, wide);
}

/// Standard reports progressively: several sink calls, whose concatenation is `runs()`.
#[test]
fn standard_reports_progressively() {
    let left: Vec<String> = (0..60)
        .map(|k| {
            if k % 10 == 5 {
                format!("old {k}")
            } else {
                format!("line {k}")
            }
        })
        .collect();
    let right: Vec<String> = (0..60)
        .map(|k| {
            if k % 10 == 5 {
                format!("new {k}")
            } else {
                format!("line {k}")
            }
        })
        .collect();
    let left: Vec<&str> = left.iter().map(String::as_str).collect();
    let right: Vec<&str> = right.iter().map(String::as_str).collect();
    let lines = Lines::new(&left, &right);
    let input = lines.input(&[], &[]);
    let mut sink = Collect::default();
    let alignment = align(
        &input,
        &standard(),
        None,
        Some(&mut sink),
        &CancelToken::never(),
    )
    .unwrap();
    assert!(
        sink.calls > 1,
        "expected progressive calls, got {}",
        sink.calls
    );
    assert_eq!(sink.runs, alignment.runs());
    // Six changed lines without a scorer: Delete + Insert each, between seven Equal runs.
    assert_eq!(alignment.runs().len(), 19);
}

/// A sink that keeps every call separately.
#[derive(Default)]
struct Calls(Vec<Vec<Run>>);

impl RunSink for Calls {
    fn runs_ready(&mut self, runs: &[Run]) {
        self.0.push(runs.to_vec());
    }
}

/// Regression (sink timing, §1.5 `CompareObserver::partial`): Myers and Histogram report the common prefix in a
/// call of its own, before the imara-diff call, instead of everything at the end.
#[test]
fn myers_and_histogram_report_the_common_prefix_first() {
    let left: Vec<String> = (0..10).map(|k| format!("line {k}")).collect();
    let mut right = left.clone();
    right[5] = "changed".to_owned();
    let left: Vec<&str> = left.iter().map(String::as_str).collect();
    let right: Vec<&str> = right.iter().map(String::as_str).collect();
    let lines = Lines::new(&left, &right);
    for algorithm in [Algorithm::Myers, Algorithm::Histogram] {
        let opts = AlignOptions {
            closeness: None,
            ..AlignOptions::with_algorithm(algorithm)
        };
        let mut calls = Calls::default();
        let alignment = align(
            &lines.input(&[], &[]),
            &opts,
            None,
            Some(&mut calls),
            &CancelToken::never(),
        )
        .unwrap();
        assert_eq!(
            calls.0.first().map(Vec::as_slice),
            Some(&[run(RunKind::Equal, 0..5, 0..5)][..]),
            "{algorithm}"
        );
        assert!(calls.0.len() >= 2, "{algorithm}: {} calls", calls.0.len());
        assert_eq!(calls.0.concat(), alignment.runs(), "{algorithm}");
    }
}

/// TC-010 / §2.14.3: Myers through imara-diff is not a minimal LCS. imara-diff's Myers preprocessing prunes a
/// token that occurs often in the other side when its neighbours are mostly unmatched, and a pruned token can
/// never match; here the one common line is pruned, so Myers pairs nothing while the LCS is 1 (Histogram finds
/// it). P-ALIGN-4 therefore only claims `Equal pairs ≤ LCS` for every engine. This pins the documented example of
/// imara-diff 0.2; if an imara-diff upgrade changes it, update TC-010 and §2.14.3 with this test.
#[test]
fn myers_is_not_guaranteed_minimal() {
    let lines = Lines::new(&["a", "a"], &["b", "c", "a", "d", "b"]);
    let lcs = lcs_len(&lines.left, &lines.right);
    assert_eq!(lcs, 1);
    let off = |algorithm| AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(algorithm)
    };
    let myers = lines.align(&off(Algorithm::Myers));
    assert_eq!(equal_pairs(&myers), 0);
    let histogram = lines.align(&off(Algorithm::Histogram));
    assert_eq!(equal_pairs(&histogram), lcs);
}

/// Every algorithm except Unaligned strips the common prefix and suffix as Equal (§2.14.1).
#[test]
fn prefix_and_suffix_are_equal_runs() {
    let lines = Lines::new(&["p", "q", "x", "s"], &["p", "q", "y", "z", "s"]);
    for algorithm in Algorithm::ALL {
        let alignment = lines.align(&AlignOptions::with_algorithm(algorithm));
        let runs = alignment.runs();
        if algorithm == Algorithm::Unaligned {
            assert_eq!(runs[0], run(RunKind::Equal, 0..2, 0..2));
            continue;
        }
        assert_eq!(runs.first(), Some(&run(RunKind::Equal, 0..2, 0..2)));
        assert_eq!(runs.last(), Some(&run(RunKind::Equal, 3..4, 4..5)));
    }
}

/// Myers pairs gaps by position; the `closeness_ignored` warning appears only when closeness is on (D-17).
#[test]
fn myers_warns_only_when_closeness_is_on() {
    let lines = Lines::new(&["a", "b"], &["c", "b"]);
    let on = lines.align(&AlignOptions::with_algorithm(Algorithm::Myers));
    assert_eq!(on.warnings().len(), 1);
    assert_eq!(on.warnings()[0].code, WarningCode::ClosenessIgnored);
    assert!(on.warnings()[0].spec.is_none());
    let off = lines.align(&AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Myers)
    });
    assert!(off.warnings().is_empty());
    // Unaligned never warns about closeness.
    let unaligned = lines.align(&AlignOptions::with_algorithm(Algorithm::Unaligned));
    assert!(unaligned.warnings().is_empty());
}

/// Without a scorer, closeness pairs only equal keys (every other similarity is 0).
#[test]
fn closeness_without_a_scorer_pairs_only_equal_keys() {
    let lines = Lines::new(&["a", "b", "c"], &["x", "y", "b", "z"]);
    let opts = AlignOptions {
        closeness: Some(Closeness {
            threshold_permille: 1,
            ..Closeness::default()
        }),
        ..AlignOptions::with_algorithm(Algorithm::Histogram)
    };
    let alignment = lines.align(&opts);
    assert!(pairs_of(&alignment, RunKind::Changed).is_empty());
    assert_eq!(pairs_of(&alignment, RunKind::Equal), [(1, 2)]);
}

/// A closeness gap larger than the total cell budget is paired by position, with one warning.
#[test]
fn the_total_cell_budget_is_shared_by_the_gaps() {
    let lines = Lines::new(
        &["a1", "k", "b1", "b2", "k2", "c1"],
        &["a2", "k", "b3", "b4", "k2", "c2"],
    );
    let opts = AlignOptions {
        closeness: Some(Closeness {
            max_cells_total: Some(5),
            ..Closeness::default()
        }),
        ..AlignOptions::with_algorithm(Algorithm::Histogram)
    };
    let alignment = lines.align(&opts);
    let codes: Vec<WarningCode> = alignment.warnings().iter().map(|w| w.code).collect();
    assert_eq!(codes, [WarningCode::ClosenessBudgetExhausted]);
    // 1 + 4 cells fit (the first two gaps: no scorer, so nothing pairs there); the third gap (1 cell) would
    // exceed 5 and is paired by position.
    assert_eq!(alignment.runs().len(), 7);
    assert_eq!(alignment.runs()[6], run(RunKind::Changed, 5..6, 5..6));
}

/// Unaligned under never-align: differing pairs become Delete + Insert, equal pairs stay (k, k).
#[test]
fn unaligned_never_align() {
    let lines = Lines::new(&["a", "b", "c"], &["a", "x", "c", "d"]);
    let alignment = lines.align(&AlignOptions {
        never_align_differences: true,
        ..AlignOptions::with_algorithm(Algorithm::Unaligned)
    });
    assert_eq!(
        alignment.runs(),
        [
            run(RunKind::Equal, 0..1, 0..1),
            run(RunKind::Delete, 1..2, 1..1),
            run(RunKind::Insert, 2..2, 1..2),
            run(RunKind::Equal, 2..3, 2..3),
            run(RunKind::Insert, 3..3, 3..4),
        ]
    );
}

/// Errors: mismatched weights (a caller bug) and a cancelled token.
#[test]
fn errors() {
    let lines = Lines::new(&["a", "b"], &["a"]);
    let input = lines.input(&[1], &[]);
    assert_eq!(
        align(&input, &standard(), None, None, &CancelToken::never()),
        Err(DiffError::WeightsLength {
            expected: 2,
            got: 1
        })
    );
    let input = lines.input(&[], &[1, 2]);
    assert_eq!(
        align(&input, &standard(), None, None, &CancelToken::never()),
        Err(DiffError::WeightsLength {
            expected: 1,
            got: 2
        })
    );
    let cancel = CancelToken::new();
    cancel.cancel();
    for algorithm in Algorithm::ALL {
        let opts = AlignOptions::with_algorithm(algorithm);
        assert_eq!(
            align(&lines.input(&[], &[]), &opts, None, None, &cancel),
            Err(DiffError::Cancelled)
        );
    }
}

/// Token ids far beyond `distinct` (a caller bug, or hash-like ids) are renumbered internally: the result is the
/// same as with dense ids and nothing is sized by the huge ids. The ids with a trivial key sit just above the
/// renumbering threshold (`4·(n+m) + 4096`), because the caller's `TrivialSet` costs one bit per id up to the
/// largest one inserted (regression: `u32::MAX`-sized ids made this test allocate 512 MiB); ids near `u32::MAX`
/// are covered without trivial keys.
#[test]
fn sparse_token_ids_give_the_same_result() {
    let dense_left: Vec<u32> = vec![0, 1, 2, 3, 1, 4];
    let dense_right: Vec<u32> = vec![0, 2, 5, 3, 4, 4];
    let dl: Vec<TokenId> = dense_left.iter().copied().map(TokenId).collect();
    let dr: Vec<TokenId> = dense_right.iter().copied().map(TokenId).collect();
    let near_threshold = |id: u32| TokenId((1 << 20) + 7 * id);
    let near_max = |id: u32| TokenId(u32::MAX - 7 * id);
    let mut dense_trivial = TrivialSet::with_capacity(6);
    dense_trivial.insert(TokenId(4));
    let mut sparse_trivial = TrivialSet::default();
    sparse_trivial.insert(near_threshold(4));
    let no_trivial = TrivialSet::default();
    let cases = [
        (
            &near_threshold as &dyn Fn(u32) -> TokenId,
            &sparse_trivial,
            &dense_trivial,
        ),
        (&near_max, &no_trivial, &no_trivial),
    ];
    for (spread, sparse_trivial, dense_trivial) in cases {
        let sl: Vec<TokenId> = dense_left.iter().map(|&t| spread(t)).collect();
        let sr: Vec<TokenId> = dense_right.iter().map(|&t| spread(t)).collect();
        same_result(&dl, &dr, dense_trivial, &sl, &sr, sparse_trivial);
    }
}

fn same_result(
    dl: &[TokenId],
    dr: &[TokenId],
    dense_trivial: &TrivialSet,
    sl: &[TokenId],
    sr: &[TokenId],
    sparse_trivial: &TrivialSet,
) {
    for algorithm in Algorithm::ALL {
        let opts = AlignOptions::with_algorithm(algorithm);
        let dense = AlignInput {
            left: dl,
            right: dr,
            left_weights: &[],
            right_weights: &[],
            trivial: dense_trivial,
            distinct: 6,
        };
        let sparse = AlignInput {
            left: sl,
            right: sr,
            trivial: sparse_trivial,
            distinct: 6,
            ..dense
        };
        let a = align(&dense, &opts, None, None, &CancelToken::never()).unwrap();
        let b = align(&sparse, &opts, None, None, &CancelToken::never()).unwrap();
        assert_eq!(a, b, "{algorithm}");
        b.check_invariants(&sparse, &opts).unwrap();
    }
}

/// The run model: rows, stable run ids (indices) and serde names.
#[test]
fn run_model_and_serialisation() {
    let lines = Lines::new(&["a", "b", "c"], &["a", "x", "c", "d"]);
    let alignment = lines.align(&AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Histogram)
    });
    assert_eq!(alignment.left_len(), 3);
    assert_eq!(alignment.right_len(), 4);
    assert_eq!(alignment.row_count(), 4);
    let json = serde_json::to_string(&alignment.runs()[1]).unwrap();
    assert_eq!(
        json,
        r#"{"kind":"changed","left":{"start":1,"end":2},"right":{"start":1,"end":2}}"#
    );
    let back: Run = serde_json::from_str(&json).unwrap();
    assert_eq!(back, alignment.runs()[1]);
    let rows: Vec<(Option<u32>, Option<u32>)> =
        alignment.rows().map(|r| (r.left, r.right)).collect();
    assert_eq!(
        rows,
        [
            (Some(0), Some(0)),
            (Some(1), Some(1)),
            (Some(2), Some(2)),
            (None, Some(3))
        ]
    );
    assert_eq!(serde_json::to_string(&TokenId(7)).unwrap(), "7");
}
