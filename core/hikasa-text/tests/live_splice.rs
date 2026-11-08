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
