//! A10.10 (D-268): the provisional splice. After every `note_edit` each live line of each side is in exactly one
//! row, rows are in line order, the rows, runs, sections and statistics are consistent (a recount), pending flags
//! cover exactly the touched rows; `install` with no newer edit is `Current`, with newer edits `Replayed` and still
//! consistent; a stale generation is never shown.

mod common;

use common::XorShift;
use common::live::{
    GENS, Gen, Setup, edit_options, editable, fresh, live, pair, random_edit, side,
};
use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::{Installed, LineIdx, RowIdx, TextRules};

fn setup(kind: Gen) -> Setup {
    Setup {
        format: common::format(if kind == Gen::Source { "c-cpp" } else { "ee" }),
        rules: TextRules::default(),
        force_tokenize: false,
        anchors: Vec::new(),
    }
}

/// Every live line is in exactly one row, in order; the structure is a recount; lines touched since the last
/// exact result are in pending rows.
fn check(
    live: &hikasa_text::LiveComparison,
    docs: Pair<&hikasa_text::edit::EditableDocument>,
    touched: &[Vec<bool>; 2],
) {
    let cmp = live.current();
    cmp.check_structure().unwrap();
    // INV-TC-8 too, except on pending rows (they keep their state until the recompare).
    live.check_invariants().unwrap();
    assert_eq!(
        cmp.document(Side::Left).line_count(),
        docs.left.line_count()
    );
    assert_eq!(
        cmp.document(Side::Right).line_count(),
        docs.right.line_count()
    );
    for r in 0..cmp.row_count() {
        let row = cmp.row(RowIdx(r)).unwrap();
        let t = row.left.is_some_and(|l| touched[0][l.0 as usize])
            || row.right.is_some_and(|l| touched[1][l.0 as usize]);
        if t {
            assert!(
                live.is_pending(RowIdx(r)),
                "row {r} {row:?} holds a touched line but is not pending"
            );
        }
        if live.is_pending(RowIdx(r)) {
            let orphan = row.left.is_none() || row.right.is_none();
            assert!(t || orphan, "row {r} {row:?} is pending but untouched");
            // A pending pair's inline differences come from the live lines, whatever its provisional state.
            if let (Some(l), Some(x)) = (row.left, row.right) {
                let equal = docs.left.line(l) == docs.right.line(x);
                assert_eq!(
                    live.inline(RowIdx(r), docs).is_none(),
                    equal,
                    "row {r} {row:?}: inline of the live lines"
                );
            }
        }
    }
}

/// The touched-line flags after `delta` on one side.
fn touch(flags: &mut Vec<bool>, d: hikasa_text::LineDelta) {
    let at = d.at as usize;
    let tail = flags.split_off((at + d.removed as usize).min(flags.len()));
    flags.truncate(at.min(flags.len()));
    flags.extend(std::iter::repeat_n(true, d.inserted as usize));
    flags.extend(tail);
}

#[test]
fn provisional_splices_keep_the_comparison_consistent() {
    for seed in 1..=60u64 {
        let kind = GENS[(seed % 4) as usize];
        let mut rng = XorShift(seed * 0x9E37_79B9);
        let n = 20 + rng.below(200);
        let (l, r) = pair(&mut rng, kind, n);
        let opts = edit_options(kind);
        let (mut left, mut right) = (editable(&l, &opts), editable(&r, &opts));
        let setup = setup(kind);
        let mut lc = live(&setup, Pair::new(&left, &right));
        let mut touched = [
            vec![false; left.line_count() as usize],
            vec![false; right.line_count() as usize],
        ];
        for step in 0..30 {
            let s = side(&mut rng);
            let doc = if s == Side::Left {
                &mut left
            } else {
                &mut right
            };
            let Some(out) = random_edit(&mut rng, kind, doc) else {
                continue;
            };
            lc.note_edit(s, doc, &out);
            for &d in &out.deltas {
                touch(&mut touched[usize::from(s == Side::Right)], d);
            }
            assert!(lc.current().provisional() || out.deltas.iter().all(|d| d.is_empty()));
            check(&lc, Pair::new(&left, &right), &touched);
            // Now and then a recompare, installed at once (Current) or after more edits (Replayed).
            if step % 7 == 6 {
                let job = lc.recompare_job(Pair::new(&left, &right));
                let done = job.run(None, &Ctl::detached()).unwrap();
                let later = rng.below(2) == 0;
                let mut more = false;
                if later {
                    let s = side(&mut rng);
                    let doc = if s == Side::Left {
                        &mut left
                    } else {
                        &mut right
                    };
                    if let Some(out) = random_edit(&mut rng, kind, doc) {
                        lc.note_edit(s, doc, &out);
                        more = out.deltas.iter().any(|d| !d.is_empty()) || true;
                        touched = [
                            vec![false; left.line_count() as usize],
                            vec![false; right.line_count() as usize],
                        ];
                        for &d in &out.deltas {
                            let k = usize::from(s == Side::Right);
                            touched[k] = vec![
                                false;
                                if k == 0 {
                                    left.line_count()
                                } else {
                                    right.line_count()
                                } as usize
                            ];
                            let _ = d;
                        }
                        // The replayed rows are pending: mark the lines of this edit.
                        let k = usize::from(s == Side::Right);
                        let count = if k == 0 {
                            left.line_count()
                        } else {
                            right.line_count()
                        };
                        touched[k] = vec![false; count as usize];
                        let mut flags = vec![
                            false;
                            (count as i64
                                - out
                                    .deltas
                                    .iter()
                                    .map(|d| i64::from(d.inserted) - i64::from(d.removed))
                                    .sum::<i64>())
                                as usize
                        ];
                        for &d in &out.deltas {
                            touch(&mut flags, d);
                        }
                        touched[k] = flags;
                    }
                }
                let installed = lc.install(done, Pair::new(&left, &right));
                if more {
                    assert!(
                        matches!(installed, Installed::Replayed { .. }),
                        "seed {seed}"
                    );
                    check(&lc, Pair::new(&left, &right), &touched);
                } else {
                    assert_eq!(installed, Installed::Current);
                    assert_eq!(lc.pending_rows(), 0);
                    assert!(!lc.current().provisional());
                    touched = [
                        vec![false; left.line_count() as usize],
                        vec![false; right.line_count() as usize],
                    ];
                    // Exact: equal to a fresh compare.
                    let want = fresh(&setup, Pair::new(&left, &right), lc.anchors());
                    assert_eq!(
                        common::canonical_json(lc.current()),
                        common::canonical_json(&want),
                        "seed {seed}"
                    );
                }
            }
        }
        let _ = LineIdx(0);
    }
}

#[test]
fn a_stale_result_is_never_shown() {
    let mut rng = XorShift(77);
    let (l, r) = pair(&mut rng, Gen::Clustered, 100);
    let opts = edit_options(Gen::Clustered);
    let (mut left, right) = (editable(&l, &opts), editable(&r, &opts));
    let setup = setup(Gen::Clustered);
    let mut lc = live(&setup, Pair::new(&left, &right));
    let out = random_edit(&mut rng, Gen::Clustered, &mut left).unwrap();
    lc.note_edit(Side::Left, &left, &out);
    let old_job = lc.recompare_job(Pair::new(&left, &right));
    let out = left
        .apply(
            &hikasa_text::EditRequest::Replace {
                range: hikasa_text::TextPos::ZERO..hikasa_text::TextPos::ZERO,
                text: "first\n".into(),
                keep_eols: false,
            },
            hikasa_text::GroupHint::None,
        )
        .unwrap();
    lc.note_edit(Side::Left, &left, &out);
    let new_job = lc.recompare_job(Pair::new(&left, &right));
    let new = new_job.run(None, &Ctl::detached()).unwrap();
    let old = old_job.run(None, &Ctl::detached()).unwrap();
    assert_eq!(
        lc.install(new, Pair::new(&left, &right)),
        Installed::Current
    );
    let shown = common::canonical_json(lc.current());
    // The older result arrives late: nothing changes.
    assert!(matches!(
        lc.install(old, Pair::new(&left, &right)),
        Installed::Replayed { .. }
    ));
    assert_eq!(common::canonical_json(lc.current()), shown);
    assert_eq!(lc.current().document(Side::Left).line(LineIdx(0)), "first");
}

/// A recompare keeps the base's `bytes_identical` only while no edit came since (n20): identical files compared
/// with `bytes_identical`, an edit and its recompare show a difference (not `BinarySame`), and the edit undone
/// is rules-same but not byte-identical any more (nothing re-reads the bytes).
#[test]
fn bytes_identical_holds_only_until_an_edit() {
    use hikasa_core::ContentResult;
    use hikasa_text::{CompareInput, RegexLimits, compare_live};
    let text: String = (0..300).map(|k| format!("line {k}\n")).collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, r) = (editable(&text, &opts), editable(&text, &opts));
    let setup = setup(Gen::Clustered);
    let mut lc = compare_live(
        CompareInput {
            left: l.snapshot(),
            right: r.snapshot(),
            format: setup.format.clone(),
            rules: setup.rules.clone(),
            bytes_identical: true,
            force_tokenize: false,
            anchors: Vec::new(),
        },
        &RegexLimits::default(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    assert_eq!(lc.current().verdict(false), ContentResult::BinarySame);
    // A recompare without an edit keeps it.
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    lc.install(done, Pair::new(&l, &r));
    assert!(lc.current().bytes_identical());
    let mut rng = XorShift(7);
    let out = loop {
        if let Some(out) = random_edit(&mut rng, Gen::Clustered, &mut l)
            && out.deltas.iter().any(|d| !d.is_empty())
        {
            break out;
        }
    };
    lc.note_edit(Side::Left, &l, &out);
    assert!(!lc.current().bytes_identical());
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    lc.install(done, Pair::new(&l, &r));
    assert!(!lc.current().bytes_identical());
    assert_ne!(lc.current().verdict(false), ContentResult::BinarySame);
    while let Some(out) = l.undo() {
        lc.note_edit(Side::Left, &l, &out);
    }
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    lc.install(done, Pair::new(&l, &r));
    assert!(!lc.current().bytes_identical());
    assert_eq!(lc.current().verdict(false), ContentResult::RulesSame);
}

/// Anchors move like line boundaries (§1.10.8): a boundary at the first line after a change moves with it, one
/// inside a removed run goes to the change's end, and an anchor that loses every line is dropped with a warning.
#[test]
fn anchors_move_with_their_lines() {
    use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
    use hikasa_text::{Anchor, AnchorSide};
    let text: String = (0..40).map(|k| format!("line {k}\n")).collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, r) = (editable(&text, &opts), editable(&text, &opts));
    let mut setup = setup(Gen::Clustered);
    setup.anchors = vec![
        Anchor::Pair {
            left: 10..12,
            right: 10..12,
        },
        Anchor::Isolate {
            side: AnchorSide::Left,
            lines: 20..22,
        },
        Anchor::Isolate {
            side: AnchorSide::Left,
            lines: 30..31,
        },
    ];
    let mut lc = live(&setup, Pair::new(&l, &r));
    let mut edit = |l: &mut hikasa_text::edit::EditableDocument, a: u32, b: u32, t: &str| {
        let out = l
            .apply(
                &EditRequest::Replace {
                    range: TextPos::new(a, 0)..TextPos::new(b, 0),
                    text: t.into(),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
        lc.note_edit(Side::Left, l, &out);
    };
    // Line 9 becomes two lines: the pair starting right after it moves by one.
    edit(&mut l, 9, 10, "a\nb\n");
    // Lines 22..25 (the isolated range's second line and two after it) removed: its end moves to the change.
    edit(&mut l, 22, 25, "");
    // Line 28 (old 30) removed: that anchor is dropped.
    edit(&mut l, 28, 29, "");
    assert_eq!(
        lc.anchors(),
        [
            Anchor::Pair {
                left: 11..13,
                right: 10..12
            },
            Anchor::Isolate {
                side: AnchorSide::Left,
                lines: 21..22
            },
        ]
    );
    let warnings = lc.take_warnings();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].code, hikasa_core::WarningCode::AnchorDropped);
}

/// F10: inside one very large section (two unrelated files are one) the provisional splice changes the section by
/// difference instead of re-deriving it: after every edit inside it — typing, new and removed lines, undo — the
/// sections, class runs and statistics still equal a recount.
#[test]
fn edits_inside_a_huge_section_keep_it_exact() {
    let n = 12_000u32;
    let left: String = (0..n).map(|k| format!("left {k}\n")).collect();
    let right: String = (0..n).map(|k| format!("right {k}\n")).collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, mut r) = (editable(&left, &opts), editable(&right, &opts));
    let setup = setup(Gen::Clustered);
    let mut lc = live(&setup, Pair::new(&l, &r));
    assert_eq!(lc.current().sections().len(), 1);
    let mut rng = XorShift(0x0F10);
    for step in 0..80 {
        let s = side(&mut rng);
        let doc = if s == Side::Left { &mut l } else { &mut r };
        let out = if step % 9 == 8 {
            doc.undo()
        } else {
            // Away from the ends, so the change is strictly inside the section.
            let line = 100 + rng.below(u64::from(doc.line_count() - 200)) as u32;
            let at = hikasa_text::edit::TextPos::new(line, 1);
            let text = ["x", "\n", "a\nb\nc\n", ""][rng.below(4) as usize];
            let to = if text.is_empty() {
                hikasa_text::edit::TextPos::new(line + 1 + rng.below(3) as u32, 0)
            } else {
                at
            };
            doc.apply(
                &hikasa_text::edit::EditRequest::Replace {
                    range: at..to,
                    text: text.into(),
                    keep_eols: false,
                },
                hikasa_text::edit::GroupHint::None,
            )
            .ok()
        };
        if let Some(out) = out {
            lc.note_edit(s, doc, &out);
        }
        lc.current().check_structure().unwrap();
        lc.check_invariants().unwrap();
    }
    let done = lc
        .recompare_job(Pair::new(&l, &r))
        .run(None, &Ctl::detached())
        .unwrap();
    lc.install(done, Pair::new(&l, &r));
    let want = fresh(&setup, Pair::new(&l, &r), &[]);
    assert_eq!(
        common::canonical_json(lc.current()),
        common::canonical_json(&want)
    );
}

/// F10: a section's kind follows its rows when it changes by difference: one Changed row inside a very large
/// section of LeftOnly rows (a near copy of one left line on the right); removing that right line leaves only
/// LeftOnly rows, a Delete section, as a recount says.
#[test]
fn a_huge_section_changes_kind_by_difference() {
    let n = 9_000u32;
    let left: String = (0..n)
        .map(|k| format!("line number {k} with some text\n"))
        .collect();
    let right = format!(
        "line number 0 with some text\nline number 4500 with some texT\nline number {} with some text\n",
        n - 1
    );
    let opts = edit_options(Gen::Clustered);
    let (l, mut r) = (editable(&left, &opts), editable(&right, &opts));
    let setup = setup(Gen::Clustered);
    let mut lc = live(&setup, Pair::new(&l, &r));
    let cmp = lc.current();
    assert_eq!(cmp.sections().len(), 1, "{:?}", cmp.sections());
    assert_eq!(cmp.sections()[0].kind, hikasa_text::SectionKind::Change);
    let out = r
        .apply(
            &hikasa_text::edit::EditRequest::Replace {
                range: hikasa_text::edit::TextPos::new(1, 0)..hikasa_text::edit::TextPos::new(2, 0),
                text: String::new(),
                keep_eols: false,
            },
            hikasa_text::edit::GroupHint::None,
        )
        .unwrap();
    lc.note_edit(Side::Right, &r, &out);
    lc.current().check_structure().unwrap();
    assert_eq!(
        lc.current().sections()[0].kind,
        hikasa_text::SectionKind::Delete
    );
}
