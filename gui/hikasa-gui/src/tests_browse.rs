//! M2 W9a acceptance A9.6: the browse dialog — Folder mode from a path edit (places with the bases, hidden entries
//! and the option, the filter, OK commits the field and runs, `mru.folders`), Esc, an archive opening as a folder and Up
//! out of its root, a listing error, File mode for the Run Script dialog (`mru.files`), SaveFile for the Export wizard
//! with the "Replace …?" question.

use hikasa_session::api::paths::PlaceKind;
use hikasa_session::ipc::ViewId;
use hikasa_session::settings::state::MruList;

use crate::browse::{BROWSE_DIALOG, BrowseDialog, BrowseMode};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel};
use crate::tests::{TestApp, World, folder_model};
use crate::tests_folder_ops::{open, press, put, top_title};

fn browse(app: &mut TestApp) -> &mut BrowseDialog {
    let w = app.win();
    app.0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<BrowseDialog>())
        .expect("the browse dialog")
}

/// Polls until the dialog's listing and places are in.
fn listed(world: &World, app: &mut TestApp) {
    let began = std::time::Instant::now();
    loop {
        world.settle(app);
        let service = world.service.clone();
        let d = browse(app);
        d.poll(&service);
        if !d.listing() && !d.places.is_empty() {
            return;
        }
        assert!(began.elapsed().as_secs() < 30, "the listing did not end");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

fn names(d: &BrowseDialog) -> Vec<String> {
    d.shown()
        .into_iter()
        .map(|i| d.entries[i].name.clone())
        .collect()
}

fn index(d: &BrowseDialog, name: &str) -> usize {
    d.entries.iter().position(|e| e.name == name).expect(name)
}

fn opened(world: &World) -> (TestApp, ViewId) {
    put(world, "l", "a.txt", "a\n", 1_000);
    put(world, "l", "sub/b.txt", "b\n", 1_000);
    put(world, "l", ".hidden/c.txt", "c\n", 1_000);
    put(world, "r", "a.txt", "a\n", 1_000);
    let zip = crate::tests_shell::stored_zip(&[("in/m.txt", b"member\n")]);
    std::fs::write(world.path("work/l/z.zip"), &zip).expect("zip");
    let mut app = world.app(None);
    let view = open(world, &mut app);
    (app, view)
}

#[test]
fn a9_6_folder_mode_from_a_path_edit() {
    let world = World::new();
    let (mut app, view) = opened(&world);
    app.execute(CommandId::BrowseLeft);
    assert_eq!(top_title(&app).as_deref(), Some("Browse for Folder"));
    listed(&world, &mut app);
    let d = browse(&mut app);
    assert_eq!(d.mode, BrowseMode::Folder);
    // Places: home, current, the root or drives, both bases, then the recent folders.
    let kinds: Vec<PlaceKind> = d.places.iter().map(|p| p.kind).collect();
    assert!(
        kinds.contains(&PlaceKind::Home) && kinds.contains(&PlaceKind::Current),
        "{kinds:?}"
    );
    assert_eq!(kinds.iter().filter(|k| **k == PlaceKind::Base).count(), 2);
    // Folders first; hidden entries off by default; the option and the filter.
    assert_eq!(names(d), ["sub", "a.txt", "z.zip"]);
    assert!(d.entries[index(d, "z.zip")].archive);
    d.show_hidden = true;
    assert_eq!(names(d), [".hidden", "sub", "a.txt", "z.zip"]);
    d.filter = "SU".into();
    assert_eq!(names(d), ["sub"]);
    d.filter.clear();
    // OK with a folder selected: the field takes it, accepts it, and the view runs on it.
    d.selected = Some(index(d, "sub"));
    press(&world, &mut app, ButtonRole::Ok);
    app.0.handle(crate::app::UiMsg::Tick);
    world.settle(&mut app);
    let left = folder_model(&mut app, view).path_edit(0).committed.clone();
    assert!(left.ends_with("sub"), "{left}");
    assert_eq!(app.0.state().mru(MruList::Folders), [left.clone()]);
    assert_eq!(app.0.state().mru(MruList::Left), [left]);
    assert_eq!(
        app.0.state().dialog_text(BROWSE_DIALOG, "show_hidden"),
        Some("true")
    );
    // Esc answers nothing; the option is kept.
    app.execute(CommandId::BrowseRight);
    listed(&world, &mut app);
    assert!(browse(&mut app).show_hidden, "remembered");
    crate::tests_folder_ops::key(&world, &mut app, "Esc");
    assert!(top_title(&app).is_none());
    assert_eq!(app.0.state().mru(MruList::Folders).len(), 1);
}

#[test]
fn a9_6_an_archive_opens_as_a_folder_and_up_leaves_it() {
    let world = World::new();
    let (mut app, _view) = opened(&world);
    app.execute(CommandId::BrowseLeft);
    listed(&world, &mut app);
    let service = world.service.clone();
    let left = browse(&mut app).location.clone();
    let i = index(browse(&mut app), "z.zip");
    browse(&mut app).open(&service, i);
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["in"]);
    let i = index(browse(&mut app), "in");
    browse(&mut app).open(&service, i);
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["m.txt"]);
    // Up: the archive's root, then out of it to its folder.
    browse(&mut app).up(&service);
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["in"]);
    browse(&mut app).up(&service);
    listed(&world, &mut app);
    let d = browse(&mut app);
    assert_eq!(d.location, left);
    assert!(names(d).contains(&"z.zip".to_owned()));
    // A listing error shows in the list.
    let missing = world.path("work/none").to_string_lossy().into_owned();
    d.go(&missing);
    listed(&world, &mut app);
    let d = browse(&mut app);
    assert!(d.entries.is_empty());
    assert!(d.error.is_some());
}

#[test]
fn a9_6_file_mode_for_run_script_and_save_file_for_export() {
    let world = World::new();
    let (mut app, _view) = opened(&world);
    let folder = world.path("work/l").to_string_lossy().into_owned();
    // Run Script ▸ Browse: File mode; OK takes the selected file into the script path; mru.files.
    app.execute(CommandId::RunScript);
    let w = app.win();
    app.0.open_browse(
        w,
        BrowseMode::File,
        folder.clone(),
        crate::browse::BrowseTarget::RunScript,
    );
    listed(&world, &mut app);
    assert_eq!(top_title(&app).as_deref(), Some("Browse for File"));
    // OK without a file is refused.
    press(&world, &mut app, ButtonRole::Ok);
    assert!(browse(&mut app).error.is_some());
    let d = browse(&mut app);
    d.selected = Some(index(d, "a.txt"));
    press(&world, &mut app, ButtonRole::Ok);
    let script = app
        .0
        .window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::script_dialog::RunScriptDialog>()
        })
        .expect("the Run Script dialog")
        .path
        .clone();
    assert!(script.ends_with("a.txt"), "{script}");
    assert_eq!(app.0.state().mru(MruList::Files), [script]);
    press(&world, &mut app, ButtonRole::Cancel);
    // Export ▸ Browse: SaveFile; an existing name asks "Replace …?"; No keeps the dialog, Yes answers.
    app.execute(CommandId::Export);
    app.0.open_browse(
        w,
        BrowseMode::SaveFile {
            default_name: "a.txt".into(),
        },
        folder,
        crate::browse::BrowseTarget::Export,
    );
    listed(&world, &mut app);
    assert_eq!(top_title(&app).as_deref(), Some("Save As"));
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(top_title(&app).as_deref(), Some("Replace"));
    press(&world, &mut app, ButtonRole::No);
    assert_eq!(top_title(&app).as_deref(), Some("Save As"));
    press(&world, &mut app, ButtonRole::Ok);
    press(&world, &mut app, ButtonRole::Yes);
    let target = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<crate::wizards::ExportWizard>())
        .expect("the Export wizard")
        .target
        .clone();
    assert!(target.ends_with("a.txt"), "{target}");
    // A new name answers at once.
    app.0.open_browse(
        w,
        BrowseMode::SaveFile {
            default_name: "new.hkpkg".into(),
        },
        world.path("work/l").to_string_lossy().into_owned(),
        crate::browse::BrowseTarget::Export,
    );
    listed(&world, &mut app);
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(top_title(&app).as_deref(), Some("Export Settings"));
}

/// Selects `name` in the listing and presses OK.
fn pick(world: &World, app: &mut TestApp, name: &str) {
    listed(world, app);
    let d = browse(app);
    d.selected = Some(index(d, name));
    press(world, app, ButtonRole::Ok);
}

#[test]
fn a9_6_home_specs_and_option_keys_take_the_answer() {
    let world = World::new();
    put(&world, "l", "sub/b.txt", "b\n", 1_000);
    put(&world, "r", "a.txt", "a\n", 1_000);
    let folder = world.path("work/l").to_string_lossy().into_owned();
    let mut app = world.app(None);
    let w = app.win();
    // Home: the panel's field, folder mode without a session.
    app.0.home_browse(w, 0);
    assert_eq!(browse(&mut app).mode, BrowseMode::Folder);
    browse(&mut app).go(&folder);
    pick(&world, &mut app, "sub");
    let left = match app.active_tab().and_then(|t| t.model()) {
        Some(crate::window::ViewModel::Home(h)) => h.panel.left.clone(),
        _ => panic!("home"),
    };
    assert!(left.ends_with("sub"), "{left}");
    // The Specs tab of Session Settings: its Browse button asks for a folder of a folder session.
    let view = open(&world, &mut app);
    app.0.execute(w, CommandId::SessionEdit);
    world.settle(&mut app);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::session_dialog::SessionSettingsDialog>()
        })
        .expect("Session Settings");
    d.specs_mut().browse = Some(1);
    let (side, start, folder_mode) = d.take_browse().expect("asked");
    assert_eq!((side, folder_mode), (1, true));
    app.0.open_browse(
        w,
        BrowseMode::Folder,
        start,
        crate::browse::BrowseTarget::Specs(1),
    );
    browse(&mut app).go(&folder);
    pick(&world, &mut app, "sub");
    let right = app
        .0
        .window_mut(w)
        .and_then(|w| {
            w.dialogs
                .find_mut::<crate::session_dialog::SessionSettingsDialog>()
        })
        .expect("Session Settings")
        .specs_mut()
        .right
        .clone();
    assert!(right.ends_with("sub"), "{right}");
    press(&world, &mut app, ButtonRole::Cancel);
    // An Options path key.
    app.execute(CommandId::ProgramOptions);
    world.settle(&mut app);
    let key = "tweaks.scripts.shared_scripts_folder";
    app.0.open_browse(
        w,
        BrowseMode::Folder,
        folder,
        crate::browse::BrowseTarget::OptionKey(key),
    );
    pick(&world, &mut app, "sub");
    let value = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<crate::options::OptionsDialog>())
        .and_then(|d| d.field(key).map(|f| f.value.clone()))
        .expect("the key");
    assert!(
        matches!(&value, hikasa_session::settings::form::FormValue::Text(t) if t.ends_with("sub")),
        "{value:?}"
    );
    let _ = view;
}
