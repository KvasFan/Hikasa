//! A10.9 (D-269): incremental = full, P-EDIT-2. Random document pairs from four generators (the G1-like clustered
//! pair at 2,000–20,000 lines, repetitive lines, source with block comments and multi-line strings, CPL-wrapped
//! mixed-terminator text), every algorithm, closeness on and off, never-align, anchors, identity and tokenized
//! paths; random edit scripts through `note_edit` with recompares after random batches: after each `install` the
//! comparison equals a fresh `compare_with_limits` of the snapshots (the canonical document: runs, rows,
//! sections, statistics, warnings, verdicts, inline differences), and so do the anchors. A deterministic sweep
//! shows every certificate condition both holding and failing and the T6 fallback taken.

mod common;

use common::XorShift;
use common::live::{
    GENS, Gen, Setup, edit_options, editable, fresh, live, pair, random_edit, side,
};
use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::{AlgorithmChoice, Anchor, AnchorSide, Installed, PathCounters, TextRules};
use proptest::prelude::*;

/// The settings of one case.
fn setup(rng: &mut XorShift, kind: Gen, lines: (u32, u32)) -> Setup {
    let mut rules = TextRules::default();
    rules.alignment.algorithm = [
        AlgorithmChoice::Auto,
        AlgorithmChoice::Auto,
        AlgorithmChoice::Standard,
        AlgorithmChoice::Standard,
        AlgorithmChoice::Histogram,
        AlgorithmChoice::Patience,
        AlgorithmChoice::Myers,
        AlgorithmChoice::Unaligned,
    ][rng.below(8) as usize];
    rules.alignment.closeness = rng.below(3) != 0;
    rules.alignment.never_align_differences = rng.below(5) == 0;
    if rng.below(3) == 0 {
        rules.alignment.skew_tolerance = [0, 3, 40][rng.below(3) as usize];
    }
    let mut anchors = Vec::new();
    if rng.below(3) == 0 && lines.0 > 4 && lines.1 > 4 {
        for _ in 0..1 + rng.below(3) {
            if rng.below(2) == 0 {
                let l = rng.below(u64::from(lines.0 - 2)) as u32;
                let r = rng.below(u64::from(lines.1 - 2)) as u32;
                anchors.push(Anchor::Pair {
                    left: l..l + 1 + rng.below(2) as u32,
                    right: r..r + 1 + rng.below(2) as u32,
                });
            } else {
                let (side, n) = if rng.below(2) == 0 {
                    (AnchorSide::Left, lines.0)
                } else {
                    (AnchorSide::Right, lines.1)
                };
                let s = rng.below(u64::from(n)) as u32;
                anchors.push(Anchor::Isolate {
                    side,
                    lines: s..s + 1 + rng.below(3) as u32,
                });
            }
        }
    }
    Setup {
        format: common::format(if kind == Gen::Source || rng.below(4) == 0 {
            "c-cpp"
        } else {
            "ee"
        }),
        rules,
        force_tokenize: rng.below(4) == 0,
        anchors,
    }
}

/// One case: returns the path counters it went through.
fn run(seed: u64, big: bool) -> PathCounters {
    let mut rng = XorShift(seed | 1);
    let kind = GENS[rng.below(4) as usize];
    let n = if big && kind == Gen::Clustered {
        2_000 + rng.below(18_000)
    } else {
        20 + rng.below(400)
    };
    let (l, r) = pair(&mut rng, kind, n);
    let opts = edit_options(kind);
    let (mut left, mut right) = (editable(&l, &opts), editable(&r, &opts));
    let setup = setup(&mut rng, kind, (left.line_count(), right.line_count()));
    let mut lc = live(&setup, Pair::new(&left, &right));
    let mut counters = PathCounters::default();
    let steps = 6 + rng.below(20);
    for _ in 0..steps {
        let burst = if rng.below(4) == 0 { 30 } else { 3 };
        let edits = 1 + rng.below(burst);
        for _ in 0..edits {
            let s = side(&mut rng);
            let doc = if s == Side::Left {
                &mut left
            } else {
                &mut right
            };
            if let Some(out) = random_edit(&mut rng, kind, doc) {
                lc.note_edit(s, doc, &out);
            }
        }
        let job = lc.recompare_job(Pair::new(&left, &right));
        let done = job.run(None, &Ctl::detached()).unwrap();
        counters.add(&done.path);
        assert_eq!(
            lc.install(done, Pair::new(&left, &right)),
            Installed::Current
        );
        let want = fresh(&setup, Pair::new(&left, &right), lc.anchors());
        let got = lc.current();
        assert_eq!(
            common::canonical_json(got),
            common::canonical_json(&want),
            "seed {seed}: the installed comparison differs from a fresh compare"
        );
        assert_eq!(got.anchors(), want.anchors(), "seed {seed}");
        assert_eq!(got.used_identity_path(), want.used_identity_path());
        let all = |c: &hikasa_text::TextComparison| -> Vec<hikasa_text::Row> {
            c.rows(0..c.row_count()).collect()
        };
        assert_eq!(all(got), all(&want), "seed {seed}: rows");
        got.check_invariants().unwrap();
    }
    counters
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_incremental.txt"))]

    /// P-EDIT-2 (A10.9).
    #[test]
    fn p_edit_2_incremental_equals_full(seed in any::<u64>(), big in prop_oneof![4 => Just(false), 1 => Just(true)]) {
        run(seed, big);
    }
}

/// The certificate conditions each hold and fail at least once, and the T6 fallback runs, over a fixed sweep.
#[test]
#[ignore = "enabled with the incremental path (next commit)"]
fn path_counters_reach_every_condition() {
    let mut total = PathCounters::default();
    for seed in 1..=120u64 {
        total.add(&run(seed * 0x2545_F491, seed % 10 == 0));
    }
    let c = total;
    assert!(c.incremental > 0 && c.full > 0 && c.fallback > 0, "{c:?}");
    assert!(c.range_held > 0 && c.range_failed > 0, "{c:?}");
    assert!(c.candidates_held > 0 && c.candidates_failed > 0, "{c:?}");
    assert!(c.window_held > 0 && c.window_failed > 0, "{c:?}");
    assert!(c.budgets_held > 0 && c.budgets_failed > 0, "{c:?}");
}
