//! M2 W4b acceptance (§6.4 A4.11–A4.15) through the toolkit-free models (§1.4.16): sessions of views, the Session
//! Settings dialog, Home, workspaces, the command line. No window opens.

use std::path::Path;

use hikasa_session::clipboard::FixedClipboard;
use hikasa_session::ipc::{CloseReason, IpcClient, IpcServer, Request, Response, ViewId};
use hikasa_session::sessions::{
    AutoId, ParentLink, Scope as RuleScope, Session, SessionName, SessionRef, SessionRules,
    SessionType,
};
use hikasa_session::settings::Shortcut;
use hikasa_session::workspaces::Workspace;
use std::sync::Arc;

use crate::app::{Effect, OpenHow, OpenTarget, PanelAction, Startup, UiMsg};
use crate::commands::{CommandId, FocusArea, Scope};
use crate::dialog::{ButtonRole, Prompt};
use crate::dnd::{DropTarget, FieldId};
use crate::home::{HomeModel, PanelMode, PanelTarget};
use crate::ipc::GuiHandler;
use crate::menus::{MenuContext, render_text, tree_menu};
use crate::session_dialog::{
    EditTarget, NameDialog, SaveSessionDialog, SaveSource, SessionSettingsDialog, SessionTab,
};
use crate::tests::{TestApp, World, folder_pair, text_pair};
use crate::view_session::{ViewOrigin, ViewSession};
use crate::window::{CommandArg, Invocation, ViewModel, WindowId};
use crate::workspace::{
    MANAGE_DELETE, MANAGE_RENAME, MANAGE_SHORTCUT, ManageWorkspacesDialog, SaveWorkspaceDialog,
};

fn r(text: &str) -> SessionRef {
    SessionRef::parse(text).expect("a reference")
}

fn n(text: &str) -> SessionName {
    SessionName::parse(text).expect("a name")
}

fn golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    if std::env::var_os("HIKASA_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("mkdir");
        std::fs::write(&path, actual).expect("bless");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (bless with HIKASA_BLESS=1)", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "{name}");
}

fn text_session(world: &World) -> Session {
    let (l, right) = text_pair(world);
    let mut s = Session::builtin(SessionType::TextCompare);
    s.specs.left = l.display().to_string();
    s.specs.right = right.display().to_string();
    s
}

fn folder_session(world: &World) -> Session {
    let (a, b) = folder_pair(world);
    let mut s = Session::builtin(SessionType::FolderCompare);
    s.specs.left = a.display().to_string();
    s.specs.right = b.display().to_string();
    s
}

fn save(world: &World, reference: &str, session: &Session) {
    world
        .service
        .session_save(&r(reference), session, false)
        .expect("save");
}

fn bytes(world: &World, reference: &str) -> Vec<u8> {
    std::fs::read(world.service.session_store().path(&r(reference))).expect("read")
}

fn autosaved(world: &World) -> Vec<String> {
    world
        .service
        .session_list(None, true, false)
        .expect("list")
        .autosaved
        .into_iter()
        .map(|a| a.id)
        .collect()
}

/// A world with an app; the catalog is read.
fn started(world: &World) -> TestApp {
    let mut app = world.app(None);
    world.settle(&mut app);
    app
}

fn refresh(world: &World, app: &mut TestApp) {
    app.refresh_sessions();
    world.settle(app);
}

/// Opens a named session in a new tab; returns its view.
fn open_named(world: &World, app: &mut TestApp, reference: &str) -> ViewId {
    let target = OpenTarget::Window(app.win());
    app.0.open_named(r(reference), OpenHow::Target(target));
    world.settle(app);
    app.active_tab().expect("a tab").id
}

fn vs(app: &TestApp, view: ViewId) -> ViewSession {
    app.view_session(view).expect("a session").clone()
}

/// A named view whose specs differ from the file (through the dialog's path).
fn make_dirty(world: &World, app: &mut TestApp, view: ViewId) {
    let (_, other) = text_pair(world);
    let copy = world.write("other.c", &std::fs::read_to_string(&other).expect("read"));
    app.0.execute(app.win(), CommandId::SessionEdit);
    world.settle(app);
    let w = app.win();
    let d = settings_dialog(app, w);
    d.specs_mut().right = copy.display().to_string();
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(app);
    assert!(vs(app, view).dirty, "the view is dirty now");
}

fn settings_dialog(app: &mut TestApp, w: WindowId) -> &mut SessionSettingsDialog {
    app.window_mut(w)
        .expect("window")
        .dialogs
        .top_mut()
        .expect("a dialog")
        .as_any_mut()
        .downcast_mut()
        .expect("the settings dialog")
}

fn prompt(app: &TestApp) -> Option<String> {
    let top = app.window(app.win())?.dialogs.top()?;
    top.as_any()
        .downcast_ref::<Prompt>()
        .map(|p| p.message.clone())
}

fn press(world: &World, app: &mut TestApp, role: ButtonRole) -> Vec<Effect> {
    let w = app.win();
    let effects = app.0.dialog_button(w, role);
    world.settle(app);
    effects
}

/// One way to close a view; `done` says the close happened.
#[derive(Clone, Copy, Debug)]
enum Way {
    Tab,
    Window,
    Exit,
    Load,
    Home,
    Combo,
}

const WAYS: [Way; 6] = [
    Way::Tab,
    Way::Window,
    Way::Exit,
    Way::Load,
    Way::Home,
    Way::Combo,
];

/// Runs the close `way` in a fresh world with one view of `reference`, returning the world and app; the two last
/// tabs of each world keep the window alive where the way needs it.
fn run_way(world: &World, app: &mut TestApp, way: Way) {
    let w = app.win();
    match way {
        Way::Tab => {
            app.0.execute(w, CommandId::CloseTab);
        }
        Way::Window => {
            app.0.execute(w, CommandId::WindowNew);
            app.0.focus(w);
            app.0.close_window(w);
        }
        Way::Exit => {
            app.0.execute(w, CommandId::Exit);
        }
        Way::Load => {
            world.write("config/workspaces/empty.toml", "schema_version = 1\n");
            app.0.execute(
                w,
                Invocation::with(
                    CommandId::WorkspaceLoadList,
                    CommandArg::Workspace(n("empty")),
                ),
            );
        }
        Way::Home => {
            app.0.execute(w, CommandId::SessionHome);
        }
        Way::Combo => {
            app.0.execute(
                w,
                Invocation::with(CommandId::SessionsCombo, CommandArg::Session(r("other"))),
            );
        }
    }
    world.settle(app);
}

fn way_world(view_text: bool) -> (World, TestApp, ViewId) {
    let world = World::new();
    let session = text_session(&world);
    save(&world, "main", &session);
    save(&world, "other", &session);
    let mut app = started(&world);
    let view = if view_text {
        open_named(&world, &mut app, "main")
    } else {
        ViewId(0)
    };
    (world, app, view)
}

/// A4.11, D-84: AskSaveChanges through every close path: Cancel stops it, No writes nothing, Yes saves.
#[test]
fn dirty_named_sessions_ask_on_every_close_path() {
    for way in WAYS {
        // Cancel: nothing closed, nothing written.
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        let before = bytes(&world, "main");
        run_way(&world, &mut app, way);
        assert_eq!(
            prompt(&app).as_deref(),
            Some("Save changes to session?"),
            "{way:?}"
        );
        press(&world, &mut app, ButtonRole::Cancel);
        assert!(app.tab(view).is_some(), "{way:?}: Cancel keeps the view");
        assert!(!app.quitting(), "{way:?}");
        assert_eq!(bytes(&world, "main"), before, "{way:?}");

        // No: the file stays.
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        let before = bytes(&world, "main");
        run_way(&world, &mut app, way);
        press(&world, &mut app, ButtonRole::No);
        assert!(app.tab(view).is_none(), "{way:?}: No closes");
        assert_eq!(bytes(&world, "main"), before, "{way:?}");

        // Yes: saved.
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        let before = bytes(&world, "main");
        run_way(&world, &mut app, way);
        press(&world, &mut app, ButtonRole::Yes);
        assert!(app.tab(view).is_none(), "{way:?}: Yes closes");
        assert_ne!(bytes(&world, "main"), before, "{way:?}");
    }
}

/// A4.11, D-84: the rows without a question, through every close path.
#[test]
fn clean_named_locked_and_unnamed_views_close_silently() {
    for way in WAYS {
        // Named, clean: only `view`/`display` are written, and only when they changed.
        let (world, mut app, view) = way_world(true);
        crate::tests::text_model(&mut app, view).execute(&world.service, CommandId::ShowThumbnail);
        run_way(&world, &mut app, way);
        assert_eq!(prompt(&app), None, "{way:?}");
        let saved = world.service.session_load(&r("main")).expect("load").value;
        assert_eq!(
            saved.view.get("thumbnail").and_then(|v| v.as_bool()),
            Some(false),
            "{way:?}"
        );

        // Named, locked, no explicit save: nothing.
        let (world, mut app, view) = way_world(true);
        world.service.session_lock(&r("main"), true).expect("lock");
        let w = app.win();
        app.0.execute(w, CommandId::CloseTab);
        world.settle(&mut app);
        let _ = view;
        let view = open_named(&world, &mut app, "main");
        assert!(vs(&app, view).locked);
        make_dirty(&world, &mut app, view);
        let before = bytes(&world, "main");
        run_way(&world, &mut app, way);
        assert_eq!(prompt(&app), None, "{way:?}");
        assert_eq!(bytes(&world, "main"), before, "{way:?}");

        // Unnamed: auto-saved.
        let (world, mut app, _) = way_world(false);
        let (l, right) = text_pair(&world);
        world.open(&mut app, &l, &right);
        world.settle(&mut app);
        assert!(autosaved(&world).is_empty());
        run_way(&world, &mut app, way);
        assert_eq!(autosaved(&world).len(), 1, "{way:?}");
    }
}

/// A4.11, D-84: the locked row with an explicit save.
#[test]
fn explicit_save_on_a_locked_session_asks_at_close() {
    let (world, mut app, _) = way_world(false);
    world.service.session_lock(&r("main"), true).expect("lock");
    let view = open_named(&world, &mut app, "main");
    make_dirty(&world, &mut app, view);
    let before = bytes(&world, "main");
    let w = app.win();
    // Save asks; No leaves the wish standing.
    app.0.execute(w, CommandId::SessionSave);
    assert_eq!(
        prompt(&app).as_deref(),
        Some("Save changes to locked session?")
    );
    press(&world, &mut app, ButtonRole::No);
    assert_eq!(bytes(&world, "main"), before);
    assert!(vs(&app, view).explicit_save);
    app.0.execute(w, CommandId::CloseTab);
    assert_eq!(
        prompt(&app).as_deref(),
        Some("Save changes to locked session?")
    );
    press(&world, &mut app, ButtonRole::Yes);
    assert_ne!(
        bytes(&world, "main"),
        before,
        "force writes the locked file"
    );
    assert!(app.tab(view).is_none());
    assert!(
        world
            .service
            .session_load(&r("main"))
            .expect("load")
            .value
            .locked
    );
}

/// A4.11, D-120: closes that cannot ask answer No.
#[test]
fn ipc_close_and_shutdown_answer_no() {
    let (world, mut app, view) = way_world(true);
    make_dirty(&world, &mut app, view);
    let before = bytes(&world, "main");
    let (l, right) = text_pair(&world);
    let unnamed = world.open(&mut app, &l, &right);
    world.settle(&mut app);
    app.handle(UiMsg::Close { view });
    assert_eq!(prompt(&app), None);
    world.settle(&mut app);
    assert!(app.tab(view).is_none());
    assert_eq!(bytes(&world, "main"), before, "the dirty file is unchanged");
    assert!(autosaved(&world).is_empty(), "no save of the named view");
    app.handle(UiMsg::Close { view: unnamed });
    world.settle(&mut app);
    assert_eq!(autosaved(&world).len(), 1, "the unnamed view is auto-saved");

    let (world, mut app, view) = way_world(true);
    make_dirty(&world, &mut app, view);
    let before = bytes(&world, "main");
    let effects = app.handle(UiMsg::Quit);
    assert!(effects.contains(&Effect::Exit));
    app.flush(std::time::Duration::from_secs(5));
    assert_eq!(bytes(&world, "main"), before);
}

/// A4.11: `SessionSave` on named and unnamed views, `SessionLocked`, Save As and its slot.
#[test]
fn save_lock_and_save_as() {
    let (world, mut app, view) = way_world(true);
    let w = app.win();
    make_dirty(&world, &mut app, view);
    let before = bytes(&world, "main");
    app.0.execute(w, CommandId::SessionSave);
    world.settle(&mut app);
    assert_ne!(bytes(&world, "main"), before);
    assert!(!vs(&app, view).dirty);

    // Locked toggles the file.
    assert_eq!(app.0.checked(w, CommandId::SessionLocked), Some(false));
    app.0.execute(w, CommandId::SessionLocked);
    world.settle(&mut app);
    assert!(world.service.session_load(&r("main")).unwrap().value.locked);
    assert_eq!(app.0.checked(w, CommandId::SessionLocked), Some(true));
    app.0.execute(w, CommandId::SessionLocked);
    world.settle(&mut app);
    assert!(!world.service.session_load(&r("main")).unwrap().value.locked);

    // Unnamed: an auto-save gives a slot; Save opens Save As; the slot goes.
    let (l, right) = text_pair(&world);
    let view = world.open(&mut app, &l, &right);
    world.settle(&mut app);
    assert!(!app.0.enabled(w, CommandId::SessionLocked), "named only");
    app.set_now(400.0);
    if let Some(s) = app.tab_mut(view).and_then(|t| t.session.as_mut()) {
        s.next_autosave_at = 300.0;
    }
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert_eq!(autosaved(&world).len(), 1);
    assert!(app.0.view_slot(view).is_some());
    app.0.execute(w, CommandId::SessionSave);
    let d: &mut SaveSessionDialog = top_dialog(&mut app, w);
    assert_eq!(d.source, SaveSource::View(view));
    d.name = "kept".into();
    d.new_folder = "work".into();
    press(&world, &mut app, ButtonRole::Ok);
    let s = vs(&app, view);
    assert_eq!(s.origin, ViewOrigin::Named(r("work/kept")));
    assert_eq!(app.0.view_slot(view), None);
    assert!(autosaved(&world).is_empty(), "Save As deletes the slot");
    assert_eq!(app.tab(view).unwrap().caption(), "work/kept");
    // Over an existing name: a prompt first.
    refresh(&world, &mut app);
    app.0.execute(w, CommandId::SessionSaveAs);
    top_dialog::<SaveSessionDialog>(&mut app, w).name = "MAIN".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(
        prompt(&app).as_deref(),
        Some("The session MAIN exists. Replace it?")
    );
    press(&world, &mut app, ButtonRole::Cancel);
    assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("work/kept")));
}

fn top_dialog<T: 'static>(app: &mut TestApp, w: WindowId) -> &mut T {
    app.window_mut(w)
        .expect("window")
        .dialogs
        .top_mut()
        .expect("a dialog")
        .as_any_mut()
        .downcast_mut()
        .expect("the dialog")
}

/// A4.11, D-121: the timer with explicit times.
#[test]
fn the_autosave_timer() {
    let world = World::new();
    let mut app = started(&world);
    let (l, right) = text_pair(&world);
    app.set_now(10.0);
    let view = world.open(&mut app, &l, &right);
    world.settle(&mut app);
    // No save before 300 s.
    app.set_now(309.0);
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert!(autosaved(&world).is_empty());
    // After 300 s: a first save.
    app.set_now(311.0);
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert_eq!(autosaved(&world).len(), 1);
    let id = AutoId::parse(&autosaved(&world)[0]).unwrap();
    let path = world.service.session_store().autosaved_path(&id);
    let first = std::fs::read(&path).unwrap();
    // No change: no save.
    app.set_now(700.0);
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), first);
    // A change of the display filter: saved in the same slot.
    crate::tests::text_model(&mut app, view).execute(&world.service, CommandId::FavDiff);
    app.set_now(1100.0);
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert_eq!(autosaved(&world).len(), 1, "the same slot");
    assert_ne!(std::fs::read(&path).unwrap(), first);

    // Never a named view.
    let (world, mut app, view) = way_world(true);
    if let Some(s) = app.tab_mut(view).and_then(|t| t.session.as_mut()) {
        s.next_autosave_at = 0.0;
    }
    app.set_now(1000.0);
    app.handle(UiMsg::Tick);
    world.settle(&mut app);
    assert!(autosaved(&world).is_empty());
}

/// A4.11: pruning to `max_auto_saved_sessions`; `0` = none.
#[test]
fn autosaves_are_pruned() {
    let options = "schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = 2\n";
    let world = World::with_files(&[("options.toml", options)]);
    let mut app = started(&world);
    let (l, right) = text_pair(&world);
    for _ in 0..4 {
        let view = world.open(&mut app, &l, &right);
        world.settle(&mut app);
        let w = app.win();
        let index = app.window(w).unwrap().index_of(view).unwrap();
        app.0.close_tab(w, index, CloseReason::User);
        world.settle(&mut app);
    }
    assert_eq!(autosaved(&world).len(), 2);

    let options = "schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = 0\n";
    let world = World::with_files(&[("options.toml", options)]);
    let mut app = started(&world);
    let (l, right) = text_pair(&world);
    let view = world.open(&mut app, &l, &right);
    world.settle(&mut app);
    assert!(!vs(&app, view).timer_due(1e9, false));
    let w = app.win();
    app.0.execute(w, CommandId::CloseTab);
    world.settle(&mut app);
    assert!(autosaved(&world).is_empty());
}

/// Selects a file pair of a folder view.
fn select_pair(app: &mut TestApp, view: ViewId) {
    let w = app.win();
    app.0.execute(w, CommandId::ExpandAll);
    let rows = crate::tests::folder_model(app, view);
    let pair = (0..rows.rows().len()).find(|i| {
        rows.select(*i);
        rows.open_request().is_some()
    });
    assert!(pair.is_some(), "the fixture has a pair");
}

/// A4.11, FC-062: a child with the parent link.
#[test]
fn opening_a_pair_gives_a_child_of_the_folder_session() {
    let world = World::new();
    save(&world, "tree", &folder_session(&world));
    let mut app = started(&world);
    let view = open_named(&world, &mut app, "tree");
    let w = app.win();
    select_pair(&mut app, view);
    app.0.execute(w, CommandId::Open);
    world.settle(&mut app);
    let child = app.active_tab().unwrap().id;
    assert_ne!(child, view);
    let s = vs(&app, child);
    assert_eq!(s.origin, ViewOrigin::Unnamed);
    let link = s.parent.expect("a parent link");
    assert_eq!(link.session, "tree");
    assert_eq!(
        (link.left.as_str(), link.right.as_str()),
        ("src/main.c", "src/main.c")
    );
    // An Unnamed folder view without a slot has no file to link to.
    let (a, b) = folder_pair(&world);
    let view = world.open(&mut app, &a, &b);
    world.settle(&mut app);
    select_pair(&mut app, view);
    app.0.execute(w, CommandId::Open);
    world.settle(&mut app);
    assert!(vs(&app, app.active_tab().unwrap().id).parent.is_none());
}

// ---- A4.12 ---------------------------------------------------------------------------------------------------

/// A rule tab of a later wave.
struct RuleTab {
    rules: Option<SessionRules>,
}

impl SessionTab for RuleTab {
    fn title(&self) -> String {
        "Rules".to_owned()
    }
    fn changes_rules(&self) -> bool {
        self.rules.is_some()
    }
    fn rules(&self, _current: &SessionRules) -> Option<SessionRules> {
        self.rules.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn changed_rules() -> SessionRules {
    let mut rules = hikasa_text::TextRules::default();
    rules.alignment.skew_tolerance = 77;
    SessionRules::Text(rules)
}

#[test]
fn the_specs_tab() {
    let (world, mut app, view) = way_world(true);
    let w = app.win();
    app.0.execute(w, CommandId::SessionEdit);
    world.settle(&mut app);
    let d = settings_dialog(&mut app, w);
    assert_eq!(d.help_topic_for_test(), "dialog-session-settings");
    let specs = d.specs();
    assert!(specs.file_type && !specs.specs_disabled);
    assert!(!specs.formats.is_empty(), "format_list");
    // An invalid encoding is refused.
    d.specs_mut().left_encoding = "no-such-encoding".into();
    press(&world, &mut app, ButtonRole::Ok);
    let d = settings_dialog(&mut app, w);
    assert!(d.error.as_ref().unwrap().contains("no-such-encoding"));
    let (_, other) = text_pair(&world);
    let copy = world.write("other.c", &std::fs::read_to_string(&other).unwrap());
    let d = settings_dialog(&mut app, w);
    d.specs_mut().left_encoding = "utf-8".into();
    d.specs_mut().right = copy.display().to_string();
    d.specs_mut().description = "new words".into();
    press(&world, &mut app, ButtonRole::Ok);
    // OK reopens the view with the new specs; the origin stays; it is dirty from then on.
    let s = vs(&app, view);
    assert_eq!(s.origin, ViewOrigin::Named(r("main")));
    assert!(s.dirty);
    let m = crate::tests::text_model(&mut app, view);
    assert_eq!(std::path::PathBuf::from(&m.request().right), copy);
    // MRU pushed.
    assert_eq!(app.state().mru.right[0], copy.display().to_string());
    // Cancel discards.
    app.0.execute(w, CommandId::SessionEdit);
    world.settle(&mut app);
    settings_dialog(&mut app, w).specs_mut().right = "/no".into();
    press(&world, &mut app, ButtonRole::Cancel);
    assert_eq!(app.window(w).unwrap().dialogs.len(), 0);
    let m = crate::tests::text_model(&mut app, view);
    assert_eq!(std::path::PathBuf::from(&m.request().right), copy);
}

#[test]
fn defaults_entries_have_specs_disabled() {
    let world = World::new();
    let mut app = started(&world);
    let w = app.win();
    if let Some(ViewModel::Home(h)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        assert!(h.select_key("new:text-compare"));
    }
    app.0.execute(w, CommandId::TreeEdit);
    world.settle(&mut app);
    let d = settings_dialog(&mut app, w);
    assert_eq!(d.target, EditTarget::Defaults(SessionType::TextCompare));
    assert!(d.specs().specs_disabled);
    d.specs_mut().description = "typed".into();
    d.specs_mut().left = "/ignored".into();
    press(&world, &mut app, ButtonRole::Ok);
    let defaults = world
        .service
        .session_defaults(SessionType::TextCompare)
        .unwrap();
    assert_eq!(defaults.description, "typed");
    assert_eq!(defaults.specs.left, "", "specs never go to the defaults");
}

/// §1.4.10: the availability of the scopes.
#[test]
fn scope_availability() {
    let world = World::new();
    let session = Session::builtin(SessionType::TextCompare);
    let dialog = |folder_parent: bool, parent: Option<String>| {
        SessionSettingsDialog::new(
            EditTarget::View(ViewId(1)),
            session.clone(),
            folder_parent,
            parent,
            [Vec::new(), Vec::new()],
            Vec::new(),
        )
    };
    let _ = &world;
    let mut none = dialog(false, None);
    let mut named = dialog(true, Some("p".into()));
    let mut auto = dialog(true, Some("_autosaved/20260101-000000-1".into()));
    let mut text_parent = dialog(false, Some("t".into()));
    for d in [&none, &named, &auto, &text_parent] {
        assert!(d.scope_available(RuleScope::View));
        for s in [RuleScope::Defaults, RuleScope::Pair, RuleScope::Type] {
            assert!(!d.scope_available(s), "nothing changed");
        }
    }
    for d in [&mut none, &mut named, &mut auto, &mut text_parent] {
        d.tabs.push(Box::new(RuleTab {
            rules: Some(changed_rules()),
        }));
    }
    assert!(none.scope_available(RuleScope::Defaults));
    assert!(!none.scope_available(RuleScope::Pair));
    assert!(!text_parent.scope_available(RuleScope::Type));
    for d in [&named, &auto] {
        assert!(d.scope_available(RuleScope::Pair) && d.scope_available(RuleScope::Type));
    }
}

/// A4.12: each scope writes what `apply_scope` writes.
#[test]
fn a_rule_tab_writes_through_its_scope() {
    for scope in [
        RuleScope::View,
        RuleScope::Defaults,
        RuleScope::Pair,
        RuleScope::Type,
    ] {
        let world = World::new();
        save(&world, "tree", &folder_session(&world));
        let mut app = started(&world);
        let (l, right) = text_pair(&world);
        let view = world.open(&mut app, &l, &right);
        world.settle(&mut app);
        let link = ParentLink {
            session: "tree".into(),
            left: "a.c".into(),
            right: "a.c".into(),
        };
        if let Some(s) = app.tab_mut(view).and_then(|t| t.session.as_mut()) {
            s.parent = Some(link);
        }
        let w = app.win();
        app.0.execute(w, CommandId::SessionEdit);
        world.settle(&mut app);
        let d = settings_dialog(&mut app, w);
        assert!(d.folder_parent);
        d.tabs.push(Box::new(RuleTab {
            rules: Some(changed_rules()),
        }));
        d.scope = scope;
        press(&world, &mut app, ButtonRole::Ok);
        let parent = world.service.session_load(&r("tree")).unwrap().value;
        let defaults = world
            .service
            .session_defaults(SessionType::TextCompare)
            .unwrap();
        match scope {
            RuleScope::View => {
                assert!(parent.overrides.is_empty());
                assert_ne!(defaults.rules, changed_rules());
            }
            RuleScope::Defaults => assert_eq!(defaults.rules, changed_rules()),
            RuleScope::Pair | RuleScope::Type => assert_eq!(parent.overrides.len(), 1),
        }
        // The view re-ran with the rules.
        let m = crate::tests::text_model(&mut app, view);
        let SessionRules::Text(rules) = changed_rules() else {
            unreachable!()
        };
        assert_eq!(m.request().base_rules.as_ref(), Some(&rules));
    }
}

// ---- A4.13 ---------------------------------------------------------------------------------------------------

fn tree_fixture(world: &World) {
    let text = text_session(world);
    let folder = folder_session(world);
    let mut described = text.clone();
    described.description = "The API".into();
    save(world, "alpha", &text);
    save(world, "Beta", &folder);
    let mut fav = text.clone();
    fav.favorite = true;
    save(world, "zeta", &fav);
    save(world, "work/api", &described);
    save(world, "work/deep/inner", &text);
    save(world, "archive/old", &text);
    world
        .service
        .session_lock(&r("archive/old"), true)
        .expect("lock");
    world.service.session_mkdir("empty").expect("mkdir");
    let mut defaults = world
        .service
        .session_defaults(SessionType::TextCompare)
        .unwrap();
    defaults.description = "mine".into();
    world.service.session_save_defaults(&defaults).unwrap();
}

/// The four auto-saved groups of the list (`session_list` computes them, D-83).
fn dated_autosaves(list: &mut hikasa_session::api::SessionListDto) {
    for (group, label, id) in [
        ("today", "newest", "20261030-120000-1"),
        ("today", "new", "20261030-110000-1"),
        ("yesterday", "last night", "20261029-220000-1"),
        ("days_2_to_6", "this week", "20261026-080000-1"),
        ("older", "long ago", "20260101-080000-1"),
    ] {
        list.autosaved.push(hikasa_session::api::AutoSavedDto {
            id: id.to_owned(),
            kind: "text-compare".to_owned(),
            saved_at: None,
            group,
            label: label.to_owned(),
        });
    }
}

fn home(app: &TestApp) -> &HomeModel {
    match app.active_tab().and_then(|t| t.model()) {
        Some(ViewModel::Home(h)) => h,
        _ => panic!("Home is not active"),
    }
}

fn home_mut(app: &mut TestApp) -> &mut HomeModel {
    match app.active_tab_mut().and_then(|t| t.model_mut()) {
        Some(ViewModel::Home(h)) => h,
        _ => panic!("Home is not active"),
    }
}

#[test]
fn the_tree_and_its_search() {
    let world = World::new();
    tree_fixture(&world);
    let mut app = started(&world);
    let mut list = world.service.session_list(None, true, true).expect("list");
    dated_autosaves(&mut list);
    home_mut(&mut app).load(&list);
    let text = home(&app).render_tree();
    golden("home-tree.txt", &text);
    // Search: name, folder, description, case-insensitive, ancestors shown.
    home_mut(&mut app).set_search("INNER");
    assert_eq!(
        home(&app).render_tree(),
        "[-] work\n  [-] deep\n    inner <text-compare>\n"
    );
    home_mut(&mut app).set_search("the api");
    assert_eq!(
        home(&app).render_tree(),
        "[-] work\n  api <text-compare> -- The API\n"
    );
    home_mut(&mut app).set_search("archive");
    assert_eq!(
        home(&app).render_tree(),
        "[-] archive\n  old <text-compare> (locked)\n"
    );
    home_mut(&mut app).set_search("");
    assert_eq!(home(&app).render_tree(), text, "empty restores");
    // Closing a folder hides its rows; a search shows them anyway; empty restores the closed state.
    home_mut(&mut app).toggle("f:work");
    let closed = home(&app).render_tree();
    assert!(!closed.contains("api"));
    home_mut(&mut app).set_search("api");
    assert!(home(&app).render_tree().contains("api"));
    home_mut(&mut app).set_search("");
    assert_eq!(home(&app).render_tree(), closed);
    let w = app.win();
    app.0.execute(w, CommandId::TreeExpandAll);
    assert_eq!(home(&app).render_tree(), text);
    app.0.execute(w, CommandId::TreeCollapseAll);
    assert!(!home(&app).render_tree().contains("inner"));
}

fn select(world: &World, app: &mut TestApp, key: &str) {
    let w = app.win();
    app.0.home_select(w, key);
    world.settle(app);
}

fn exists(world: &World, reference: &str) -> bool {
    world.service.session_load(&r(reference)).is_ok()
}

#[test]
fn every_tree_command() {
    let world = World::new();
    tree_fixture(&world);
    let mut app = started(&world);
    let w = app.win();

    // TreeNewFolder, TreeNewSession (in the selected folder).
    select(&world, &mut app, "f:work");
    app.0.execute(w, CommandId::TreeNewFolder);
    top_dialog::<NameDialog>(&mut app, w).name = "sub".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert!(home(&app).render_tree().contains("sub"));
    app.0.execute(w, CommandId::TreeNewSession);
    top_dialog::<NameDialog>(&mut app, w).name = "bad/name*".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert!(top_dialog::<NameDialog>(&mut app, w).error.is_some());
    top_dialog::<NameDialog>(&mut app, w).name = "fresh".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert!(exists(&world, "work/fresh"));

    // TreeFavorite: toggles; disabled on locked.
    select(&world, &mut app, "n:alpha");
    app.0.execute(w, CommandId::TreeFavorite);
    world.settle(&mut app);
    assert!(
        world
            .service
            .session_load(&r("alpha"))
            .unwrap()
            .value
            .favorite
    );
    select(&world, &mut app, "n:archive/old");
    assert!(!app.0.enabled(w, CommandId::TreeFavorite));
    assert!(!app.0.enabled(w, CommandId::TreeClear));

    // TreeClear.
    select(&world, &mut app, "n:work/api");
    app.0.execute(w, CommandId::TreeClear);
    world.settle(&mut app);
    let cleared = world.service.session_load(&r("work/api")).unwrap().value;
    assert_eq!(cleared.specs.left, "");

    // TreeSaveAs.
    select(&world, &mut app, "n:alpha");
    app.0.execute(w, CommandId::TreeSaveAs);
    top_dialog::<SaveSessionDialog>(&mut app, w).name = "alpha2".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert!(exists(&world, "alpha2"));

    // TreeRename inline: a session, a folder; a locked one asks.
    select(&world, &mut app, "n:alpha2");
    app.0.execute(w, CommandId::TreeRename);
    home_mut(&mut app).rename.as_mut().unwrap().text = "alpha3".into();
    app.0.commit_rename(w);
    world.settle(&mut app);
    assert!(exists(&world, "alpha3") && !exists(&world, "alpha2"));
    select(&world, &mut app, "f:empty");
    app.0.execute(w, CommandId::TreeRename);
    home_mut(&mut app).rename.as_mut().unwrap().text = "void".into();
    app.0.commit_rename(w);
    world.settle(&mut app);
    assert!(home(&app).render_tree().contains("void"));
    select(&world, &mut app, "n:archive/old");
    app.0.execute(w, CommandId::TreeRename);
    home_mut(&mut app).rename.as_mut().unwrap().text = "older".into();
    app.0.commit_rename(w);
    assert!(prompt(&app).unwrap().contains("locked"));
    press(&world, &mut app, ButtonRole::Cancel);
    assert!(exists(&world, "archive/old"));
    app.0.execute(w, CommandId::TreeRename);
    home_mut(&mut app).rename.as_mut().unwrap().text = "older".into();
    app.0.commit_rename(w);
    press(&world, &mut app, ButtonRole::Yes);
    assert!(exists(&world, "archive/older"));

    // Drag onto a folder; a locked one asks.
    app.0.tree_move(w, r("alpha3"), "work");
    world.settle(&mut app);
    assert!(exists(&world, "work/alpha3"));
    app.0.tree_move(w, r("archive/older"), "work");
    assert!(prompt(&app).unwrap().contains("locked"));
    press(&world, &mut app, ButtonRole::Yes);
    assert!(exists(&world, "work/older"));

    // TreeDelete: a locked session asks with force; a folder names the count; an auto-saved file.
    select(&world, &mut app, "n:work/older");
    app.0.execute(w, CommandId::TreeDelete);
    assert!(prompt(&app).unwrap().contains("locked"));
    press(&world, &mut app, ButtonRole::Cancel);
    assert!(exists(&world, "work/older"));
    app.0.execute(w, CommandId::TreeDelete);
    press(&world, &mut app, ButtonRole::Yes);
    assert!(!exists(&world, "work/older"));
    select(&world, &mut app, "f:work/deep");
    app.0.execute(w, CommandId::TreeDelete);
    world.settle(&mut app);
    assert_eq!(
        prompt(&app).as_deref(),
        Some("Delete the folder work/deep and the 1 sessions in it?")
    );
    press(&world, &mut app, ButtonRole::Yes);
    assert!(!exists(&world, "work/deep/inner"));
    let id = world
        .service
        .session_autosave(None, &text_session(&world))
        .unwrap()
        .unwrap();
    refresh(&world, &mut app);
    select(&world, &mut app, &format!("a:{id}"));
    app.0.execute(w, CommandId::TreeDelete);
    press(&world, &mut app, ButtonRole::Yes);
    assert!(autosaved(&world).is_empty());
}

#[test]
fn tree_opens() {
    let world = World::new();
    tree_fixture(&world);
    let id = world
        .service
        .session_autosave(None, &text_session(&world))
        .unwrap()
        .unwrap();
    let mut app = started(&world);
    let w = app.win();
    // Open replaces Home; origin Named.
    select(&world, &mut app, "n:alpha");
    app.0.execute(w, CommandId::TreeOpen);
    world.settle(&mut app);
    let view = app.active_tab().unwrap().id;
    assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("alpha")));
    assert_eq!(app.tabs().len(), 1);
    // Open of an auto-saved session: its slot.
    app.0.execute(w, CommandId::TabNew);
    world.settle(&mut app);
    select(&world, &mut app, &format!("a:{id}"));
    app.0.execute(w, CommandId::TreeOpen);
    world.settle(&mut app);
    let view = app.active_tab().unwrap().id;
    assert_eq!(vs(&app, view).origin, ViewOrigin::AutoSaved(id.clone()));
    assert_eq!(app.0.view_slot(view), Some(&id));
    // Open as New: unnamed.
    app.0.execute(w, CommandId::TabNew);
    world.settle(&mut app);
    select(&world, &mut app, &format!("a:{id}"));
    app.0.execute(w, CommandId::TreeOpenAsNew);
    world.settle(&mut app);
    let view = app.active_tab().unwrap().id;
    assert_eq!(vs(&app, view).origin, ViewOrigin::Unnamed);
    assert_eq!(app.0.view_slot(view), None);
    // Open All: each session directly in the folder.
    app.0.execute(w, CommandId::TabNew);
    world.settle(&mut app);
    select(&world, &mut app, "f:work");
    let before = app.tabs().len();
    app.0.execute(w, CommandId::TreeOpenAll);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), before, "one session: it replaces Home");
    assert_eq!(app.active_tab().unwrap().caption(), "work/api");
    // The tree's menu follows the selection.
    app.0.execute(w, CommandId::TabNew);
    world.settle(&mut app);
    select(&world, &mut app, "n:archive/old");
    let none = |_: CommandId| None;
    let text = render_text(&[crate::menus::Menu {
        title: crate::menus::MenuTitle::Session,
        items: tree_menu(&MenuContext {
            tables: app.tables(),
            scope: Scope::Home,
            ctx: app.0.ctx(w),
            checked: &none,
            workspaces: &[],
        }),
    }]);
    assert!(text.contains("Open") && text.contains("Delete"));
    assert!(!text.contains("Clear") && !text.contains("Favorite"));
}

#[test]
fn the_management_panel() {
    let world = World::new();
    tree_fixture(&world);
    let mut app = started(&world);
    let w = app.win();
    select(&world, &mut app, "n:alpha");
    assert_eq!(home(&app).panel.target, PanelTarget::Named(r("alpha")));
    let left = home(&app).panel.left.clone();
    assert!(!left.is_empty());
    // Cancel reverts.
    home_mut(&mut app).panel.description = "edited".into();
    assert!(home(&app).panel.changed());
    app.0.panel_action(w, PanelAction::Cancel);
    assert_eq!(home(&app).panel.description, "");
    // Save.
    home_mut(&mut app).panel.description = "edited".into();
    app.0.panel_action(w, PanelAction::Save);
    world.settle(&mut app);
    assert_eq!(
        world
            .service
            .session_load(&r("alpha"))
            .unwrap()
            .value
            .description,
        "edited"
    );
    assert!(!home(&app).panel.changed());
    // Save As.
    app.0.panel_action(w, PanelAction::SaveAs);
    top_dialog::<SaveSessionDialog>(&mut app, w).name = "copy".into();
    press(&world, &mut app, ButtonRole::Ok);
    assert_eq!(
        world
            .service
            .session_load(&r("copy"))
            .unwrap()
            .value
            .description,
        "edited"
    );
    // Factory Defaults: the panel's copy, written on Save.
    app.0.panel_action(w, PanelAction::FactoryDefaults);
    world.settle(&mut app);
    assert_eq!(home(&app).panel.left, "");
    assert!(
        !world
            .service
            .session_load(&r("alpha"))
            .unwrap()
            .value
            .specs
            .left
            .is_empty()
    );
    app.0.panel_action(w, PanelAction::Save);
    world.settle(&mut app);
    assert_eq!(
        world
            .service
            .session_load(&r("alpha"))
            .unwrap()
            .value
            .specs
            .left,
        ""
    );
    // An auto-saved one is read-only: Open as New, Delete.
    let id = world
        .service
        .session_autosave(None, &text_session(&world))
        .unwrap()
        .unwrap();
    refresh(&world, &mut app);
    select(&world, &mut app, &format!("a:{id}"));
    assert_eq!(home(&app).panel.target, PanelTarget::AutoSaved(id));
    app.0.panel_action(w, PanelAction::Delete);
    press(&world, &mut app, ButtonRole::Yes);
    assert!(autosaved(&world).is_empty());
    // The panel's modes.
    assert_eq!(home(&app).panel_mode, PanelMode::Shown);
    app.0.execute(w, CommandId::MaximizeManage);
    assert_eq!(app.0.checked(w, CommandId::MaximizeManage), Some(true));
    app.0.execute(w, CommandId::ShowManage);
    assert_eq!(app.0.checked(w, CommandId::ShowManage), Some(false));
    app.0.execute(w, CommandId::Find);
    assert!(home(&app).focus_search);
}

#[test]
fn home_opens_the_panel_specs_and_takes_drops() {
    let world = World::new();
    save(&world, "alpha", &text_session(&world));
    let mut app = started(&world);
    let w = app.win();
    let (l, right) = text_pair(&world);
    let (a, b) = folder_pair(&world);

    // One path on a type button: the panel of a new session.
    app.0.drop_paths(
        w,
        DropTarget::HomeType(SessionType::TextCompare),
        vec![l.clone()],
    );
    world.settle(&mut app);
    assert_eq!(
        home(&app).panel.target,
        PanelTarget::New(SessionType::TextCompare)
    );
    assert_eq!(home(&app).panel.left, l.display().to_string());
    // Two paths: that type's view.
    app.0.drop_paths(
        w,
        DropTarget::HomeType(SessionType::TextCompare),
        vec![l.clone(), right.clone()],
    );
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    // A mismatch: the panel's error.
    app.0.execute(w, CommandId::TabNew);
    world.settle(&mut app);
    app.0.drop_paths(
        w,
        DropTarget::HomeType(SessionType::FolderCompare),
        vec![l.clone(), right.clone()],
    );
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Home);
    assert!(home(&app).error.is_some());
    // Path fields still take drops.
    app.0
        .drop_paths(w, DropTarget::PathField(FieldId::HomeLeft), vec![a, b]);
    assert_eq!(home(&app).error, None);

    // `HomeCompare` of a named selection with changed specs: origin Named, dirty.
    select(&world, &mut app, "n:alpha");
    let copy = world.write("other.c", &std::fs::read_to_string(&right).unwrap());
    home_mut(&mut app).panel.right = copy.display().to_string();
    app.0.execute(w, CommandId::HomeCompare);
    world.settle(&mut app);
    let view = app.active_tab().unwrap().id;
    let s = vs(&app, view);
    assert_eq!(s.origin, ViewOrigin::Named(r("alpha")));
    assert!(s.dirty);
}

#[test]
fn home_compare_reads_the_clipboard() {
    let world = World::with_init(|init| {
        init.clipboard = Arc::new(FixedClipboard::text("from the clipboard\n"));
    });
    let (_, right) = text_pair(&world);
    let mut app = started(&world);
    let w = app.win();
    home_mut(&mut app).panel.left = "clipboard://".into();
    home_mut(&mut app).panel.right = right.display().to_string();
    app.0.execute(w, CommandId::HomeCompare);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
}

#[test]
fn session_home_and_the_sessions_combo() {
    let (world, mut app, view) = way_world(true);
    let w = app.win();
    refresh(&world, &mut app);
    assert_eq!(app.sessions_of(Scope::Text), ["main", "other"]);
    assert!(app.sessions_of(Scope::Folder).is_empty());
    app.0.execute(
        w,
        Invocation::with(CommandId::SessionsCombo, CommandArg::Session(r("other"))),
    );
    world.settle(&mut app);
    assert!(app.tab(view).is_none());
    let view = app.active_tab().unwrap().id;
    assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("other")));
    assert_eq!(app.tabs().len(), 1, "in this tab");
    app.0.execute(w, CommandId::SessionHome);
    world.settle(&mut app);
    assert!(app.tab(view).is_none());
    assert_eq!(app.scope(), Scope::Home);
    assert!(home(&app).render_tree().contains("main"), "Home is filled");
}

/// §1.4.3: the dynamic Session menu.
#[test]
fn the_session_menu_lists_types_and_workspaces() {
    let world = World::new();
    let app = started(&world);
    let w = app.win();
    let checked = |id: CommandId| app.0.checked(w, id);
    let names = ["first".to_owned()];
    let mc = MenuContext {
        tables: app.tables(),
        scope: Scope::Home,
        ctx: app.0.ctx(w),
        checked: &checked,
        workspaces: &names,
    };
    let text = render_text(&crate::menus::menu_bar(&mc));
    assert!(
        text.contains("> New Session\n    Folder Compare\n    Text Compare\n"),
        "{text}"
    );
    assert!(
        text.contains("> Load Workspace\n    first\n    ---\n    Manage Workspaces...\n"),
        "{text}"
    );
}

// ---- A4.14 ---------------------------------------------------------------------------------------------------

/// Two windows: named, unnamed, auto-saved, pending and Home tabs.
fn workspace_world() -> (World, TestApp, WindowId, WindowId) {
    let world = World::new();
    save(&world, "a", &text_session(&world));
    save(&world, "b", &folder_session(&world));
    let id = world
        .service
        .session_autosave(None, &text_session(&world))
        .unwrap()
        .unwrap();
    let mut app = started(&world);
    let first = app.win();
    app.0
        .open_named(r("a"), OpenHow::Target(OpenTarget::Window(first)));
    world.settle(&mut app);
    let (l, right) = text_pair(&world);
    world.open(&mut app, &l, &right);
    world.settle(&mut app);
    app.0.execute(first, CommandId::WindowNew);
    let second = app.win();
    world.settle(&mut app);
    app.0.home_select(second, &format!("a:{id}"));
    world.settle(&mut app);
    app.0.execute(second, CommandId::TreeOpen);
    world.settle(&mut app);
    (world, app, first, second)
}

fn save_workspace(world: &World, app: &mut TestApp, name: &str) {
    let w = app.win();
    app.0.execute(w, CommandId::WorkspaceSaveAs);
    top_dialog::<SaveWorkspaceDialog>(app, w).name = name.into();
    press(world, app, ButtonRole::Ok);
}

/// The paths of a test world become `<W>` in a golden.
fn scrub(world: &World, text: &str) -> String {
    let root = world.dir.path().display().to_string();
    text.replace(&root, "<W>")
}

#[test]
fn save_and_restore_a_workspace() {
    let (world, mut app, first, second) = workspace_world();
    // A pending tab in the second window: restored once below; a Home tab is not saved.
    let mut g = app.window(first).unwrap().geometry.clone();
    (g.width, g.height, g.x, g.y) = (1000, 700, 10, 20);
    app.set_geometry(first, g, true);
    app.0.execute(second, CommandId::TabNew);
    world.settle(&mut app);
    save_workspace(&world, &mut app, "two");
    let dto = world.service.workspace_show(&n("two")).expect("show");
    assert!(dto.warnings.is_empty());
    let json = serde_json::to_string_pretty(&dto.workspace).unwrap();
    golden("workspace-two.json", &scrub(&world, &json));
    let saved: Workspace = world.service.workspace_load(&n("two")).unwrap().workspace;
    assert_eq!(saved.windows.len(), 2);
    assert_eq!(saved.windows[0].tabs[0].session, "a");
    assert!(!saved.windows[0].tabs[1].inline.is_empty());
    assert_eq!(saved.windows[1].tabs.len(), 1, "the Home tab is not saved");
    assert_eq!(saved.windows[0].active_tab, 1);

    // Load closes the views through their close actions and restores lazily.
    let built = app.views_built();
    app.0.execute(
        first,
        Invocation::with(
            CommandId::WorkspaceLoadList,
            CommandArg::Workspace(n("two")),
        ),
    );
    world.settle(&mut app);
    assert_eq!(app.windows().len(), 2);
    let root = app.root();
    assert_eq!(root, first, "windows[0] takes the first window");
    let w0 = app.window(root).unwrap();
    assert_eq!((w0.geometry.width, w0.geometry.x), (1000, 10));
    assert_eq!(w0.tabs.len(), 2);
    assert_eq!(w0.active, 1);
    assert!(w0.tabs[0].pending_tab().is_some(), "not activated yet");
    assert!(w0.tabs[1].pending_tab().is_none(), "the active tab started");
    assert_eq!(app.views_built() - built, 2, "one per window's active tab");
    assert_eq!(app.pending_tabs(), 1);
    // `gui list` lists the pending tab.
    let pending = w0.tabs[0].id;
    assert!(world.shared.list().iter().any(|v| v.view == pending));
    assert_eq!(w0.tabs[0].caption(), "a", "the reference is its caption");
    // It starts on activation.
    app.0.activate(root, 0);
    world.settle(&mut app);
    assert_eq!(app.pending_tabs(), 0);
    assert_eq!(app.views_built() - built, 3);
    assert_eq!(vs(&app, pending).origin, ViewOrigin::Named(r("a")));

    // A pending tab is captured as it is, and closes unopened.
    app.0.execute(
        first,
        Invocation::with(
            CommandId::WorkspaceLoadList,
            CommandArg::Workspace(n("two")),
        ),
    );
    world.settle(&mut app);
    save_workspace_confirmed(&world, &mut app, "two");
    let again: Workspace = world.service.workspace_load(&n("two")).unwrap().workspace;
    assert_eq!(again.windows[0].tabs[0].session, "a");
    let pending = app.window(root).unwrap().tabs[0].id;
    app.0.close_tab(root, 0, CloseReason::User);
    assert!(!world.shared.contains(pending));
}

fn save_workspace_confirmed(world: &World, app: &mut TestApp, name: &str) {
    let w = app.win();
    refresh(world, app);
    app.0.execute(w, CommandId::WorkspaceSaveAs);
    top_dialog::<SaveWorkspaceDialog>(app, w).name = name.into();
    press(world, app, ButtonRole::Ok);
    assert!(prompt(app).unwrap().contains("Replace"));
    press(world, app, ButtonRole::Yes);
}

#[test]
fn loading_a_workspace_runs_the_close_actions() {
    let world = World::new();
    let session = text_session(&world);
    save(&world, "a", &session);
    save(&world, "gone", &session);
    let mut app = started(&world);
    let w = app.win();
    world.write(
        "config/workspaces/w.toml",
        "schema_version = 1\n[[window]]\nwidth = 900\nheight = 600\n[[window.tab]]\nsession = \"a\"\n[[window.tab]]\nsession = \"missing\"\n",
    );
    let view = open_named(&world, &mut app, "a");
    make_dirty(&world, &mut app, view);
    let load = Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w")));
    app.0.execute(w, load.clone());
    world.settle(&mut app);
    assert_eq!(prompt(&app).as_deref(), Some("Save changes to session?"));
    // Cancel aborts with nothing closed.
    press(&world, &mut app, ButtonRole::Cancel);
    assert!(app.tab(view).is_some());
    app.0.execute(w, load);
    world.settle(&mut app);
    press(&world, &mut app, ButtonRole::No);
    assert!(app.tab(view).is_none());
    let tabs = &app.window(w).unwrap().tabs;
    assert_eq!(tabs.len(), 2);
    // A missing session: a Home tab and a notice.
    assert!(matches!(tabs[1].model(), Some(ViewModel::Home(_))));
    let notices = &mut app.window_mut(w).unwrap().notices;
    assert!(
        notices
            .visible(0.0)
            .iter()
            .any(|n| n.text.contains("session_missing")),
    );
    // Extra windows close.
    app.0.execute(w, CommandId::WindowNew);
    world.settle(&mut app);
    assert_eq!(app.windows().len(), 2);
    let effects = app.0.execute(
        w,
        Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
    );
    let _ = effects;
    let closed = world.settle(&mut app);
    assert_eq!(app.windows().len(), 1);
    assert!(closed.iter().any(|e| matches!(e, Effect::CloseViewport(_))));
}

#[test]
fn manage_workspaces() {
    let world = World::new();
    world.write(
        "config/workspaces/one.toml",
        "schema_version = 1\nshortcut = \"Ctrl+Alt+1\"\n",
    );
    world.write("config/workspaces/two.toml", "schema_version = 1\n");
    let mut app = started(&world);
    let w = app.win();
    app.0.execute(w, CommandId::WorkspaceManager);
    let d: &mut ManageWorkspacesDialog = top_dialog(&mut app, w);
    assert_eq!(d.rows.len(), 2);
    d.select(1);
    // A conflict is shown, never blocking.
    d.shortcut = "Ctrl+Alt+1".into();
    assert_eq!(d.conflicts(), ["Ctrl+Alt+1 is also the workspace one."]);
    d.shortcut = "Ctrl+W".into();
    assert_eq!(d.conflicts(), ["Ctrl+W is also the command Close Tab."]);
    d.shortcut = "Ctrl+Alt+2".into();
    assert!(d.conflicts().is_empty());
    press(&world, &mut app, MANAGE_SHORTCUT);
    assert_eq!(
        world
            .service
            .workspace_load(&n("two"))
            .unwrap()
            .workspace
            .shortcut,
        "Ctrl+Alt+2"
    );
    // A workspace shortcut loads it unless a command takes the key.
    let keys = [Shortcut::parse("Ctrl+Alt+2").unwrap()];
    assert_eq!(
        app.0.dispatch_keys(w, &keys, FocusArea::Root),
        None,
        "no command has this key"
    );
    assert_eq!(
        app.0.dispatch_workspace(w, &keys, FocusArea::Root),
        Some(Invocation::with(
            CommandId::WorkspaceLoadList,
            CommandArg::Workspace(n("two"))
        ))
    );
    app.0.execute(w, CommandId::WorkspaceManager);
    assert_eq!(
        app.0.dispatch_workspace(w, &keys, FocusArea::Root),
        None,
        "not while a dialog is open"
    );
    let d: &mut ManageWorkspacesDialog = top_dialog(&mut app, w);
    d.select(1);
    d.rename = "renamed".into();
    press(&world, &mut app, MANAGE_RENAME);
    assert!(world.service.workspace_load(&n("renamed")).is_ok());
    app.0.execute(w, CommandId::WorkspaceManager);
    let d: &mut ManageWorkspacesDialog = top_dialog(&mut app, w);
    d.select(0);
    press(&world, &mut app, MANAGE_DELETE);
    assert!(prompt(&app).unwrap().contains("Delete the workspace one"));
    press(&world, &mut app, ButtonRole::Yes);
    assert!(world.service.workspace_load(&n("one")).is_err());
}

#[test]
fn startup_and_exit_workspaces() {
    let options = "schema_version = 1\n[startup]\nload_workspace = \"w\"\non_exit_save_workspace_as = \"last\"\n";
    let world = World::with_files(&[
        ("options.toml", options),
        (
            "workspaces/w.toml",
            "schema_version = 1\n[[window]]\n[[window.tab]]\nsession = \"a\"\n",
        ),
    ]);
    save(&world, "a", &text_session(&world));
    let mut app = started(&world);
    let w = app.win();
    app.set_startup(Startup::Options);
    app.after_first_frame();
    world.settle(&mut app);
    assert_eq!(app.active_tab().unwrap().caption(), "a");
    app.0.execute(w, CommandId::Exit);
    world.settle(&mut app);
    app.flush(std::time::Duration::from_secs(5));
    assert!(app.quitting());
    let last = world.service.workspace_load(&n("last")).expect("saved");
    assert_eq!(last.workspace.windows[0].tabs[0].session, "a");

    // A missing startup workspace: a warning notice (not an error) and Home.
    let options = "schema_version = 1\n[startup]\nload_workspace = \"nope\"\n";
    let world = World::with_files(&[("options.toml", options)]);
    let mut app = started(&world);
    app.set_startup(Startup::Options);
    app.after_first_frame();
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Home);
    let w = app.win();
    assert!(
        app.window_mut(w)
            .unwrap()
            .notices
            .visible(0.0)
            .iter()
            .any(|n| n.text.contains("nope") && n.kind == crate::notify::NoticeKind::Warning),
        "D-125: a warning",
    );
}

#[test]
fn ipc_load_workspace_restores_windows_and_ipc_load_session_is_named() {
    let world = World::new();
    let endpoint = world.service.endpoint().expect("endpoint");
    save(&world, "a", &text_session(&world));
    save(&world, "b", &text_session(&world));
    world.write(
        "config/workspaces/w.toml",
        "schema_version = 1\n[[window]]\n[[window.tab]]\nsession = \"a\"\n[[window.tab]]\nsession = \"b\"\n[[window]]\n[[window.tab]]\nsession = \"b\"\n",
    );
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = started(&world);
    let mut client =
        IpcClient::connect(&endpoint, "test", std::time::Duration::from_secs(5)).expect("client");
    let (_, response) = client
        .request(Request::LoadWorkspace { name: "w".into() })
        .expect("load");
    let Response::Views { views } = response else {
        panic!("{response:?}")
    };
    assert_eq!(views.len(), 3);
    world.settle(&mut app);
    assert_eq!(app.windows().len(), 2);
    assert_eq!(app.pending_tabs(), 1);
    let (_, response) = client
        .request(Request::LoadSession {
            name: "a".into(),
            wait: false,
        })
        .expect("load");
    let Response::ViewOpened { view } = response else {
        panic!("{response:?}")
    };
    world.settle(&mut app);
    assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("a")));
    assert_eq!(app.tab(view).unwrap().caption(), "a");
    drop(client);
    drop(handle);
}

// ---- A4.15 ---------------------------------------------------------------------------------------------------

/// A server that records what it receives.
struct Recorder(std::sync::Mutex<Vec<Request>>);

impl hikasa_session::ipc::IpcHandler for Recorder {
    fn server_info(&self) -> hikasa_session::ipc::ServerInfo {
        hikasa_session::ipc::ServerInfo {
            server: "fake".into(),
            version: hikasa_session::ipc::PROTOCOL_VERSION,
        }
    }

    fn handle(
        &self,
        request: Request,
        _waiter: Option<hikasa_session::ipc::ViewWaiter>,
    ) -> Response {
        let response = match &request {
            Request::LoadSession { .. } => Response::ViewOpened { view: ViewId(7) },
            _ => Response::Views { views: Vec::new() },
        };
        self.0.lock().expect("lock").push(request);
        response
    }
}

/// D-127: with a GUI running, the command line's session or workspace is forwarded; golden frames.
#[test]
fn the_command_line_forwards_sessions_and_workspaces() {
    use hikasa_session::api::{OpenArgs, SingleArg};
    let world = World::new();
    save(&world, "work/api", &text_session(&world));
    world.write("config/workspaces/w.toml", "schema_version = 1\n");
    let endpoint = world.service.endpoint().expect("endpoint");
    let recorder = Arc::new(Recorder(std::sync::Mutex::new(Vec::new())));
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handle = server
        .spawn(Arc::clone(&recorder) as Arc<dyn hikasa_session::ipc::IpcHandler>)
        .expect("spawn");
    let classify = |arg: &str| world.service.classify_single_arg(std::ffi::OsStr::new(arg));
    let retry = hikasa_session::ipc::RetryPolicy::default();
    let open = OpenArgs::default();
    // Case-insensitive, as D-88 resolves a name; an existing path wins.
    assert_eq!(classify("WORK/API"), SingleArg::Session(r("work/api")));
    assert_eq!(classify("W"), SingleArg::Workspace(n("w")));
    let path = world.write("work/api", "a file\n");
    assert_eq!(
        world.service.classify_single_arg(path.as_os_str()),
        SingleArg::Spec
    );
    let session = SingleArg::Session(r("work/api"));
    assert_eq!(crate::forward(&world.service, &open, &session, &retry), 0);
    let workspace = SingleArg::Workspace(n("w"));
    assert_eq!(crate::forward(&world.service, &open, &workspace, &retry), 0);
    drop(handle);
    let frames = recorder.0.lock().expect("lock").clone();
    let text = serde_json::to_string_pretty(&frames).expect("json");
    golden("forward-frames.json", &(text + "\n"));
}

/// D-127: the window's own command line.
#[test]
fn the_command_line_opens_sessions_and_workspaces() {
    let world = World::new();
    save(&world, "a", &text_session(&world));
    world.write(
        "config/workspaces/w.toml",
        "schema_version = 1\n[[window]]\n[[window.tab]]\nsession = \"a\"\n",
    );
    let mut app = started(&world);
    app.set_startup(Startup::Session(r("a")));
    app.after_first_frame();
    world.settle(&mut app);
    let view = app.active_tab().unwrap().id;
    assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("a")));
    assert_eq!(app.tabs().len(), 1, "the untouched Home tab is replaced");

    let mut app = started(&world);
    app.set_startup(Startup::Workspace("w".into()));
    app.after_first_frame();
    world.settle(&mut app);
    assert_eq!(app.active_tab().unwrap().caption(), "a");
}

/// Regression tests of the W4b review.
mod regressions {
    use super::*;

    fn close_active(world: &World, app: &mut TestApp) {
        app.0.execute(app.win(), CommandId::CloseTab);
        world.settle(app);
    }

    /// An unnamed view closed: one auto-saved file, its id.
    fn make_auto(world: &World, app: &mut TestApp) -> String {
        let (l, right) = text_pair(world);
        let view = world.open(app, &l, &right);
        world.settle(app);
        let w = app.win();
        let index = app.window(w).unwrap().index_of(view).unwrap();
        app.0.close_tab(w, index, CloseReason::User);
        world.settle(app);
        refresh(world, app);
        let ids = autosaved(world);
        assert_eq!(ids.len(), 1);
        ids[0].clone()
    }

    fn open_auto(world: &World, app: &mut TestApp, id: &str) -> ViewId {
        let w = app.win();
        app.0.home_select(w, &format!("a:{id}"));
        world.settle(app);
        app.0.execute(w, CommandId::TreeOpen);
        world.settle(app);
        app.active_tab().unwrap().id
    }

    fn save_as_dialog(world: &World, app: &mut TestApp, name: &str) {
        let w = app.win();
        app.0.execute(w, CommandId::SessionSaveAs);
        world.settle(app);
        let d: &mut SaveSessionDialog = top_dialog(app, w);
        d.name = name.into();
    }

    fn ask_tree_rename(world: &World, app: &mut TestApp, key: &str, name: &str) {
        let w = app.win();
        app.0.home_select(w, key);
        world.settle(app);
        app.0.execute(w, CommandId::TreeRename);
        home_mut(app).rename.as_mut().expect("a rename").text = name.into();
        app.0.commit_rename(w);
        world.settle(app);
    }

    /// A Home tab beside the view, for the tree commands.
    fn home_tab(world: &World, app: &mut TestApp) {
        app.0.execute(app.win(), CommandId::TabNew);
        world.settle(app);
    }

    fn notice_texts(app: &mut TestApp) -> Vec<String> {
        let w = app.win();
        app.window_mut(w)
            .unwrap()
            .notices
            .visible(0.0)
            .iter()
            .map(|n| n.text.clone())
            .collect()
    }

    /// Runs `f` with the sessions folder read-only (a disk that refuses writes).
    #[cfg(unix)]
    fn read_only<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        use std::os::unix::fs::PermissionsExt;
        let set = |mode| {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).expect("chmod");
        };
        set(0o555);
        let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        set(0o755);
        match out {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    // ---- H1 ----------------------------------------------------------------------------------------------------

    #[test]
    fn a_reopened_autosave_closes_into_its_own_file() {
        let world = World::new();
        let mut app = started(&world);
        let id = make_auto(&world, &mut app);
        let v = open_auto(&world, &mut app, &id);
        assert_eq!(app.0.view_slot(v).unwrap().to_string(), id);
        close_active(&world, &mut app);
        assert_eq!(autosaved(&world), vec![id]);
    }

    #[test]
    fn the_timer_on_a_reopened_autosave_makes_no_second_file() {
        let world = World::new();
        let mut app = started(&world);
        let id = make_auto(&world, &mut app);
        let v = open_auto(&world, &mut app, &id);
        assert!(vs(&app, v).last_autosaved.is_some());
        let path = world.path(&format!("config/sessions/_autosaved/{id}.toml"));
        let before = std::fs::read(&path).unwrap();
        app.set_now(1000.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        assert_eq!(autosaved(&world), vec![id]);
        assert_eq!(std::fs::read(&path).unwrap(), before, "an unchanged view");
    }

    #[test]
    fn save_as_of_a_reopened_autosave_deletes_its_file() {
        let world = World::new();
        let mut app = started(&world);
        let id = make_auto(&world, &mut app);
        let v = open_auto(&world, &mut app, &id);
        save_as_dialog(&world, &mut app, "kept");
        press(&world, &mut app, ButtonRole::Ok);
        assert!(autosaved(&world).is_empty());
        assert_eq!(vs(&app, v).origin, ViewOrigin::Named(r("kept")));
    }

    // ---- H2 ----------------------------------------------------------------------------------------------------

    fn assert_kept_with_error(world: &World, app: &mut TestApp, view: ViewId) {
        assert!(app.tab(view).is_some(), "the tab stays");
        let text = prompt(app).expect("an error prompt");
        assert!(text.contains("could not be saved"), "{text}");
        assert!(vs(app, view).dirty && !vs(app, view).decided);
        assert!(!app.quitting());
        press(world, app, ButtonRole::Ok);
    }

    /// r03: the file went behind the GUI's back: there is no place to save to, so the tab must stay.
    #[test]
    fn yes_after_an_outside_rename_keeps_the_tab() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        world
            .service
            .session_rename(&r("main"), &r("moved"), false)
            .unwrap();
        let before = bytes(&world, "moved");
        app.0.execute(app.win(), CommandId::CloseTab);
        world.settle(&mut app);
        press(&world, &mut app, ButtonRole::Yes);
        assert_kept_with_error(&world, &mut app, view);
        assert_eq!(bytes(&world, "moved"), before);
        // The next try asks again, and Save As is still open to the user.
        app.0.execute(app.win(), CommandId::CloseTab);
        world.settle(&mut app);
        assert_eq!(prompt(&app).as_deref(), Some("Save changes to session?"));
        press(&world, &mut app, ButtonRole::Cancel);
        save_as_dialog(&world, &mut app, "again");
        press(&world, &mut app, ButtonRole::Ok);
        assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("again")));
    }

    #[test]
    fn yes_on_a_refused_disk_keeps_the_tab_on_every_close_path() {
        for way in WAYS {
            let (world, mut app, view) = way_world(true);
            make_dirty(&world, &mut app, view);
            let before = bytes(&world, "main");
            run_way(&world, &mut app, way);
            let dir = world.service.session_store().dir().to_path_buf();
            read_only(&dir, || press(&world, &mut app, ButtonRole::Yes));
            assert_kept_with_error(&world, &mut app, view);
            assert_eq!(bytes(&world, "main"), before, "{way:?}");
            // The disk is back: the same close works.
            run_way(&world, &mut app, way);
            press(&world, &mut app, ButtonRole::Yes);
            assert_ne!(bytes(&world, "main"), before, "{way:?}");
            assert!(app.tab(view).is_none(), "{way:?}");
        }
    }

    #[test]
    fn a_failed_save_at_exit_does_not_quit_and_the_workspace_is_saved_again() {
        let options = "schema_version = 1\n[startup]\non_exit_save_workspace_as = \"last\"\n";
        let world = World::with_files(&[("options.toml", options)]);
        save(&world, "main", &text_session(&world));
        let mut app = started(&world);
        let view = open_named(&world, &mut app, "main");
        make_dirty(&world, &mut app, view);
        run_way(&world, &mut app, Way::Exit);
        world
            .service
            .session_lock(&r("main"), true)
            .expect("locked behind the view");
        let effects = press(&world, &mut app, ButtonRole::Yes);
        assert!(!effects.contains(&Effect::Exit) && !app.quitting());
        assert_kept_with_error(&world, &mut app, view);
        let path = world.path("config/workspaces/last.toml");
        std::fs::remove_file(&path).expect("saved at the first try");
        run_way(&world, &mut app, Way::Exit);
        press(&world, &mut app, ButtonRole::Cancel);
        assert!(path.exists(), "a new exit saves the workspace again");
    }

    #[test]
    fn a_session_locked_behind_the_view_refuses_yes() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        world.service.session_lock(&r("main"), true).unwrap();
        let before = bytes(&world, "main");
        app.0.execute(app.win(), CommandId::CloseTab);
        world.settle(&mut app);
        press(&world, &mut app, ButtonRole::Yes);
        assert_kept_with_error(&world, &mut app, view);
        assert_eq!(bytes(&world, "main"), before);
    }

    // ---- H3 ----------------------------------------------------------------------------------------------------

    /// r04: an unrelated session under the old name is not overwritten.
    #[test]
    fn save_after_an_outside_rename_never_overwrites_a_new_session() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        world
            .service
            .session_rename(&r("main"), &r("moved"), false)
            .unwrap();
        let mut other = text_session(&world);
        other.description = "another session".into();
        save(&world, "main", &other);
        let before = bytes(&world, "main");
        app.0.execute(app.win(), CommandId::SessionSave);
        world.settle(&mut app);
        assert_eq!(before, bytes(&world, "main"));
        assert!(vs(&app, view).dirty);
        assert!(
            notice_texts(&mut app)
                .iter()
                .any(|t| t.contains("no longer the one"))
        );
    }

    #[test]
    fn a_rename_in_the_tree_moves_the_open_view_and_its_save() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        home_tab(&world, &mut app);
        refresh(&world, &mut app);
        ask_tree_rename(&world, &mut app, "n:main", "moved");
        assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("moved")));
        assert_eq!(app.tab(view).unwrap().caption(), "moved");
        let before = bytes(&world, "moved");
        app.activate(0);
        app.0.execute(app.win(), CommandId::SessionSave);
        world.settle(&mut app);
        assert_ne!(bytes(&world, "moved"), before);
        assert!(!world.service.session_store().path(&r("main")).exists());
    }

    #[test]
    fn a_folder_rename_and_move_reach_views_and_pending_tabs() {
        let world = World::new();
        save(&world, "dir/a", &text_session(&world));
        save(&world, "dir/sub/b", &text_session(&world));
        world.write(
            "config/workspaces/w.toml",
            "schema_version = 1\n[[window]]\nactive_tab = 0\n[[window.tab]]\nsession = \"dir/a\"\n[[window.tab]]\nsession = \"DIR/sub/b\"\n",
        );
        let mut app = started(&world);
        let w = app.win();
        app.0.execute(
            w,
            Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
        );
        world.settle(&mut app);
        assert_eq!(app.0.pending_tabs(), 1);
        let view = app.tabs()[0].id;
        home_tab(&world, &mut app);
        refresh(&world, &mut app);
        ask_tree_rename(&world, &mut app, "f:dir", "new");
        assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("new/a")));
        let tab = app.tabs().iter().find(|t| t.pending_tab().is_some());
        let pending = tab.expect("pending").pending_tab().expect("pending");
        assert_eq!(pending.reference, Some(r("new/sub/b")));
        assert_eq!(pending.caption, "new/sub/b");
    }

    #[test]
    fn a_delete_in_the_tree_makes_the_view_unnamed_and_its_close_autosaves() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        home_tab(&world, &mut app);
        refresh(&world, &mut app);
        select(&world, &mut app, "n:main");
        app.0.execute(app.win(), CommandId::TreeDelete);
        press(&world, &mut app, ButtonRole::Yes);
        let s = vs(&app, view);
        assert_eq!(
            (s.origin, s.loaded, s.dirty),
            (ViewOrigin::Unnamed, None, false)
        );
        assert_ne!(app.tab(view).unwrap().caption(), "main");
        app.activate(0);
        close_active(&world, &mut app);
        assert_eq!(
            autosaved(&world).len(),
            1,
            "the D-84 row of an unnamed view"
        );
    }

    #[test]
    fn a_clean_close_of_a_stale_view_gives_no_error() {
        for replaced in [false, true] {
            let (world, mut app, _) = way_world(true);
            world
                .service
                .session_rename(&r("main"), &r("moved"), false)
                .unwrap();
            let mut other = text_session(&world);
            other.description = "another".into();
            if replaced {
                save(&world, "main", &other);
            }
            let before = replaced.then(|| bytes(&world, "main"));
            close_active(&world, &mut app);
            assert!(prompt(&app).is_none());
            assert!(notice_texts(&mut app).is_empty(), "replaced={replaced}");
            assert_eq!(before, replaced.then(|| bytes(&world, "main")));
        }
    }

    // ---- H4 ----------------------------------------------------------------------------------------------------

    /// r06: the prompt names the locks; Delete after it is the force.
    #[test]
    fn a_folder_delete_names_its_locked_sessions() {
        let (world, mut app, _) = way_world(false);
        let w = app.win();
        save(&world, "dir/a", &text_session(&world));
        save(&world, "dir/b", &text_session(&world));
        world.service.session_lock(&r("dir/a"), true).unwrap();
        refresh(&world, &mut app);
        app.0.home_select(w, "f:dir");
        world.settle(&mut app);
        app.0.execute(w, CommandId::TreeDelete);
        world.settle(&mut app);
        let text = prompt(&app).unwrap();
        assert!(
            text.contains("the 2 sessions") && text.contains("1 of them are locked"),
            "{text}"
        );
        assert!(text.contains("dir/a"), "{text}");
        press(&world, &mut app, ButtonRole::Cancel);
        assert!(exists(&world, "dir/a"));
        app.0.execute(w, CommandId::TreeDelete);
        world.settle(&mut app);
        press(&world, &mut app, ButtonRole::Yes);
        assert!(!exists(&world, "dir/a"), "confirmed");
    }

    #[test]
    fn a_folder_rename_asks_for_locked_sessions() {
        let (world, mut app, _) = way_world(false);
        save(&world, "dir/a", &text_session(&world));
        world.service.session_lock(&r("dir/a"), true).unwrap();
        refresh(&world, &mut app);
        ask_tree_rename(&world, &mut app, "f:dir", "new");
        let text = prompt(&app).expect("a prompt");
        assert!(text.contains("1 locked sessions: dir/a"), "{text}");
        assert!(exists(&world, "dir/a"), "nothing moved yet");
        press(&world, &mut app, ButtonRole::Yes);
        assert!(exists(&world, "new/a") && !exists(&world, "dir/a"));
    }

    // ---- M ----------------------------------------------------------------------------------------------------

    /// r05: M1.
    #[test]
    fn the_panel_lets_go_of_a_deleted_session() {
        let (world, mut app, _) = way_world(false);
        let w = app.win();
        refresh(&world, &mut app);
        app.0.home_select(w, "n:main");
        world.settle(&mut app);
        // Deleted, the panel not told yet.
        world.service.session_delete(&r("main"), false).unwrap();
        app.0.panel_action(w, PanelAction::Save);
        world.settle(&mut app);
        assert!(!exists(&world, "main"), "a deleted session came back");
        refresh(&world, &mut app);
        assert_eq!(home(&app).panel.target, PanelTarget::None);
        app.0.panel_action(w, PanelAction::Save);
        world.settle(&mut app);
        assert!(!exists(&world, "main"));
    }

    #[test]
    fn the_settings_dialog_of_the_tree_does_not_recreate_a_deleted_session() {
        let (world, mut app, _) = way_world(false);
        let w = app.win();
        refresh(&world, &mut app);
        select(&world, &mut app, "n:main");
        app.0.execute(w, CommandId::TreeEdit);
        world.settle(&mut app);
        world.service.session_delete(&r("main"), false).unwrap();
        press(&world, &mut app, ButtonRole::Ok);
        assert!(!exists(&world, "main"));
    }

    /// r07: M2.
    #[test]
    fn auto_save_keeps_the_overrides_of_a_parent() {
        let world = World::new();
        let mut app = started(&world);
        let (a, b) = folder_pair(&world);
        app.set_now(0.0);
        let view = world.open(&mut app, &a, &b);
        world.settle(&mut app);
        app.set_now(1000.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        let id = autosaved(&world)[0].clone();
        assert_eq!(app.0.view_slot(view).unwrap().to_string(), id);
        let mut child = text_session(&world);
        world
            .service
            .session_apply_scope(
                RuleScope::Type,
                &changed_rules(),
                &mut child,
                Some(&format!("_autosaved/{id}")),
            )
            .unwrap();
        let parsed = AutoId::parse(&id).unwrap();
        let count = |w: &World| {
            let file = w.service.session_load_autosaved(&parsed).unwrap().value;
            file.overrides.len()
        };
        assert_eq!(count(&world), 1);
        // A workspace taken from the view holds them too.
        save_workspace(&world, &mut app, "w");
        let text = std::fs::read_to_string(world.path("config/workspaces/w.toml")).unwrap();
        assert!(text.contains("window.tab.inline.override"), "{text}");
        close_active(&world, &mut app);
        assert_eq!(count(&world), 1, "the parent's override survives");
    }

    /// M3: nothing drives a frame here: the core itself says when the timer needs one.
    #[test]
    fn the_timer_schedules_its_own_wake() {
        let world = World::new();
        let mut app = started(&world);
        assert_eq!(app.0.autosave_wake_in(), None, "no view");
        let (l, right) = text_pair(&world);
        app.set_now(10.0);
        let view = world.open(&mut app, &l, &right);
        world.settle(&mut app);
        assert_eq!(app.0.autosave_wake_in(), Some(300.0));
        app.0.set_timer_now(110.0);
        assert_eq!(app.0.autosave_wake_in(), Some(200.0));
        // egui's frame time stands still in a hidden window; the timer's does not.
        app.0.set_timer_now(310.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        assert!(app.0.view_slot(view).is_some(), "the file was written");
        assert_eq!(app.0.autosave_wake_in(), Some(300.0), "and it wakes again");
    }

    #[test]
    fn a_named_view_or_auto_save_off_needs_no_wake() {
        let (_world, app, _) = way_world(true);
        assert_eq!(app.0.autosave_wake_in(), None);
        let options = "schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = 0\n";
        let world = World::with_files(&[("options.toml", options)]);
        let mut app = started(&world);
        let (l, right) = text_pair(&world);
        world.open(&mut app, &l, &right);
        world.settle(&mut app);
        assert_eq!(app.0.autosave_wake_in(), None);
    }

    /// r11: M4.
    #[test]
    fn a_close_before_the_settings_result_asks_and_saves_them() {
        let (world, mut app, view) = way_world(true);
        let (_, other) = text_pair(&world);
        let copy = world.write("other.c", &std::fs::read_to_string(&other).unwrap());
        let w = app.win();
        app.0.execute(w, CommandId::SessionEdit);
        world.settle(&mut app);
        settings_dialog(&mut app, w).specs_mut().right = copy.display().to_string();
        app.0.dialog_button(w, ButtonRole::Ok);
        assert!(vs(&app, view).settings_pending && vs(&app, view).dirty);
        app.0.execute(w, CommandId::CloseTab);
        assert_eq!(prompt(&app).as_deref(), Some("Save changes to session?"));
        // Yes before the result: the save runs after it, with the new specs.
        press(&world, &mut app, ButtonRole::Yes);
        assert!(app.tab(view).is_none());
        let text = String::from_utf8(bytes(&world, "main")).unwrap();
        assert!(text.contains("other.c"), "{text}");
    }

    /// r12.
    #[test]
    fn ok_without_a_change_is_not_dirty() {
        for folder in [false, true] {
            let world = World::new();
            let s = if folder {
                folder_session(&world)
            } else {
                text_session(&world)
            };
            save(&world, "main", &s);
            let mut app = started(&world);
            let view = open_named(&world, &mut app, "main");
            let w = app.win();
            app.0.execute(w, CommandId::SessionEdit);
            world.settle(&mut app);
            press(&world, &mut app, ButtonRole::Ok);
            assert!(!vs(&app, view).dirty, "folder={folder}");
        }
    }

    /// r09: M5.
    #[test]
    fn the_view_table_of_a_named_workspace_tab_applies() {
        let world = World::new();
        save(&world, "main", &text_session(&world));
        world.write(
            "config/workspaces/w.toml",
            "schema_version = 1\n[[window]]\nactive_tab = 0\n[[window.tab]]\nsession = \"main\"\n[window.tab.view]\nfont_size = 23.0\n",
        );
        let mut app = started(&world);
        let w = app.win();
        app.0.execute(
            w,
            Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
        );
        world.settle(&mut app);
        let v = app.active_tab().unwrap().id;
        let keys = crate::tests::text_model(&mut app, v).layout_keys();
        assert!(format!("{keys:?}").contains("23"), "{keys:?}");
    }

    // ---- L ----------------------------------------------------------------------------------------------------

    /// r08: L1.
    #[test]
    fn save_as_on_a_case_variant_replaces_the_existing_spelling() {
        let (world, mut app, view) = way_world(true);
        refresh(&world, &mut app);
        make_dirty(&world, &mut app, view);
        save_as_dialog(&world, &mut app, "OTHER");
        press(&world, &mut app, ButtonRole::Ok);
        assert!(prompt(&app).unwrap().contains("exists"));
        let before = bytes(&world, "other");
        press(&world, &mut app, ButtonRole::Yes);
        assert_ne!(before, bytes(&world, "other"), "Replace was refused");
        assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("other")));
    }

    /// r10: L2.
    #[test]
    fn neither_timer_nor_close_after_save_as_leaves_an_autosaved_file() {
        for close in [false, true] {
            let world = World::new();
            let mut app = started(&world);
            let (l, right) = text_pair(&world);
            app.set_now(0.0);
            let view = world.open(&mut app, &l, &right);
            world.settle(&mut app);
            let w = app.win();
            save_as_dialog(&world, &mut app, "kept");
            app.0.dialog_button(w, ButtonRole::Ok);
            if close {
                let index = app.window(w).unwrap().index_of(view).unwrap();
                app.0.close_tab(w, index, CloseReason::User);
            } else {
                app.set_now(1000.0);
                app.handle(UiMsg::Tick);
            }
            world.settle(&mut app);
            assert!(autosaved(&world).is_empty(), "close={close}");
            assert!(exists(&world, "kept"));
        }
    }

    /// r14: L3.
    #[test]
    fn a_zero_limit_prunes_at_startup_and_on_a_change_of_the_option() {
        let auto = "schema_version = 1\ntype = \"text-compare\"\nsaved_at = \"2026-01-01T00:00:00+00:00\"\n";
        let name = "config/sessions/_autosaved/20260101-000000-1.toml";
        let options = "schema_version = 1\n[tweaks.metrics]\nmax_auto_saved_sessions = 0\n";
        let world = World::with_files(&[("options.toml", options)]);
        world.write(name, auto);
        let mut app = started(&world);
        assert_eq!(autosaved(&world).len(), 1);
        app.after_first_frame();
        world.settle(&mut app);
        assert!(autosaved(&world).is_empty());

        let world = World::new();
        world.write(name, auto);
        let mut app = started(&world);
        app.after_first_frame();
        world.settle(&mut app);
        assert_eq!(autosaved(&world).len(), 1, "the limit allows it");
        let (mut watcher, _link) = crate::watch::Watcher::new(world.service.clone());
        world.write("config/options.toml", options);
        // The stamp of a file written within the same tick may match: the watch needs a real change of size.
        app.on_watch(watcher.poll_once().expect("polled"));
        world.settle(&mut app);
        assert!(autosaved(&world).is_empty());
    }

    /// L4, L5: a second close is refused, and says so for a workspace.
    #[test]
    fn a_refused_exit_saves_no_workspace_and_a_refused_load_says_so() {
        let options = "schema_version = 1\n[startup]\non_exit_save_workspace_as = \"last\"\n";
        let world = World::with_files(&[("options.toml", options)]);
        save(&world, "main", &text_session(&world));
        world.write("config/workspaces/empty.toml", "schema_version = 1\n");
        let mut app = started(&world);
        let view = open_named(&world, &mut app, "main");
        make_dirty(&world, &mut app, view);
        run_way(&world, &mut app, Way::Tab);
        assert!(prompt(&app).is_some(), "a close is asking");
        app.0.execute(app.win(), CommandId::Exit);
        world.settle(&mut app);
        assert!(!world.path("config/workspaces/last.toml").exists());
        app.0.load_workspace(n("empty"));
        world.settle(&mut app);
        assert!(
            notice_texts(&mut app)
                .iter()
                .any(|t| t.contains("not loaded")),
            "the user is told"
        );
        press(&world, &mut app, ButtonRole::Cancel);
        assert!(app.tab(view).is_some());
    }

    /// L6.
    #[test]
    fn a_prompt_in_another_window_raises_it() {
        let (world, mut app, view) = way_world(true);
        make_dirty(&world, &mut app, view);
        let first = app.win();
        app.0.execute(first, CommandId::WindowNew);
        world.settle(&mut app);
        let second = app.win();
        assert_ne!(first, second);
        let effects = app.0.execute(second, CommandId::Exit);
        let effects = if effects.contains(&Effect::Raise(first)) {
            effects
        } else {
            press(&world, &mut app, ButtonRole::Yes)
        };
        assert!(effects.contains(&Effect::Raise(first)), "{effects:?}");
        assert_eq!(app.win(), first, "the asking window has the focus");
    }

    /// L7.
    #[test]
    fn reopen_keeps_the_display_filter() {
        let (world, mut app, view) = way_world(true);
        crate::tests::text_model(&mut app, view).execute(&world.service, CommandId::FavDiff);
        let wanted = crate::tests::text_model(&mut app, view)
            .view_options()
            .clone();
        make_dirty(&world, &mut app, view);
        assert_eq!(
            crate::tests::text_model(&mut app, view).view_options(),
            &wanted
        );
    }

    /// L8, L9: a tab that cannot start, and one closed while it starts.
    #[test]
    fn a_failed_pending_tab_says_why_in_its_own_place_and_a_closed_one_is_left_alone() {
        let world = World::new();
        save(&world, "a", &text_session(&world));
        save(&world, "b", &text_session(&world));
        world.write(
            "config/workspaces/w.toml",
            "schema_version = 1\n[[window]]\nactive_tab = 0\n[[window.tab]]\nsession = \"a\"\n[[window.tab]]\nsession = \"b\"\n",
        );
        let mut app = started(&world);
        let w = app.win();
        app.0.execute(
            w,
            Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
        );
        world.settle(&mut app);
        let second = app.tabs()[1].id;
        world.service.session_delete(&r("b"), false).unwrap();
        // Activated, then tab 0 shown again before the job answers: the reason stays in the failed tab's place.
        app.activate(1);
        app.activate(0);
        world.settle(&mut app);
        assert!(app.tab(second).is_none());
        assert_eq!(app.tabs().len(), 2);
        assert!(
            app.tabs()[1].message.is_some(),
            "the failed tab's Home says why"
        );
        assert!(app.tabs()[0].message.is_none());

        // L9.
        let world = World::new();
        save(&world, "a", &text_session(&world));
        world.write(
            "config/workspaces/w.toml",
            "schema_version = 1\n[[window]]\nactive_tab = 0\n[[window.tab]]\nsession = \"a\"\n",
        );
        let mut app = started(&world);
        let w = app.win();
        app.0.execute(
            w,
            Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
        );
        while app.0.pending_tabs() == 0 {
            app.handle(UiMsg::Tick);
        }
        let index = app.active();
        app.0.close_tab(w, index, CloseReason::User);
        world.settle(&mut app);
        assert_eq!(app.scope(), Scope::Home);
        assert_eq!(app.0.pending_tabs(), 0);
    }

    /// L11, r15.
    #[test]
    fn inline_tabs_keep_explicit_and_unknown_keys() {
        let world = World::new();
        let (l, right) = text_pair(&world);
        let table = |extra: &str| {
            format!(
                "[[window.tab]]\n[window.tab.inline]\ntype = \"text-compare\"\nfuture_top = 1\ndescription = \"{extra}\"\n[window.tab.inline.specs]\nleft = {:?}\nright = {:?}\n[window.tab.inline.rules]\nfuture_rule = 7\n[window.tab.inline.rules.alignment]\nskew_tolerance = {}\n",
                l.display().to_string(),
                right.display().to_string(),
                hikasa_text::TextRules::default().alignment.skew_tolerance,
            )
        };
        world.write(
            "config/workspaces/w.toml",
            &format!(
                "schema_version = 1\n[[window]]\nactive_tab = 0\n{}{}",
                table("live"),
                table("pending")
            ),
        );
        let mut app = started(&world);
        let w = app.win();
        app.0.execute(
            w,
            Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(n("w"))),
        );
        world.settle(&mut app);
        assert_eq!(app.0.pending_tabs(), 1, "one live, one pending");
        save_workspace_confirmed(&world, &mut app, "w");
        let after = std::fs::read_to_string(world.path("config/workspaces/w.toml")).unwrap();
        for key in ["future_top", "future_rule", "skew_tolerance"] {
            assert_eq!(after.matches(key).count(), 2, "{key}: {after}");
        }
    }

    /// L12.
    #[test]
    fn panel_save_as_of_a_locked_session_clears_the_lock() {
        let (world, mut app, _) = way_world(false);
        let w = app.win();
        world.service.session_lock(&r("main"), true).unwrap();
        refresh(&world, &mut app);
        select(&world, &mut app, "n:main");
        app.0.panel_action(w, PanelAction::SaveAs);
        let d: &mut SaveSessionDialog = top_dialog(&mut app, w);
        d.name = "copy".into();
        press(&world, &mut app, ButtonRole::Ok);
        let copy = world.service.session_load(&r("copy")).unwrap().value;
        assert!(!copy.locked);
    }

    /// L13.
    #[cfg(unix)]
    #[test]
    fn a_failed_delete_of_the_autosave_still_names_the_view() {
        let world = World::new();
        let mut app = started(&world);
        let (l, right) = text_pair(&world);
        app.set_now(0.0);
        let view = world.open(&mut app, &l, &right);
        world.settle(&mut app);
        app.set_now(1000.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        assert_eq!(autosaved(&world).len(), 1);
        save_as_dialog(&world, &mut app, "kept");
        let dir = world.path("config/sessions/_autosaved");
        read_only(&dir, || press(&world, &mut app, ButtonRole::Ok));
        assert_eq!(vs(&app, view).origin, ViewOrigin::Named(r("kept")));
        assert!(exists(&world, "kept"));
        assert!(
            notice_texts(&mut app).iter().any(|t| t.contains("stays")),
            "the file left behind is named"
        );
    }

    /// L14: the OS close takes one dialog, the window and what lies under it stay.
    #[test]
    fn the_os_close_cancels_only_the_top_dialog() {
        let (world, mut app, view) = way_world(true);
        let w = app.win();
        save_as_dialog(&world, &mut app, "typed");
        let top = Prompt::confirm("Question", "On top?", None, Vec::new());
        app.window_mut(w).unwrap().dialogs.push(Box::new(top));
        let effects = app.0.close_window(w);
        assert!(effects.is_empty());
        assert!(app.tab(view).is_some(), "the window stays");
        let under: &mut SaveSessionDialog = top_dialog(&mut app, w);
        assert_eq!(under.name, "typed", "the dialog under it is not lost");
        // A flow's question in window one is not ended by an OS close of window two.
        while !app.window(w).unwrap().dialogs.is_empty() {
            app.0.close_window(w);
        }
        make_dirty(&world, &mut app, view);
        app.0.execute(w, CommandId::WindowNew);
        world.settle(&mut app);
        let second = app.win();
        app.0.focus(w);
        app.0.execute(w, CommandId::CloseTab);
        assert!(prompt(&app).is_some());
        app.0.close_window(second);
        world.settle(&mut app);
        assert!(prompt(&app).is_some(), "the question stays");
        press(&world, &mut app, ButtonRole::Cancel);
        assert!(app.tab(view).is_some());
    }

    /// L15.
    #[test]
    fn ipc_load_session_honours_the_toolbar_key() {
        let world = World::new();
        let endpoint = world.service.endpoint().expect("endpoint");
        let mut s = text_session(&world);
        s.view.insert("toolbar".into(), false.into());
        save(&world, "main", &s);
        let server = IpcServer::bind(&endpoint).expect("bind");
        let handler = GuiHandler::new(
            world.service.clone(),
            Arc::clone(&world.shared),
            world.mailbox.clone(),
        );
        let handle = server.spawn(Arc::new(handler)).expect("spawn");
        let mut app = started(&world);
        let mut client = IpcClient::connect(&endpoint, "test", std::time::Duration::from_secs(5))
            .expect("client");
        let (_, response) = client
            .request(Request::LoadSession {
                name: "main".into(),
                wait: false,
            })
            .expect("load");
        let Response::ViewOpened { view } = response else {
            panic!("{response:?}")
        };
        world.settle(&mut app);
        assert!(!app.tab(view).unwrap().toolbar);
        drop(client);
        drop(handle);
    }

    /// L16.
    #[test]
    fn a_locked_auto_saved_file_keeps_the_view_dirty_and_the_close_asks() {
        let world = World::new();
        let mut app = started(&world);
        let id = make_auto(&world, &mut app);
        let path = world.path(&format!("config/sessions/_autosaved/{id}.toml"));
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace(
                "schema_version = 1\n",
                "schema_version = 1\nlocked = true\n",
            ),
        )
        .unwrap();
        let view = open_auto(&world, &mut app, &id);
        assert!(vs(&app, view).slot_locked);
        let before = std::fs::read(&path).unwrap();
        // Unchanged: nothing to lose.
        app.set_now(1000.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        assert!(!vs(&app, view).dirty);
        // Changed: not written, and the view says so.
        app.tab_mut(view)
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .description = "changed".into();
        app.set_now(2000.0);
        app.handle(UiMsg::Tick);
        world.settle(&mut app);
        assert!(vs(&app, view).dirty);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        app.0.execute(app.win(), CommandId::CloseTab);
        assert_eq!(prompt(&app).as_deref(), Some("Save changes to session?"));
        press(&world, &mut app, ButtonRole::Yes);
        assert!(app.tab(view).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(autosaved(&world).len(), 2, "the changes are in a new file");
    }

    /// r13: D-90 through the GUI.
    #[test]
    fn gui_saves_keep_explicit_and_unknown_keys() {
        let (world, mut app, _) = way_world(false);
        let (l, right) = text_pair(&world);
        let text = format!(
            "schema_version = 1\ntype = \"text-compare\"\nfuture_top = 1\n[specs]\nleft = {:?}\nright = {:?}\n[rules]\nfuture_rule = 7\n[rules.alignment]\nskew_tolerance = {}\nfuture_nested = 3\n",
            l.display().to_string(),
            right.display().to_string(),
            hikasa_text::TextRules::default().alignment.skew_tolerance,
        );
        let path = world.service.session_store().path(&r("main"));
        std::fs::write(&path, text).unwrap();
        let view = open_named(&world, &mut app, "main");
        crate::tests::text_model(&mut app, view).execute(&world.service, CommandId::FavDiff);
        close_active(&world, &mut app);
        let view = open_named(&world, &mut app, "main");
        make_dirty(&world, &mut app, view);
        app.0.execute(app.win(), CommandId::SessionSave);
        world.settle(&mut app);
        let after = std::fs::read_to_string(&path).unwrap();
        for key in [
            "future_top",
            "future_rule",
            "future_nested",
            "skew_tolerance",
        ] {
            assert!(after.contains(key), "{key} lost");
        }
    }
}
