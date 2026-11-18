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

// ---- W11b review fixes ------------------------------------------------------------------------------------------

fn word(text: &str) -> FindQuery {
    FindQuery {
        whole_word: true,
        ..query(text)
    }
}

/// Finding 2: Replace All of both sides with whole words matches each document by its own word characters (a COBOL
/// pane's `foo-bar` is one word), equal to core's `replace_all` of each document alone.
#[test]
fn w11b_review_replace_all_uses_each_documents_word_characters() {
    for cobol in [Side::Left, Side::Right] {
        let mut f = Fixture::named("foo-bar foo\n", "foo-bar foo\n", "a.txt", "b.txt");
        f.m().request_format(cobol, Some("cobol".into()));
        f.settle();
        let docs = [f.doc(Side::Left), f.doc(Side::Right)];
        assert_ne!(
            docs[0].options().word_chars,
            docs[1].options().word_chars,
            "fixture: different word characters"
        );
        f.run(CommandId::Replace);
        f.m().find.query = word("foo");
        f.m().find.sides = FindSides::Both;
        f.m().find.replace = "X".into();
        act(&mut f, FindAction::ReplaceAll);
        assert!(f.prompt().is_some());
        f.answer(ButtonRole::Yes);
        for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            let doc = &docs[k];
            let finder = Finder::new(
                &word("foo"),
                &doc.options().word_chars,
                &hikasa_text::RegexLimits::default(),
            )
            .expect("finder");
            let with = Replacement::new("X", &finder).expect("with");
            let (req, _) = replace_all(doc, &finder, &with, None, &Ctl::detached()).expect("plan");
            let mut expected = doc.clone();
            if let Some(req) = req {
                expected.apply(&req, GroupHint::None).expect("apply");
            }
            assert_eq!(
                f.text(side),
                expected.text(TextPos::ZERO..doc_end(&expected)),
                "{side:?} (COBOL {cobol:?})"
            );
        }
        let cobol_text = f.text(cobol);
        assert_eq!(
            cobol_text, "foo-bar X\n",
            "COBOL's `foo-bar` holds no word `foo`"
        );
    }
}

/// Finding 2: Find Next in a comparison matches each side by its own word characters.
#[test]
fn w11b_review_find_uses_each_documents_word_characters() {
    let mut f = Fixture::named("foo-bar\n", "foo-bar\n", "a.txt", "b.txt");
    f.m().request_format(Side::Right, Some("cobol".into()));
    f.settle();
    f.run(CommandId::Find);
    f.m().find.query = word("foo");
    f.m().find.sides = FindSides::Both;
    f.place(Side::Left, 0, 0);
    act(&mut f, FindAction::Next);
    // The left (plain text) holds `foo`; the right (COBOL) does not.
    assert_eq!(
        hit(&mut f),
        (Side::Left, TextPos::new(0, 0), TextPos::new(0, 3))
    );
    act(&mut f, FindAction::Next);
    let at = hit(&mut f);
    assert_eq!(f.m().find.note.as_deref(), Some(NOT_FOUND), "{at:?}");
    // The highlights too: none on the right.
    assert!(find::hits_on_screen(f.m(), Side::Right, 0).is_empty());
    assert_eq!(find::hits_on_screen(f.m(), Side::Left, 0), vec![0..3]);
}

/// Finding 9: Replace (one) never writes a pane the panel's sides leave out: it is Find Next.
#[test]
fn w11b_review_replace_one_keeps_to_the_sides() {
    let mut f = Fixture::new("foo\n", "foo\n");
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Right;
    f.m().find.replace = "X".into();
    f.m().set_focus(Side::Left);
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(0, 0), TextPos::new(0, 3));
    act(&mut f, FindAction::Replace);
    assert_eq!(f.text(Side::Left), "foo\n", "the left side is not searched");
    assert_eq!(
        hit(&mut f),
        (Side::Right, TextPos::new(0, 0), TextPos::new(0, 3))
    );
    act(&mut f, FindAction::Replace);
    assert_eq!(f.text(Side::Right), "X\n");
}

/// C1: an edit between Replace All's plan and its answer: the plan is made and asked again, never applied stale.
#[test]
fn w11b_review_replace_all_after_an_edit_plans_again() {
    let mut f = Fixture::new("foo foo\n", "x\n");
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.sides = FindSides::Left;
    f.m().find.replace = "X".into();
    act(&mut f, FindAction::ReplaceAll);
    assert!(
        f.prompt().is_some_and(|p| p.contains('2')),
        "{:?}",
        f.prompt()
    );
    f.place(Side::Left, 0, 0);
    f.type_text("foo ");
    f.answer(ButtonRole::Yes);
    // Asked again with the count of now.
    assert!(
        f.prompt().is_some_and(|p| p.contains('3')),
        "{:?}",
        f.prompt()
    );
    f.answer(ButtonRole::Yes);
    assert_eq!(f.text(Side::Left), "X X X\n");
}

/// C2: each Replace All is its own undo step.
#[test]
fn w11b_review_each_replace_all_is_one_undo_step() {
    let mut f = Fixture::new("foo bar\n", "x\n");
    f.run(CommandId::Replace);
    f.m().find.sides = FindSides::Left;
    for (q, with) in [("foo", "X"), ("bar", "Y")] {
        f.m().find.query = query(q);
        f.m().find.replace = with.into();
        act(&mut f, FindAction::ReplaceAll);
        f.answer(ButtonRole::Yes);
    }
    assert_eq!(f.text(Side::Left), "X Y\n");
    f.m().set_focus(Side::Left);
    f.run(CommandId::Undo);
    assert_eq!(
        f.text(Side::Left),
        "X bar\n",
        "only the second Replace All is undone"
    );
}

/// C3: a hit far down is brought into view (the cursor row is the hit's).
#[test]
fn w11b_review_a_hit_is_revealed() {
    let mut l = String::new();
    for k in 0..400 {
        l.push_str(&format!("line {k}\n"));
    }
    l.push_str("needle\n");
    let mut f = Fixture::new(&l, &l);
    f.run(CommandId::Find);
    f.m().find.query = query("needle");
    f.m().find.sides = FindSides::Left;
    f.place(Side::Left, 0, 0);
    act(&mut f, FindAction::Next);
    let row = f.m().row_of_line(Side::Left, 400).expect("row");
    assert_eq!(f.m().cursor().map(|r| r.0), Some(row));
    let scroll = f.m().meta().scroll;
    assert!(scroll >= 390, "{scroll}");
}

/// C5: a selection over lines never becomes the query (the word at the caret does not either).
#[test]
fn w11b_review_a_multi_line_selection_is_no_query() {
    let mut f = Fixture::new(L, R);
    let mut t = *f.m().tweaks();
    t.init_find_with_current_word = true;
    f.m().set_tweaks(t);
    f.m().find.query = query("kept");
    f.m().set_focus(Side::Left);
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(0, 0), TextPos::new(1, 3));
    f.run(CommandId::Find);
    assert_eq!(f.m().find.query.text, "kept");
    // One line: the selection is.
    f.m()
        .pane_mut(Side::Left)
        .select(TextPos::new(0, 4), TextPos::new(0, 7));
    f.run(CommandId::Find);
    assert_eq!(f.m().find.query.text, "bar");
}

/// C6, C7: the panel remembers the replace text (`mru.replace`, with the Replace panel shown) and its sides, and reads
/// them back (`left` is Left).
#[test]
fn w11b_review_the_remembered_texts_and_sides() {
    let mut f = Fixture::new(L, R);
    f.run(CommandId::Replace);
    f.m().find.query = query("foo");
    f.m().find.replace = "with this".into();
    for sides in [FindSides::Left, FindSides::Right, FindSides::Both] {
        f.m().find.sides = sides;
        let mut state = hikasa_session::settings::State::default();
        find::remember(f.m(), &mut state, 10);
        assert_eq!(
            state.mru(hikasa_session::settings::state::MruList::Replace),
            ["with this".to_owned()]
        );
        let mut back = find::FindPanel::default();
        back.sides = FindSides::Both;
        if sides == FindSides::Both {
            back.sides = FindSides::Left;
        }
        back.read_state(&state);
        assert_eq!(back.sides, sides);
        assert_eq!(back.recent_replace, ["with this".to_owned()]);
    }
}

/// C8: Select All whose document changed while it ran is computed again on the document of now.
#[test]
fn w11b_review_select_all_of_a_changed_document_runs_again() {
    let mut f = Fixture::new("a\nfoo\nb\n", "x\n");
    f.run(CommandId::Find);
    f.m().find.query = query("foo");
    f.m().set_focus(Side::Left);
    let service = f.world.service.clone();
    assert!(find::select_all(f.m(), &service));
    let req = hikasa_text::edit::EditRequest::Replace {
        range: TextPos::ZERO..TextPos::ZERO,
        text: "foo\n".into(),
        keep_eols: false,
    };
    assert!(crate::text::edit::edit(
        f.m(),
        Side::Left,
        req,
        crate::text::edit::Origin::Single,
        None
    ));
    assert_eq!(f.text(Side::Left), "foo\na\nfoo\nb\n");
    f.settle();
    let lines = f.m().pane(Side::Left).lines.clone().expect("a line set");
    assert_eq!(lines, LineSet::from_ranges(vec![0..1, 2..3]));
}

/// Finding 4: Browse By ▸ Find never searches on the UI thread: the arrows are available while a query is typed, a
/// step is the panel's job (with `[next_difference] wrap_around`), *nothing found* comes from the job.
#[test]
fn w11b_review_find_browse_mode_steps_by_job() {
    let mut f = Fixture::new(L, R);
    f.m().display_mut().browse_mode = hikasa_text::nav::BrowseMode::Find;
    assert_eq!(f.m().arrows(), [false, false], "no query");
    f.m().find.query = query("zzz-none");
    f.place(Side::Left, 1, 0);
    assert_eq!(
        f.m().arrows(),
        [true, true],
        "a query: no search for the arrows"
    );
    let started = f.m().find.started;
    f.run(CommandId::DiffItemNext);
    assert_eq!(
        f.m().find.started,
        started,
        "the step runs at the next poll"
    );
    f.settle();
    assert_eq!(f.m().find.started, started + 1, "one search job");
    assert_eq!(f.m().find.note.as_deref(), Some(NOT_FOUND));
    f.m().find.query = query("foo");
    f.place(Side::Left, 0, 0);
    f.run(CommandId::DiffItemNext);
    f.settle();
    assert_eq!(
        hit(&mut f),
        (Side::Left, TextPos::new(0, 0), TextPos::new(0, 3))
    );
    f.run(CommandId::DiffItemPrev);
    f.settle();
    // Backward from the first hit without the tweak's wrap: nothing.
    assert_eq!(
        hit(&mut f),
        (Side::Left, TextPos::new(0, 0), TextPos::new(0, 3))
    );
    let mut t = *f.m().tweaks();
    t.wrap_around = true;
    f.m().set_tweaks(t);
    f.run(CommandId::DiffItemPrev);
    f.settle();
    assert_eq!(f.m().find.note.as_deref(), Some(PASSED_END));
    assert_eq!(hit(&mut f).0, Side::Right, "the last hit, after wrapping");
    // The panel stays open with *Close automatically* (a browse step is no panel search).
    f.run(CommandId::Find);
    f.m().find.close_automatically = true;
    f.run(CommandId::DiffItemNext);
    f.settle();
    assert_ne!(f.m().display().find_panel, FindShown::None);
}
