//! A10.21 (D-282, TC-024): ignored lines. Rows carry the flag, `stats.ignored_rows` counts them, and an ignored
//! row counts as same for the importance counts, the verdict (a pair whose only differences are ignored is *rules
//! same*), the display filters, the thumbnail and the section navigation (*next unignored section*); alignment
//! never changes. The marks follow edits in a live comparison; replacing them re-derives only what depends on
//! them and equals a fresh compare. The session keys, `ignored_lines_dropped`, the reports and the flags are
//! tested in hikasa-session and hikasa-cli.

mod common;

use std::sync::Arc;

use common::XorShift;
use common::live::{Gen, Setup, edit_options, editable, live, random_edit, side};
use hikasa_core::{ContentResult, Ctl, Pair, Side};
use hikasa_text::{
    CompareInput, CompiledFormat, Direction, IgnoredLines, Importance, RangeSet, RowIdx, RowState,
    TextComparison, TextDisplayFilter, TextRules, ThumbCell, ViewItem, ViewOptions, compare,
};

fn set(ranges: &[std::ops::Range<u32>]) -> RangeSet {
    RangeSet::from_ranges(ranges.to_vec())
}

fn compare_ignored(left: &str, right: &str, ignored: IgnoredLines) -> TextComparison {
    compare_full(
        left,
        right,
        &common::format("ee"),
        &TextRules::default(),
        ignored,
    )
}

fn compare_full(
    left: &str,
    right: &str,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    ignored: IgnoredLines,
) -> TextComparison {
    let cmp = compare(
        CompareInput {
            left: common::doc(left),
            right: common::doc(right),
            format: format.clone(),
            rules: rules.clone(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored,
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

const LEFT: &str = "keep 0\nold 1\nkeep 2\nkeep 3\ngone 4\nkeep 5\nkeep 6\nold 7\nkeep 8\n";
const RIGHT: &str = "keep 0\nnew 1\nkeep 2\nkeep 3\nkeep 5\nkeep 6\nnew 7\nkeep 8\nadded 8\n";

/// A pair whose only differences are ignored is *rules same*: the rows keep their alignment states and carry the
/// flag, `ignored_rows` counts every ignored row, and the importance counts leave them out.
#[test]
fn only_ignored_differences_are_rules_same() {
    let plain = compare_ignored(LEFT, RIGHT, IgnoredLines::default());
    assert_eq!(plain.verdict(false), ContentResult::ImportantDifferences);
    let s = plain.stats();
    assert_eq!((s.ignored_rows, s.sections), (0, 4));
    let differing = s.rows - s.same_rows;
    assert_eq!(s.important_rows, differing);

    // Every differing line ignored: left lines 1, 4, 7 and right lines 1, 6, 8 (two marks on a changed row).
    let ignored = IgnoredLines {
        left: set(&[1..2, 4..5, 7..8]),
        right: set(&[1..2, 6..7, 8..9]),
    };
    let cmp = compare_ignored(LEFT, RIGHT, ignored.clone());
    assert_eq!(cmp.ignored(), &ignored);
    assert_eq!(cmp.runs(), plain.runs(), "alignment never changes");
    assert_eq!(cmp.verdict(false), ContentResult::RulesSame);
    assert_eq!(cmp.verdict(true), ContentResult::RulesSame);
    let s = cmp.stats();
    assert_eq!(s.ignored_rows, differing);
    assert_eq!((s.important_rows, s.unimportant_rows), (0, 0));
    // The alignment's own counts stay.
    assert_eq!(
        (s.changed_rows, s.left_only_rows, s.right_only_rows),
        (
            plain.stats().changed_rows,
            plain.stats().left_only_rows,
            plain.stats().right_only_rows
        )
    );
    // Every section is ignored: none is counted, none is navigable.
    assert_eq!(cmp.sections().len(), plain.sections().len());
    assert!(cmp.sections().iter().all(|s| cmp.section_ignored(s.id)));
    assert_eq!((s.sections, s.important_sections), (0, 0));
    assert_eq!(s.important, Default::default());
    assert!(s.identities_hold());
    assert_eq!(cmp.next_section(RowIdx(0), Direction::Forward, false), None);
    for (k, row) in cmp.rows(0..cmp.row_count()).enumerate() {
        assert_eq!(row.ignored, row.state != RowState::Same, "row {k}");
        let p = plain.row(RowIdx(k as u32)).unwrap();
        assert_eq!((row.state, row.importance), (p.state, p.importance));
    }
}

/// An ignored row on a same line is flagged and counted, but changes nothing else.
#[test]
fn ignored_same_rows_are_flagged_and_counted() {
    let cmp = compare_ignored(
        LEFT,
        RIGHT,
        IgnoredLines {
            left: set(&[0..1]),
            right: set(&[2..4]),
        },
    );
    let plain = compare_ignored(LEFT, RIGHT, IgnoredLines::default());
    assert_eq!(cmp.stats().ignored_rows, 3);
    assert_eq!(cmp.stats().important_rows, plain.stats().important_rows);
    assert_eq!(cmp.verdict(false), plain.verdict(false));
    assert_eq!(cmp.sections(), plain.sections());
    assert!(cmp.row(RowIdx(0)).unwrap().ignored);
}

/// Some differences ignored: the others still count, and a partly ignored section keeps its kind and lines while
/// its importance comes from its rows that are not ignored.
#[test]
fn ignored_rows_leave_the_other_differences_counted() {
    // Comments unimportant in C: the section of lines 1–2 holds an important row (1) and an unimportant one (2).
    let c = common::format("c-cpp");
    let mut rules = TextRules::default();
    rules
        .importance
        .elements
        .insert("Comment".to_owned(), false);
    let left = "int a;\nint b;\nint c; // x\nint d;\nint e;\n";
    let right = "int a;\nint B;\nint c; // y\nint d;\nint E;\n";
    let plain = compare_full(left, right, &c, &rules, IgnoredLines::default());
    assert_eq!(plain.stats().important_rows, 2);
    assert_eq!(plain.stats().unimportant_rows, 1);
    let ignored = IgnoredLines {
        left: set(&[1..2]),
        right: RangeSet::new(),
    };
    let cmp = compare_full(left, right, &c, &rules, ignored);
    let s = cmp.stats();
    assert_eq!(
        (s.important_rows, s.unimportant_rows, s.ignored_rows),
        (1, 1, 1)
    );
    assert_eq!(cmp.verdict(false), ContentResult::ImportantDifferences);
    let first = &cmp.sections()[0];
    assert_eq!(first.rows, plain.sections()[0].rows);
    assert_eq!(first.kind, plain.sections()[0].kind);
    assert_eq!(first.importance, Importance::Unimportant);
    assert!(!cmp.section_ignored(first.id));
    // With ignore-unimportant the partly ignored section is skipped, the last one is not.
    assert_eq!(
        cmp.next_section_from(RowIdx(0), Direction::Forward, true),
        Some(cmp.sections()[1].id)
    );
    assert_eq!(s.sections, 2);
    assert!(s.identities_hold());
    // Ignoring the last important line too: only the unimportant difference is left.
    let cmp = compare_full(
        left,
        right,
        &c,
        &rules,
        IgnoredLines {
            left: set(&[1..2, 4..5]),
            right: RangeSet::new(),
        },
    );
    assert_eq!(cmp.verdict(false), ContentResult::UnimportantOnly);
    assert_eq!(cmp.verdict(true), ContentResult::RulesSame);
    assert_eq!(cmp.stats().sections, 1);
}

/// *Next unignored section* (TC-050): a section whose rows are all ignored is skipped both ways.
#[test]
fn section_navigation_skips_ignored_sections() {
    let ignored = IgnoredLines {
        left: set(&[4..5]),
        right: RangeSet::new(),
    };
    let cmp = compare_ignored(LEFT, RIGHT, ignored);
    let ids: Vec<u32> = cmp.sections().iter().map(|s| s.id.0).collect();
    assert_eq!(ids.len(), 4);
    let gone = cmp
        .sections()
        .iter()
        .find(|s| cmp.section_ignored(s.id))
        .unwrap()
        .clone();
    assert_eq!(gone.left_lines, 4..5);
    let before = &cmp.sections()[gone.id.0 as usize - 1];
    let after = &cmp.sections()[gone.id.0 as usize + 1];
    assert_eq!(
        cmp.next_section(RowIdx(before.rows.start), Direction::Forward, false),
        Some(after.id)
    );
    assert_eq!(
        cmp.next_section(RowIdx(after.rows.start), Direction::Backward, false),
        Some(before.id)
    );
    assert_eq!(cmp.section_at(RowIdx(gone.rows.start)), Some(gone.id));
    assert_eq!(cmp.stats().sections, 3);
}

/// Display filters and the thumbnail: an ignored row is in the *same* category (hidden by Show Differences,
/// shown by Show Same) and its bucket is `SAME | IGNORED`.
#[test]
fn display_filters_and_thumbnail_treat_ignored_rows_as_same() {
    let ignored = IgnoredLines {
        left: set(&[1..2]),
        right: set(&[1..2]),
    };
    let cmp = compare_ignored(LEFT, RIGHT, ignored);
    let row1 = cmp
        .row_for_line(Side::Left, hikasa_text::LineIdx(1))
        .unwrap();
    let visible = |filter| -> Vec<u32> {
        cmp.view(&ViewOptions {
            filter,
            ..ViewOptions::default()
        })
        .iter()
        .filter_map(|i| match i {
            ViewItem::Row { row } => Some(row.0),
            ViewItem::Gap { .. } => None,
        })
        .collect()
    };
    assert!(!visible(TextDisplayFilter::Differences).contains(&row1.0));
    assert!(visible(TextDisplayFilter::Same).contains(&row1.0));
    let thumb = cmp.thumbnail(cmp.row_count(), false);
    assert_eq!(thumb[row1.0 as usize], ThumbCell::SAME | ThumbCell::IGNORED);
    let plain = compare_ignored(LEFT, RIGHT, IgnoredLines::default());
    assert!(
        plain.thumbnail(plain.row_count(), false)[row1.0 as usize].contains(ThumbCell::IMPORTANT)
    );
}

/// Lines beyond a document are dropped; marking again removes the marks.
#[test]
fn ranges_are_clipped_and_toggled() {
    let cmp = compare_ignored(
        LEFT,
        RIGHT,
        IgnoredLines {
            left: set(&[8..40]),
            right: set(&[100..200]),
        },
    );
    assert_eq!(cmp.ignored().left.ranges(), &[8..9]);
    assert!(cmp.ignored().right.is_empty());
    let mut s = set(&[2..5]);
    s.toggle(2..5);
    assert!(s.is_empty());
}

/// Replacing the ignored lines of a live comparison (the *Ignored* command) re-derives what depends on them,
/// over windows of whole sections or the whole comparison, and equals a fresh compare with the new marks; edits
/// move the marks (a removed range disappears) and every recompare stays exact.
#[test]
fn live_marks_follow_edits_and_toggles() {
    for seed in 1..=60u64 {
        let mut rng = XorShift(seed * 0x9E37_79B9);
        let kind = [Gen::Clustered, Gen::Repetitive, Gen::Source][rng.below(3) as usize];
        let n = 20 + rng.below(200);
        let (l, r) = common::live::pair(&mut rng, kind, n);
        let opts = edit_options(kind);
        let (mut left, mut right) = (editable(&l, &opts), editable(&r, &opts));
        let setup = Setup {
            format: common::format(if kind == Gen::Source { "c-cpp" } else { "ee" }),
            rules: TextRules::default(),
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored: common::live::random_ignored(
                &mut rng,
                [left.line_count(), right.line_count()],
            ),
        };
        let mut lc = live(&setup, Pair::new(&left, &right));
        let mut marks =
            common::live::IgnoredModel::new(lc.ignored(), [left.line_count(), right.line_count()]);
        for step in 0..12 {
            if rng.below(2) == 0 {
                // A toggle: small, or most of a side (the whole re-derivation).
                let s = side(&mut rng);
                let count = if s == Side::Left {
                    left.line_count()
                } else {
                    right.line_count()
                };
                if count > 0 {
                    let a = rng.below(u64::from(count)) as u32;
                    let len = if rng.below(4) == 0 {
                        count
                    } else {
                        1 + rng.below(6) as u32
                    };
                    marks.toggle(s, a..(a + len).min(count));
                    lc.set_ignored(marks.ignored());
                    let want = common::live::fresh_ignored(
                        &setup,
                        Pair::new(&left, &right),
                        lc.anchors(),
                        &marks.ignored(),
                    );
                    if lc.pending_rows() == 0 {
                        assert_eq!(
                            common::canonical_json_all(lc.current()),
                            common::canonical_json_all(&want),
                            "seed {seed} step {step}: set_ignored"
                        );
                    }
                }
            }
            for _ in 0..1 + rng.below(3) {
                let s = side(&mut rng);
                let doc = if s == Side::Left {
                    &mut left
                } else {
                    &mut right
                };
                if let Some(out) = random_edit(&mut rng, kind, doc) {
                    lc.note_edit(s, doc, &out);
                    for &d in &out.deltas {
                        marks.apply(s, d);
                    }
                }
            }
            assert_eq!(lc.ignored(), &marks.ignored(), "seed {seed} step {step}");
            lc.check_invariants().unwrap();
            if rng.below(2) == 0 {
                let done = lc
                    .recompare_job(Pair::new(&left, &right))
                    .run(None, &Ctl::detached())
                    .unwrap();
                lc.install(done, Pair::new(&left, &right));
                let want = common::live::fresh_ignored(
                    &setup,
                    Pair::new(&left, &right),
                    lc.anchors(),
                    &marks.ignored(),
                );
                assert_eq!(
                    common::canonical_json_all(lc.current()),
                    common::canonical_json_all(&want),
                    "seed {seed} step {step}: recompare"
                );
            }
        }
    }
}

/// A toggle while a recompare runs: the job's result is installed with the newer marks applied on top.
#[test]
fn marks_set_during_a_recompare_are_applied_at_install() {
    let text: String = (0..60).map(|k| format!("line {k}\n")).collect();
    let other: String = (0..60)
        .map(|k| {
            if k % 10 == 3 {
                format!("changed {k}\n")
            } else {
                format!("line {k}\n")
            }
        })
        .collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, r) = (editable(&text, &opts), editable(&other, &opts));
    let setup = Setup {
        format: common::format("ee"),
        rules: TextRules::default(),
        force_tokenize: false,
        anchors: Vec::new(),
        right_format: None,
        ignored: IgnoredLines::default(),
    };
    let mut lc = live(&setup, Pair::new(&l, &r));
    let at = hikasa_text::TextPos::new(5, 0);
    let out = l
        .apply(
            &hikasa_text::EditRequest::Replace {
                range: at..at,
                text: "inserted\n".into(),
                keep_eols: false,
            },
            hikasa_text::GroupHint::None,
        )
        .unwrap();
    lc.note_edit(Side::Left, &l, &out);
    let job = lc.recompare_job(Pair::new(&l, &r));
    let marks = IgnoredLines {
        left: set(&[0..61]),
        right: set(&[0..60]),
    };
    lc.set_ignored(marks.clone());
    let done = job.run(None, &Ctl::detached()).unwrap();
    lc.install(done, Pair::new(&l, &r));
    assert_eq!(lc.ignored(), &marks);
    let want = common::live::fresh_ignored(&setup, Pair::new(&l, &r), &[], &marks);
    assert_eq!(
        common::canonical_json_all(lc.current()),
        common::canonical_json_all(&want)
    );
    assert_eq!(lc.current().verdict(false), ContentResult::RulesSame);
}
