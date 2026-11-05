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
    assert_eq!(
        app.0.state().mru(MruList::Folders),
        std::slice::from_ref(&left)
    );
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
    let left = browse(&mut app).location.clone();
    let i = index(browse(&mut app), "z.zip");
    browse(&mut app).open(i);
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["in"]);
    let i = index(browse(&mut app), "in");
    browse(&mut app).open(i);
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["m.txt"]);
    // Up: the archive's root, then out of it to its folder.
    browse(&mut app).up();
    listed(&world, &mut app);
    assert_eq!(names(browse(&mut app)), ["in"]);
    browse(&mut app).up();
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

// ---- after the W9a review ------------------------------------------------------------------------------------------

/// A9.6 places (review): Home and the current folder name the service's folders; the view's bases come before the
/// recent folders (mutation Y).
#[test]
fn a9_6_home_current_and_the_bases_before_the_recent() {
    let world = World::with_files(&[(
        "state.toml",
        "schema_version = 1\n\n[mru]\nfolders = [\"/recent/one\"]\n",
    )]);
    let (mut app, _view) = opened(&world);
    app.execute(CommandId::BrowseLeft);
    listed(&world, &mut app);
    let d = browse(&mut app);
    let spec_of = |kind: PlaceKind| {
        d.places
            .iter()
            .find(|p| p.kind == kind)
            .map(|p| std::path::PathBuf::from(&p.spec))
    };
    assert_eq!(spec_of(PlaceKind::Home), Some(world.path("home")));
    assert_eq!(spec_of(PlaceKind::Current), Some(world.path("work")));
    let at = |kind: PlaceKind| d.places.iter().position(|p| p.kind == kind);
    let last_base = d.places.iter().rposition(|p| p.kind == PlaceKind::Base);
    assert!(at(PlaceKind::Recent).is_some(), "{:?}", d.places);
    assert!(last_base < at(PlaceKind::Recent), "{:?}", d.places);
}

/// Mutation X: the filter is a case-insensitive substring.
#[test]
fn a9_6_the_filter_ignores_case() {
    let world = World::new();
    let (mut app, _view) = opened(&world);
    app.execute(CommandId::BrowseLeft);
    listed(&world, &mut app);
    let d = browse(&mut app);
    d.filter = "Z.ZI".into();
    assert_eq!(names(d), ["z.zip"]);
}

/// Review finding 9: `-` and `clipboard://` typed as the location are refused with a message; nothing is listed
/// (standard input and the clipboard are never read by the dialog).
#[test]
fn a9_6_input_specs_are_refused() {
    let world = World::new();
    let (mut app, _view) = opened(&world);
    app.execute(CommandId::BrowseLeft);
    listed(&world, &mut app);
    for typed in ["-", "clipboard://"] {
        browse(&mut app).go(typed);
        listed(&world, &mut app);
        let d = browse(&mut app);
        assert_eq!(
            d.error.as_deref(),
            Some(hikasa_session::api::paths::INPUT_SPEC_REFUSED),
            "{typed}"
        );
        assert!(d.entries.is_empty());
        // Up and OK have nothing to act on.
        d.up();
        assert_eq!(d.location, typed);
    }
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(
        top_title(&app).as_deref(),
        Some("Browse for Folder"),
        "OK refused"
    );
}

/// Review finding 11: SaveFile's question follows the platform's case rule, is asked again for another name, and
/// nothing is answered while a new folder is listed (the old entries are gone at once).
#[test]
fn a9_6_save_file_replace_follows_the_name_and_the_listing() {
    let world = World::new();
    let (mut app, _view) = opened(&world);
    let folder = world.path("work/l").to_string_lossy().into_owned();
    let w = app.win();
    app.execute(CommandId::Export);
    app.0.open_browse(
        w,
        BrowseMode::SaveFile {
            default_name: "A.TXT".into(),
        },
        folder.clone(),
        crate::browse::BrowseTarget::Export,
    );
    listed(&world, &mut app);
    let d = browse(&mut app);
    let folds = matches!(
        hikasa_session::Platform::HOST,
        hikasa_session::Platform::Windows | hikasa_session::Platform::MacOs
    );
    assert_eq!(d.same_name("a.txt", "A.TXT"), folds);
    assert!(d.same_name("a.txt", "a.txt"));
    // `a.txt` exists: confirmed for it, then the name changes to another existing one: asked again.
    d.name = "a.txt".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(top_title(&app).as_deref(), Some("Replace"));
    press(&world, &mut app, ButtonRole::No);
    let d = browse(&mut app);
    d.replace_asked = Some("a.txt".into());
    d.name = "z.zip".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(
        top_title(&app).as_deref(),
        Some("Replace"),
        "another name asks again"
    );
    press(&world, &mut app, ButtonRole::No);
    // Going to another folder: its entries are not known yet, so OK answers nothing.
    let d = browse(&mut app);
    d.go(&world.path("work/l/sub").to_string_lossy());
    assert!(d.entries.is_empty(), "the old entries went");
    assert!(!d.buttons()[0].enabled, "OK waits for the listing");
    d.name = "a.txt".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(top_title(&app).as_deref(), Some("Save As"));
    listed(&world, &mut app);
    press(&world, &mut app, ButtonRole::Ok);
    // `sub` holds only `b.txt`: a new name there answers.
    assert_eq!(top_title(&app).as_deref(), Some("Export Settings"));
}

/// Review finding 12: a name that is not valid Unicode is listed and marked, and never picked (no spec with U+FFFD).
#[cfg(target_os = "linux")]
#[test]
fn a9_6_a_lossy_name_is_shown_but_never_picked() {
    use std::os::unix::ffi::OsStrExt;
    let world = World::new();
    let (mut app, _view) = opened(&world);
    std::fs::write(
        world
            .path("work/l")
            .join(std::ffi::OsStr::from_bytes(b"bad\xff.txt")),
        b"x",
    )
    .expect("a lossy name");
    let folder = world.path("work/l").to_string_lossy().into_owned();
    let w = app.win();
    app.execute(CommandId::RunScript);
    app.0.open_browse(
        w,
        BrowseMode::File,
        folder,
        crate::browse::BrowseTarget::RunScript,
    );
    listed(&world, &mut app);
    let d = browse(&mut app);
    let i = d
        .entries
        .iter()
        .position(|e| e.name.starts_with("bad"))
        .expect("listed");
    assert!(d.entries[i].lossy && d.entries[i].name.contains('\u{fffd}'));
    assert_eq!(d.child(i), None, "no spec names it");
    assert_eq!(d.open(i), crate::dialog::DialogStep::Stay);
    assert_eq!(d.error.as_deref(), Some(crate::browse::LOSSY_NAME));
    d.selected = Some(i);
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(
        top_title(&app).as_deref(),
        Some("Browse for File"),
        "not picked"
    );
    assert!(app.0.state().mru(MruList::Files).is_empty());
}
