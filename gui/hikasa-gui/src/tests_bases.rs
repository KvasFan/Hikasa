//! M2 W9a acceptance A9.9 (W9a's rows): the bases table — Up One Level per side and both (availability; an archive
//! base's parent; a root has none), Set as Base Folders, Set as Base on Other Side, Compare Parent Folders from a folder
//! row and from a text view; Swap Sides of a folder and a text view; Clear Session gives an empty view that runs after
//! Accept. (`SyncBaseView`/`CompareBaseView` and the folder-sync Swap need the Folder Sync view: W9b.)

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_session::settings::state::MruList;

use crate::commands::{CommandId, Ctx, Predicate};
use crate::folder::selection::{Hit, Mods};
use crate::tests::{TestApp, World, folder_model, text_pair};
use crate::tests_folder_ops::{open, put, select};
use crate::window::ViewModel;

fn tree(world: &World) {
    put(world, "l", "d/x.txt", "left\n", 1_000);
    put(world, "r", "d/x.txt", "right\n", 1_000);
    put(world, "l", "e/y.txt", "y\n", 1_000);
    put(world, "r", "f/z.txt", "z\n", 1_000);
}

fn committed(app: &mut TestApp, view: ViewId) -> [String; 2] {
    let m = folder_model(app, view);
    [
        m.path_edit(0).committed.clone(),
        m.path_edit(1).committed.clone(),
    ]
}

fn run(world: &World, app: &mut TestApp, id: CommandId) {
    app.execute(id);
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(app);
    }
}

fn path(world: &World, rel: &str) -> String {
    world.path(rel).to_string_lossy().into_owned()
}

/// Clicks row `name` on `hit` (Ctrl adds).
fn click(app: &mut TestApp, view: ViewId, name: &str, hit: Hit, ctrl: bool) {
    let m = folder_model(app, view);
    let model = Arc::clone(m.prepared().expect("prepared").model());
    let index = {
        let read = model.read();
        m.rows()
            .iter()
            .position(|r| read.path(r.node).as_str() == name)
            .expect(name)
    };
    let mods = Mods {
        ctrl,
        ..Mods::default()
    };
    m.click(index, hit, mods);
}

#[test]
fn a9_9_up_one_level_per_side_and_both() {
    let world = World::new();
    tree(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert!(app.enabled(CommandId::UpOneLevelLeft) && app.enabled(CommandId::UpOneLevelBoth));
    run(&world, &mut app, CommandId::UpOneLevelLeft);
    let [left, right] = committed(&mut app, view);
    assert_eq!(left, path(&world, "work"));
    assert_eq!(right, path(&world, "work/r"));
    assert_eq!(app.0.state().mru(MruList::Left), [left]);
    run(&world, &mut app, CommandId::UpOneLevelBoth);
    let root = world
        .path("work")
        .parent()
        .expect("the test folder")
        .to_string_lossy()
        .into_owned();
    assert_eq!(committed(&mut app, view), [root, path(&world, "work")]);
    // A root has no parent: the command is off.
    let ctx = Ctx {
        base_parent: [false, true],
        idle: true,
        ..Ctx::default()
    };
    assert!(!Predicate::BaseParent(Side::Left).holds(&ctx));
    assert!(Predicate::BaseParent(Side::Right).holds(&ctx));
    assert!(
        !Predicate::BaseParent(Side::Center).holds(&ctx),
        "both need a parent"
    );
    // An operation under way: off (§1.9.2 rule 4).
    let busy = Ctx { idle: false, ..ctx };
    assert!(!Predicate::BaseParent(Side::Right).holds(&busy));
}

#[test]
fn a9_9_an_archive_base_goes_up_to_its_folder() {
    let world = World::new();
    put(&world, "r", "m.txt", "m\n", 1_000);
    let zip = crate::tests_shell::stored_zip(&[("m.txt", b"member\n")]);
    std::fs::create_dir_all(world.path("work/l")).expect("mkdir");
    std::fs::write(world.path("work/l/z.zip"), &zip).expect("zip");
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let zip_spec = path(&world, "work/l/z.zip");
    folder_model(&mut app, view).path_edit(0).text = zip_spec.clone();
    folder_model(&mut app, view).accept_paths();
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
    }
    assert_eq!(committed(&mut app, view)[0], zip_spec);
    let parents = folder_model(&mut app, view).base_parents();
    assert_eq!(parents[0].as_deref(), Some(path(&world, "work/l").as_str()));
}

#[test]
fn a9_9_set_as_base_folders_and_on_the_other_side() {
    let world = World::new();
    tree(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    // A file row: neither.
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    select(&mut app, view, "d/x.txt");
    assert!(!app.enabled(CommandId::SetAsBaseFolders));
    assert!(!app.enabled(CommandId::SetAsOtherBaseFolder));
    // One folder row on both sides.
    select(&mut app, view, "d");
    assert!(app.enabled(CommandId::SetAsBaseFolders));
    assert!(
        !app.enabled(CommandId::SetAsOtherBaseFolder),
        "both sides selected"
    );
    run(&world, &mut app, CommandId::SetAsBaseFolders);
    assert_eq!(
        committed(&mut app, view),
        [path(&world, "work/l/d"), path(&world, "work/r/d")]
    );
    // One folder on each side: e on the left, f on the right.
    run(&world, &mut app, CommandId::UpOneLevelBoth);
    click(&mut app, view, "e", Hit::Side(Side::Left), false);
    click(&mut app, view, "f", Hit::Side(Side::Right), true);
    assert!(app.enabled(CommandId::SetAsBaseFolders));
    run(&world, &mut app, CommandId::SetAsBaseFolders);
    assert_eq!(
        committed(&mut app, view),
        [path(&world, "work/l/e"), path(&world, "work/r/f")]
    );
    // One folder on one side becomes the other side's base.
    run(&world, &mut app, CommandId::UpOneLevelBoth);
    click(&mut app, view, "e", Hit::Side(Side::Left), false);
    assert!(app.enabled(CommandId::SetAsOtherBaseFolder));
    run(&world, &mut app, CommandId::SetAsOtherBaseFolder);
    assert_eq!(
        committed(&mut app, view),
        [path(&world, "work/l"), path(&world, "work/l/e")]
    );
}

#[test]
fn a9_9_compare_parent_folders_from_a_row_and_from_a_text_view() {
    let world = World::new();
    tree(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    select(&mut app, view, "d");
    app.execute(CommandId::Expand);
    world.settle(&mut app);
    select(&mut app, view, "d/x.txt");
    assert!(app.enabled(CommandId::CompareParents));
    let before = app.tabs().len();
    run(&world, &mut app, CommandId::CompareParents);
    assert_eq!(app.tabs().len(), before + 1);
    let new = app.tabs()[app.active()].id;
    assert_eq!(
        committed(&mut app, new),
        [path(&world, "work/l/d"), path(&world, "work/r/d")]
    );
    // A text view: the folders of its two files.
    let (l, r) = text_pair(&world);
    let text = world.open(&mut app.0, &l, &r);
    world.settle(&mut app);
    assert!(app.enabled(CommandId::CompareParents));
    let before = app.tabs().len();
    run(&world, &mut app, CommandId::CompareParents);
    assert_eq!(app.tabs().len(), before + 1);
    let new = app.tabs()[app.active()].id;
    let folders =
        [l.parent(), r.parent()].map(|p| p.expect("a parent").to_string_lossy().into_owned());
    assert_eq!(committed(&mut app, new), folders);
    let _ = text;
}

#[test]
fn a9_9_swap_sides_of_a_folder_and_a_text_view() {
    let world = World::new();
    tree(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    let [l, r] = committed(&mut app, view);
    run(&world, &mut app, CommandId::Swap);
    assert_eq!(committed(&mut app, view), [r.clone(), l.clone()]);
    let m = folder_model(&mut app, view);
    assert!(
        m.rows().iter().any(|row| {
            m.prepared()
                .is_some_and(|p| p.model().read().path(row.node).as_str() == "f")
        }),
        "it ran again on the swapped sides"
    );
    // A text view.
    let (tl, tr) = text_pair(&world);
    let text = world.open(&mut app.0, &tl, &tr);
    world.settle(&mut app);
    run(&world, &mut app, CommandId::Swap);
    let Some(ViewModel::Text(m)) = app.0.tab(text).and_then(|t| t.model()) else {
        panic!("a text view");
    };
    assert_eq!(m.request().left, tr.into_os_string());
    assert_eq!(m.request().right, tl.into_os_string());
}

#[test]
fn a9_9_clear_session_gives_an_empty_view_that_runs_after_accept() {
    let world = World::new();
    tree(&world);
    let mut app = world.app(None);
    let view = open(&world, &mut app);
    assert!(app.enabled(CommandId::SessionClear));
    run(&world, &mut app, CommandId::SessionClear);
    assert_eq!(committed(&mut app, view), [String::new(), String::new()]);
    let m = folder_model(&mut app, view);
    assert!(m.prepared().is_none(), "no comparison");
    assert!(!m.busy());
    // The view's session changed (mutation AI: a cleared session not marked dirty).
    assert!(
        app.view_session(view).is_some_and(|s| s.dirty),
        "the cleared session is dirty"
    );
    // Clear Session again on the empty view: nothing happens, no message.
    let notices = crate::tests_folder_ops::notices(&app).len();
    run(&world, &mut app, CommandId::SessionClear);
    assert_eq!(
        crate::tests_folder_ops::notices(&app).len(),
        notices,
        "no error"
    );
    assert_eq!(committed(&mut app, view), [String::new(), String::new()]);
    // Accept with both fields typed runs it.
    let m = folder_model(&mut app, view);
    m.path_edit(0).text = path(&world, "work/l");
    m.path_edit(1).text = path(&world, "work/r");
    m.accept_paths();
    for _ in 0..3 {
        app.0.handle(crate::app::UiMsg::Tick);
        world.settle(&mut app);
    }
    let m = folder_model(&mut app, view);
    assert!(m.prepared().is_some(), "it ran");
    assert_eq!(
        committed(&mut app, view),
        [path(&world, "work/l"), path(&world, "work/r")]
    );
}
