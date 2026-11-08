//! M2 W9c acceptance A9.21 (D-253) and A9.22's GUI part (D-254): the pane context menus as text goldens per area
//! (`tests/golden/menus/*.txt`) with their availability for fixture selections (a file pair, an orphan, a folder, two
//! items, a read-only side, a flat view), before and after a fixture `commands.toml`; a right-click outside the
//! selection selects first; an entry of a side acts on the clicked side. The file-manager entries on a file, a folder,
//! a member (its archive) and a spec off the disk (disabled), through a recording launcher; a launcher that cannot
//! start them gives a notice.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_folder::select::SideSet;
use hikasa_folder::sync::SyncOptions;
use hikasa_session::launcher::{Recorded, RecordingLauncher};

use crate::commands::{Area, CommandId};
use crate::folder::selection::Hit;
use crate::menus::{Menu, MenuItem, MenuTitle, render_text};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{node_of, notices, open, open_with, put, select};
use crate::window::{CommandArg, ExplorerAction, ExplorerItem, Invocation};

/// The commands file of the "after" goldens: a caption, a shortcut, an entry hidden from the menus.
const COMMANDS: &str = "schema_version = 1\n\
[view.folder-compare.command.CopySide]\ncaption = \"Duplicate on the Other Side\"\n\
[view.folder-compare.command.Paste]\nshortcuts = [\"Shift+Ins\"]\n\
[view.folder-compare.command.Touch]\nmenu = false\n";

fn fixture(world: &World) {
    put(world, "l", "a.txt", "left a\n", 3_000);
    put(world, "r", "a.txt", "right a\n", 1_000);
    put(world, "l", "only-left.txt", "l\n", 1_000);
    put(world, "l", "d/x.txt", "x\n", 1_000);
    put(world, "r", "d/x.txt", "x\n", 1_000);
    put(world, "l", "e/y.txt", "y\n", 1_000);
}

/// The menu of `area` as text.
fn menu(app: &TestApp, area: Area) -> String {
    let w = app.win();
    render_text(&[Menu {
        title: MenuTitle::Edit,
        items: app.0.pane_menu(w, area),
    }])
}

/// Selects the rows at `paths` (each on the sides it has).
fn select_rows(app: &mut TestApp, view: hikasa_session::ipc::ViewId, paths: &[&str]) {
    let nodes: Vec<_> = paths
        .iter()
        .map(|p| node_of(app, view, p).unwrap_or_else(|| panic!("no row {p}")))
        .collect();
    folder_model(app, view).with_selection(|sel, _, present| {
        sel.items = Default::default();
        for node in &nodes {
            sel.items.insert(*node, present(*node));
        }
        sel.current = nodes.first().copied();
        sel.moves += 1;
    });
}

/// The goldens of one world: every fixture selection, every area of the Folder Compare view; the sync panes.
fn goldens(world: &World, suffix: &str) {
    fixture(world);
    let mut app = world.app(None);
    let view = open(world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    let selections: [(&str, &[&str]); 4] = [
        ("one file pair", &["a.txt"]),
        ("an orphan", &["only-left.txt"]),
        ("a folder", &["d"]),
        ("two items", &["a.txt", "only-left.txt"]),
    ];
    let mut pane = String::new();
    let mut centre = String::new();
    for (name, paths) in selections {
        select_rows(&mut app, view, paths);
        for side in [Side::Left, Side::Right] {
            pane.push_str(&format!("## {name}, {side:?} pane\n"));
            pane.push_str(&menu(&app, Area::FolderPane(side)));
        }
        centre.push_str(&format!("## {name}\n"));
        centre.push_str(&menu(&app, Area::FolderCentre));
    }
    // A flat view.
    app.execute(CommandId::DirFlatten);
    world.settle(&mut app);
    select_rows(&mut app, view, &["a.txt"]);
    pane.push_str("## a flat view, Left pane\n");
    pane.push_str(&menu(&app, Area::FolderPane(Side::Left)));
    // A read-only side.
    let ro = open_with(world, &mut app, |o| o.read_only.right = true);
    world.settle(&mut app);
    select_rows(&mut app, ro, &["a.txt"]);
    for side in [Side::Left, Side::Right] {
        pane.push_str(&format!("## a read-only right side, {side:?} pane\n"));
        pane.push_str(&menu(&app, Area::FolderPane(side)));
    }
    crate::tests_shell::golden(&format!("menus/folder-pane{suffix}.txt"), &pane);
    crate::tests_shell::golden(&format!("menus/folder-centre{suffix}.txt"), &centre);
    crate::tests_shell::golden(
        &format!("menus/folder-header{suffix}.txt"),
        &menu(&app, Area::FolderHeader),
    );
    // The Folder Sync view's panes.
    let sync = crate::tests_sync_view::open_sync(world, &mut app.0, SyncOptions::default());
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    select_rows(&mut app, sync, &["a.txt"]);
    let mut text = String::new();
    for side in [Side::Left, Side::Right] {
        text.push_str(&format!("## one file pair, {side:?} pane\n"));
        text.push_str(&menu(&app, Area::SyncPane(side)));
    }
    crate::tests_shell::golden(&format!("menus/sync-pane{suffix}.txt"), &text);
}

#[test]
fn a9_21_the_pane_menus_per_area() {
    goldens(&World::new(), "");
    goldens(
        &World::with_files(&[("commands.toml", COMMANDS)]),
        "-custom",
    );
}

/// The invocations of a menu, flattened (submenus too).
fn invocations(items: &[MenuItem]) -> Vec<(String, Invocation, bool)> {
    let mut out = Vec::new();
    for item in items {
        match item {
            MenuItem::Command {
                invocation,
                caption,
                enabled,
                ..
            } => out.push((caption.clone(), invocation.clone(), *enabled)),
            MenuItem::Submenu { items, .. } => out.extend(invocations(items)),
            MenuItem::Separator => {}
        }
    }
    out
}

#[test]
fn a9_21_a_right_click_selects_first_and_entries_act_on_their_side() {
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "only-left.txt");
    // A right-click on another row's right side: that row on that side becomes the selection.
    let index = {
        let node = node_of(&mut app, view, "a.txt").expect("a.txt");
        folder_model(&mut app, view)
            .rows()
            .iter()
            .position(|r| r.node == node)
            .expect("shown")
    };
    folder_model(&mut app, view).context_press(index, Hit::Side(Side::Right));
    let a = node_of(&mut app, view, "a.txt").expect("a.txt");
    let m = folder_model(&mut app, view);
    assert_eq!(m.selection().items.len(), 1);
    assert_eq!(m.selection().items.get(a), SideSet::RIGHT);
    // A right-click inside the selection keeps it.
    select_rows(&mut app, view, &["a.txt", "only-left.txt"]);
    folder_model(&mut app, view).context_press(index, Hit::Side(Side::Left));
    assert_eq!(folder_model(&mut app, view).selection().items.len(), 2);
    // The side-specific entries carry the clicked pane.
    let w = app.win();
    let items = invocations(&app.0.pane_menu(w, Area::FolderPane(Side::Right)));
    for id in [
        CommandId::CopySide,
        CommandId::MoveSide,
        CommandId::Copy,
        CommandId::Paste,
    ] {
        let found = items
            .iter()
            .find(|(_, inv, _)| inv.id == id)
            .unwrap_or_else(|| panic!("{id:?} in the menu"));
        assert_eq!(found.1.arg, Some(CommandArg::Side(Side::Right)), "{id:?}");
    }
    // Copy to Other Side from the right pane copies the right item to the left.
    select_rows(&mut app, view, &["a.txt"]);
    let (_, copy_side, enabled) = invocations(&app.0.pane_menu(w, Area::FolderPane(Side::Right)))
        .into_iter()
        .find(|(_, inv, _)| inv.id == CommandId::CopySide)
        .expect("CopySide");
    assert!(enabled);
    app.0.execute(w, copy_side);
    world.settle(&mut app);
    // The preview's Yes to All, then any question that follows.
    for _ in 0..4 {
        let w = app.win();
        let asking = app
            .0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<crate::op_dialogs::PlanConfirmDialog>())
            .is_some();
        if asking {
            crate::tests_folder_ops::press(&world, &mut app, crate::op_dialogs::YES_TO_ALL);
        }
    }
    world.settle(&mut app);
    assert_eq!(
        crate::tests_folder_ops::read(&world, "l", "a.txt").as_deref(),
        Some("right a\n"),
        "the right side was copied to the left"
    );
}

fn launcher_world(launcher: Arc<RecordingLauncher>) -> World {
    World::with_init(move |init| {
        init.launcher = launcher as Arc<dyn hikasa_session::launcher::AppLauncher>
    })
}

fn explorer(app: &mut TestApp, world: &World, action: ExplorerAction, side: Side) {
    let w = app.win();
    app.0.execute(
        w,
        Invocation::with(
            CommandId::ExplorerList,
            CommandArg::Explorer(ExplorerItem {
                action,
                side: Some(side),
            }),
        ),
    );
    world.settle(app);
}

#[test]
fn a9_22_the_file_manager_entries() {
    let launcher = Arc::new(RecordingLauncher::default());
    let world = launcher_world(Arc::clone(&launcher));
    fixture(&world);
    let zip = crate::tests_shell::stored_zip(&[("m.txt", b"member\n")]);
    std::fs::write(world.path("work/l/z.zip"), &zip).expect("zip");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    // A file: itself selected in its folder; its folder opened.
    select(&mut app, view, "d/x.txt");
    explorer(&mut app, &world, ExplorerAction::ShowItem, Side::Right);
    explorer(&mut app, &world, ExplorerAction::OpenFolder, Side::Left);
    // A folder: opened itself.
    select(&mut app, view, "e");
    explorer(&mut app, &world, ExplorerAction::OpenFolder, Side::Left);
    // A member: its archive.
    select(&mut app, view, "z.zip");
    app.execute(CommandId::Open);
    world.settle(&mut app);
    app.execute(CommandId::ExpandAll);
    world.settle(&mut app);
    select(&mut app, view, "z.zip/m.txt");
    explorer(&mut app, &world, ExplorerAction::ShowItem, Side::Left);
    explorer(&mut app, &world, ExplorerAction::OpenFolder, Side::Left);
    assert_eq!(
        launcher.calls(),
        [
            Recorded::Reveal(world.path("work/r/d/x.txt")),
            Recorded::Associated(world.path("work/l/d")),
            Recorded::Associated(world.path("work/l/e")),
            Recorded::Reveal(world.path("work/l/z.zip")),
            Recorded::Associated(world.path("work/l")),
        ]
    );
    // The entries are enabled for these; a side without the item has them disabled.
    let w = app.win();
    select(&mut app, view, "e");
    let entries = |app: &TestApp, side| -> Vec<bool> {
        invocations(&app.0.pane_menu(w, Area::FolderPane(side)))
            .into_iter()
            .filter(|(_, inv, _)| inv.id == CommandId::ExplorerList)
            .map(|(_, _, enabled)| enabled)
            .collect()
    };
    assert_eq!(entries(&app, Side::Left), [true, true]);
    assert_eq!(
        entries(&app, Side::Right),
        [false, false],
        "e is on the left only"
    );
}

#[test]
fn a9_22_a_spec_off_the_disk_is_disabled_and_errors_are_notices() {
    // Off the disk: the pure targets refuse it, so the context says no.
    assert!(
        hikasa_session::api::file_manager_targets(&hikasa_core::Spec::Clipboard, false).is_none()
    );
    let mut ctx = crate::commands::Ctx {
        scope: crate::commands::Scope::Folder,
        row_selected: true,
        folder_pane: true,
        ..crate::commands::Ctx::default()
    };
    ctx.explorer = [false, false];
    let tables = crate::commands::CommandTables::default();
    let none = |_: CommandId| None;
    let mc = crate::menus::MenuContext {
        tables: &tables,
        scope: crate::commands::Scope::Folder,
        ctx,
        checked: &none,
        workspaces: &[],
        columns: &[],
        open_with: &[],
    };
    assert!(
        crate::menus::explorer_items(&mc, Some(Side::Left))
            .iter()
            .all(|i| matches!(i, MenuItem::Command { enabled: false, .. }))
    );
    // No launcher here: the core message is a notice.
    let world = World::new();
    fixture(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "a.txt");
    let before = notices(&app).len();
    explorer(&mut app, &world, ExplorerAction::ShowItem, Side::Left);
    assert_eq!(notices(&app).len(), before + 1);
}
