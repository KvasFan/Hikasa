//! A11.16 (D-309): navigation of a text view. Each row of §1.11.18's table equals core's `nav::step` from the same place;
//! the toolbar arrows per browse mode; the bookmarks 0–9 (the other pane's when the focus pane lacks one); Ctrl+0 is
//! bookmark 0 in a text view while *Reset Display Font Size* keeps its menu entry; Go To's errors and clamping; the pane
//! moves keep the row; *nothing found* is the message panel or a beep; the first difference on load.

use hikasa_core::{Pair, Side};
use hikasa_session::settings::Shortcut;
use hikasa_text::Direction;
use hikasa_text::edit::TextPos;
use hikasa_text::find::FindSides;
use hikasa_text::nav::{BrowseMode, NavContext, NavPos, step};

use crate::app::Effect;
use crate::commands::{CommandId, FocusArea, Scope};
use crate::tests::World;
use crate::tests_text::Fixture;
use crate::window::{CommandArg, Invocation};

/// Two sides with sections at lines 3, 8 (an inline difference of two words) and 14 (an orphan on the right).
fn pair() -> (String, String) {
    let mut l = String::new();
    let mut r = String::new();
    for i in 0..20 {
        let line = format!("line {i} alpha beta\n");
        l.push_str(&line);
        match i {
            3 => r.push_str("line 3 ALPHA beta\n"),
            8 => r.push_str("line 8 alpha BETA gamma\n"),
            14 => {
                r.push_str(&line);
                r.push_str("an added line\n");
            }
            _ => r.push_str(&line),
        }
    }
    (l, r)
}

/// Core's step from the focus pane's place (selection end forward, start backward).
fn core_step(
    f: &mut Fixture,
    mode: BrowseMode,
    dir: Direction,
) -> Option<(Side, TextPos, TextPos)> {
    let m = f.m();
    let side = m.focus();
    let r = m.pane(side).selection().range();
    let pos = if dir == Direction::Forward {
        r.end
    } else {
        r.start
    };
    let ctx = NavContext {
        cmp: m.comparison().map(|c| &**c),
        docs: Pair::new(m.doc(Side::Left), m.doc(Side::Right)),
        view: m.filtered(),
        ignore_unimportant: m.ignore_unimportant(),
        finder: m.find.compiled().map(|f| &**f),
        sides: FindSides::Both,
    };
    let from = NavPos {
        row: m.row_of_line(side, pos.line.0).map(hikasa_text::RowIdx),
        side,
        pos,
    };
    step(mode, &ctx, from, dir, false).map(|t| (t.side, t.range.start, t.range.end))
}

/// The focus pane's selection as (side, start, end).
fn selected(f: &mut Fixture) -> (Side, TextPos, TextPos) {
    let m = f.m();
    let side = m.focus();
    let r = m.pane(side).selection().range();
    (side, r.start, r.end)
}

#[test]
fn a11_16_each_step_equals_core_from_the_same_place() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    // Edits of the left pane for the Edit mode.
    f.place(Side::Left, 5, 0);
    f.type_text("x");
    f.place(Side::Left, 12, 0);
    f.type_text("y");
    f.settle();
    for (id, mode, dir) in [
        (
            CommandId::DiffSectionNext,
            BrowseMode::DiffSection,
            Direction::Forward,
        ),
        (
            CommandId::DiffSectionPrev,
            BrowseMode::DiffSection,
            Direction::Backward,
        ),
        (
            CommandId::DiffNext,
            BrowseMode::Difference,
            Direction::Forward,
        ),
        (
            CommandId::DiffPrev,
            BrowseMode::Difference,
            Direction::Backward,
        ),
        (CommandId::EditNext, BrowseMode::Edit, Direction::Forward),
        (CommandId::EditPrev, BrowseMode::Edit, Direction::Backward),
    ] {
        for start in [0u32, 4, 9, 13, 19] {
            f.place(Side::Left, start, 0);
            let expected = core_step(&mut f, mode, dir);
            f.run(id);
            match expected {
                Some(t) => {
                    assert_eq!(
                        {
                            let s = selected(&mut f);
                            (s.0, s.1.min(s.2), s.1.max(s.2))
                        },
                        (t.0, t.1.min(t.2), t.1.max(t.2)),
                        "{id:?} from line {start}"
                    );
                    // Line targets: the caret at their start; an inline difference: at its end.
                    let caret = f.caret(t.0);
                    let at_end = mode == BrowseMode::Difference;
                    assert_eq!(
                        caret,
                        if at_end { t.2 } else { t.1 },
                        "{id:?} from line {start}: the caret"
                    );
                }
                None => assert_eq!(f.caret(Side::Left), TextPos::new(start, 0), "{id:?} stays"),
            }
        }
    }
}

#[test]
fn a11_16_the_arrows_follow_the_browse_mode() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    let win = f.app.win();
    for mode in [
        BrowseMode::DiffSection,
        BrowseMode::Difference,
        BrowseMode::Edit,
    ] {
        f.app.0.execute(
            win,
            Invocation::with(CommandId::BrowseMode, CommandArg::BrowseMode(mode)),
        );
        assert_eq!(f.m().display().browse_mode, mode);
        f.place(Side::Left, 0, 0);
        let expected = core_step(&mut f, mode, Direction::Forward);
        assert_eq!(
            f.app.enabled(CommandId::DiffItemNext),
            expected.is_some(),
            "{mode:?}"
        );
        if let Some(e) = expected {
            f.run(CommandId::DiffItemNext);
            let s = selected(&mut f);
            assert_eq!(
                (s.1.min(s.2), s.1.max(s.2)),
                (e.1.min(e.2), e.1.max(e.2)),
                "{mode:?}"
            );
        }
    }
    // No edit yet: Edit has nothing either way, the arrows are off.
    assert!(!f.app.enabled(CommandId::DiffItemNext));
    assert!(!f.app.enabled(CommandId::DiffItemPrev));
}

#[test]
fn a11_16_bookmarks_toggle_and_go() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    f.place(Side::Left, 6, 2);
    f.run(CommandId::BookmarkToggle3);
    assert_eq!(f.doc(Side::Left).bookmark(3).map(|l| l.0), Some(6));
    // From the right pane, which has no bookmark 3: the left pane's, which takes the focus.
    f.place(Side::Right, 0, 0);
    f.run(CommandId::BookmarkGoTo3);
    assert_eq!(f.m().focus(), Side::Left);
    assert_eq!(f.caret(Side::Left), TextPos::new(6, 0));
    // The list's item with its argument does the same.
    let win = f.app.win();
    f.place(Side::Left, 10, 0);
    f.app.0.execute(
        win,
        Invocation::with(CommandId::BookmarkToggleList, CommandArg::Bookmark(0)),
    );
    f.place(Side::Left, 0, 0);
    f.app.0.execute(
        win,
        Invocation::with(CommandId::BookmarkGoToList, CommandArg::Bookmark(0)),
    );
    assert_eq!(f.caret(Side::Left), TextPos::new(10, 0));
    // Toggled again: gone; not set anywhere: the message panel.
    f.run(CommandId::BookmarkToggle0);
    assert_eq!(f.doc(Side::Left).bookmark(0), None);
    f.run(CommandId::BookmarkGoTo0);
    f.settle();
    assert_eq!(f.message().as_deref(), Some("Bookmark 0 is not set."));
    f.run(CommandId::BookmarkClear);
    assert_eq!(f.doc(Side::Left).bookmark(3), None);
}

#[test]
fn a11_16_ctrl_0_is_bookmark_0_in_a_text_view() {
    let (l, r) = pair();
    let f = Fixture::new(&l, &r);
    let win = f.app.win();
    let ctx = f.app.0.ctx(win);
    let tables = f.app.0.shared_tables();
    let key = Shortcut::parse("Ctrl+0").expect("shortcut");
    assert_eq!(
        tables.dispatch(Scope::Text, &ctx, &key, FocusArea::ViewMain),
        Some(CommandId::BookmarkGoTo0)
    );
    // Reset Display Font Size keeps its menu entry (with its own numeric-keypad shortcut).
    assert!(crate::commands::shown(
        &tables,
        Scope::Text,
        CommandId::DisplayFontReset
    ));
    assert_ne!(
        tables.shortcut(Scope::Text, CommandId::DisplayFontReset),
        Some(&key)
    );
}

#[test]
fn a11_16_go_to_parses_and_clamps() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    f.run(CommandId::GoTo);
    let win = f.app.win();
    let service = f.world.service.clone();
    let answer = |f: &mut Fixture, text: &str| {
        let w = f.app.window_mut(win).expect("window");
        let d = w
            .dialogs
            .find_mut::<crate::text::nav::GoToLineDialog>()
            .expect("Go To");
        assert_eq!(d.range_text(), "Line[:column] (1–20)");
        d.text = text.to_owned();
        crate::dialog::DialogModel::on_button(d, crate::dialog::ButtonRole::Ok, &service)
    };
    assert_eq!(answer(&mut f, "x"), crate::dialog::DialogStep::Stay);
    let error = f
        .app
        .window_mut(win)
        .and_then(|w| w.dialogs.find_mut::<crate::text::nav::GoToLineDialog>())
        .and_then(|d| d.error.clone());
    assert_eq!(error, hikasa_text::nav::parse_goto("x").err());
    assert_eq!(answer(&mut f, "0"), crate::dialog::DialogStep::Stay);
    assert_eq!(answer(&mut f, "12:6"), crate::dialog::DialogStep::Close);
    let closed = f
        .app
        .window_mut(win)
        .and_then(|w| w.dialogs.pop())
        .expect("dialog");
    f.app.0.dialog_closed_for_test(closed);
    let focus = f.m().focus();
    assert_eq!(f.caret(focus), TextPos::new(11, 5));
    // Past the last line: the last; past the line's end: its end.
    let side = f.m().focus();
    crate::text::nav::go_to_line(f.m(), 99, Some(999));
    let end = f.doc(side).line_len(hikasa_text::LineIdx(19));
    assert_eq!(f.caret(side), TextPos::new(19, end));
}

#[test]
fn a11_16_pane_moves_keep_the_row() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    // Line 16 of the left is row 17 (the right's added line); the right's line there is 17.
    f.place(Side::Left, 16, 3);
    let row = f.m().cursor().expect("cursor");
    f.run(CommandId::GoRight);
    assert_eq!(f.m().focus(), Side::Right);
    let line = f.caret(Side::Right).line.0;
    assert_eq!(f.m().row_of_line(Side::Right, line), Some(row.0));
    f.run(CommandId::GoLeft);
    assert_eq!(f.m().focus(), Side::Left);
    f.run(CommandId::GoNext);
    assert_eq!(f.m().focus(), Side::Right);
    f.run(CommandId::GoPrev);
    assert_eq!(f.m().focus(), Side::Left);
    // On a gap of the left (the right's added line), the left's next line.
    f.place(Side::Right, 15, 0);
    f.run(CommandId::GoLeft);
    assert_eq!(f.caret(Side::Left).line.0, 15);
    // Next/Previous Existing Line of the focus side skip the gap rows.
    f.place(Side::Left, 14, 0);
    f.run(CommandId::ExistsNext);
    assert_eq!(f.caret(Side::Left).line.0, 15);
    f.run(CommandId::ExistsPrev);
    assert_eq!(f.caret(Side::Left).line.0, 14);
}

#[test]
fn a11_16_nothing_found_is_the_message_or_a_beep() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    // Next Edit without an edit (the section steps are greyed when there is none: their predicate asks first).
    f.place(Side::Left, 19, 0);
    f.run(CommandId::EditNext);
    f.settle();
    assert_eq!(f.message().as_deref(), Some(crate::text::nav::NO_MORE));
    // Without the message panel: the window beeps.
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[next_difference]\nshow_message_panel = false\n",
    )]);
    let mut f = Fixture::in_world(world, &l, &r);
    f.place(Side::Left, 19, 0);
    f.run(CommandId::EditNext);
    let effects = f.app.0.handle(crate::app::UiMsg::Tick);
    let win = f.app.win();
    assert!(effects.contains(&Effect::Beep(win)), "{effects:?}");
}

#[test]
fn a11_16_the_first_difference_on_load() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    // On by default (OPT-007): the first section's row, the caret on its line.
    assert_eq!(f.m().cursor().map(|c| c.0), Some(3));
    assert_eq!(f.caret(Side::Left).line.0, 3);
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[next_difference]\ngo_to_first_difference_on_load = false\n",
    )]);
    let mut f = Fixture::in_world(world, &l, &r);
    assert_eq!(f.caret(Side::Left), TextPos::ZERO);
}

/// W11b review: a navigation command given while the pane is frozen (a relive runs) is queued and runs at its turn
/// (it was dropped at the replay).
#[test]
fn w11b_review_a_queued_navigation_command_runs_at_its_turn() {
    let (l, r) = pair();
    let mut f = Fixture::new(&l, &r);
    f.place(Side::Left, 0, 0);
    let expected = core_step(&mut f, BrowseMode::Difference, Direction::Forward).expect("a target");
    let service = f.world.service.clone();
    let request = f.m().request().clone();
    assert!(crate::text::relive::start(
        f.m(),
        &service,
        request,
        crate::text::relive::Cause::Rules
    ));
    assert!(f.m().frozen(Side::Left));
    f.run(CommandId::DiffNext);
    assert_eq!(f.caret(Side::Left), TextPos::new(0, 0), "queued");
    f.settle();
    assert_eq!(selected(&mut f), expected);
}
