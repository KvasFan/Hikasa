//! M2 W9a acceptance A9.7: Find Filename through the view model on a disk tree — each query option, a hit in a
//! collapsed folder opens its ancestors and is selected on the side that matched, not found → the message panel, a bad
//! regular expression shown in the panel, F3/Shift+F3 with the panel closed, `mru.find` and the remembered options.
//! (`ExistsNext`/`ExistsPrev` on both focus sides: `tests_folder_commands::a9_7_exists_next_and_prev_step_over_the_gaps`.)

use hikasa_folder::select::SideSet;
use hikasa_session::ipc::ViewId;
use hikasa_session::settings::state::MruList;

use crate::commands::CommandId;
use crate::find_panel::{FIND_DIALOG, FindOutcome, FindPanel, NOT_FOUND};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{open, put};

/// `a/Report.txt` both sides; `a/b/report_old.txt` left only; `notes.md`, `REPORT` right only; `x/y/z/deep-report.txt`
/// left only, three folders down.
fn opened(world: &World) -> (TestApp, ViewId) {
    put(world, "l", "a/Report.txt", "1\n", 1_000);
    put(world, "r", "a/Report.txt", "2\n", 1_000);
    put(world, "l", "a/b/report_old.txt", "o\n", 1_000);
    put(world, "l", "notes.md", "n\n", 1_000);
    put(world, "r", "notes.md", "n\n", 1_000);
    put(world, "r", "REPORT", "R\n", 1_000);
    put(world, "l", "x/y/z/deep-report.txt", "d\n", 1_000);
    let mut app = world.app(None);
    let view = open(world, &mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    (app, view)
}

/// Sets the query and finds (`reverse`: Find Previous) through the window; returns the selected row's name and
/// sides.
fn find(
    app: &mut TestApp,
    view: ViewId,
    set: impl FnOnce(&mut FindPanel),
    reverse: bool,
) -> Option<(String, SideSet)> {
    set(folder_model(app, view).find_panel());
    let id = if reverse {
        CommandId::FindFilenamePrev
    } else {
        CommandId::FindFilenameNext
    };
    app.execute(id);
    selected(app, view)
}

fn selected(app: &mut TestApp, view: ViewId) -> Option<(String, SideSet)> {
    let m = folder_model(app, view);
    let current = m.selection().current?;
    let sides = m.selection().items.get(current);
    let model = m.prepared()?.model().clone();
    let read = model.read();
    let path = read.path(current).as_str().to_owned();
    Some((path, sides))
}

fn message(app: &TestApp) -> Option<String> {
    app.tabs()[app.active()]
        .message
        .as_ref()
        .map(|m| m.text.clone())
}

#[test]
fn a9_7_a_hit_in_a_closed_folder_opens_its_ancestors_on_the_matching_side() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    let m = folder_model(&mut app, view);
    assert!(m.rows().len() < 8, "folders start closed");
    folder_model(&mut app, view).find_panel().text = "deep".into();
    let effects = app.execute(CommandId::FindFilenameNext);
    let hit = selected(&mut app, view);
    assert_eq!(hit, Some(("x/y/z/deep-report.txt".into(), SideSet::LEFT)));
    let m = folder_model(&mut app, view);
    let index = m
        .rows()
        .iter()
        .position(|r| Some(r.node) == m.selection().current)
        .expect("the row is shown");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, crate::app::Effect::FolderScroll(v, row) if *v == view && *row as usize == index)),
        "scrolled into view"
    );
}

#[test]
fn a9_7_each_query_option() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    // Case folded, both sides, wrap, down: in the tree's order, then around.
    let mut seen = Vec::new();
    for _ in 0..5 {
        seen.push(find(&mut app, view, |p| p.text = "report".into(), false).expect("a hit"));
    }
    let names: Vec<&str> = seen.iter().map(|(p, _)| p.as_str()).collect();
    // Folders first in the tree's order (note 2), `notes.md` before `REPORT` (folded).
    assert_eq!(
        names[..4],
        [
            "a/b/report_old.txt",
            "a/Report.txt",
            "x/y/z/deep-report.txt",
            "REPORT"
        ],
        "{names:?}"
    );
    assert_eq!(names[4], names[0], "wrapped around");
    assert_eq!(seen[1].1, SideSet::all(), "a pair matches on both sides");
    assert_eq!(seen[2].1, SideSet::LEFT);
    assert_eq!(seen[3].1, SideSet::RIGHT);
    // Search up: the previous one, around the top; Find Previous reverses the direction again.
    let up = find(&mut app, view, |p| p.up = true, false).expect("a hit");
    assert_eq!(up.0, "REPORT");
    let down = find(&mut app, view, |_| {}, true).expect("a hit");
    assert_eq!(down.0, "a/b/report_old.txt");
    // Match case.
    let hit = find(
        &mut app,
        view,
        |p| {
            p.up = false;
            p.case = true;
            p.text = "Report".into();
        },
        false,
    );
    assert_eq!(hit.map(|h| h.0).as_deref(), Some("a/Report.txt"));
    // Whole word: `report_old` and `deep-report` — `_` is a word character, `-` is not.
    let mut words = Vec::new();
    for _ in 0..3 {
        let hit = find(
            &mut app,
            view,
            |p| {
                p.case = false;
                p.whole_word = true;
                p.text = "report".into();
            },
            false,
        );
        words.push(hit.expect("a hit").0);
    }
    assert!(
        !words.contains(&"a/b/report_old.txt".to_owned()),
        "{words:?}"
    );
    assert!(
        words.contains(&"x/y/z/deep-report.txt".to_owned()),
        "{words:?}"
    );
    // Regular expression on the left only: REPORT (right only) is never found.
    let mut regex = Vec::new();
    for _ in 0..4 {
        let hit = find(
            &mut app,
            view,
            |p| {
                p.whole_word = false;
                p.regex = true;
                p.text = "^rep.*t$".into();
                p.sides = SideSet::LEFT;
            },
            false,
        );
        regex.push(hit.expect("a hit"));
    }
    assert!(
        regex
            .iter()
            .all(|(p, s)| p != "REPORT" && *s == SideSet::LEFT),
        "{regex:?}"
    );
    // No wrap: past the last hit nothing is found and the selection stays.
    let last = find(
        &mut app,
        view,
        |p| {
            p.regex = false;
            p.sides = SideSet::all();
            p.wrap = false;
            p.text = "deep".into();
        },
        false,
    );
    assert_eq!(
        last.as_ref().map(|h| h.0.as_str()),
        Some("x/y/z/deep-report.txt")
    );
    assert_eq!(find(&mut app, view, |_| {}, false), last);
    assert_eq!(message(&app).as_deref(), Some(NOT_FOUND));
}

#[test]
fn a9_7_not_found_bad_regex_f3_without_the_panel_and_memory() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    // F3 with nothing to look for opens the panel.
    assert!(!folder_model(&mut app, view).show_find());
    app.execute(CommandId::FindFilenameNext);
    assert!(folder_model(&mut app, view).show_find());
    folder_model(&mut app, view).close_find();
    // Not found: the message panel.
    let none = find(&mut app, view, |p| p.text = "absent".into(), false);
    assert_eq!(none, None);
    assert_eq!(message(&app).as_deref(), Some(NOT_FOUND));
    let m = folder_model(&mut app, view);
    assert_eq!(m.find(false), FindOutcome::NotFound);
    // A bad regular expression: the message in the panel, nothing selected.
    let m = folder_model(&mut app, view);
    m.find_panel().regex = true;
    m.find_panel().text = "(".into();
    assert!(matches!(m.find(false), FindOutcome::Invalid(_)));
    assert!(m.find_panel().error.is_some());
    // F3 and Shift+F3 with the panel closed use the last query.
    let m = folder_model(&mut app, view);
    m.find_panel().regex = false;
    m.find_panel().text = "notes".into();
    assert!(!m.show_find());
    app.execute(CommandId::FindFilenameNext);
    assert_eq!(
        selected(&mut app, view).map(|h| h.0).as_deref(),
        Some("notes.md")
    );
    assert!(folder_model(&mut app, view).find_panel().error.is_none());
    app.execute(CommandId::FindFilenamePrev);
    assert_eq!(
        selected(&mut app, view).map(|h| h.0).as_deref(),
        Some("notes.md")
    );
    // Remembered: the texts newest first in mru.find, the options in [dialogs.find-filename].
    let state = app.0.state();
    assert_eq!(
        state.mru(MruList::Find),
        ["notes".to_owned(), "absent".to_owned()]
    );
    assert_eq!(state.dialog_text(FIND_DIALOG, "regex"), Some("false"));
    assert_eq!(state.dialog_text(FIND_DIALOG, "sides"), Some("both"));
    let mut state = state.clone();
    state.set_dialog_text(FIND_DIALOG, "match_case", "true");
    state.set_dialog_text(FIND_DIALOG, "sides", "right");
    let panel = FindPanel::from_state(&state);
    assert!(panel.case && panel.wrap && !panel.up);
    assert_eq!(panel.sides, SideSet::RIGHT);
    assert_eq!(panel.text, "notes");
    // The panel's visibility is a layout key.
    app.execute(CommandId::FindFilename);
    let keys = folder_model(&mut app, view).layout_keys();
    assert!(keys.contains(&(
        "find_panel",
        hikasa_session::settings::state::LayoutValue::Bool(true)
    )));
}
