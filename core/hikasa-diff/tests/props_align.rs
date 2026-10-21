//! Alignment properties P-ALIGN-1..8 and P-CLOSE-1..3 (§6.2.3), and P-ALIGN-10 (wave 2: the bounded fallback of
//! Standard, §2.14.5). P-ALIGN-9 (`DenseRemap`, crate-private) is a unit property in `src/dense.rs`.

mod common;

use common::*;
use hikasa_core::{CancelToken, WarningCode};
use hikasa_diff::testing::{align_with_fallback_budget, align_with_helper_threshold};
use hikasa_diff::{Algorithm, AlignOptions, Closeness, DiffWarningCode, RunKind, align};
use proptest::prelude::*;

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_align.txt"))]

    /// P-ALIGN-1: `check_invariants` holds for every algorithm and option combination.
    #[test]
    fn p_align_1_invariants_hold(case in case_strategy(), opts in options_strategy()) {
        let alignment = run(&case, &opts).unwrap();
        prop_assert_eq!(alignment.check_invariants(&case.input(), &opts), Ok(()));
        prop_assert_eq!(alignment.algorithm(), opts.algorithm);
        prop_assert_eq!(alignment.rows().count() as u64, alignment.row_count());
        // Each warning code appears at most once.
        let codes: Vec<WarningCode> = alignment.warnings().iter().map(|w| w.code).collect();
        for (k, code) in codes.iter().enumerate() {
            prop_assert!(!codes[..k].contains(code));
        }
    }

    /// P-ALIGN-2: `apply` reconstructs the right side.
    #[test]
    fn p_align_2_apply_reconstructs_the_right_side(case in case_strategy(), opts in options_strategy()) {
        let alignment = run(&case, &opts).unwrap();
        prop_assert_eq!(alignment.apply(&case.left, &case.right), case.right.clone());
    }

    /// P-ALIGN-3: identical inputs give one Equal run (none when empty).
    #[test]
    fn p_align_3_identical_inputs_give_one_equal_run(case in case_strategy(), opts in options_strategy()) {
        let case = case.identical();
        let alignment = run(&case, &opts).unwrap();
        if case.left.is_empty() {
            prop_assert!(alignment.runs().is_empty());
        } else {
            prop_assert_eq!(alignment.runs().len(), 1);
            prop_assert_eq!(alignment.runs()[0].kind, RunKind::Equal);
        }
    }

    /// P-ALIGN-4: the number of Equal pairs is ≤ the LCS computed by an O(nm) DP oracle; the oracle agrees with
    /// `similar`'s LCS length.
    #[test]
    fn p_align_4_equal_pairs_are_bounded_by_the_lcs(case in case_strategy(), opts in options_strategy()) {
        let alignment = run(&case, &opts).unwrap();
        let lcs = lcs_len(&case.left, &case.right);
        prop_assert!(equal_pairs(&alignment) <= lcs);
        prop_assert_eq!(lcs, similar_lcs(&case.left, &case.right));
    }

    /// P-ALIGN-5: never-align produces no Changed runs; Unaligned produces only (k, k) pairs.
    #[test]
    fn p_align_5_never_align_and_unaligned(case in case_strategy(), opts in options_strategy()) {
        let never = AlignOptions { never_align_differences: true, ..opts };
        let alignment = run(&case, &never).unwrap();
        prop_assert!(alignment.runs().iter().all(|r| r.kind != RunKind::Changed));
        let unaligned = AlignOptions { algorithm: Algorithm::Unaligned, ..opts };
        let alignment = run(&case, &unaligned).unwrap();
        for kind in [RunKind::Equal, RunKind::Changed] {
            prop_assert!(pairs_of(&alignment, kind).iter().all(|&(i, j)| i == j));
        }
    }

    /// P-ALIGN-6: Standard with S = 0, n = m and closeness off gives i = j for every Equal pair.
    #[test]
    fn p_align_6_standard_with_zero_skew_stays_on_the_diagonal(
        case in case_strategy(),
        never in any::<bool>(),
    ) {
        let mut case = case;
        let n = case.left.len().min(case.right.len());
        case.left.truncate(n);
        case.right.truncate(n);
        case.left_weights.truncate(n);
        case.right_weights.truncate(n);
        let opts = AlignOptions {
            algorithm: Algorithm::Standard,
            skew_tolerance: 0,
            closeness: None,
            never_align_differences: never,
            work_budget: None,
        };
        let alignment = run(&case, &opts).unwrap();
        prop_assert!(pairs_of(&alignment, RunKind::Equal).iter().all(|&(i, j)| i == j));
    }

    /// P-ALIGN-7: two runs give equal results, the result is the same with and without a `RunSink`, and the
    /// concatenated sink output equals `runs()`. The helper-thread path of D-54 (threshold lowered to 0 tokens)
    /// gives the same result too (T6).
    #[test]
    fn p_align_7_determinism_and_sinks(case in case_strategy(), opts in options_strategy()) {
        let first = run(&case, &opts).unwrap();
        let second = run(&case, &opts).unwrap();
        prop_assert_eq!(&first, &second);
        let mut sink = Collect::default();
        let scorer = case.scorer();
        let with_sink = align(&case.input(), &opts, Some(&scorer), Some(&mut sink), &CancelToken::never())
            .unwrap();
        prop_assert_eq!(&first, &with_sink);
        prop_assert_eq!(sink.runs.as_slice(), first.runs());
        let helper = align_with_helper_threshold(
            &case.input(), &opts, Some(&scorer), None, &CancelToken::never(), 0,
        )
        .unwrap();
        prop_assert_eq!(&first, &helper);
    }

    /// P-ALIGN-8: `work_budget = 0` gives the Histogram result plus warning `work_budget_exhausted` (whenever
    /// any work is needed, i.e. both sides of the stripped middle are non-empty).
    #[test]
    fn p_align_8_zero_budget_falls_back_to_histogram(case in case_strategy(), opts in options_strategy()) {
        let standard = AlignOptions { algorithm: Algorithm::Standard, work_budget: Some(0), ..opts };
        let histogram = AlignOptions { algorithm: Algorithm::Histogram, ..opts };
        let a = run(&case, &standard).unwrap();
        let b = run(&case, &histogram).unwrap();
        prop_assert_eq!(a.runs(), b.runs());
        prop_assert_eq!(a.algorithm(), Algorithm::Standard);
        let prefix = case.left.iter().zip(&case.right).take_while(|(x, y)| x == y).count();
        let suffix = case.left[prefix..]
            .iter()
            .rev()
            .zip(case.right[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let work = case.left.len() > prefix + suffix && case.right.len() > prefix + suffix;
        let codes: Vec<WarningCode> = a.warnings().iter().map(|w| w.code).collect();
        prop_assert_eq!(codes.contains(&WarningCode::WorkBudgetExhausted), work);
        let others: Vec<WarningCode> =
            codes.into_iter().filter(|&c| c != WarningCode::WorkBudgetExhausted).collect();
        let expected: Vec<WarningCode> = b.warnings().iter().map(|w| w.code).collect();
        prop_assert_eq!(others, expected);
    }

    /// P-CLOSE-1..3 on whole-input gaps: with disjoint alphabets on the two sides, Standard, Histogram and
    /// Patience leave the whole input as one gap, so the pairs of the result are exactly the closeness pairs.
    /// They must be strictly increasing (INV-CLOSE-1), each at least the threshold (INV-CLOSE-2), and of maximal
    /// total similarity, checked by exhaustive search for p, q ≤ 6 (INV-CLOSE-3).
    #[test]
    fn p_close_1_3_closeness_pairs_are_monotone_allowed_and_optimal(
        left in proptest::collection::vec(0u32..4, 0..=6),
        right in proptest::collection::vec(4u32..8, 0..=6),
        table in proptest::collection::vec(0u16..=1000, 64),
        trivial_symbols in proptest::collection::vec(any::<bool>(), 8),
        threshold in 0u16..=1000,
        algorithm in prop::sample::select(vec![Algorithm::Standard, Algorithm::Histogram, Algorithm::Patience]),
    ) {
        let case = build_case(8, &left, &right, trivial_symbols, false, &[], table);
        let opts = AlignOptions {
            closeness: Some(Closeness { threshold_permille: threshold, ..Closeness::default() }),
            ..AlignOptions::with_algorithm(algorithm)
        };
        let alignment = run(&case, &opts).unwrap();
        prop_assert!(alignment.warnings().is_empty());
        let pairs = pairs_of(&alignment, RunKind::Changed);
        prop_assert!(pairs_of(&alignment, RunKind::Equal).is_empty());
        for w in pairs.windows(2) {
            prop_assert!(w[0].0 < w[1].0 && w[0].1 < w[1].1, "INV-CLOSE-1");
        }
        let mut total = 0u64;
        for &(i, j) in &pairs {
            let s = case.sim(i, j);
            prop_assert!(s >= threshold, "INV-CLOSE-2: pair ({}, {}) has similarity {}", i, j, s);
            total += u64::from(s);
        }
        prop_assert_eq!(total, best_pairing(&case, threshold, 0, 0), "INV-CLOSE-3");
    }

    /// INV-CLOSE-2 on general inputs: with closeness on, budgets at their defaults and never-align off, every
    /// Changed pair of Standard, Histogram and Patience comes from the DP and is at least the threshold.
    #[test]
    fn p_close_2_changed_pairs_reach_the_threshold(
        case in case_strategy(),
        threshold in 0u16..=1000,
        algorithm in prop::sample::select(vec![Algorithm::Standard, Algorithm::Histogram, Algorithm::Patience]),
        skew in prop_oneof![0u32..=6, Just(300u32)],
        budget in prop_oneof![Just(None), (0u64..600).prop_map(Some)],
    ) {
        let opts = AlignOptions {
            algorithm,
            skew_tolerance: skew,
            closeness: Some(Closeness { threshold_permille: threshold, ..Closeness::default() }),
            never_align_differences: false,
            work_budget: budget,
        };
        let alignment = run(&case, &opts).unwrap();
        prop_assert!(alignment.warnings().iter().all(|w| w.code != WarningCode::ClosenessBudgetExhausted));
        for (i, j) in pairs_of(&alignment, RunKind::Changed) {
            prop_assert!(case.sim(i, j) >= threshold);
        }
    }
}

/// `similar`'s LCS length (the independent oracle of P-ALIGN-4): the classic LCS table algorithm without
/// heuristics.
fn similar_lcs<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    use similar::DiffOp;
    use similar::algorithms::{Capture, lcs};
    let mut capture = Capture::new();
    lcs::diff_deadline_raw(&mut capture, a, 0..a.len(), b, 0..b.len(), None).unwrap();
    capture
        .into_ops()
        .iter()
        .map(|op| match *op {
            DiffOp::Equal { len, .. } => len,
            _ => 0,
        })
        .sum()
}

/// Best total similarity of a monotone pairing of `left[i..]` with `right[j..]` (exhaustive search).
fn best_pairing(case: &Case, threshold: u16, i: usize, j: usize) -> u64 {
    if i == case.left.len() || j == case.right.len() {
        return 0;
    }
    let mut best =
        best_pairing(case, threshold, i + 1, j).max(best_pairing(case, threshold, i, j + 1));
    let s = case.sim(i as u32, j as u32);
    if s >= threshold {
        best = best.max(u64::from(s) + best_pairing(case, threshold, i + 1, j + 1));
    }
    best
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_align_fallback.txt"))]

    /// P-ALIGN-10 (wave 2, §2.14.5): Standard with any work budget and any fallback budget, down to 0 so that
    /// small inputs reach the degraded split, keeps INV-ALIGN-1..7, reconstructs the right side, pairs at most the
    /// LCS, never pairs differing lines under never-align, and gives the same result with and without a sink
    /// (whose concatenated output equals `runs()`). With both budgets 0, any work degrades; with an unlimited
    /// fallback budget the result is the production one.
    #[test]
    fn p_align_10_the_bounded_fallback_keeps_every_invariant(
        case in case_strategy(),
        opts in options_strategy(),
        fallback in prop_oneof![Just(0u64), 0u64..3000, Just(u64::MAX)],
    ) {
        let opts = AlignOptions { algorithm: Algorithm::Standard, ..opts };
        let scorer = case.scorer();
        let never = CancelToken::never();
        let input = case.input();
        let a = align_with_fallback_budget(&input, &opts, Some(&scorer), None, &never, fallback).unwrap();
        prop_assert_eq!(a.check_invariants(&input, &opts), Ok(()));
        prop_assert_eq!(a.apply(&case.left, &case.right), case.right.clone());
        prop_assert!(equal_pairs(&a) <= lcs_len(&case.left, &case.right));
        if opts.never_align_differences {
            prop_assert!(a.runs().iter().all(|r| r.kind != RunKind::Changed));
        }
        let mut sink = Collect::default();
        let b = align_with_fallback_budget(&input, &opts, Some(&scorer), Some(&mut sink), &never, fallback)
            .unwrap();
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(sink.runs.as_slice(), a.runs());
        let degraded = a.diff_warnings().iter().any(|w| w.code == DiffWarningCode::AlignmentDegraded);
        if fallback == u64::MAX {
            prop_assert!(!degraded);
            prop_assert_eq!(&a, &run(&case, &opts).unwrap());
        }
        let prefix = case.left.iter().zip(&case.right).take_while(|(x, y)| x == y).count();
        let suffix = case.left[prefix..]
            .iter()
            .rev()
            .zip(case.right[prefix..].iter().rev())
            .take_while(|(x, y)| x == y)
            .count();
        let work = case.left.len() > prefix + suffix && case.right.len() > prefix + suffix;
        if fallback == 0 && opts.work_budget == Some(0) {
            prop_assert_eq!(degraded, work);
        }
    }
}
