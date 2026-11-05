//! M2 W9a after its review (§1.9.25 notes 23 on): CRC on demand never beside an operation and never in the way of its
//! plan, its errors asked again after a re-run; the base commands wait for a running operation; status counts that
//! count each item once; a tree order that is remembered; base checks out of order; the browse dialog's and the
//! completions' rough edges; and the tests the review's surviving mutations showed missing. Files live in the World's
//! private temporary folder.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_core::{JobKind, Side};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_session::api::{ContentsKind, ContentsRequest};
use hikasa_session::ipc::ViewId;

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::folder::columns::ColumnId;
use crate::folder_view::{FolderViewModel, select_request};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{
    dialogs, gate, node_of, notices, open, poll_until, press, put, read, select, set_gate,
};

// ---- helpers ------------------------------------------------------------------------------------------------------

/// Shows the CRC column (if hidden), renders rows `first..first + count` and lets the list stay still for longer than
/// the settle time at frame time `at`: the next poll may start the CRC job.
fn crc_due(app: &mut TestApp, view: ViewId, first: u32, count: u32, at: Instant) {
    let m = folder_model(app, view);
    if !m.shown_columns().contains(&ColumnId::Crc) {
        m.toggle_column(ColumnId::Crc);
    }
    m.set_scroll(first, count);
    m.crc_frame(at);
    assert_eq!(m.crc_frame(at + Duration::from_millis(300)), None, "due");
}

/// Polls until the view's CRC job ended (bounded by count).
fn crc_settle(world: &World, app: &mut TestApp, view: ViewId) {
    poll_until(world, app, |a| !folder_model(a, view).crc_pending());
}

/// The CRC cells of the rendered row `index`: (left, right).
fn crc_cells(m: &FolderViewModel, index: u32) -> (String, String) {
    let window = m.window();
    let row = window
        .rows
        .iter()
        .find(|r| r.index == index)
        .expect("a rendered row");
    let cell = |c: Option<&crate::folder_view::Cell>| c.map(|c| c.crc.clone()).unwrap_or_default();
    (cell(row.left.as_ref()), cell(row.right.as_ref()))
}

fn hex(text: &str) -> bool {
    text.len() == 8 && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Forty different pairs (rows `f00.txt`…) and `a.txt` on the left only, every row shown.
fn forty(world: &World) -> (TestApp, ViewId) {
    for i in 0..40 {
        let name = format!("f{i:02}.txt");
        put(world, "l", &name, &format!("left {i}\n"), 1_000);
        put(world, "r", &name, &format!("right {i}\n"), 2_000);
    }
    put(world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(world, &mut app);
    (app, view)
}

// ---- 1: CRC on demand and operations ------------------------------------------------------------------------------

/// Review finding 1 (R1): while an operation's question is open no CRC job starts, and a CRC that is still found
/// (a job that was cancelled when the operation started finishes its file) does not refuse the plan.
#[test]
fn crc_waits_for_a_question_and_never_refuses_its_plan() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1, "the copy question");
    crc_due(&mut app, view, 0, 10, Instant::now());
    world.settle(&mut app);
    assert_eq!(
        folder_model(&mut app, view).crc_jobs().0,
        0,
        "no CRC job beside the question"
    );
    // A CRC found now (as by a job cancelled when the operation started): a cached value, not a change of the
    // comparison the plan was made on.
    let model = Arc::clone(
        folder_model(&mut app, view)
            .prepared()
            .expect("prepared")
            .model(),
    );
    let f = node_of(&mut app, view, "f00.txt").expect("f00");
    let mut items = Selection::default();
    items.insert(f, SideSet::all());
    let display = *folder_model(&mut app, view).display_options();
    let request = ContentsRequest {
        select: select_request(items, display),
        kind: ContentsKind::Crc(SideSet::all()),
        text_rules: None,
    };
    let control = world
        .service
        .job(JobKind::FolderOp, Arc::new(hikasa_core::NullProgress));
    let dto = world
        .service
        .folder_contents(&model, &request, &control)
        .expect("crc");
    assert!(dto.items.iter().all(|i| i.error.is_none()));
    press(&world, &mut app, ButtonRole::Yes);
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        !m.busy() && !m.settling()
    });
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(
        read(&world, "r", "a.txt").as_deref(),
        Some("left\n"),
        "the copy ran"
    );
    assert!(
        notices(&app)
            .iter()
            .all(|(_, text, _)| !text.contains("changed")),
        "{:?}",
        notices(&app)
    );
    // Once the operation ended the rendered rows are asked for again.
    crc_due(
        &mut app,
        view,
        0,
        10,
        Instant::now() + Duration::from_secs(10),
    );
    crc_settle(&world, &mut app, view);
    assert!(folder_model(&mut app, view).crc_jobs().0 >= 1);
}

/// A CRC job in flight when an operation starts gives way at once (stale until it ended).
#[test]
fn an_operation_cancels_the_running_crc_job() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    let g = gate(false);
    folder_model(&mut app, view).crc_gate = Some(Arc::clone(&g));
    crc_due(&mut app, view, 0, 10, Instant::now());
    world.settle(&mut app);
    assert!(
        folder_model(&mut app, view).crc_jobs().1.is_some(),
        "in flight"
    );
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    assert_eq!(
        folder_model(&mut app, view).crc_jobs().1,
        None,
        "the operation cancelled the CRC job"
    );
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1, "the copy question");
    press(&world, &mut app, ButtonRole::Yes);
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        !m.busy() && !m.settling()
    });
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 0);
    assert_eq!(
        read(&world, "r", "a.txt").as_deref(),
        Some("left\n"),
        "the copy ran"
    );
}

/// The view's end cancels its CRC job.
#[test]
fn retire_cancels_the_crc_job() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    let g = gate(false);
    folder_model(&mut app, view).crc_gate = Some(Arc::clone(&g));
    crc_due(&mut app, view, 0, 10, Instant::now());
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(m.crc_jobs().1.is_some(), "in flight");
    let _ = m.retire();
    assert_eq!(m.crc_jobs().1, None, "cancelled with the view");
    set_gate(&g, true);
}

/// Mutation L: no CRC job while the comparison runs.
#[test]
fn no_crc_job_while_the_comparison_runs() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    let g = gate(false);
    folder_model(&mut app, view).scan_gate = Some(Arc::clone(&g));
    app.execute(CommandId::FullRefresh);
    poll_until(&world, &mut app, |a| {
        let m = folder_model(a, view);
        m.scanning() && m.rows().len() > 10
    });
    crc_due(&mut app, view, 0, 10, Instant::now());
    for _ in 0..5 {
        app.handle(crate::app::UiMsg::Tick);
    }
    assert_eq!(
        folder_model(&mut app, view).crc_jobs().0,
        0,
        "not while scanning"
    );
    set_gate(&g, true);
    world.settle(&mut app);
    crc_due(
        &mut app,
        view,
        0,
        10,
        Instant::now() + Duration::from_secs(10),
    );
    crc_settle(&world, &mut app, view);
    assert_eq!(
        folder_model(&mut app, view).crc_jobs().0,
        1,
        "once it ended"
    );
}

/// Mutation K: hiding the column cancels the running job.
#[test]
fn hiding_the_crc_column_cancels_the_running_job() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    let g = gate(false);
    folder_model(&mut app, view).crc_gate = Some(Arc::clone(&g));
    let t0 = Instant::now();
    crc_due(&mut app, view, 0, 10, t0);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(m.crc_jobs().1.is_some(), "in flight");
    m.toggle_column(ColumnId::Crc);
    assert_eq!(m.crc_frame(t0 + Duration::from_secs(1)), None);
    assert_eq!(m.crc_jobs().1, None, "hiding the column cancelled it");
    set_gate(&g, true);
    crc_settle(&world, &mut app, view);
    let m = folder_model(&mut app, view);
    assert_eq!(
        crc_cells(m, 0),
        (String::new(), String::new()),
        "its answer dropped"
    );
}

/// Mutations J and M: folder rows are never asked for, and one job asks for at most 256 rows.
#[test]
fn crc_asks_for_file_rows_only_at_most_256() {
    let world = World::new();
    for d in 0..3 {
        put(&world, "l", &format!("d{d}/x.txt"), "x\n", 1_000);
        put(&world, "r", &format!("d{d}/x.txt"), "y\n", 2_000);
    }
    for i in 0..300 {
        let name = format!("f{i:03}.txt");
        put(&world, "l", &name, &format!("left {i}\n"), 1_000);
        put(&world, "r", &name, &format!("right {i}\n"), 2_000);
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    let g = gate(false);
    folder_model(&mut app, view).crc_gate = Some(Arc::clone(&g));
    let rows = folder_model(&mut app, view).rows().len() as u32;
    assert!(rows > 300, "{rows}");
    crc_due(&mut app, view, 0, rows, Instant::now());
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let asked = m.crc_jobs().1.expect("in flight").to_vec();
    set_gate(&g, true);
    assert_eq!(asked.len(), 256, "one batch");
    let read = model.read();
    assert!(
        asked
            .iter()
            .all(|&n| read.node(n).is_some_and(|v| !v.is_folder())),
        "file rows only"
    );
    let folders = m.rows()[..10]
        .iter()
        .filter(|r| read.node(r.node).is_some_and(|v| v.is_folder()))
        .count();
    assert_eq!(folders, 3, "the folders were rendered first");
    drop(read);
    crc_settle(&world, &mut app, view);
}

/// Mutation AK and review finding 6 (R3): a CRC that failed shows `error` on its own side only and is asked again
/// once the comparison runs again.
#[cfg(unix)]
#[test]
fn a_crc_error_is_on_its_side_and_asked_again_after_a_re_run() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    put(&world, "r", "a.txt", "right\n", 2_000);
    let path = world.path("work/r/a.txt");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    if std::fs::read(&path).is_ok() {
        // Running with the rights to read anything (root): nothing fails to read here.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
        return;
    }
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    crc_due(&mut app, view, 0, 10, Instant::now());
    crc_settle(&world, &mut app, view);
    let (left, right) = crc_cells(folder_model(&mut app, view), 0);
    assert!(hex(&left), "the left side read: {left:?}");
    assert_eq!(right, "error", "the right side failed");
    // Not asked again while the comparison stays.
    let jobs = folder_model(&mut app, view).crc_jobs().0;
    crc_due(
        &mut app,
        view,
        1,
        10,
        Instant::now() + Duration::from_secs(5),
    );
    crc_due(
        &mut app,
        view,
        0,
        10,
        Instant::now() + Duration::from_secs(6),
    );
    crc_settle(&world, &mut app, view);
    assert_eq!(
        folder_model(&mut app, view).crc_jobs().0,
        jobs,
        "not asked again"
    );
    // The comparison runs again: asked again, and readable now.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    app.execute(CommandId::FullRefresh);
    world.settle(&mut app);
    poll_until(&world, &mut app, |a| !folder_model(a, view).scanning());
    crc_due(
        &mut app,
        view,
        1,
        10,
        Instant::now() + Duration::from_secs(10),
    );
    crc_due(
        &mut app,
        view,
        0,
        10,
        Instant::now() + Duration::from_secs(11),
    );
    crc_settle(&world, &mut app, view);
    let (left, right) = crc_cells(folder_model(&mut app, view), 0);
    assert!(hex(&left) && hex(&right), "{left:?} {right:?}");
}

/// A sort on CRC follows the values found on demand; without that sort the rows are not rebuilt for them.
#[test]
fn crc_results_rebuild_the_rows_only_for_a_sort_on_crc() {
    let world = World::new();
    let (mut app, view) = forty(&world);
    let before = folder_model(&mut app, view).rebuild_stats().samples.len();
    crc_due(&mut app, view, 0, 10, Instant::now());
    crc_settle(&world, &mut app, view);
    for _ in 0..5 {
        world.settle(&mut app);
    }
    let m = folder_model(&mut app, view);
    let after = m.rebuild_stats().samples.len();
    // The job's end rebuilds once (and the column's toggle); the results themselves do not.
    assert!(after - before <= 2, "{before} -> {after}");
    let (left, _) = crc_cells(m, 0);
    assert!(hex(&left), "{left:?}");
}

// ---- 2: the base commands wait for a running operation ------------------------------------------------------------

fn committed(app: &mut TestApp, view: ViewId) -> [String; 2] {
    let m = folder_model(app, view);
    [
        m.path_edit(0).committed.clone(),
        m.path_edit(1).committed.clone(),
    ]
}

/// The notice of a re-root while an operation runs.
const WAIT: &str = "Wait for the current operation to end.";

fn waits(app: &TestApp) -> bool {
    notices(app).iter().any(|(_, text, _)| text == WAIT)
}

/// Review finding 2: while an operation's plan or execution runs, the base commands are off, and an Accept of new
/// bases is refused with "Wait for the current operation to end." (the operation is not cancelled).
#[test]
fn the_base_commands_wait_for_a_running_operation() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    put(&world, "r", "b.txt", "b\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    assert!(app.enabled(CommandId::UpOneLevelBoth) && app.enabled(CommandId::Swap));
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    assert!(folder_model(&mut app, view).busy(), "the plan runs");
    for id in [
        CommandId::UpOneLevelLeft,
        CommandId::UpOneLevelRight,
        CommandId::UpOneLevelBoth,
        CommandId::Swap,
        CommandId::SessionClear,
    ] {
        assert!(!app.enabled(id), "{id:?} while an operation runs");
    }
    // An Accept of a new base (as typed, or a browse answer).
    let up = world.path("work").to_string_lossy().into_owned();
    let m = folder_model(&mut app, view);
    m.path_edit(0).set_text(&up);
    m.accept_paths();
    for _ in 0..5 {
        app.0.handle(crate::app::UiMsg::Tick);
    }
    assert!(waits(&app), "{:?}", notices(&app));
    assert_eq!(committed(&mut app, view), before, "not re-rooted");
    assert!(
        folder_model(&mut app, view).busy(),
        "the plan was not cancelled"
    );
    set_gate(&g, true);
    world.settle(&mut app);
    press(&world, &mut app, ButtonRole::Yes);
    world.settle(&mut app);
    assert_eq!(
        read(&world, "r", "a.txt").as_deref(),
        Some("left\n"),
        "the copy ran"
    );
    assert!(app.enabled(CommandId::UpOneLevelBoth), "idle again");
}

/// A check of new bases that ends after an operation started does not re-root the view.
#[test]
fn a_base_check_that_ends_during_an_operation_is_refused() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    let up = world.path("work").to_string_lossy().into_owned();
    let m = folder_model(&mut app, view);
    m.path_edit(0).set_text(&up);
    m.accept_paths();
    // This poll takes the Accept and starts the check; its answer is read at a later poll.
    app.0.handle(crate::app::UiMsg::Tick);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    poll_until(&world, &mut app, waits_core);
    assert_eq!(committed(&mut app, view), before, "not re-rooted");
    assert!(
        app.state()
            .mru(hikasa_session::settings::state::MruList::Left)
            .is_empty(),
        "nothing loaded"
    );
    set_gate(&g, true);
    world.settle(&mut app);
}

fn waits_core(app: &mut crate::app::AppCore) -> bool {
    let w = app.focused();
    app.window(w)
        .is_some_and(|w| w.notices.all().iter().any(|n| n.text == WAIT))
}

/// A reopen (Swap Sides' store job) that ends during an operation does not replace the view: its operation runs on.
#[test]
fn a_reopen_that_ends_during_an_operation_is_refused() {
    let world = World::new();
    put(&world, "l", "a.txt", "left\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let before = committed(&mut app, view);
    app.execute(CommandId::Swap);
    let g = gate(false);
    folder_model(&mut app, view).gate = Some(Arc::clone(&g));
    select(&mut app, view, "a.txt");
    app.execute(CommandId::CopyRight);
    poll_until(&world, &mut app, waits_core);
    assert_eq!(committed(&mut app, view), before, "not swapped");
    assert!(folder_model(&mut app, view).busy(), "the plan runs on");
    set_gate(&g, true);
    world.settle(&mut app);
    assert_eq!(dialogs(&app), 1, "its question");
}

// ---- 4: status counts ---------------------------------------------------------------------------------------------

/// Review finding 4 (R2): a folder and a file below it, both selected, count two items and the file's bytes once.
#[test]
fn counts_take_each_byte_once() {
    use crate::folder::selection::{Hit, Mods};
    let world = World::new();
    put(&world, "l", "d/f.txt", &"x".repeat(100), 1_000);
    put(&world, "r", "d/f.txt", &"x".repeat(100), 1_000);
    put(&world, "l", "d/g.txt", &"y".repeat(10), 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    let d = node_of(&mut app, view, "d").expect("d");
    let f = node_of(&mut app, view, "d/f.txt").expect("f");
    let m = folder_model(&mut app, view);
    let di = m.rows().iter().position(|r| r.node == d).expect("d row");
    let fi = m.rows().iter().position(|r| r.node == f).expect("f row");
    m.click(di, Hit::Side(hikasa_core::Side::Left), Mods::default());
    m.click(
        fi,
        Hit::Centre,
        Mods {
            ctrl: true,
            shift: false,
        },
    );
    // Left: d (110 bytes below it) and d/f.txt inside it; right: d/f.txt alone (100).
    assert_eq!(m.selection_counts(), [(2, 110), (1, 100)]);
    let items = m.status_items();
    assert!(
        items
            .right
            .iter()
            .any(|t| t.starts_with("Selected: L 2 (110) · R 1 (100)")),
        "{:?}",
        items.right
    );
}

/// A folder counts the bytes of its shown rows only (§1.9.7): a display that hides a file below it takes its bytes
/// away; and the counts follow a rebuild that keeps the selection (mutation AF: a memo that ignores the rows).
#[test]
fn counts_follow_the_shown_rows() {
    let world = World::new();
    put(&world, "l", "d/same.txt", &"s".repeat(7), 1_000);
    put(&world, "r", "d/same.txt", &"s".repeat(7), 1_000);
    put(&world, "l", "d/diff.txt", &"l".repeat(30), 3_000);
    put(&world, "r", "d/diff.txt", &"r".repeat(20), 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    let m = folder_model(&mut app, view);
    assert_eq!(m.selection_counts(), [(1, 37), (1, 27)], "every row shown");
    app.execute(CommandId::FavDiff);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(
        m.selection()
            .items
            .get(node_of_m(m, "d"))
            .contains(SideSet::LEFT),
        "d stays selected"
    );
    assert_eq!(
        m.selection_counts(),
        [(1, 30), (1, 20)],
        "the same file is hidden"
    );
}

fn node_of_m(m: &FolderViewModel, path: &str) -> hikasa_folder::NodeId {
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let read = model.read();
    read.preorder()
        .map(|(id, _)| id)
        .find(|&id| read.path(id).as_str() == path)
        .expect(path)
}

// ---- 5: the layout's sort and its entries -------------------------------------------------------------------------

/// Review finding 5 (R5): a sort set once is forgotten when the header goes back to the tree order; and a session's
/// tree order wins over the view type's remembered sort (D-108 overlays key by key).
#[test]
fn the_tree_order_is_remembered_and_a_session_s_wins() {
    use crate::folder::columns::ColumnLayout;
    use hikasa_session::settings::State;
    let mut state = State::default();
    let mut layout = ColumnLayout::default();
    layout.header_click(ColumnId::Size, Side::Left);
    for (k, v) in layout.to_layout() {
        state.set_layout_value("folder-compare", k, v);
    }
    let sorted = layout.clone();
    layout.header_click(ColumnId::Size, Side::Left);
    layout.header_click(ColumnId::Size, Side::Left);
    assert_eq!(layout.sort, None, "back to the tree order");
    let tree_keys = layout.to_layout();
    for (k, v) in tree_keys.clone() {
        state.set_layout_value("folder-compare", k, v);
    }
    let (read, warning) =
        ColumnLayout::from_kept(&|k| state.layout_value_kept("folder-compare", k));
    assert_eq!(read.sort, None, "the tree order is remembered");
    assert_eq!(warning, None);
    // The view type remembers a sort; a session saved in the tree order opens in the tree order.
    let mut global = State::default();
    for (k, v) in sorted.to_layout() {
        global.set_layout_value("folder-compare", k, v);
    }
    let mut session_view = hikasa_session::sessions::OpaqueTable::new();
    for (k, v) in tree_keys {
        session_view.insert(k.to_owned(), v.to_toml());
    }
    global.overlay_view("folder-compare", &session_view);
    let (read, _) = ColumnLayout::from_kept(&|k| global.layout_value_kept("folder-compare", k));
    assert_eq!(read.sort, None, "the session's tree order wins");
}

/// Item 13 and mutation AJ: a repeated id, an entry that is not a word and a width that is not a number are dropped
/// with one warning and the good entries kept; a sort on a column that is not shown leaves the tree order.
#[test]
fn bad_layout_entries_are_dropped_one_by_one_with_a_warning() {
    use hikasa_session::log::LogKind;
    let (size, crc, owner) = (
        ColumnId::Size.as_str(),
        ColumnId::Crc.as_str(),
        ColumnId::Owner.as_str(),
    );
    let state = format!(
        "schema_version = 1\n\n[layout.folder-compare]\ncolumns = [\"{size}\", 3, \"{crc}\", \"{size}\"]\n\
         sort = \"{owner}\"\n\n[layout.folder-compare.widths]\n{crc} = 90\n{owner} = \"wide\"\n"
    );
    let world = World::with_files(&[("state.toml", &state)]);
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let m = folder_model(&mut app, view);
    let columns = m.columns().clone();
    assert_eq!(
        columns.visible,
        [ColumnId::Name, ColumnId::Size, ColumnId::Crc],
        "kept once each"
    );
    assert_eq!(columns.width(ColumnId::Crc), 90.0, "the good width kept");
    assert_eq!(columns.sort, None, "no sort on a column not shown");
    let warnings: Vec<String> = crate::tests_folder_ops::logged(&app, LogKind::Warning)
        .into_iter()
        .filter(|w| w.contains("folder layout"))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];
    assert!(warning.contains("invalid_config_value"), "{warning}");
    assert!(
        warning.contains("`3`") && warning.contains(owner) && warning.contains("again"),
        "{warning}"
    );
}

// ---- A9.1: the layout written on close, a session's columns first -------------------------------------------------

/// A9.1 (review item 13): a folder view's columns, widths and sort go to `[layout.folder-compare]` on close (golden
/// `state.toml`) and come back at the next start.
#[test]
fn a9_1_the_folder_columns_are_written_on_close_and_read_back() {
    let world = World::new();
    put(&world, "l", "a.txt", "a\n", 1_000);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let m = folder_model(&mut app, view);
    m.change_columns(|c| {
        c.toggle(ColumnId::Crc);
        c.toggle(ColumnId::Ext);
        c.move_to(ColumnId::Ext, 1);
        c.resize(ColumnId::Crc, 90.0);
    });
    m.header_click(ColumnId::Size, Side::Right);
    m.header_click(ColumnId::Size, Side::Right);
    let columns = m.columns().clone();
    app.shutdown();
    app.flush(Duration::from_secs(10));
    let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state.toml");
    crate::tests_shell::golden("state-folder-columns.toml", &state);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert_eq!(folder_model(&mut app, view).columns(), &columns);
}

/// A9.1 (review item 13): a session's `[view]` columns and tree order win over the view type's layout (D-108).
#[test]
fn a9_1_a_session_s_columns_win_over_the_layout() {
    use crate::folder::columns::ColumnLayout;
    use hikasa_session::sessions::{Session, SessionType};
    let (size, owner) = (ColumnId::Size.as_str(), ColumnId::Owner.as_str());
    let state = format!(
        "schema_version = 1\n\n[layout.folder-compare]\ncolumns = [\"{size}\", \"{owner}\"]\nsort = \"{size}\"\n\
         sort_side = \"right\"\nsort_descending = true\n"
    );
    let world = World::with_files(&[("state.toml", &state)]);
    put(&world, "l", "a.txt", "a\n", 1_000);
    put(&world, "r", "a.txt", "b\n", 1_000);
    let mut session = Session::builtin(SessionType::FolderCompare);
    session.specs.left = world.path("work/l").to_string_lossy().into_owned();
    session.specs.right = world.path("work/r").to_string_lossy().into_owned();
    let mut mine = ColumnLayout::default();
    mine.visible = vec![ColumnId::Name, ColumnId::Ext];
    for (key, value) in mine.to_layout() {
        session.view.insert(key.to_owned(), value.to_toml());
    }
    world
        .service
        .session_save(&crate::tests_sessions::r("mine"), &session, false)
        .expect("save");
    let mut app = world.app(None);
    // Without the session: the view type's layout.
    let plain = open(&world, &mut app);
    let columns = folder_model(&mut app, plain).columns().clone();
    assert_eq!(
        columns.visible,
        [ColumnId::Name, ColumnId::Size, ColumnId::Owner]
    );
    assert!(columns.sort.is_some());
    // The session's own columns and its tree order.
    let target = crate::app::OpenTarget::Window(app.win());
    app.0.open_named(
        crate::tests_sessions::r("mine"),
        crate::app::OpenHow::Target(target),
    );
    world.settle(&mut app);
    let view = app.active_tab().expect("a tab").id;
    let columns = folder_model(&mut app, view).columns().clone();
    assert_eq!(columns.visible, [ColumnId::Name, ColumnId::Ext]);
    assert_eq!(columns.sort, None, "the session's tree order wins");
}
