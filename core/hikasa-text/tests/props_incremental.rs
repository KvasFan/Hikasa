//! A10.9 (D-269): incremental = full, P-EDIT-2. Random document pairs from four generators (the G1-like clustered
//! pair at 2,000–20,000 lines, repetitive lines, source with block comments and multi-line strings, CPL-wrapped
//! mixed-terminator text), every algorithm, closeness on and off, never-align, anchors, a format per pane (W10b),
//! ignored lines toggled between edits (W10b, checked against a per-line oracle), identity and tokenized
//! paths; random edit scripts through `note_edit` with recompares after random batches: after each `install` the
//! comparison equals a fresh `compare_with_limits` of the snapshots (the canonical document: runs, rows,
//! sections, statistics, warnings, verdicts, the inline differences of every row), and the anchors equal an
//! oracle that tracks line identities. Further generators: nested documents (unique headers around blocks
//! whose items are unique inside a block, recorded child nodes with chains of their own) with every skew and
//! duplicate, move and paste edits; `Batch` requests; jobs run while edits come and installed out of order. A deterministic sweep
//! shows every certificate condition both holding and failing and the T6 fallback taken.

mod common;

use common::XorShift;
use common::live::{
    GENS, Gen, Setup, edit_options, editable, fresh, live, pair, random_edit, side,
};
use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::{AlgorithmChoice, Anchor, AnchorSide, Installed, PathCounters, TextRules};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

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
    let left_id = if line_one {
        "hdr"
    } else if c {
        "c-cpp"
    } else {
        "ee"
    };
    // A format per pane (D-281): the right side has its own format in a quarter of the cases (sharing the
    // `Comment` element name, or plain text, or a first-line heading).
    let right_format = (rng.below(4) == 0).then(|| {
        let choices = ["ee", "c-cpp", "java", "python", "hdr"];
        let mut id = choices[rng.below(choices.len() as u64) as usize];
        if id == left_id {
            id = if left_id == "ee" { "c-cpp" } else { "ee" };
        }
        if id == "hdr" {
            common::live::line_one_format(rng.below(3) as u32)
        } else {
            common::format(id)
        }
    });
    // Ignored lines (§1.10.17) in a third of the cases.
    let ignored = if rng.below(3) == 0 {
        common::live::random_ignored(rng, [lines.0, lines.1])
    } else {
        Default::default()
    };
    Setup {
        format: if line_one {
            common::live::line_one_format(rng.below(3) as u32)
        } else {
            common::format(left_id)
        },
        rules,
        force_tokenize: rng.below(4) == 0,
        anchors,
        right_format,
        ignored,
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
    // The anchors' oracle: line identities, independent of the live comparison's own bookkeeping.
    let mut model =
        common::live::AnchorModel::new(lc.anchors(), [left.line_count(), right.line_count()]);
    // The ignored lines' oracle: one mark per line.
    let mut marks =
        common::live::IgnoredModel::new(lc.ignored(), [left.line_count(), right.line_count()]);
    let toggles = !setup.ignored.is_empty() || rng.below(4) == 0;
    let mut counters = PathCounters::default();
    let steps = 6 + rng.below(20);
    for _ in 0..steps {
        // The *Ignored* command now and then (marking again removes the marks).
        if toggles && rng.below(3) == 0 {
            let s = side(&mut rng);
            let n = if s == Side::Left {
                left.line_count()
            } else {
                right.line_count()
            };
            if n > 0 {
                let a = rng.below(u64::from(n)) as u32;
                let lines = a..(a + 1 + rng.below(8) as u32).min(n);
                marks.toggle(s, lines);
                lc.set_ignored(marks.ignored());
            }
        }
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
                for &d in &out.deltas {
                    model.apply(s, d);
                    marks.apply(s, d);
                }
            }
        }
        assert_eq!(lc.anchors(), model.anchors(), "seed {seed}: anchors");
        assert_eq!(lc.ignored(), &marks.ignored(), "seed {seed}: ignored lines");
        lc.check_invariants().unwrap();
        let job = lc.recompare_job(Pair::new(&left, &right));
        let done = job.run(None, &Ctl::detached()).unwrap();
        counters.add(&done.path);
        assert_eq!(
            lc.install(done, Pair::new(&left, &right)),
            Installed::Current
        );
        let want = common::live::fresh_ignored(
            &setup,
            Pair::new(&left, &right),
            &model.anchors(),
            &marks.ignored(),
        );
        let got = lc.current();
        let (g, w) = (
            common::canonical_json_all(got),
            common::canonical_json_all(&want),
        );
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
    // Nested documents with small skews: pieces of diagonal runs cross the window's edge.
    for seed in 1..=40u64 {
        total.add(&nested_run(seed * 0x9E37_79B9).unwrap());
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
        right_format: None,
        ignored: Default::default(),
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
        right_format: None,
        ignored: Default::default(),
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
                    right_format: None,
                    ignored: Default::default(),
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
        right_format: None,
        ignored: Default::default(),
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

/// The installed comparison equals a fresh compare (every row, the inline differences of every row).
fn assert_exact(
    tag: &str,
    setup: &Setup,
    lc: &hikasa_text::LiveComparison,
    l: &hikasa_text::edit::EditableDocument,
    r: &hikasa_text::edit::EditableDocument,
) -> Result<(), TestCaseError> {
    let want = fresh(setup, Pair::new(l, r), lc.anchors());
    let (g, w) = (
        common::canonical_json_all(lc.current()),
        common::canonical_json_all(&want),
    );
    if g != w {
        let first = g
            .lines()
            .zip(w.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        let (gl, wl): (Vec<&str>, Vec<&str>) = (g.lines().collect(), w.lines().collect());
        let around = |v: &[&str]| v[first.saturating_sub(3)..(first + 5).min(v.len())].join("\n");
        return Err(TestCaseError::fail(format!(
            "{tag}: differs at line {first}:\n got {}\nwant {}",
            around(&gl),
            around(&wl)
        )));
    }
    Ok(())
}

/// One recompare, installed (it must be current).
fn recompare(
    lc: &mut hikasa_text::LiveComparison,
    l: &hikasa_text::edit::EditableDocument,
    r: &hikasa_text::edit::EditableDocument,
) -> PathCounters {
    let done = lc
        .recompare_job(Pair::new(l, r))
        .run(None, &Ctl::detached())
        .unwrap();
    let path = done.path;
    assert_eq!(lc.install(done, Pair::new(l, r)), Installed::Current);
    path
}

/// Nested documents: unique headers (the root's chain) around blocks of 30–120 items that are unique inside a
/// block and repeated across blocks (recorded child nodes with chains of their own), plus repeated filler.
fn nested_text(rng: &mut XorShift, blocks: u64, items: u64) -> Vec<String> {
    let mut out = Vec::new();
    for b in 0..blocks {
        out.push(format!("header {b} {}", rng.below(1_000_000)));
        let mut order: Vec<u64> = (0..items).collect();
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        for j in order {
            out.push(format!("item {j}"));
            for _ in 0..rng.below(3) {
                out.push(["a", "b", "}", "", "  x = 1;"][rng.below(5) as usize].to_owned());
            }
        }
    }
    out
}

/// The other side of a nested pair: removed and inserted runs, moved and duplicated lines, changed lines.
fn perturb(rng: &mut XorShift, lines: &[String]) -> Vec<String> {
    let mut v = lines.to_vec();
    for _ in 0..1 + rng.below(12) {
        if v.is_empty() {
            break;
        }
        let at = rng.below(v.len() as u64) as usize;
        match rng.below(6) {
            0 => {
                for _ in 0..(1 + rng.below(40) as usize).min(v.len() - at) {
                    v.remove(at);
                }
            }
            1 => {
                for k in 0..1 + rng.below(40) {
                    v.insert(at, format!("ins {k} {}", rng.below(50)));
                }
            }
            2 => {
                let l = v.remove(at);
                let to = rng.below(v.len() as u64 + 1) as usize;
                v.insert(to, l);
            }
            3 => {
                let l = v[at].clone();
                let to = rng.below(v.len() as u64 + 1) as usize;
                v.insert(to, l);
            }
            _ => v[at].push_str(" changed"),
        }
    }
    v
}

/// An edit of a nested document: a copied line (a duplicate key), a removed block, a pasted run of up to 300
/// lines (shifts beyond any band), a line replaced by an item key, undo, redo, or a generic random edit.
fn nested_edit(
    rng: &mut XorShift,
    doc: &mut hikasa_text::edit::EditableDocument,
) -> Option<hikasa_text::EditOutcome> {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    let n = doc.line_count();
    if n < 2 {
        return random_edit(rng, Gen::Clustered, doc);
    }
    let replace = |doc: &mut hikasa_text::edit::EditableDocument, a: TextPos, b: TextPos, text| {
        doc.apply(
            &EditRequest::Replace {
                range: a..b,
                text,
                keep_eols: false,
            },
            GroupHint::None,
        )
        .ok()
    };
    match rng.below(10) {
        0 => {
            let from = rng.below(u64::from(n)) as u32;
            let text = doc.line(hikasa_text::LineIdx(from)).into_owned() + "\n";
            let at = TextPos::new(rng.below(u64::from(n)) as u32, 0);
            replace(doc, at, at, text)
        }
        1 => {
            let at = rng.below(u64::from(n)) as u32;
            let e = (at + 1 + rng.below(60) as u32).min(n - 1);
            replace(doc, TextPos::new(at, 0), TextPos::new(e, 0), String::new())
        }
        2 => {
            let at = TextPos::new(rng.below(u64::from(n)) as u32, 0);
            let k = 1 + rng.below(300);
            let text: String = (0..k)
                .map(|i| format!("pasted {i} {}\n", rng.below(7)))
                .collect();
            replace(doc, at, at, text)
        }
        3 => {
            let at = rng.below(u64::from(n)) as u32;
            let len = doc.line_len(hikasa_text::LineIdx(at));
            let text = format!("item {}", rng.below(40));
            replace(doc, TextPos::new(at, 0), TextPos::new(at, len), text)
        }
        4 => doc.undo(),
        5 => doc.redo(),
        _ => random_edit(rng, Gen::Clustered, doc),
    }
}

/// One nested case (12 recompares).
fn nested_run(seed: u64) -> Result<PathCounters, TestCaseError> {
    let mut rng = XorShift(seed | 1);
    let blocks = 3 + rng.below(12);
    let items = 30 + rng.below(90);
    let left = nested_text(&mut rng, blocks, items);
    let right = perturb(&mut rng, &left);
    let (lt, rt) = (left.join("\n") + "\n", right.join("\n") + "\n");
    let mut rules = TextRules::default();
    rules.alignment.algorithm = [
        AlgorithmChoice::Standard,
        AlgorithmChoice::Standard,
        AlgorithmChoice::Patience,
        AlgorithmChoice::Auto,
    ][rng.below(4) as usize];
    rules.alignment.closeness = rng.below(3) != 0;
    rules.alignment.never_align_differences = rng.below(6) == 0;
    rules.alignment.skew_tolerance = [300, 300, 0, 3, 20, 40, 100][rng.below(7) as usize];
    let fmt = if rng.below(2) == 0 { "ee" } else { "c-cpp" };
    let setup = Setup {
        format: common::format(fmt),
        rules,
        force_tokenize: rng.below(4) == 0,
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
    };
    let opts = edit_options(Gen::Clustered);
    let (mut l, mut r) = (editable(&lt, &opts), editable(&rt, &opts));
    let mut lc = live(&setup, Pair::new(&l, &r));
    let mut total = PathCounters::default();
    for step in 0..12 {
        let burst = if rng.below(4) == 0 { 6 } else { 2 };
        for _ in 0..1 + rng.below(burst) {
            let s = side(&mut rng);
            let doc = if s == Side::Left { &mut l } else { &mut r };
            if let Some(out) = nested_edit(&mut rng, doc) {
                lc.note_edit(s, doc, &out);
            }
        }
        total.add(&recompare(&mut lc, &l, &r));
        let a = &setup.rules.alignment;
        assert_exact(
            &format!(
                "nested seed {seed} step {step} ({fmt}, {:?}, skew {})",
                a.algorithm, a.skew_tolerance
            ),
            &setup,
            &lc,
            &l,
            &r,
        )?;
    }
    Ok(total)
}

/// One case of `Batch` requests (several deltas in one outcome) with their undo and redo.
fn batch_run(seed: u64) -> Result<(), TestCaseError> {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    let mut rng = XorShift(seed | 1);
    let kind = GENS[rng.below(4) as usize];
    let n = 50 + rng.below(600);
    let (lt, rt) = pair(&mut rng, kind, n);
    let opts = edit_options(kind);
    let (mut l, mut r) = (editable(&lt, &opts), editable(&rt, &opts));
    let mut rules = TextRules::default();
    rules.alignment.algorithm =
        [AlgorithmChoice::Standard, AlgorithmChoice::Patience][rng.below(2) as usize];
    let fmt = if kind == Gen::Source { "c-cpp" } else { "ee" };
    if fmt == "c-cpp" && rng.below(2) == 0 {
        rules
            .importance
            .elements
            .insert("Comment".to_owned(), false);
    }
    let setup = Setup {
        format: common::format(fmt),
        rules,
        force_tokenize: false,
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
    };
    let mut lc = live(&setup, Pair::new(&l, &r));
    for step in 0..8 {
        for _ in 0..1 + rng.below(3) {
            let s = side(&mut rng);
            let doc = if s == Side::Left { &mut l } else { &mut r };
            let out = if rng.below(3) == 0 {
                if rng.below(2) == 0 {
                    doc.undo()
                } else {
                    doc.redo()
                }
            } else {
                let lines = doc.position_lines();
                let mut starts: Vec<u32> = (0..1 + rng.below(6))
                    .map(|_| rng.below(u64::from(lines)) as u32)
                    .collect();
                starts.sort_unstable();
                starts.dedup();
                let mut edits = Vec::new();
                let mut prev_end: Option<TextPos> = None;
                for s0 in starts {
                    let a = TextPos::new(s0, 0);
                    if prev_end.is_some_and(|p| p >= a) {
                        continue;
                    }
                    let e_line = (s0 + rng.below(3) as u32).min(lines - 1);
                    let b = doc.clamp(TextPos::new(e_line, rng.below(8) as u32));
                    let b = if b < a { a } else { b };
                    let t = match rng.below(4) {
                        0 => String::new(),
                        1 => "\n".into(),
                        2 => {
                            let k = 1 + rng.below(3);
                            common::live::text(&mut rng, kind, k)
                        }
                        _ => "zz".into(),
                    };
                    edits.push((a..b, t));
                    prev_end = Some(b);
                }
                doc.apply(
                    &EditRequest::Batch {
                        edits,
                        keep_eols: rng.below(2) == 0,
                    },
                    GroupHint::None,
                )
                .ok()
            };
            if let Some(out) = out {
                lc.note_edit(s, doc, &out);
            }
        }
        recompare(&mut lc, &l, &r);
        assert_exact(
            &format!("batch seed {seed} step {step}"),
            &setup,
            &lc,
            &l,
            &r,
        )?;
    }
    Ok(())
}

/// One case of jobs made while edits keep coming, run, then installed in a random order (stale results change
/// nothing, newer ones replay the later edits); the last job is current and exact.
fn out_of_order_run(seed: u64) -> Result<(), TestCaseError> {
    let mut rng = XorShift(seed | 1);
    let kind = [Gen::Clustered, Gen::Source][rng.below(2) as usize];
    let setup = Setup {
        format: common::format(if kind == Gen::Source { "c-cpp" } else { "ee" }),
        rules: common::rules_with(
            [AlgorithmChoice::Standard, AlgorithmChoice::Patience][rng.below(2) as usize],
        ),
        force_tokenize: false,
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
    };
    let nn = 300 + rng.below(500);
    let (lt, rt) = pair(&mut rng, kind, nn);
    let opts = edit_options(kind);
    let (mut l, mut r) = (editable(&lt, &opts), editable(&rt, &opts));
    let mut lc = live(&setup, Pair::new(&l, &r));
    for round in 0..6 {
        let mut jobs = Vec::new();
        for _ in 0..3 {
            for _ in 0..1 + rng.below(3) {
                let s = side(&mut rng);
                let doc = if s == Side::Left { &mut l } else { &mut r };
                if let Some(out) = random_edit(&mut rng, kind, doc) {
                    lc.note_edit(s, doc, &out);
                }
            }
            jobs.push(lc.recompare_job(Pair::new(&l, &r)));
        }
        let mut done: Vec<_> = jobs
            .into_iter()
            .map(|j| j.run(None, &Ctl::detached()).unwrap())
            .collect();
        while !done.is_empty() {
            let k = rng.below(done.len() as u64) as usize;
            let d = done.remove(k);
            lc.install(d, Pair::new(&l, &r));
            prop_assert_eq!(
                lc.current().document(Side::Left).line_count(),
                l.line_count()
            );
            prop_assert_eq!(
                lc.current().document(Side::Right).line_count(),
                r.line_count()
            );
            lc.check_invariants()
                .map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        }
        recompare(&mut lc, &l, &r);
        assert_exact(
            &format!("out of order seed {seed} round {round}"),
            &setup,
            &lc,
            &l,
            &r,
        )?;
    }
    Ok(())
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props_incremental_more.txt"))]

    /// P-EDIT-2 on nested documents (W10a review): recorded children with chains of their own, every skew,
    /// duplicate, move and paste edits.
    #[test]
    fn p_edit_2_nested_documents(seed in any::<u64>()) {
        nested_run(seed)?;
    }

    /// P-EDIT-2 with `Batch` requests and their undo and redo.
    #[test]
    fn p_edit_2_batch_requests(seed in any::<u64>()) {
        batch_run(seed)?;
    }

    /// P-EDIT-2 with jobs installed out of order while edits come (A10.10's replay, stale results dropped).
    #[test]
    fn p_edit_2_out_of_order_installs(seed in any::<u64>()) {
        out_of_order_run(seed)?;
    }
}

/// A nested case found by 4,000 cases: new blocks split an old gap into several new gaps, and the old gap's
/// child was processed on each part (records with pairs outside their ranges); a split old gap's parts are re-run.
#[test]
fn a_split_old_gap_is_never_processed_in_parts() {
    nested_run(15954470204392332736).unwrap();
}

/// A format per pane and ignored lines keep the incremental path (§1.10.11 with §1.10.16–§1.10.17): each side is
/// re-keyed with its own format, the marks move with the edit, and the result equals a fresh compare.
#[test]
fn per_side_formats_and_ignored_lines_stay_incremental() {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    let mut rules = common::rules_with(AlgorithmChoice::Standard);
    rules
        .importance
        .elements
        .insert("Comment".to_owned(), false);
    for (left_id, right_id) in [("c-cpp", "ee"), ("ee", "java"), ("java", "c-cpp")] {
        let mut rng = XorShift(77);
        let (l, r) = pair(&mut rng, Gen::Source, 600);
        let opts = edit_options(Gen::Source);
        let (mut left, mut right) = (editable(&l, &opts), editable(&r, &opts));
        let setup = Setup {
            format: common::format(left_id),
            rules: rules.clone(),
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: Some(common::format(right_id)),
            ignored: hikasa_text::IgnoredLines {
                left: hikasa_text::RangeSet::from_ranges(vec![100..110, 300..302]),
                right: std::iter::once(200..230).collect(),
            },
        };
        let mut lc = live(&setup, Pair::new(&left, &right));
        let mut incremental = 0;
        for (k, side) in [(250u32, Side::Left), (105, Side::Left), (210, Side::Right)] {
            let doc = if side == Side::Left {
                &mut left
            } else {
                &mut right
            };
            let at = TextPos::new(k, 0);
            let out = doc
                .apply(
                    &EditRequest::Replace {
                        range: at..at,
                        text: "int z9 = 99;\n".into(),
                        keep_eols: false,
                    },
                    GroupHint::None,
                )
                .unwrap();
            lc.note_edit(side, doc, &out);
            let done = lc
                .recompare_job(Pair::new(&left, &right))
                .run(None, &Ctl::detached())
                .unwrap();
            incremental += done.path.incremental;
            lc.install(done, Pair::new(&left, &right));
            let want =
                common::live::fresh_ignored(&setup, Pair::new(&left, &right), &[], lc.ignored());
            assert_eq!(
                common::canonical_json_all(lc.current()),
                common::canonical_json_all(&want),
                "{left_id} / {right_id}, edit at {k}"
            );
        }
        assert!(incremental > 0, "{left_id} / {right_id}: never incremental");
        // The insertion inside the left range 100..110 joined it; both insertions moved 300..302.
        assert_eq!(
            lc.ignored().left.ranges(),
            &[100..111, 302..304],
            "{left_id} / {right_id}"
        );
    }
}
