//! A11.14 (D-307): the Find and Replace panels. The panel's query reaches core's `Finder` unchanged (each option); Find
//! Next/Previous in a comparison go by row, the left side first, with *Wrap around* and *passed the end*; *Close
//! automatically*; *init with the current word*; a key is applied while a search runs; the highlights on screen are
//! `hits_in_line`; *Select All* is a line set, then Copy (`lines_text`) and Delete (one step); Replace one; Replace All in
//! a selection and on both sides, one step each, after the question, equal to `replace_all` on the same documents.

use hikasa_core::Ctl;
use hikasa_core::Side;
use hikasa_text::edit::{GroupHint, LineSet, TextPos, doc_end, lines_text};
use hikasa_text::find::{FindQuery, FindSides, Finder, Replacement, replace_all};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::tests_text::Fixture;
use crate::text::find::{self, FindAction, NOT_FOUND, PASSED_END};
use crate::text::layout::FindShown;

const L: &str = "foo bar\nbar foo foo\nnone here\nFOO end\n";
const R: &str = "bar\nfoo bar foo\nnone\nfoo\n";

fn query(text: &str) -> FindQuery {
    FindQuery {
        text: text.to_owned(),
        regex: false,
        match_case: false,
        whole_word: false,
    }
}

/// Runs a panel button and settles.
fn act(f: &mut Fixture, action: FindAction) {
    f.m().find.ask = Some(action);
    f.settle();
}

/// The focus pane's selection as (side, start, end).
fn hit(f: &mut Fixture) -> (Side, TextPos, TextPos) {
    let side = f.m().focus();
    let r = f.m().pane(side).selection().range();
    (side, r.start, r.end)
}

#[test]
fn a11_14_the_query_reaches_the_finder_unchanged() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Find);
    for (regex, match_case, whole_word) in [
        (false, false, false),
        (true, true, false),
        (false, true, true),
    ] {
        let q = FindQuery {
            text: if regex { "fo+".into() } else { "foo".into() },
            regex,
            match_case,
            whole_word,
        };
        f.m().find.query = q.clone();
        act(&mut f, FindAction::Next);
        assert_eq!(f.m().find.compiled().expect("a finder").query(), &q);
    }
    // An invalid pattern: core's message under the field, no search.
    f.m().find.query = FindQuery {
        text: "(".into(),
        regex: true,
        match_case: false,
        whole_word: false,
    };
    act(&mut f, FindAction::Next);
    let expected = Finder::new(
        &f.m().find.query.clone(),
        "",
        &hikasa_text::RegexLimits::default(),
    )
    .err()
    .map(|e| e.to_string());
    assert_eq!(f.m().find.error, expected);
}

#[test]
fn a11_14_find_by_row_left_first_with_wrap() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::Find);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Both;
    // Row 0: left "foo" at 0..3; then row 1: left's two, right's two; then row 3: left FOO, right foo.
    let mut seen = Vec::new();
    for _ in 0..8 {
        act(&mut f, FindAction::Next);
        let (side, a, b) = hit(&mut f);
        seen.push((side, a.line.0, a.byte, b.byte));
    }
    assert_eq!(
        seen,
        vec![
            (Side::Left, 0, 0, 3),
            (Side::Left, 1, 4, 7),
            (Side::Left, 1, 8, 11),
            (Side::Right, 1, 0, 3),
            (Side::Right, 1, 8, 11),
            (Side::Left, 3, 0, 3),
            (Side::Right, 3, 0, 3),
            (Side::Right, 3, 0, 3),
        ],
        "the last one finds nothing (no wrap): the hit stays"
    );
    assert_eq!(f.m().find.note.as_deref(), Some(NOT_FOUND));
    // Wrap around: from the end to the first hit, *passed the end*.
    f.m().find.wrap = true;
    act(&mut f, FindAction::Next);
    assert_eq!(
        hit(&mut f),
        (Side::Left, TextPos::new(0, 0), TextPos::new(0, 3))
    );
    assert_eq!(f.m().find.note.as_deref(), Some(PASSED_END));
    // Find Previous from there wraps to the last.
    act(&mut f, FindAction::Prev);
    assert_eq!(
        hit(&mut f),
        (Side::Right, TextPos::new(3, 0), TextPos::new(3, 3))
    );
    // F3 without the panel reuses the query.
    crate::text::find::close(f.m());
    f.place(Side::Left, 0, 0);
    f.run(CommandId::FindNext);
    f.settle();
    assert_eq!(hit(&mut f).1, TextPos::new(0, 0));
    // One side only.
    f.m().find.sides = FindSides::Right;
    f.place(Side::Left, 0, 0);
    f.run(CommandId::FindNext);
    f.settle();
    assert_eq!(hit(&mut f).0, Side::Right);
}

#[test]
fn a11_14_close_automatically_and_the_current_word() {
    let mut f = Fixture::new(L, R);
    // *init_find_with_current_word* (on by default): the word at the caret.
    f.place(Side::Left, 2, 1);
    f.run(CommandId::Find);
    assert_eq!(f.m().find.query.text, "none");
    // A selection on one line wins.
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(1, 0), TextPos::new(1, 7));
    f.run(CommandId::Find);
    assert_eq!(f.m().find.query.text, "bar foo");
    // Close automatically: the panel goes after a hit.
    f.m().find.close_automatically = true;
    f.m().find.query = query("end");
    act(&mut f, FindAction::Next);
    assert_eq!(f.m().display().find_panel, FindShown::None);
    assert_eq!(
        hit(&mut f),
        (Side::Left, TextPos::new(3, 4), TextPos::new(3, 7))
    );
}

#[test]
fn a11_14_a_search_does_not_block_a_key() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Left, 2, 0);
    f.run(CommandId::Find);
    f.m().find.query = query("foo");
    let service = f.world.service.clone();
    assert!(find::find(f.m(), &service, hikasa_text::Direction::Forward));
    // The search is a job; the pane is not frozen by it: a key now is an edit now.
    assert!(!f.m().frozen(Side::Left));
    f.type_text("Z");
    assert!(f.text(Side::Left).contains("Znone"));
    // Its result was taken on documents of another generation: searched again on the documents of now.
    let started = f.m().find.started;
    f.settle();
    assert!(f.m().find.started > started, "searched again");
    let (side, a, _) = hit(&mut f);
    assert_eq!((side, a), (Side::Left, TextPos::new(3, 0)));
}

#[test]
fn a11_14_highlights_are_hits_in_line() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Find);
    f.m().find.query = query("foo");
    act(&mut f, FindAction::Next);
    let finder =
        Finder::new(&query("foo"), "", &hikasa_text::RegexLimits::default()).expect("finder");
    for side in [Side::Left, Side::Right] {
        let doc = f.doc(side);
        for line in 0..doc.line_count() {
            let text = doc.line(hikasa_text::LineIdx(line));
            let mut expected = Vec::new();
            finder.hits_in_line(&text, 0, |r| {
                expected.push(r);
                std::ops::ControlFlow::Continue(())
            });
            assert_eq!(
                find::hits_on_screen(f.m(), side, line),
                expected,
                "{side:?} {line}"
            );
        }
    }
}

#[test]
fn a11_14_select_all_is_a_line_set() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::Find);
    f.m().find.query = query("foo");
    act(&mut f, FindAction::SelectAll);
    let lines = f.m().pane(Side::Left).lines.clone().expect("a line set");
    assert_eq!(lines, LineSet::from_ranges(vec![0..2, 3..4]));
    assert_eq!(f.m().find.note.as_deref(), Some("3 lines selected"));
    // Copy writes core's `lines_text`; Delete removes the lines as one step.
    let doc = f.doc(Side::Left);
    let expected = lines_text(&doc, &lines);
    f.run(CommandId::Copy);
    f.settle();
    let placed = f.clip.payloads().last().map(|p| p.text.clone());
    assert_eq!(placed, Some(expected));
    let mut expected = doc.clone();
    let req = hikasa_text::edit::delete_lines(&expected, &lines).expect("a request");
    expected.apply(&req, GroupHint::None).expect("apply");
    f.run(CommandId::Delete);
    assert_eq!(
        f.text(Side::Left),
        expected.text(TextPos::ZERO..doc_end(&expected))
    );
    assert_eq!(f.doc(Side::Left).undo_steps(), 1);
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), L);
}

#[test]
fn a11_14_replace_one_then_next() {
    let mut f = Fixture::new(L, R);
    f.place(Side::Left, 0, 0);
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Left;
    f.m().find.replace = "qux".into();
    act(&mut f, FindAction::Next);
    act(&mut f, FindAction::Replace);
    assert_eq!(
        f.text(Side::Left),
        "qux bar\nbar foo foo\nnone here\nFOO end\n"
    );
    // The next hit is selected; a selection that does not match is not replaced (Replace is Find Next then).
    assert_eq!(hit(&mut f).1, TextPos::new(1, 4));
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(2, 0), TextPos::new(2, 4));
    act(&mut f, FindAction::Replace);
    assert_eq!(
        f.text(Side::Left),
        "qux bar\nbar foo foo\nnone here\nFOO end\n"
    );
}

#[test]
fn a11_14_replace_all_equals_core() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Both;
    f.m().find.replace = "X".into();
    let docs = [f.doc(Side::Left), f.doc(Side::Right)];
    act(&mut f, FindAction::ReplaceAll);
    assert_eq!(f.prompt().as_deref(), Some("Replace 7 occurrences?"));
    f.answer(ButtonRole::Yes);
    let finder =
        Finder::new(&query("foo"), "", &hikasa_text::RegexLimits::default()).expect("finder");
    let with = Replacement::new("X", &finder).expect("with");
    for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
        let mut expected = docs[k].clone();
        let (req, _) =
            replace_all(&expected, &finder, &with, None, &Ctl::detached()).expect("plan");
        expected
            .apply(&req.expect("a request"), GroupHint::None)
            .expect("apply");
        assert_eq!(
            f.text(side),
            expected.text(TextPos::ZERO..doc_end(&expected)),
            "{side:?}"
        );
        // One step per pane.
        assert_eq!(f.doc(side).undo_steps(), 1, "{side:?}");
    }
    assert_eq!(f.m().find.count, Some(7));
    // In a selection that spans lines: only there.
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Left;
    f.m().find.replace = "Y".into();
    f.m().set_focus(Side::Left);
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(1, 0), TextPos::new(2, 0));
    act(&mut f, FindAction::ReplaceAll);
    assert_eq!(f.prompt().as_deref(), Some("Replace 2 occurrences?"));
    f.answer(ButtonRole::Yes);
    assert_eq!(f.text(Side::Left), "foo bar\nbar Y Y\nnone here\nFOO end\n");
    assert_eq!(
        f.text(Side::Right),
        R,
        "the right side was not in the sides"
    );
    // Cancel replaces nothing.
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.replace = "Z".into();
    act(&mut f, FindAction::ReplaceAll);
    f.answer(ButtonRole::Cancel);
    assert_eq!(f.text(Side::Left), L);
    // A read-only pane is searched, not replaced: both sides, the right one read-only.
    let mut f = Fixture::new(L, R);
    f.m().set_write_policy(crate::folder_view::WritePolicy {
        read_only: hikasa_folder::ops::ReadOnlySides {
            left: false,
            right: true,
        },
        ..crate::folder_view::WritePolicy::default()
    });
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Both;
    f.m().find.replace = "W".into();
    act(&mut f, FindAction::ReplaceAll);
    assert_eq!(f.prompt().as_deref(), Some("Replace 4 occurrences?"));
    f.answer(ButtonRole::Yes);
    assert_eq!(f.text(Side::Left), "W bar\nbar W W\nnone here\nW end\n");
    assert_eq!(f.text(Side::Right), R, "the read-only side");
}
