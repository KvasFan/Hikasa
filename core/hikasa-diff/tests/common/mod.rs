//! Shared helpers of the hikasa-diff integration tests: the golden-file helper (§6.1), the proptest
//! configuration, proptest strategies, owned alignment inputs, scorers, a sink and the O(nm) LCS oracle.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use hikasa_core::CancelToken;
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Alignment, Closeness, DiffError, PairScorer, Run, RunKind,
    RunSink, TokenId, TrivialSet, align,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence};

/// Compares `actual` with the golden file `expected`; with `HIKASA_BLESS=1` the golden file is (re)written
/// instead. Text files are shown as a `pretty_assertions` diff.
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    let bless = std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1");
    if bless {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let golden = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "missing golden file {} ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if golden != actual {
        match (std::str::from_utf8(&golden), std::str::from_utf8(actual)) {
            (Ok(want), Ok(got)) => {
                pretty_assertions::assert_eq!(want, got, "golden {}", expected.display())
            }
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// `tests/fixtures/<suite>`.
pub fn fixtures(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are persisted
/// in `tests/proptest-regressions/<suite>.txt` (relative to the package, the working directory of `cargo test`).
pub fn config(regressions: &'static str) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// An owned alignment input over a small alphabet: tokens are symbols `0..alphabet` (already dense).
#[derive(Debug, Clone)]
pub struct Case {
    pub alphabet: u32,
    pub left: Vec<TokenId>,
    pub right: Vec<TokenId>,
    pub trivial: TrivialSet,
    pub trivial_symbols: Vec<bool>,
    pub left_weights: Vec<u8>,
    pub right_weights: Vec<u8>,
    /// Similarity of symbol pairs, `table[a * 8 + b]` per mille.
    pub table: Vec<u16>,
}

impl Case {
    pub fn input(&self) -> AlignInput<'_> {
        AlignInput {
            left: &self.left,
            right: &self.right,
            left_weights: &self.left_weights,
            right_weights: &self.right_weights,
            trivial: &self.trivial,
            distinct: self.alphabet,
        }
    }

    pub fn scorer(&self) -> TableScorer<'_> {
        TableScorer { case: self }
    }

    /// Similarity of left line `l` and right line `r` as the closeness DP sees it (§2.15.2).
    pub fn sim(&self, l: u32, r: u32) -> u16 {
        let (x, y) = (self.left[l as usize], self.right[r as usize]);
        if x == y {
            1000
        } else if self.trivial.contains(x) || self.trivial.contains(y) {
            0
        } else {
            self.scorer().similarity(l, r)
        }
    }

    /// The same case with identical sides (left copied to the right).
    pub fn identical(&self) -> Case {
        let mut case = self.clone();
        case.right = case.left.clone();
        case.right_weights = case.left_weights.clone();
        case
    }
}

/// A scorer that looks up the symbol pair in the case's table.
pub struct TableScorer<'c> {
    case: &'c Case,
}

impl PairScorer for TableScorer<'_> {
    fn similarity(&self, l: u32, r: u32) -> u16 {
        let a = self.case.left[l as usize].0 as usize;
        let b = self.case.right[r as usize].0 as usize;
        self.case.table[a * 8 + b]
    }
}

fn symbols(alphabet: u32, max_len: usize) -> impl Strategy<Value = Vec<u32>> {
    proptest::collection::vec(0..alphabet, 0..=max_len)
}

/// An edit of a sequence: (position seed, kind 0 = insert / 1 = delete / 2 = replace, symbol).
fn mutate(base: &[u32], edits: &[(usize, u8, u32)]) -> Vec<u32> {
    let mut out = base.to_vec();
    for &(seed, kind, symbol) in edits {
        let at = if out.is_empty() {
            0
        } else {
            seed % (out.len() + 1)
        };
        match kind {
            0 => out.insert(at, symbol),
            1 if at < out.len() => {
                out.remove(at);
            }
            2 if at < out.len() => out[at] = symbol,
            _ => out.push(symbol),
        }
    }
    out
}

/// Cases of §6.2.3: alphabets of 1–8 symbols, lengths 0–200, random trivial sets and weights; the right side is
/// either independent or an edited copy of the left side.
pub fn case_strategy() -> impl Strategy<Value = Case> {
    (1u32..=8)
        .prop_flat_map(|alphabet| {
            (
                Just(alphabet),
                symbols(alphabet, 200),
                symbols(alphabet, 200),
                proptest::collection::vec((any::<usize>(), 0u8..3, 0..alphabet), 0..20),
                any::<bool>(),
                proptest::collection::vec(any::<bool>(), alphabet as usize),
                prop_oneof![Just(false), Just(true)],
                proptest::collection::vec(0u8..=4, 400),
                proptest::collection::vec(0u16..=1000, 64),
            )
        })
        .prop_map(
            |(alphabet, left, other, edits, edited, trivial_symbols, weighted, weights, table)| {
                let right = if edited { mutate(&left, &edits) } else { other };
                build_case(
                    alphabet,
                    &left,
                    &right,
                    trivial_symbols,
                    weighted,
                    &weights,
                    table,
                )
            },
        )
}

/// Builds a case from symbol sequences.
pub fn build_case(
    alphabet: u32,
    left: &[u32],
    right: &[u32],
    trivial_symbols: Vec<bool>,
    weighted: bool,
    weights: &[u8],
    table: Vec<u16>,
) -> Case {
    let mut trivial = TrivialSet::with_capacity(alphabet);
    for (symbol, &is_trivial) in trivial_symbols.iter().enumerate() {
        if is_trivial {
            trivial.insert(TokenId(symbol as u32));
        }
    }
    let (left_weights, right_weights) = if weighted {
        (
            weights.iter().copied().take(left.len()).collect(),
            weights.iter().copied().rev().take(right.len()).collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    Case {
        alphabet,
        left: left.iter().copied().map(TokenId).collect(),
        right: right.iter().copied().map(TokenId).collect(),
        trivial,
        trivial_symbols,
        left_weights,
        right_weights,
        table,
    }
}

pub fn closeness_strategy() -> impl Strategy<Value = Option<Closeness>> {
    prop_oneof![
        Just(None),
        Just(Some(Closeness::default())),
        (
            0u16..=1000,
            prop_oneof![Just(4_000_000u64), 0u64..60],
            prop_oneof![Just(None), (0u64..400).prop_map(Some)],
        )
            .prop_map(|(threshold, per_gap, total)| Some(Closeness {
                threshold_permille: threshold,
                max_cells_per_gap: per_gap,
                max_cells_total: total,
            })),
    ]
}

pub fn options_strategy() -> impl Strategy<Value = AlignOptions> {
    (
        prop::sample::select(Algorithm::ALL.to_vec()),
        prop_oneof![0u32..=6, Just(300u32), Just(u32::MAX)],
        closeness_strategy(),
        any::<bool>(),
        prop_oneof![Just(None), (0u64..600).prop_map(Some)],
    )
        .prop_map(
            |(algorithm, skew_tolerance, closeness, never_align_differences, work_budget)| {
                AlignOptions {
                    algorithm,
                    skew_tolerance,
                    closeness,
                    never_align_differences,
                    work_budget,
                }
            },
        )
}

/// A sink that records every call.
#[derive(Debug, Default)]
pub struct Collect {
    pub runs: Vec<Run>,
    pub calls: usize,
}

impl RunSink for Collect {
    fn runs_ready(&mut self, runs: &[Run]) {
        assert!(!runs.is_empty(), "a sink call carries at least one run");
        self.runs.extend_from_slice(runs);
        self.calls += 1;
    }
}

/// `align` with the case's scorer and no sink.
pub fn run(case: &Case, opts: &AlignOptions) -> Result<Alignment, DiffError> {
    align(
        &case.input(),
        opts,
        Some(&case.scorer()),
        None,
        &CancelToken::never(),
    )
}

/// Length of the longest common subsequence (O(nm) DP oracle).
pub fn lcs_len<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev = vec![0usize; b.len() + 1];
    let mut cur = vec![0usize; b.len() + 1];
    for x in a {
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y {
                prev[j] + 1
            } else {
                prev[j + 1].max(cur[j])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Number of Equal pairs of an alignment.
pub fn equal_pairs(alignment: &Alignment) -> usize {
    alignment
        .runs()
        .iter()
        .filter(|run| run.kind == RunKind::Equal)
        .map(|run| (run.left.end - run.left.start) as usize)
        .sum()
}

/// Every paired `(i, j)` of the given kind.
pub fn pairs_of(alignment: &Alignment, kind: RunKind) -> Vec<(u32, u32)> {
    alignment
        .runs()
        .iter()
        .filter(|run| run.kind == kind)
        .flat_map(|run| run.left.clone().zip(run.right.clone()))
        .collect()
}

/// Seeded xorshift64* (§6.1: deterministic generators without the `rand` crate).
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}
