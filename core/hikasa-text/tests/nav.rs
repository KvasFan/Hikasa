//! A10.25 (D-286): navigation data (§1.10.20). Every row of the table on a fixture comparison: sections (ignored ones
//! and unimportant ones skipped, the display filter), inline differences range by range (a 50,000-character changed
//! line), edits, find; wrap and `None`; `existing_line` over gaps; `parse_goto` forms and errors; `goto` on CPL physical
//! lines and clamping; `column_of` with tabs and combining marks.

mod common;

use std::sync::Arc;

use common::{compare_texts, format};
use hikasa_core::{Pair, Side};
use hikasa_text::edit::{EditOptions, EditRequest, Editability, EditableDocument, GroupHint};
use hikasa_text::find::{FindQuery, FindSides, Finder};
use hikasa_text::nav::{
    BrowseMode, NavContext, NavPos, NavTarget, column_of, column_of_tabs, existing_line, goto,
    parse_goto, step,
};
use hikasa_text::{
    Direction, RegexLimits, RowIdx, TextComparison, TextDocument, TextPos, TextRules,
};

fn editable(text: &str) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions::default(),
        Editability::EDITABLE,
    )
}

const LEFT: &str = "same\nold one\nkeep\nkeep2\ngone\nend\n";
const RIGHT: &str = "same\nnew one\nkeep\nkeep2\nend\nadded\n";

struct Fixture {
    cmp: TextComparison,
    docs: Pair<EditableDocument>,
}

fn fixture(left: &str, right: &str) -> Fixture {
    Fixture {
        cmp: compare_texts(left, right, &format("ee"), &TextRules::default()),
        docs: Pair::new(editable(left), editable(right)),
    }
}

fn ctx<'a>(f: &'a Fixture, finder: Option<&'a Finder>) -> NavContext<'a> {
    NavContext {
        cmp: Some(&f.cmp),
        docs: Pair::new(Some(&f.docs.left), Some(&f.docs.right)),
        view: None,
        ignore_unimportant: false,
        finder,
        sides: FindSides::Both,
    }
}

fn at(row: u32, side: Side, line: u32, byte: u32) -> NavPos {
    NavPos {
        row: Some(RowIdx(row)),
        side,
        pos: TextPos::new(line, byte),
    }
}

fn rows(t: &Option<NavTarget>) -> Option<(u32, u32, u32, bool)> {
    t.as_ref().map(|t| {
        (
            t.row.map_or(u32::MAX, |r| r.0),
            t.range.start.line.0,
            t.range.start.byte,
            t.wrapped,
        )
    })
}

#[test]
fn sections_forward_backward_and_wrap() {
    let f = fixture(LEFT, RIGHT);
    let c = ctx(&f, None);
    let sections: Vec<u32> = f.cmp.sections().iter().map(|s| s.rows.start).collect();
    assert!(sections.len() >= 2, "{sections:?}");
    let next = step(
        BrowseMode::DiffSection,
        &c,
        at(0, Side::Left, 0, 0),
        Direction::Forward,
        false,
    );
    assert_eq!(next.as_ref().unwrap().row, Some(RowIdx(sections[0])));
    let last = *sections.last().unwrap();
    assert_eq!(
        step(
            BrowseMode::DiffSection,
            &c,
            at(last, Side::Left, 0, 0),
            Direction::Forward,
            false
        ),
        None
    );
    let wrapped = step(
        BrowseMode::DiffSection,
        &c,
        at(last, Side::Left, 0, 0),
        Direction::Forward,
        true,
    )
    .unwrap();
    assert_eq!(
        (wrapped.row, wrapped.wrapped),
        (Some(RowIdx(sections[0])), true)
    );
    let back = step(
        BrowseMode::DiffSection,
        &c,
        at(last, Side::Right, 0, 0),
        Direction::Backward,
        false,
    )
    .unwrap();
    assert_eq!(back.row, Some(RowIdx(sections[sections.len() - 2])));
    assert_eq!(
        step(
            BrowseMode::DiffSection,
            &c,
            at(sections[0], Side::Left, 1, 0),
            Direction::Backward,
            false
        ),
        None
    );
    // A display filter that hides every difference hides every section and difference from the steps.
    let view = f.cmp.view(&hikasa_text::ViewOptions {
        filter: hikasa_text::TextDisplayFilter::Same,
        ..hikasa_text::ViewOptions::default()
    });
    let hidden = NavContext {
        view: Some(&view),
        ..c
    };
    for mode in [BrowseMode::DiffSection, BrowseMode::Difference] {
        assert_eq!(
            step(
                mode,
                &hidden,
                at(0, Side::Left, 0, 0),
                Direction::Forward,
                true
            ),
            None
        );
    }
}

#[test]
fn unimportant_and_ignored_sections_are_skipped() {
    // Whitespace-only change: unimportant under the default rules of a C-like format.
    let left = "int a;\nint  b;\nint c;\nint d;\n";
    let right = "int a;\nint b;\nint c;\nint e;\n";
    let mut rules = TextRules::default();
    rules.importance.embedded_whitespace = false;
    let cmp = compare_texts(left, right, &format("ee"), &rules);
    let f = Fixture {
        cmp,
        docs: Pair::new(editable(left), editable(right)),
    };
    let mut c = ctx(&f, None);
    let first = step(
        BrowseMode::DiffSection,
        &c,
        at(0, Side::Left, 0, 0),
        Direction::Forward,
        false,
    )
    .unwrap();
    c.ignore_unimportant = true;
    let important = step(
        BrowseMode::DiffSection,
        &c,
        at(0, Side::Left, 0, 0),
        Direction::Forward,
        false,
    )
    .unwrap();
    assert!(
        important.row.unwrap().0 > first.row.unwrap().0,
        "{first:?} {important:?}"
    );
    assert_eq!(important.range.start.line.0, 3);
}

#[test]
fn a_long_changed_line_is_walked_range_by_range() {
    let base: String = "abcdefghij".repeat(5_000);
    let mut changed: Vec<char> = base.chars().collect();
    for k in [10usize, 20_000, 49_990] {
        changed[k] = 'Z';
    }
    let changed: String = changed.into_iter().collect();
    let left = format!("x\n{base}\ny\n");
    let right = format!("x\n{changed}\ny\n");
    let f = fixture(&left, &right);
    let c = ctx(&f, None);
    let mut walked = Vec::new();
    let mut from = at(0, Side::Right, 0, 0);
    while let Some(t) = step(BrowseMode::Difference, &c, from, Direction::Forward, false) {
        walked.push((t.range.start.line.0, t.range.start.byte, t.range.end.byte));
        from = NavPos {
            row: t.row,
            side: Side::Right,
            pos: t.range.end,
        };
        assert!(walked.len() < 100);
    }
    assert!(!walked.is_empty());
    assert!(walked.iter().all(|w| w.0 == 1));
    // In order, disjoint, covering the changed characters.
    assert!(walked.windows(2).all(|w| w[0].2 <= w[1].1));
    for k in [10u32, 20_000, 49_990] {
        assert!(
            walked.iter().any(|w| w.1 <= k && k < w.2),
            "{k}: {walked:?}"
        );
    }
    // Backward from the end of the line walks the same ranges in reverse.
    let mut back = Vec::new();
    let mut from = at(1, Side::Right, 1, changed.len() as u32);
    while let Some(t) = step(BrowseMode::Difference, &c, from, Direction::Backward, false) {
        back.push((t.range.start.line.0, t.range.start.byte, t.range.end.byte));
        from = NavPos {
            row: t.row,
            side: Side::Right,
            pos: t.range.start,
        };
        assert!(back.len() < 100);
    }
    back.reverse();
    assert_eq!(back, walked);
}

#[test]
fn one_sided_rows_are_whole_lines_and_wrap() {
    let f = fixture(LEFT, RIGHT);
    let c = ctx(&f, None);
    let all = |side| {
        let mut out = Vec::new();
        let mut from = at(0, side, 0, 0);
        while let Some(t) = step(BrowseMode::Difference, &c, from, Direction::Forward, false) {
            out.push(rows(&Some(t.clone())).unwrap());
            from = NavPos {
                row: t.row,
                side,
                pos: t.range.end,
            };
            assert!(out.len() < 50);
        }
        out
    };
    let left = all(Side::Left);
    // `old one` (its inline range) and the deleted `gone` line.
    assert_eq!(left.iter().map(|x| x.1).collect::<Vec<_>>(), [1, 4]);
    let right = all(Side::Right);
    assert_eq!(right.iter().map(|x| x.1).collect::<Vec<_>>(), [1, 5]);
    let wrapped = step(
        BrowseMode::Difference,
        &c,
        at(f.cmp.row_count(), Side::Right, 6, 0),
        Direction::Forward,
        true,
    );
    assert_eq!(rows(&wrapped).map(|x| (x.1, x.3)), Some((1, true)));
}

#[test]
fn edits_are_walked_by_runs() {
    let mut f = fixture(LEFT, RIGHT);
    for line in [1u32, 2, 4] {
        f.docs
            .left
            .apply(
                &EditRequest::Replace {
                    range: TextPos::new(line, 0)..TextPos::new(line, 0),
                    text: "*".into(),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
    }
    let c = ctx(&f, None);
    let a = step(
        BrowseMode::Edit,
        &c,
        at(0, Side::Left, 0, 0),
        Direction::Forward,
        false,
    )
    .unwrap();
    assert_eq!((a.range.start.line.0, a.range.end.line.0), (1, 2));
    let b = step(
        BrowseMode::Edit,
        &c,
        at(1, Side::Left, 1, 0),
        Direction::Forward,
        false,
    )
    .unwrap();
    assert_eq!(b.range.start.line.0, 4);
    assert_eq!(
        step(
            BrowseMode::Edit,
            &c,
            at(4, Side::Left, 4, 0),
            Direction::Forward,
            false
        ),
        None
    );
    let w = step(
        BrowseMode::Edit,
        &c,
        at(4, Side::Left, 4, 0),
        Direction::Forward,
        true,
    )
    .unwrap();
    assert_eq!((w.range.start.line.0, w.wrapped), (1, true));
    let back = step(
        BrowseMode::Edit,
        &c,
        at(4, Side::Left, 4, 0),
        Direction::Backward,
        false,
    )
    .unwrap();
    assert_eq!(back.range.start.line.0, 1);
    // The right pane has no edits.
    assert_eq!(
        step(
            BrowseMode::Edit,
            &c,
            at(0, Side::Right, 0, 0),
            Direction::Forward,
            true
        ),
        None
    );
}

#[test]
fn find_steps_through_the_comparison() {
    let f = fixture(LEFT, RIGHT);
    let q = FindQuery {
        text: "one".into(),
        regex: false,
        match_case: true,
        // Not the `one` of `gone`.
        whole_word: true,
    };
    let finder = Finder::new(&q, "", &RegexLimits::default()).unwrap();
    let c = ctx(&f, Some(&finder));
    let a = step(
        BrowseMode::Find,
        &c,
        at(0, Side::Left, 0, 0),
        Direction::Forward,
        false,
    )
    .unwrap();
    assert_eq!((a.side, a.range.start), (Side::Left, TextPos::new(1, 4)));
    let b = step(
        BrowseMode::Find,
        &c,
        at(1, Side::Left, 1, 7),
        Direction::Forward,
        false,
    )
    .unwrap();
    assert_eq!((b.side, b.range.start), (Side::Right, TextPos::new(1, 4)));
    assert_eq!(
        step(
            BrowseMode::Find,
            &c,
            at(1, Side::Right, 1, 7),
            Direction::Forward,
            false
        ),
        None
    );
    // Without a finder or a comparison.
    assert_eq!(
        step(
            BrowseMode::Find,
            &ctx(&f, None),
            at(0, Side::Left, 0, 0),
            Direction::Forward,
            true
        ),
        None
    );
    let edit = NavContext {
        cmp: None,
        docs: Pair::new(Some(&f.docs.left), None),
        view: None,
        ignore_unimportant: false,
        finder: Some(&finder),
        sides: FindSides::Both,
    };
    let t = step(
        BrowseMode::Find,
        &edit,
        NavPos {
            row: None,
            side: Side::Left,
            pos: TextPos::ZERO,
        },
        Direction::Forward,
        false,
    )
    .unwrap();
    assert_eq!((t.row, t.range.start), (None, TextPos::new(1, 4)));
}

#[test]
fn existing_lines_skip_gaps() {
    let f = fixture("a\nb\nc\n", "a\nx\ny\nz\nc\n");
    let cmp = &f.cmp;
    let left_rows: Vec<u32> = (0..cmp.row_count())
        .filter(|&r| cmp.row(RowIdx(r)).unwrap().left.is_some())
        .collect();
    for (k, &r) in left_rows.iter().enumerate() {
        let next = existing_line(cmp, RowIdx(r), Side::Left, Direction::Forward);
        assert_eq!(next, left_rows.get(k + 1).map(|&r| RowIdx(r)));
        let prev = existing_line(cmp, RowIdx(r), Side::Left, Direction::Backward);
        assert_eq!(prev, k.checked_sub(1).map(|k| RowIdx(left_rows[k])));
    }
    // From a gap row of the left side.
    if let Some(gap) = (0..cmp.row_count()).find(|&r| cmp.row(RowIdx(r)).unwrap().left.is_none()) {
        let next = existing_line(cmp, RowIdx(gap), Side::Left, Direction::Forward).unwrap();
        assert!(next.0 > gap && cmp.row(next).unwrap().left.is_some());
        let prev = existing_line(cmp, RowIdx(gap), Side::Left, Direction::Backward).unwrap();
        assert!(prev.0 < gap && cmp.row(prev).unwrap().left.is_some());
    }
}

#[test]
fn go_to_forms_and_errors() {
    assert_eq!(parse_goto("120"), Ok((120, None)));
    assert_eq!(parse_goto(" 120:7 "), Ok((120, Some(7))));
    assert_eq!(parse_goto(":7"), Ok((0, Some(7))));
    for bad in ["", "0", "x", "12:", "12:0", "-3", "1:2:3"] {
        assert!(parse_goto(bad).is_err(), "{bad}");
    }
}

#[test]
fn go_to_physical_lines_with_cpl_and_clamping() {
    let doc = EditableDocument::new(
        Arc::new(TextDocument::from_text(
            "short\n0123456789abcdefghij\nlast\n",
        )),
        EditOptions {
            chars_per_line_limit: Some(8),
            ..EditOptions::default()
        },
        Editability::EDITABLE,
    );
    // Physical line 2 is cut into pieces 1, 2, 3; line 3 is table line 4.
    assert_eq!(goto(&doc, 1, None), TextPos::new(0, 0));
    assert_eq!(goto(&doc, 2, None), TextPos::new(1, 0));
    assert_eq!(goto(&doc, 3, None), TextPos::new(4, 0));
    assert_eq!(goto(&doc, 2, Some(11)), TextPos::new(2, 2));
    assert_eq!(goto(&doc, 2, Some(99)), TextPos::new(3, 4));
    assert_eq!(goto(&doc, 99, Some(2)), TextPos::new(4, 1));
    assert_eq!(goto(&doc, 1, Some(99)), TextPos::new(0, 5));
}

#[test]
fn columns_count_characters_and_display_cells() {
    assert_eq!(column_of("abc", 0), (1, 1));
    assert_eq!(column_of("\tx", 1), (2, 5));
    assert_eq!(column_of_tabs("\tx", 1, 8), (2, 9));
    assert_eq!(column_of("e\u{301}x", 3), (3, 2));
    assert_eq!(column_of("界x", 3), (2, 3));
    assert_eq!(column_of("abc", 99), (4, 4));
}
