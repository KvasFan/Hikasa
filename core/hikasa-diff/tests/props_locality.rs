//! Node locality (W10a review, §1.10.11): for every recorded node of a derivation, aligning the node's ranges
//! alone gives the full run's pairing of those lines — what the incremental recompare's local re-run of a node
//! relies on. Nested inputs (unique headers around blocks whose items are unique inside a block and repeated
//! across blocks, plus repeated filler), Standard and Patience, skews 0–300, closeness on and off, weights.

mod common;

use common::config;
use hikasa_core::CancelToken;
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, RunKind, TokenId, TrivialSet, align_traced,
};
use proptest::prelude::*;

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D) % n.max(1)
    }
}

fn generate(rng: &mut Rng) -> (Vec<u32>, Vec<u32>) {
    // ids: 0..4 filler (repeated), 100+b headers (unique), 1000+j items unique per block, repeated across blocks
    let blocks = 2 + rng.below(6);
    let items = 20 + rng.below(80);
    let mut left = Vec::new();
    for b in 0..blocks {
        left.push(100 + b as u32);
        let mut order: Vec<u32> = (0..items as u32).collect();
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        for j in order {
            left.push(1000 + j);
            for _ in 0..rng.below(3) {
                left.push(rng.below(5) as u32);
            }
        }
    }
    let mut right = left.clone();
    for _ in 0..1 + rng.below(10) {
        let at = rng.below(right.len() as u64) as usize;
        match rng.below(5) {
            0 => {
                for _ in 0..(1 + rng.below(30) as usize).min(right.len() - at) {
                    right.remove(at);
                }
            }
            1 => {
                for _ in 0..1 + rng.below(30) {
                    right.insert(at, 5000 + rng.below(40) as u32);
                }
            }
            2 => {
                let x = right.remove(at);
                let to = rng.below(right.len() as u64 + 1) as usize;
                right.insert(to, x);
            }
            3 => {
                let x = right[at];
                let to = rng.below(right.len() as u64 + 1) as usize;
                right.insert(to, x);
            }
            _ => right[at] = 9000 + rng.below(5) as u32,
        }
    }
    (left, right)
}

/// Partner of every left line in `l` and right line in `r` within the runs.
fn pairing(
    runs: &[hikasa_diff::Run],
    l: std::ops::Range<u32>,
    r: std::ops::Range<u32>,
) -> Vec<(Option<u32>, Option<u32>, &'static str)> {
    let mut out = Vec::new();
    for run in runs {
        match run.kind {
            RunKind::Equal | RunKind::Changed => {
                for k in 0..run.left.end - run.left.start {
                    let (x, y) = (run.left.start + k, run.right.start + k);
                    if l.contains(&x) || r.contains(&y) {
                        out.push((Some(x), Some(y), run.kind.as_str()));
                    }
                }
            }
            RunKind::Delete => {
                for x in run.left.clone() {
                    if l.contains(&x) {
                        out.push((Some(x), None, "delete"));
                    }
                }
            }
            RunKind::Insert => {
                for y in run.right.clone() {
                    if r.contains(&y) {
                        out.push((None, Some(y), "insert"));
                    }
                }
            }
        }
    }
    out
}

/// One case: every recorded node aligned alone pairs its lines as the full run does.
fn check(seed: u64) -> Result<(), TestCaseError> {
    let never = CancelToken::never();
    let mut rng = Rng(seed | 1);
    let (l, r) = generate(&mut rng);
    let left: Vec<TokenId> = l.iter().map(|&x| TokenId(x)).collect();
    let right: Vec<TokenId> = r.iter().map(|&x| TokenId(x)).collect();
    let lw: Vec<u8> = l.iter().map(|&x| (x % 5) as u8).collect();
    let rw: Vec<u8> = r.iter().map(|&x| (x % 5) as u8).collect();
    let mut trivial = TrivialSet::default();
    trivial.insert(TokenId(0));
    let algorithm = [Algorithm::Standard, Algorithm::Patience][rng.below(2) as usize];
    let skew = [300u32, 3, 20, 0, 40, 100][rng.below(6) as usize];
    let closeness = rng.below(2) == 0;
    let opts = AlignOptions {
        algorithm,
        skew_tolerance: skew,
        closeness: closeness.then_some(Closeness::DEFAULT),
        never_align_differences: false,
        work_budget: None,
    };
    let sim = |a: u32, b: u32| -> u16 {
        if (l[a as usize] ^ r[b as usize]) & 3 == 0 {
            700
        } else {
            100
        }
    };
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &lw,
        right_weights: &rw,
        trivial: &trivial,
        distinct: 10_000,
    };
    let (full, d) = align_traced(&input, &opts, &[], false, Some(&sim), None, &never).unwrap();
    for seg in &d.segments {
        for node in &seg.nodes {
            let (a, b) = (node.left.clone(), node.right.clone());
            let sub = AlignInput {
                left: &left[a.start as usize..a.end as usize],
                right: &right[b.start as usize..b.end as usize],
                left_weights: &lw[a.start as usize..a.end as usize],
                right_weights: &rw[b.start as usize..b.end as usize],
                trivial: &trivial,
                distinct: 10_000,
            };
            let (dx, dy) = (a.start, b.start);
            let local = |x: u32, y: u32| sim(x + dx, y + dy);
            let (alone, _) =
                align_traced(&sub, &opts, &[], false, Some(&local), None, &never).unwrap();
            let moved: Vec<hikasa_diff::Run> = alone
                .runs()
                .iter()
                .map(|r| hikasa_diff::Run {
                    kind: r.kind,
                    left: r.left.start + dx..r.left.end + dx,
                    right: r.right.start + dy..r.right.end + dy,
                })
                .collect();
            let want = pairing(full.runs(), a.clone(), b.clone());
            let got = pairing(&moved, a.clone(), b.clone());
            prop_assert_eq!(
                want,
                got,
                "seed {} {:?} skew {} closeness {}: node {:?} x {:?}",
                seed,
                algorithm,
                skew,
                closeness,
                a,
                b
            );
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_locality.txt"))]

    /// Node locality (§1.10.11): a recorded node aligned alone pairs its lines as the full run.
    #[test]
    fn p_node_locality_every_recorded_node_aligns_alone_as_in_the_full_run(seed in any::<u64>()) {
        check(seed)?;
    }
}
