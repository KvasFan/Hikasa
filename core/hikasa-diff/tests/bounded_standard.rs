//! Bounded Standard (D-16 and §2.14.5 as amended in wave 2): Standard alignment runs in O(n log n) on every
//! input. After its work budget runs out, a remaining range goes to Histogram only while its estimated cost fits
//! the fallback budget (`AlignOptions::fallback_budget_for`); beyond it, the range is split at unique lines and
//! its gaps are paired (diff warning `alignment_degraded`). Random input over a small alphabet is the worst case
//! of imara-diff (quadratic: 80,000 tokens per side took 1.7 s, 1,000,000 would take minutes), and Standard used
//! to hand all of it to Histogram.
//!
//! Release builds (`cargo test --release`) check the absolute bound on 1,000,000 lines per side; debug builds
//! check the growth from 25,000 to 200,000 lines (×8 the input: about ×7 the time; the old Histogram fallback grew
//! by several hundred). Each time is the best of three runs, and the tests of this binary run one at a time,
//! because the machine may be shared.

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use hikasa_core::{CancelToken, WarningCode};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, DiffWarningCode, TokenId, TrivialSet, align,
};
use inputs::XorShift;

/// Serialises the tests of this binary (the timing test must not share the machine with its neighbours).
static SERIAL: Mutex<()> = Mutex::new(());

/// Random sequences of `n` tokens over 50 symbols.
fn random(n: usize, seed: u64) -> (Vec<TokenId>, Vec<TokenId>) {
    let mut rng = XorShift(seed);
    let mut side = || (0..n).map(|_| TokenId(rng.below(50) as u32)).collect();
    (side(), side())
}

/// Best of three times of Standard (default options, no scorer) on `n` random lines per side; checks the
/// invariants, `apply`, warning `work_budget_exhausted`, and (when `degraded`) the diff warning
/// `alignment_degraded` (small inputs may fit the fallback budget and get Histogram instead).
fn standard_time(n: usize, degraded: bool) -> Duration {
    let (left, right) = random(n, 0x5EED_0000 + n as u64);
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 50,
    };
    let opts = AlignOptions::with_algorithm(Algorithm::Standard);
    let mut best = Duration::MAX;
    for _ in 0..3 {
        let start = Instant::now();
        let alignment = align(&input, &opts, None, None, &CancelToken::never()).unwrap();
        best = best.min(start.elapsed());
        alignment.check_invariants(&input, &opts).unwrap();
        assert_eq!(alignment.apply(&left, &right), right);
        assert!(
            alignment
                .warnings()
                .iter()
                .any(|w| w.code == WarningCode::WorkBudgetExhausted)
        );
        if degraded {
            assert!(
                alignment
                    .diff_warnings()
                    .iter()
                    .any(|w| w.code == DiffWarningCode::AlignmentDegraded),
                "{n} lines: the remaining ranges exceed the fallback budget"
            );
        }
    }
    best
}

#[test]
fn standard_is_n_log_n_on_random_small_alphabet_input() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if cfg!(debug_assertions) {
        let small = standard_time(25_000, false);
        let large = standard_time(200_000, true);
        let ratio = large.as_secs_f64() / small.as_secs_f64();
        println!(
            "Standard, random lines over 50 symbols: 25,000 → {small:?}, 200,000 → {large:?} (×{ratio:.1})"
        );
        assert!(
            ratio < 32.0,
            "×8 the input took ×{ratio:.1} the time ({small:?} → {large:?}); quadratic growth would be ×64"
        );
    } else {
        let took = standard_time(1_000_000, true);
        println!("Standard, 1,000,000 random lines over 50 symbols: {took:?}");
        assert!(
            took < Duration::from_secs(2),
            "Standard on 1,000,000 random lines per side took {took:?}"
        );
    }
}

/// Owned input: `left`/`right` over symbols, anchors are ids ≥ 1000.
struct Owned {
    left: Vec<TokenId>,
    right: Vec<TokenId>,
    trivial: TrivialSet,
}

impl Owned {
    fn input(&self) -> AlignInput<'_> {
        AlignInput {
            left: &self.left,
            right: &self.right,
            left_weights: &[],
            right_weights: &[],
            trivial: &self.trivial,
            distinct: 0,
        }
    }

    fn align(&self, opts: &AlignOptions) -> hikasa_diff::Alignment {
        let alignment = align(&self.input(), opts, None, None, &CancelToken::never()).unwrap();
        alignment.check_invariants(&self.input(), opts).unwrap();
        assert_eq!(alignment.apply(&self.left, &self.right), self.right);
        alignment
    }
}

/// `blocks` blocks of `len` random lines over 4 symbols on each side (independent), each block followed by a
/// unique anchor line present on both sides.
fn anchored(blocks: usize, len: usize, seed: u64) -> Owned {
    let mut rng = XorShift(seed);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for b in 0..blocks {
        for _ in 0..len {
            left.push(TokenId(rng.below(4) as u32));
            right.push(TokenId(rng.below(4) as u32));
        }
        left.push(TokenId(1000 + b as u32));
        right.push(TokenId(1000 + b as u32));
    }
    Owned {
        left,
        right,
        trivial: TrivialSet::default(),
    }
}

fn degraded(alignment: &hikasa_diff::Alignment) -> bool {
    alignment
        .diff_warnings()
        .iter()
        .any(|w| w.code == DiffWarningCode::AlignmentDegraded)
}

/// Equal pairs `(i, j)` of an alignment.
fn equal_pairs(alignment: &hikasa_diff::Alignment) -> Vec<(u32, u32)> {
    alignment
        .runs()
        .iter()
        .filter(|run| run.kind == hikasa_diff::RunKind::Equal)
        .flat_map(|run| run.left.clone().zip(run.right.clone()))
        .collect()
}

/// A range beyond the fallback budget is split at its unique lines: every anchor is an Equal pair, the gaps
/// between the anchors are small enough for Histogram, and the diff warning `alignment_degraded` is added. A
/// range within the budget is aligned with Histogram exactly (no degraded warning).
#[test]
fn ranges_beyond_the_fallback_budget_are_split_at_unique_lines() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let budget_zero = |algorithm| AlignOptions {
        closeness: None,
        work_budget: Some(0),
        ..AlignOptions::with_algorithm(algorithm)
    };
    // 120 × 51 = 6,120 lines per side: 6,120² = 37.5 M cells > 128 · 12,240 + 32,000,000 = 33.6 M.
    let big = anchored(120, 50, 31);
    let n = big.left.len();
    assert!((n * n) as u64 > AlignOptions::fallback_budget_for(n, n));
    let alignment = big.align(&budget_zero(Algorithm::Standard));
    assert!(degraded(&alignment));
    let pairs = equal_pairs(&alignment);
    for b in 0..120u32 {
        let at = 51 * b + 50;
        assert!(pairs.contains(&(at, at)), "anchor {b} at line {at}");
    }
    // 60 × 51 = 3,060 lines per side fit the fallback budget: the result is Histogram's.
    let small = anchored(60, 50, 32);
    let standard = small.align(&budget_zero(Algorithm::Standard));
    assert!(!degraded(&standard));
    let histogram = small.align(&budget_zero(Algorithm::Histogram));
    assert_eq!(standard.runs(), histogram.runs());
}

/// A gap of the degraded split that alone exceeds the fallback budget is never split again: it is paired by
/// position (closeness off), so its pairs are `(i, i)`; the small gap after the anchor still gets Histogram.
#[test]
fn a_residual_beyond_the_fallback_budget_is_paired_by_position() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = XorShift(33);
    let mut side =
        |len: usize| -> Vec<TokenId> { (0..len).map(|_| TokenId(rng.below(4) as u32)).collect() };
    let (mut left, mut right) = (side(6000), side(6000));
    left.push(TokenId(1000));
    right.push(TokenId(1000));
    left.extend(side(10));
    right.extend(side(10));
    let owned = Owned {
        left,
        right,
        trivial: TrivialSet::default(),
    };
    let opts = AlignOptions {
        closeness: None,
        work_budget: Some(0),
        ..AlignOptions::with_algorithm(Algorithm::Standard)
    };
    let alignment = owned.align(&opts);
    assert!(degraded(&alignment));
    for run in alignment.runs() {
        if run.left.end <= 6000 {
            assert!(
                matches!(
                    run.kind,
                    hikasa_diff::RunKind::Equal | hikasa_diff::RunKind::Changed
                ) && run.left == run.right,
                "positional pairing expected before the anchor, got {run:?}"
            );
        }
    }
    assert!(equal_pairs(&alignment).contains(&(6000, 6000)));
}
