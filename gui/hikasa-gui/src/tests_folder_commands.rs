//! M2 W9a acceptance A9.1, A9.2, A9.4 and A9.10 (the `FOLDER_COMMANDS` parity table) through the view model on disk trees: the Edit commands equal the
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
    // Select All (the bench's; the command is W9c's): every shown row, every present side.
    folder_model(&mut app, view).select_all();
    let ours = folder_model(&mut app, view).selection().items.clone();
    assert_eq!(
        ours,
        cli_selection(
            &world,
            &mut app,
            view,
            &word(SelSide::All, SelState::All, SelKind::All)
        )
    );
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

#[test]
fn a9_1_col_list_and_the_header_menu_toggle_the_columns() {
    use crate::menus::{Menu, MenuContext, MenuTitle, header_menu, render_text};
    use crate::window::{CommandArg, Invocation};
    let world = World::new();
    let (mut app, view) = opened(&world);
    let win = app.win();
    let menu = |app: &crate::tests::TestApp| {
        let checked = |_: CommandId| None;
        let columns = app.0.column_menu(win);
        let mc = MenuContext {
            tables: app.tables(),
            scope: app.scope(),
            ctx: app.ctx(),
            checked: &checked,
            workspaces: &[],
            columns: &columns,
            open_with: &[],
            formats: &[],
        };
        render_text(&[Menu {
            title: MenuTitle::View,
            items: header_menu(&mc),
        }])
    };
    // Every column but Name; Path only while flat.
    let offered = app.0.column_menu(win);
    assert!(
        offered
            .iter()
            .all(|(c, _)| *c != ColumnId::Name && *c != ColumnId::Path)
    );
    assert_eq!(
        offered
            .iter()
            .filter(|(_, on)| *on)
            .map(|(c, _)| *c)
            .collect::<Vec<_>>(),
        [ColumnId::Size, ColumnId::Modified]
    );
    crate::tests_shell::golden("menus-folder-header.txt", &menu(&app));
    app.0.execute(
        win,
        Invocation::with(CommandId::ColList, CommandArg::Column(ColumnId::Crc)),
    );
    assert!(
        folder_model(&mut app, view)
            .shown_columns()
            .contains(&ColumnId::Crc)
    );
    assert!(menu(&app).contains("[x] CRC"), "{}", menu(&app));
    app.0.execute(
        win,
        Invocation::with(CommandId::ColList, CommandArg::Column(ColumnId::Crc)),
    );
    assert!(
        !folder_model(&mut app, view)
            .shown_columns()
            .contains(&ColumnId::Crc)
    );
    // Path is not offered (nor toggled) until the comparison is flat.
    app.0.execute(
        win,
        Invocation::with(CommandId::ColList, CommandArg::Column(ColumnId::Path)),
    );
    assert!(
        folder_model(&mut app, view)
            .columns()
            .is_visible(ColumnId::Path)
    );
    app.execute(CommandId::DirFlatten);
    world.settle(&mut app);
    assert!(
        app.0
            .column_menu(win)
            .iter()
            .any(|(c, on)| *c == ColumnId::Path && *on)
    );
    app.0.execute(
        win,
        Invocation::with(CommandId::ColList, CommandArg::Column(ColumnId::Path)),
    );
    assert!(
        !folder_model(&mut app, view)
            .shown_columns()
            .contains(&ColumnId::Path)
    );
}

/// Polls the app until the view's CRC job ended (bounded).
fn crc_settle(world: &World, app: &mut crate::tests::TestApp, view: ViewId) {
    let began = std::time::Instant::now();
    loop {
        world.settle(app);
        if !folder_model(app, view).crc_pending() {
            return;
        }
        assert!(
            began.elapsed() < std::time::Duration::from_secs(60),
            "CRC job did not end"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a9_3_crc_on_demand_asks_for_the_rendered_rows_once_still() {
    use std::time::{Duration, Instant};
    let world = World::new();
    for i in 0..40 {
        let name = format!("f{i:02}.txt");
        put(&world, "l", &name, &format!("left {i}\n"), 1_000);
        put(&world, "r", &name, &format!("right {i}\n"), 2_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    let t0 = Instant::now();
    let ms = |n: u64| t0 + Duration::from_millis(n);
    let crc_column = ColumnId::Crc;
    // Hidden: never asked.
    let m = folder_model(&mut app, view);
    m.set_scroll(0, 10);
    assert_eq!(m.crc_frame(ms(0)), None);
    assert_eq!(m.crc_frame(ms(1_000)), None);
    world.settle(&mut app);
    assert_eq!(folder_model(&mut app, view).crc_jobs().0, 0);
    // Shown: nothing before the list has been still for 200 ms.
    let m = folder_model(&mut app, view);
    m.toggle_column(crc_column);
    assert!(m.crc_frame(ms(1_000)).is_some(), "the wait starts");
    assert_eq!(m.crc_frame(ms(1_150)), Some(Duration::from_millis(50)));
    world.settle(&mut app);
    assert_eq!(folder_model(&mut app, view).crc_jobs().0, 0);
    // Still for 200 ms: one job for the ten rendered file rows.
    let m = folder_model(&mut app, view);
    assert_eq!(m.crc_frame(ms(1_200)), None);
    let rendered: Vec<NodeId> = m.rows()[0..10].iter().map(|r| r.node).collect();
    app.0.handle(crate::app::UiMsg::Tick);
    let (jobs, asked) = folder_model(&mut app, view).crc_jobs();
    assert_eq!(jobs, 1);
    assert_eq!(asked.map(<[NodeId]>::to_vec), Some(rendered));
    // A scroll cancels it; the new window is asked for once still again.
    let m = folder_model(&mut app, view);
    m.set_scroll(20, 10);
    assert!(m.crc_frame(ms(1_210)).is_some());
    assert_eq!(m.crc_jobs().1, None, "the scroll cancelled the job");
    assert_eq!(m.crc_frame(ms(1_420)), None);
    crc_settle(&world, &mut app, view);
    let m = folder_model(&mut app, view);
    assert_eq!(m.crc_jobs().0, 2);
    // The cells of the rendered window (the window's overscan rows aside).
    let crc = |m: &crate::folder_view::FolderViewModel, from: u32| -> Vec<String> {
        m.window()
            .rows
            .iter()
            .filter(|r| (from..from + 10).contains(&r.index))
            .flat_map(|r| [r.left.as_ref(), r.right.as_ref()])
            .map(|c| c.map(|c| c.crc.clone()).unwrap_or_default())
            .collect()
    };
    let shown = crc(m, 20);
    assert_eq!(shown.len(), 20);
    assert!(
        shown
            .iter()
            .all(|c| c.len() == 8 && c.chars().all(|ch| ch.is_ascii_hexdigit())),
        "{shown:?}"
    );
    // Answered: a still frame asks nothing more; rows outside every window stay without.
    assert_eq!(m.crc_frame(ms(2_000)), None);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert_eq!(m.crc_jobs().0, 2, "nothing left to ask for");
    m.set_scroll(30, 10);
    assert!(
        crc(m, 30).iter().all(String::is_empty),
        "never rendered, never asked"
    );
    // Hidden while waiting: no job.
    assert!(m.crc_frame(ms(2_010)).is_some());
    m.toggle_column(crc_column);
    assert_eq!(m.crc_frame(ms(3_000)), None);
    world.settle(&mut app);
    assert_eq!(folder_model(&mut app, view).crc_jobs().0, 2);
}

/// What a row of [`FOLDER_COMMANDS`] needs before it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    /// The opened fixture, nothing selected.
    None,
    /// The file pair `d/x.txt` selected (`d` open).
    File,
    /// The closed folder `d` selected.
    ClosedFolder,
    /// The open folder `d` selected.
    OpenFolder,
    /// The folder `e` selected on the left only.
    LeftFolder,
    /// The last row selected.
    Last,
    /// Every folder open.
    Expanded,
    /// Another display filter first (a radio entry).
    OtherFilter,
    /// Another folder mode first (a radio entry).
    OtherMode,
    /// The Find query `x`.
    FindText,
    /// A comparison held while it runs.
    Running,
    /// The view's session is named.
    Named,
}

/// The effect a row's `Invocation` has on the fixture view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// A `…List` entry: a submenu of its children (no effect of its own).
    Submenu,
    /// A dialog opens on the window's stack (after its job, if one plans first).
    Dialog,
    /// The view model changes (selection, rows, columns, panels, the display).
    Model,
    /// The check state of a check or radio entry changes.
    Checked,
    /// The view runs on other bases (its committed specs change).
    Bases,
    /// A tab opens, or the tab becomes another view.
    Tab,
    /// A job of the view starts.
    Job,
    /// The window's session combo takes the focus.
    Focus,
    /// A notice appears (an application started without a launcher: the core message, M2 W9b).
    Notice,
}

/// A9.10: one row per `implemented` entry of the `folder-compare`, `folder-shared`, `folder-sync` and `session`
/// sections that a Folder Compare view offers, with its effect.
const FOLDER_COMMANDS: &[(CommandId, Prep, Expect)] = {
    use CommandId as C;
    use Expect as E;
    use Prep as P;
    &[
        // folder-compare
        (C::UpOneLevelList, P::None, E::Submenu),
        (C::UpOneLevelLeft, P::None, E::Bases),
        (C::UpOneLevelRight, P::None, E::Bases),
        (C::UpOneLevelBoth, P::None, E::Bases),
        (C::CopyRight, P::File, E::Dialog),
        (C::CopyLeft, P::File, E::Dialog),
        // M2 W9b (§1.9.13): every verb.
        (C::CopyPrompt, P::File, E::Dialog),
        (C::CopySide, P::LeftFolder, E::Dialog),
        (C::MoveRight, P::File, E::Dialog),
        (C::MoveLeft, P::File, E::Dialog),
        (C::MovePrompt, P::File, E::Dialog),
        (C::MoveSide, P::LeftFolder, E::Dialog),
        (C::MoveBoth, P::File, E::Dialog),
        (C::CopyTo, P::File, E::Dialog),
        (C::MoveTo, P::File, E::Dialog),
        (C::Rename, P::File, E::Model),
        (C::Touch, P::File, E::Dialog),
        (C::NewFolder, P::None, E::Dialog),
        (C::Ignored, P::File, E::Checked),
        (C::Exclude, P::File, E::Dialog),
        (C::Compare, P::File, E::Dialog),
        (C::QuickCompare, P::File, E::Dialog),
        (C::CompareTo, P::File, E::Tab),
        (C::OpenWithList, P::File, E::Submenu),
        (C::AssocApp, P::File, E::Notice),
        (C::SyncList, P::None, E::Submenu),
        (C::UpdateRight, P::None, E::Dialog),
        (C::UpdateLeft, P::None, E::Dialog),
        (C::UpdateBoth, P::None, E::Dialog),
        (C::MirrorRight, P::None, E::Dialog),
        (C::MirrorLeft, P::None, E::Dialog),
        (C::SyncBaseView, P::None, E::Tab),
        (C::Delete, P::File, E::Dialog),
        (C::PermanentDelete, P::File, E::Dialog),
        (C::SelectAllFiles, P::None, E::Model),
        (C::SelectNewerList, P::None, E::Submenu),
        (C::SelectNewerLeft, P::None, E::Model),
        (C::SelectNewerRight, P::None, E::Model),
        (C::SelectNewerBoth, P::None, E::Model),
        (C::SelectOrphanList, P::None, E::Submenu),
        (C::SelectOrphanLeft, P::None, E::Model),
        (C::SelectOrphanRight, P::None, E::Model),
        (C::SelectOrphanBoth, P::None, E::Model),
        (C::DiffNext, P::None, E::Model),
        (C::DiffPrev, P::Last, E::Model),
        (C::FavCompareAll, P::OtherFilter, E::Checked),
        (C::FavDiff, P::OtherFilter, E::Checked),
        (C::FavSame, P::OtherFilter, E::Checked),
        (C::FavNoOrphan, P::OtherFilter, E::Checked),
        (C::FavDiffNoOrphan, P::OtherFilter, E::Checked),
        (C::FavOrphan, P::OtherFilter, E::Checked),
        (C::FavLeftNewer, P::OtherFilter, E::Checked),
        (C::FavRightNewer, P::OtherFilter, E::Checked),
        (C::FavLeftNewerOrphan, P::OtherFilter, E::Checked),
        (C::FavRightNewerOrphan, P::OtherFilter, E::Checked),
        (C::FavLeftOrphan, P::OtherFilter, E::Checked),
        (C::FavRightOrphan, P::OtherFilter, E::Checked),
        (C::FavCompareNone, P::OtherFilter, E::Checked),
        (C::TogSame, P::None, E::Checked),
        (C::TogLeftOrphan, P::None, E::Checked),
        (C::TogRightOrphan, P::None, E::Checked),
        (C::TogLeftNewer, P::None, E::Checked),
        (C::TogRightNewer, P::None, E::Checked),
        (C::TogDiff, P::None, E::Checked),
        (C::DirShowAll, P::OtherMode, E::Checked),
        (C::DirCompareStructure, P::OtherMode, E::Checked),
        (C::DirCompareFilesOnly, P::OtherMode, E::Checked),
        (C::DirFlatten, P::None, E::Checked),
        // M2 W9c (D-249, D-250).
        (C::NameFilter, P::None, E::Dialog),
        (C::NameFilterEdit, P::None, E::Focus),
        // folder-shared
        (C::Swap, P::None, E::Bases),
        (C::BrowseList, P::None, E::Submenu),
        (C::BrowseLeft, P::None, E::Dialog),
        (C::BrowseRight, P::None, E::Dialog),
        (C::Stop, P::Running, E::Model),
        (C::Expand, P::ClosedFolder, E::Model),
        (C::Collapse, P::OpenFolder, E::Model),
        (C::ExpandSubfolders, P::ClosedFolder, E::Model),
        (C::CollapseSubfolders, P::OpenFolder, E::Model),
        (C::SetAsBaseFolders, P::ClosedFolder, E::Bases),
        (C::SetAsOtherBaseFolder, P::LeftFolder, E::Bases),
        (C::Open, P::File, E::Tab),
        (C::ExpandAll, P::None, E::Model),
        (C::CollapseAll, P::Expanded, E::Model),
        (C::SelectInvert, P::None, E::Model),
        (C::FullRefresh, P::None, E::Job),
        (C::ExistsNext, P::None, E::Model),
        (C::ExistsPrev, P::Last, E::Model),
        (C::FindFilename, P::None, E::Model),
        (C::FindFilenameNext, P::FindText, E::Model),
        (C::FindFilenamePrev, P::FindText, E::Model),
        (C::SuppressFilters, P::None, E::Checked),
        (C::ColList, P::None, E::Model),
        (C::Legend, P::None, E::Checked),
        (C::ShowLog, P::None, E::Checked),
        // M2 W9c (FC-063, D-252, D-254): the test window has no clipboard and no launcher: the core message.
        (C::CopyFilenames, P::File, E::Notice),
        (C::ExplorerList, P::File, E::Notice),
        // session
        (C::SessionHome, P::None, E::Tab),
        (C::SessionsCombo, P::None, E::Focus),
        (C::SessionSave, P::None, E::Dialog),
        (C::SessionSaveAs, P::None, E::Dialog),
        (C::SessionEdit, P::None, E::Dialog),
        (C::SessionLocked, P::Named, E::Checked),
        (C::SessionClear, P::None, E::Bases),
        (C::Info, P::None, E::Dialog),
        // M2 W13c (D-366): the report dialog.
        (C::Report, P::None, E::Dialog),
        (C::Rules, P::None, E::Dialog),
        (C::CompareParents, P::File, E::Tab),
        (C::InsigIgnored, P::None, E::Checked),
        (C::ShowToolbar, P::None, E::Checked),
    ]
};

/// The fixture: a folder pair with a difference, a newer file each way, orphans and a left-only folder.
fn parity_fixture(world: &World) -> (crate::tests::TestApp, ViewId) {
    put(world, "l", "d/x.txt", "left x\n", 3_000);
    put(world, "r", "d/x.txt", "right\n", 1_000);
    put(world, "l", "e/y.txt", "y\n", 1_000);
    put(world, "l", "lnew.txt", "new left\n", 3_000);
    put(world, "r", "lnew.txt", "old\n", 1_000);
    put(world, "l", "rnew.txt", "old\n", 1_000);
    put(world, "r", "rnew.txt", "new right\n", 3_000);
    put(world, "r", "ronly.txt", "r\n", 1_000);
    let mut app = world.app(None);
    let view = open(world, &mut app);
    app.execute(CommandId::FavCompareAll);
    world.settle(&mut app);
    (app, view)
}

/// What the test compares before and after a row's invocation.
fn fingerprint(app: &mut crate::tests::TestApp, view: ViewId) -> String {
    let m = folder_model(app, view);
    format!(
        "{:?}",
        (
            m.rows().iter().map(|r| r.node).collect::<Vec<_>>(),
            m.selection().items.iter().collect::<Vec<_>>(),
            m.selection().current,
            m.shown_columns(),
            *m.display_options(),
            (m.show_log(), m.show_legend(), m.show_find()),
            m.busy(),
            m.inline_rename().is_some(),
        )
    )
}

fn settle_all(world: &World, app: &mut crate::tests::TestApp) {
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    }
}

/// Prepares the fixture for `id`; a scan held at its gate comes back (the test opens it once the command ran).
fn prepare(
    world: &World,
    app: &mut crate::tests::TestApp,
    view: ViewId,
    id: CommandId,
    prep: Prep,
) -> Option<crate::folder_view::Gate> {
    use crate::tests_folder_ops::select;
    let mut held = None;
    match prep {
        Prep::None => {}
        Prep::File => {
            select(app, view, "d");
            app.execute(CommandId::Expand);
            world.settle(app);
            select(app, view, "d/x.txt");
        }
        Prep::ClosedFolder => select(app, view, "d"),
        Prep::OpenFolder => {
            select(app, view, "d");
            app.execute(CommandId::Expand);
            world.settle(app);
            select(app, view, "d");
        }
        Prep::LeftFolder => {
            let m = folder_model(app, view);
            let model = Arc::clone(m.prepared().expect("prepared").model());
            let read = model.read();
            let index = m
                .rows()
                .iter()
                .position(|r| read.path(r.node).as_str() == "e")
                .expect("e");
            m.click(index, Hit::Side(Side::Left), Mods::default());
        }
        Prep::Last => {
            let m = folder_model(app, view);
            let last = m.rows().len() - 1;
            m.select(last);
        }
        Prep::Expanded => {
            app.execute(CommandId::ExpandAll);
            world.settle(app);
        }
        Prep::OtherFilter => {
            let other = if id == CommandId::FavCompareAll {
                CommandId::FavDiff
            } else {
                CommandId::FavCompareAll
            };
            app.execute(other);
            world.settle(app);
        }
        Prep::OtherMode => {
            let other = if id == CommandId::DirShowAll {
                CommandId::DirCompareStructure
            } else {
                CommandId::DirShowAll
            };
            app.execute(other);
            world.settle(app);
        }
        Prep::FindText => folder_model(app, view).find_panel().text = "x".into(),
        Prep::Running => {
            let g = crate::tests_folder_ops::gate(false);
            folder_model(app, view).scan_gate = Some(Arc::clone(&g));
            app.execute(CommandId::FullRefresh);
            crate::tests_folder_ops::poll_until(world, app, |a| folder_model(a, view).scanning());
            // The gate opens once Stop has been asked for (the scan then ends as stopped).
            held = Some(g);
        }
        Prep::Named => {
            app.execute(CommandId::SessionSaveAs);
            world.settle(app);
            let w = app.win();
            let d = app
                .0
                .window_mut(w)
                .and_then(|w| {
                    w.dialogs
                        .find_mut::<crate::session_dialog::SaveSessionDialog>()
                })
                .expect("Save As");
            d.name = "parity".into();
            crate::tests_folder_ops::press(world, app, crate::dialog::ButtonRole::Ok);
            settle_all(world, app);
        }
    }
    held
}

#[test]
fn a9_10_every_implemented_folder_entry_has_a_row_and_its_effect() {
    use crate::commands::{Scope, list_children, lookup};
    use crate::window::{CommandArg, Invocation};
    use hikasa_session::commands::{CommandStatus, catalogue};
    // Every implemented entry the Folder Compare view offers has a row, and no row is unknown.
    let core = catalogue();
    let mut implemented = Vec::new();
    for section in ["folder-compare", "folder-shared", "folder-sync", "session"] {
        let Some(s) = core.section(section) else {
            continue;
        };
        for entry in s
            .commands
            .iter()
            .filter(|e| e.status == CommandStatus::Implemented)
        {
            let id = CommandId::from_name(&entry.id).expect("known to the GUI");
            if lookup(Scope::Folder, id).is_some() && !implemented.contains(&id) {
                implemented.push(id);
            }
        }
    }
    for id in &implemented {
        assert!(
            FOLDER_COMMANDS.iter().any(|(c, _, _)| c == id),
            "{id:?} is implemented but FOLDER_COMMANDS has no row"
        );
    }
    for (id, _, _) in FOLDER_COMMANDS {
        assert!(
            implemented.contains(id),
            "{id:?} has a row but is not an implemented folder entry"
        );
    }
    // Each row's effect happens on a fixture view of its own.
    for &(id, prep, expect) in FOLDER_COMMANDS {
        let world = World::new();
        let (mut app, view) = parity_fixture(&world);
        let held = prepare(&world, &mut app, view, id, prep);
        if expect == Expect::Submenu {
            assert!(!list_children(id).is_empty(), "{id:?}: a submenu");
            continue;
        }
        assert!(
            app.enabled(id),
            "{id:?} is not available on the fixture ({prep:?})"
        );
        let w = app.win();
        let dialogs = crate::tests_folder_ops::dialogs(&app);
        let tabs = (app.tabs().len(), app.active(), app.tabs()[app.active()].id);
        let checked = app.checked(id);
        let notices = crate::tests_folder_ops::notices(&app).len();
        let before = fingerprint(&mut app, view);
        let committed = {
            let m = folder_model(&mut app, view);
            [
                m.path_edit(0).committed.clone(),
                m.path_edit(1).committed.clone(),
            ]
        };
        let invocation = if id == CommandId::ColList {
            Invocation::with(id, CommandArg::Column(ColumnId::Ext))
        } else {
            Invocation::new(id)
        };
        app.0.execute(w, invocation);
        if let Some(g) = &held {
            crate::tests_folder_ops::set_gate(g, true);
        }
        if expect == Expect::Job {
            assert!(folder_model(&mut app, view).busy(), "{id:?}: a job runs");
            settle_all(&world, &mut app);
            continue;
        }
        settle_all(&world, &mut app);
        match expect {
            Expect::Dialog => assert!(
                crate::tests_folder_ops::dialogs(&app) > dialogs,
                "{id:?}: a dialog"
            ),
            Expect::Checked => assert_ne!(app.checked(id), checked, "{id:?}: the check state"),
            Expect::Model => assert_ne!(fingerprint(&mut app, view), before, "{id:?}: the model"),
            Expect::Bases => {
                let m = folder_model(&mut app, view);
                let now = [
                    m.path_edit(0).committed.clone(),
                    m.path_edit(1).committed.clone(),
                ];
                assert_ne!(now, committed, "{id:?}: the bases");
            }
            Expect::Tab => {
                let now = (app.tabs().len(), app.active(), app.tabs()[app.active()].id);
                let home = matches!(
                    app.tabs()[app.active()].model(),
                    Some(crate::window::ViewModel::Home(_))
                );
                assert!(now != tabs || home, "{id:?}: a tab");
            }
            Expect::Focus => {
                assert!(
                    app.0
                        .window(w)
                        .is_some_and(|w| w.focus_sessions || w.focus_filters),
                    "{id:?}: the focus"
                );
            }
            Expect::Notice => assert!(
                crate::tests_folder_ops::notices(&app).len() > notices,
                "{id:?}: a notice"
            ),
            Expect::Submenu | Expect::Job => unreachable!(),
        }
    }
}
