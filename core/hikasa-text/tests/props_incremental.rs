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
    let c = kind == Gen::Source || rng.below(4) == 0;
    if c && rng.below(2) == 0 {
        // Comments unimportant: their keys depend on the tokenizer's state (an opened block comment).
        rules
            .importance
            .elements
            .insert("Comment".to_owned(), false);
    }
    // A first-line heading item (FF-024 `or_line_one`): the key of line 0 depends on its line number.
    let line_one = !c && rng.below(5) == 0;
    if line_one && rng.below(2) == 0 {
        rules.importance.elements.insert("Head".to_owned(), false);
    }
    Setup {
        format: if line_one {
            common::live::line_one_format(rng.below(3) as u32)
        } else {
            common::format(if c { "c-cpp" } else { "ee" })
        },
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
        let (g, w) = (common::canonical_json(got), common::canonical_json(&want));
        if g != w {
            let first = g.lines().zip(w.lines()).position(|(a, b)| a != b);
            eprintln!("seed {seed}: first difference at line {first:?}; path {counters:?}");
        }
        assert_eq!(
            g, w,
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
fn path_counters_reach_every_condition() {
    let mut total = PathCounters::default();
    for seed in 1..=120u64 {
        total.add(&run(seed * 0x2545_F491, seed % 10 == 0));
    }
    // A pair without common lines whose one gap passes the closeness budget per gap: a recorded refusal.
    total.add(&refusal_case());
    let c = total;
    assert!(c.incremental > 0 && c.full > 0 && c.fallback > 0, "{c:?}");
    assert!(c.range_held > 0 && c.range_failed > 0, "{c:?}");
    assert!(c.candidates_held > 0 && c.candidates_failed > 0, "{c:?}");
    assert!(c.window_held > 0 && c.window_failed > 0, "{c:?}");
    assert!(c.budgets_held > 0 && c.budgets_failed > 0, "{c:?}");
}

/// Two documents of 2,100 different lines: the gap of 2,100 × 2,100 cells passes the 4,000,000 cells a gap may
/// take, so closeness pairs it by position (a recorded refusal; condition 4 fails).
fn refusal_case() -> PathCounters {
    let left: String = (0..2_100).map(|k| format!("left line {k}\n")).collect();
    let right: String = (0..2_100).map(|k| format!("right text {k}\n")).collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, r) = (editable(&left, &opts), editable(&right, &opts));
    let setup = Setup {
        format: common::format("ee"),
        rules: common::rules_with(AlgorithmChoice::Standard),
        force_tokenize: false,
        anchors: Vec::new(),
    };
    let mut lc = live(&setup, Pair::new(&l, &r));
    let mut rng = XorShift(5);
    let out = random_edit(&mut rng, Gen::Clustered, &mut l).unwrap();
    lc.note_edit(Side::Left, &l, &out);
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    let path = done.path;
    lc.install(done, Pair::new(&l, &r));
    let want = fresh(&setup, Pair::new(&l, &r), &[]);
    assert_eq!(
        common::canonical_json(lc.current()),
        common::canonical_json(&want)
    );
    path
}

/// Condition 4 near the limit: a pair whose last exact run spent almost all of Standard's work budget, and an
/// insertion into a repetitive gap after which the full run exhausts it. The installed comparison must be the
/// full run's (warning `work_budget_exhausted` included), which the incremental path only gets right by keeping
/// its totals.
#[test]
fn a_budget_exhausted_by_an_edit_is_never_computed_incrementally() {
    let opts = edit_options(Gen::Clustered);
    let setup = Setup {
        format: common::format("ee"),
        rules: common::rules_with(AlgorithmChoice::Standard),
        force_tokenize: false,
        anchors: Vec::new(),
    };
    let mut found = 0;
    for seed in 1..400u64 {
        let mut rng = XorShift(seed * 0x9E37_79B9 + 17);
        // Unique lines around repetitive regions: Standard spends its budget in the regions' stage B.
        let gaps = 3 + rng.below(3);
        let size = 300 + rng.below(500);
        let mut left = String::new();
        let mut right = String::new();
        for g in 0..gaps {
            for u in 0..20 {
                left.push_str(&format!("unique {g} {u}\n"));
                right.push_str(&format!("unique {g} {u}\n"));
            }
            for _ in 0..size {
                left.push_str(["a\n", "b\n"][rng.below(2) as usize]);
                right.push_str(["a\n", "b\n"][rng.below(2) as usize]);
            }
        }
        let (mut l, r) = (editable(&left, &opts), editable(&right, &opts));
        let mut lc = live(&setup, Pair::new(&l, &r));
        if lc
            .current()
            .warnings()
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::WorkBudgetExhausted)
        {
            continue;
        }
        // Insert repetitive lines into the first region.
        let insert: String = (0..40 + rng.below(200))
            .map(|_| ["a\n", "b\n"][rng.below(2) as usize])
            .collect();
        let at = hikasa_text::TextPos::new(30, 0);
        let out = l
            .apply(
                &hikasa_text::EditRequest::Replace {
                    range: at..at,
                    text: insert,
                    keep_eols: false,
                },
                hikasa_text::GroupHint::None,
            )
            .unwrap();
        lc.note_edit(Side::Left, &l, &out);
        let want = fresh(&setup, Pair::new(&l, &r), &[]);
        if !want
            .warnings()
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::WorkBudgetExhausted)
        {
            continue;
        }
        found += 1;
        let done = lc
            .recompare_job(Pair::new(&l, &r))
            .run(None, &Ctl::detached())
            .unwrap();
        lc.install(done, Pair::new(&l, &r));
        assert_eq!(
            common::canonical_json(lc.current()),
            common::canonical_json(&want),
            "seed {seed}"
        );
        if found == 3 {
            return;
        }
    }
    assert!(found > 0, "no case reached the budget");
}

/// Edits at the top of a document whose format starts an item at the first line (FF-024 `or_line_one`): the key
/// of line 0 depends on its line number, so the old and the new line 0 are re-keyed (an insertion at the top, a
/// deletion of the first lines, both sides, with the item unimportant and with following lines).
#[test]
fn edits_at_the_first_line_rekey_the_old_and_new_line_zero() {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    for next_lines in [0u32, 1, 3] {
        for unimportant in [false, true] {
            for (edit, side) in [
                (0, Side::Left),
                (1, Side::Left),
                (2, Side::Right),
                (3, Side::Left),
            ] {
                let mut rules = common::rules_with(AlgorithmChoice::Standard);
                if unimportant {
                    rules.importance.elements.insert("Head".to_owned(), false);
                }
                let setup = Setup {
                    format: common::live::line_one_format(next_lines),
                    rules,
                    force_tokenize: false,
                    anchors: Vec::new(),
                };
                let text: String = (0..200).map(|k| format!("row {k}\n")).collect();
                let opts = edit_options(Gen::Clustered);
                let (mut l, mut r) = (editable(&text, &opts), editable(&text, &opts));
                let mut lc = live(&setup, Pair::new(&l, &r));
                let doc = if side == Side::Left { &mut l } else { &mut r };
                let (range, text) = match edit {
                    0 => (
                        TextPos::new(0, 0)..TextPos::new(0, 0),
                        "new a\nnew b\nnew c\n",
                    ),
                    1 => (TextPos::new(0, 0)..TextPos::new(1, 0), ""),
                    2 => (TextPos::new(0, 0)..TextPos::new(2, 0), "row 0\n"),
                    _ => (TextPos::new(0, 2)..TextPos::new(0, 2), "x"),
                };
                let out = doc
                    .apply(
                        &EditRequest::Replace {
                            range,
                            text: text.into(),
                            keep_eols: false,
                        },
                        GroupHint::None,
                    )
                    .unwrap();
                lc.note_edit(side, doc, &out);
                let done = lc
                    .recompare_job(Pair::new(&l, &r))
                    .run(None, &Ctl::detached())
                    .unwrap();
                lc.install(done, Pair::new(&l, &r));
                let want = fresh(&setup, Pair::new(&l, &r), &[]);
                assert_eq!(
                    common::canonical_json(lc.current()),
                    common::canonical_json(&want),
                    "next_lines {next_lines}, unimportant {unimportant}, edit {edit}"
                );
            }
        }
    }
}

/// *Auto* with a grammar: the recorded engine is Standard (a large stripped middle); an edit that removes the
/// first difference shrinks the middle so much that a fresh compare picks Histogram. The engine is decided again
/// from the new prefix and suffix, and a different decision takes the full compare.
#[test]
fn auto_is_decided_again_from_the_new_prefix_and_suffix() {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    let mut rules = TextRules::default();
    rules.alignment.algorithm = AlgorithmChoice::Auto;
    let setup = Setup {
        format: common::format("c-cpp"),
        rules,
        force_tokenize: false,
        anchors: Vec::new(),
    };
    let n = 12_000;
    let left: String = (0..n).map(|k| format!("int v{k} = {k};\n")).collect();
    let mut right: Vec<String> = (0..n).map(|k| format!("int v{k} = {k};")).collect();
    right[0] = "int changed = 0;".to_owned();
    right[n - 1] = "int changed_end = 1;".to_owned();
    right.swap(n - 100, n - 50);
    right.swap(n - 200, n - 20);
    let right = right.join("\n") + "\n";
    let opts = edit_options(Gen::Clustered);
    let (l, mut r) = (editable(&left, &opts), editable(&right, &opts));
    let mut lc = live(&setup, Pair::new(&l, &r));
    assert_eq!(lc.current().algorithm(), hikasa_text::Algorithm::Standard);
    let len = r.line_len(hikasa_text::LineIdx(0));
    let out = r
        .apply(
            &EditRequest::Replace {
                range: TextPos::new(0, 0)..TextPos::new(0, len),
                text: "int v0 = 0;".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    lc.note_edit(Side::Right, &r, &out);
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    assert_eq!(done.path.full, 1, "{:?}", done.path);
    lc.install(done, Pair::new(&l, &r));
    let want = fresh(&setup, Pair::new(&l, &r), &[]);
    assert_eq!(want.algorithm(), hikasa_text::Algorithm::Histogram);
    assert_eq!(
        common::canonical_json(lc.current()),
        common::canonical_json(&want)
    );
}
