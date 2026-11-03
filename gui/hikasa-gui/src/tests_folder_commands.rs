//! M2 W9a acceptance A9.1, A9.2, A9.4 and A9.10 through the view model on disk trees: the Edit commands equal the
//! CLI's `--select` words on the same tree, gap navigation, Ignore Folder Structure, the sort's header cycle (a plan's
//! selection unchanged by any sort), the status counts once per change, and the columns written to and read from the
//! layout.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_folder::NodeId;
use hikasa_folder::select::{SelKind, SelSide, SelState, SelectMask, Selection, SideSet};
use hikasa_session::api::{ExpandWords, SelectRequest};
use hikasa_session::ipc::ViewId;

use crate::app::AppCore;
use crate::commands::CommandId;
use crate::folder::columns::ColumnId;
use crate::folder::selection::{Hit, Mods};
use crate::tests::{World, folder_model};
use crate::tests_folder_ops::{open, put};

/// A tree with newer files on both sides, orphans of both kinds, a pair the same, and a folder holding more.
fn tree(world: &World) {
    put(world, "l", "same.txt", "s\n", 1_000);
    put(world, "r", "same.txt", "s\n", 1_000);
    put(world, "l", "lnew.txt", "new left\n", 3_000);
    put(world, "r", "lnew.txt", "old\n", 1_000);
    put(world, "l", "rnew.txt", "old\n", 1_000);
    put(world, "r", "rnew.txt", "new right\n", 3_000);
    put(world, "l", "lonly.txt", "l\n", 1_000);
    put(world, "r", "ronly.txt", "r\n", 1_000);
    put(world, "l", "d/inner.txt", "a newer one\n", 4_000);
    put(world, "r", "d/inner.txt", "b\n", 1_000);
    put(world, "l", "d/deep.txt", "x\n", 1_000);
}

fn opened(world: &World) -> (crate::tests::TestApp, ViewId) {
    tree(world);
    let mut app = world.app(None);
    let view = open(world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    (app, view)
}

/// The selection the Service's `--select <word>` gives on the view's model, display and open folders.
fn cli_selection(world: &World, app: &mut AppCore, view: ViewId, word: &str) -> Selection {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let display = *m.display_options();
    world
        .service
        .folder_selection(
            &model,
            &SelectRequest {
                masks: vec![word.to_owned()],
                display: Some(display),
                expand: ExpandWords::All,
                iu: display.ignore_unimportant,
                ..SelectRequest::default()
            },
        )
        .expect("selection")
}

#[test]
fn a9_4_each_edit_command_equals_the_cli_selection() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    let word = |side, state, kind| SelectMask::Result { side, state, kind }.to_string();
    let cases = [
        (
            CommandId::SelectNewerLeft,
            word(SelSide::Left, SelState::Newer, SelKind::Files),
        ),
        (
            CommandId::SelectNewerRight,
            word(SelSide::Right, SelState::Newer, SelKind::Files),
        ),
        (
            CommandId::SelectNewerBoth,
            word(SelSide::All, SelState::Newer, SelKind::Files),
        ),
        (
            CommandId::SelectOrphanLeft,
            word(SelSide::Left, SelState::Orphan, SelKind::All),
        ),
        (
            CommandId::SelectOrphanRight,
            word(SelSide::Right, SelState::Orphan, SelKind::All),
        ),
        (
            CommandId::SelectOrphanBoth,
            word(SelSide::All, SelState::Orphan, SelKind::All),
        ),
        (
            CommandId::SelectAllFiles,
            word(SelSide::All, SelState::All, SelKind::Files),
        ),
    ];
    for (id, word) in cases {
        app.execute(id);
        let ours = folder_model(&mut app, view).selection().items.clone();
        assert!(!ours.is_empty(), "{id:?} selects something");
        assert_eq!(
            ours,
            cli_selection(&world, &mut app, view, &word),
            "{id:?} = {word}"
        );
    }
    // Invert: every shown row's sides not selected; twice is the identity.
    app.execute(CommandId::SelectNewerLeft);
    let before = folder_model(&mut app, view).selection().items.clone();
    app.execute(CommandId::SelectInvert);
    let inverted = folder_model(&mut app, view).selection().items.clone();
    for (node, sides) in before.iter() {
        assert!(!inverted.get(node).intersects(sides));
    }
    app.execute(CommandId::SelectInvert);
    assert_eq!(folder_model(&mut app, view).selection().items, before);
    // The plan's request is explicit and independent of the sort (D-235: plans read the model in tree order).
    let m = folder_model(&mut app, view);
    let unsorted = crate::folder_view::select_request(before.clone(), *m.display_options());
    m.header_click(ColumnId::Size, Side::Left);
    m.header_click(ColumnId::Size, Side::Left);
    let sorted =
        crate::folder_view::select_request(m.selection().items.clone(), *m.display_options());
    assert_eq!(unsorted, sorted);
    assert_eq!(unsorted.expand, ExpandWords::None);
}

#[test]
fn a9_2_the_header_cycle_and_a_sort_kept_across_a_rebuild() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    let names = |app: &mut AppCore| -> Vec<String> {
        let m = folder_model(app, view);
        let model = Arc::clone(m.prepared().expect("prepared").model());
        let read = model.read();
        m.rows()
            .iter()
            .map(|r| read.path(r.node).as_str().to_owned())
            .collect()
    };
    let tree_order = names(&mut app);
    let m = folder_model(&mut app, view);
    m.header_click(ColumnId::Modified, Side::Left);
    assert!(m.columns().sort.is_some_and(|s| !s.descending));
    let up = names(&mut app);
    folder_model(&mut app, view).header_click(ColumnId::Modified, Side::Left);
    let down = names(&mut app);
    assert_ne!(up, down);
    assert_eq!(up.len(), tree_order.len());
    // The folder stays first and its children under it.
    assert_eq!(down[0], "d");
    // A display change rebuilds with the sort kept.
    app.execute(CommandId::FavDiff);
    app.execute(CommandId::FavCompareAll);
    assert_eq!(names(&mut app), down);
    folder_model(&mut app, view).header_click(ColumnId::Modified, Side::Left);
    assert_eq!(folder_model(&mut app, view).columns().sort, None);
    assert_eq!(names(&mut app), tree_order);
}

#[test]
fn a9_7_exists_next_and_prev_step_over_the_gaps() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    let path_of = |app: &mut AppCore| -> Option<String> {
        let m = folder_model(app, view);
        let id = m.selected()?;
        let model = Arc::clone(m.prepared()?.model());
        let path = model.read().path(id).as_str().to_owned();
        Some(path)
    };
    // Focus the right side: ExistsNext visits only rows that have a right item.
    let m = folder_model(&mut app, view);
    m.click(0, Hit::Side(Side::Right), Mods::default());
    let mut visited = Vec::new();
    for _ in 0..12 {
        app.execute(CommandId::ExistsNext);
        visited.push(path_of(&mut app).expect("current"));
    }
    assert!(
        !visited
            .iter()
            .any(|p| p == "lonly.txt" || p == "d/deep.txt"),
        "{visited:?}"
    );
    assert!(visited.iter().any(|p| p == "ronly.txt"));
    let last = visited.last().cloned();
    app.execute(CommandId::ExistsPrev);
    assert_ne!(path_of(&mut app), last);
}

#[test]
fn a9_1_ignore_folder_structure_and_the_path_column() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    assert!(!folder_model(&mut app, view).flat());
    assert!(
        !folder_model(&mut app, view)
            .shown_columns()
            .contains(&ColumnId::Path)
    );
    assert_eq!(app.checked(CommandId::DirFlatten), Some(false));
    app.execute(CommandId::DirFlatten);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(m.flat(), "the comparison ran again flat");
    assert!(m.shown_columns().contains(&ColumnId::Path));
    assert_eq!(app.checked(CommandId::DirFlatten), Some(true));
    app.execute(CommandId::DirFlatten);
    world.settle(&mut app);
    assert!(!folder_model(&mut app, view).flat());
}

#[test]
fn a9_4_status_counts_per_side_once_per_change() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    app.execute(CommandId::SelectOrphanBoth);
    let m = folder_model(&mut app, view);
    let before = m.counts_computed();
    let counts = m.selection_counts();
    // lonly.txt (2 bytes) and d/deep.txt (2 bytes) on the left, ronly.txt (2 bytes) on the right.
    assert_eq!(counts, [(2, 4), (1, 2)]);
    for _ in 0..10 {
        let _ = m.selection_counts();
        let _ = m.status_items();
    }
    assert_eq!(
        m.counts_computed(),
        before + 1,
        "computed once for the change"
    );
    let items = m.status_items();
    assert!(
        items
            .right
            .iter()
            .any(|t| t.starts_with("Selected: L 2 (4) · R 1 (2)")),
        "{:?}",
        items.right
    );
    m.with_selection(|sel, _, _| sel.clear());
    let _ = m.selection_counts();
    assert_eq!(m.counts_computed(), before + 2);
}

#[test]
fn a9_1_the_columns_go_to_the_layout_and_come_back() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    let m = folder_model(&mut app, view);
    m.change_columns(|c| {
        c.toggle(ColumnId::Crc);
        c.resize(ColumnId::Crc, 90.0);
    });
    m.header_click(ColumnId::Name, Side::Right);
    let keys = m.layout_keys();
    let mut state = hikasa_session::settings::State::default();
    for (key, value) in keys {
        state.set_layout_value("folder-compare", key, value);
    }
    let columns = m.columns().clone();
    m.change_columns(|c| *c = crate::folder::columns::ColumnLayout::default());
    m.apply_layout(&state);
    assert_eq!(m.columns(), &columns);
    let _ = NodeId::ROOT;
    let _ = SideSet::all();
}
