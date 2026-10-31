//! M2 W4a acceptance (§6.4 A4.1–A4.9) through the toolkit-free models (§1.4.16): windows and tabs, layout
//! memory, commands and menus, dialogs, theme and fonts, status, jobs, messages and the log, polling and
//! notices, drops. No window opens.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hikasa_session::ipc::{
    CloseReason, IpcClient, IpcServer, OpenOptions, Request, Response, ViewKind,
};
use hikasa_session::settings::Shortcut;
use hikasa_session::settings::commands::DisplayFilterMode;

use crate::app::{AppCore, Effect, OpenTarget};
use crate::commands::{CommandId, Scope};
use crate::dialog::ButtonRole;
use crate::dnd::{DROP_ONE_MORE, DROP_ONE_OR_TWO, DropTarget, FieldId};
use crate::ipc::GuiHandler;
use crate::menus::{MenuContext, menu_bar, render_text, tab_menu};
use crate::notify::{NoticeAction, NoticeKind};
use crate::status::MessageKind;
use crate::tests::{TestApp, World, folder_pair, text_pair};
use crate::watch::{WatchEvent, Watcher};
use crate::window::{CommandArg, Invocation, ViewModel, WindowId};

fn sc(text: &str) -> Shortcut {
    Shortcut::parse(text).expect("a shortcut")
}

/// Compares `actual` with the golden file `tests/golden/<name>` (`HIKASA_BLESS=1` writes it).
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

fn windows(app: &AppCore) -> Vec<WindowId> {
    app.windows().iter().map(|w| w.id).collect()
}

fn tab_count(app: &AppCore, w: WindowId) -> usize {
    app.window(w).map_or(0, |w| w.tabs.len())
}

fn dialogs(app: &AppCore, w: WindowId) -> usize {
    app.window(w).map_or(0, |w| w.dialogs.len())
}

fn prompt_text(app: &AppCore, w: WindowId) -> Option<String> {
    let top = app.window(w)?.dialogs.top()?;
    top.as_any()
        .downcast_ref::<crate::dialog::Prompt>()
        .map(|p| p.message.clone())
}

/// Opens the text pair `n` times in window `w` (each a new tab).
fn open_texts(
    world: &World,
    app: &mut TestApp,
    w: WindowId,
    n: usize,
) -> Vec<hikasa_session::ipc::ViewId> {
    let (l, r) = text_pair(world);
    let mut views = Vec::new();
    for _ in 0..n {
        app.focus(w);
        views.push(world.open(app, &l, &r));
    }
    views
}

fn menus_text(app: &TestApp, w: WindowId) -> String {
    let checked = |id: CommandId| app.0.checked(w, id);
    let mc = MenuContext {
        tables: app.tables(),
        scope: app.0.scope(w),
        ctx: app.0.ctx(w),
        checked: &checked,
        workspaces: &[],
    };
    render_text(&menu_bar(&mc))
}

// ---- A4.1 windows and tabs ------------------------------------------------------------------------------------

#[test]
fn window_new_and_tab_placement() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    assert_eq!(windows(&app), [root]);
    // The root's position is known: the new window is placed 32 points down and right.
    let mut g = app.window(root).unwrap().geometry.clone();
    (g.x, g.y, g.width, g.height) = (100, 50, 900, 700);
    app.set_geometry(root, g, true);
    let effects = app.0.execute(root, CommandId::WindowNew);
    let all = windows(&app);
    assert_eq!(all.len(), 2);
    let second = all[1];
    assert!(effects.contains(&Effect::OpenViewport(second)));
    let w2 = app.window(second).unwrap();
    assert_eq!((w2.geometry.x, w2.geometry.y), (132, 82));
    // Size: `state.toml [window]` (defaults here).
    assert_eq!((w2.geometry.width, w2.geometry.height), (1200, 800));
    assert!(matches!(w2.tabs[0].model(), Some(ViewModel::Home(_))));
    assert_eq!(app.focused(), second, "the new window has the focus");
    // Unknown position: the new one's is unknown too.
    let g2 = app.window(second).unwrap().geometry.clone();
    app.set_geometry(second, g2, false);
    app.0.execute(second, CommandId::WindowNew);
    let third = windows(&app)[2];
    assert_eq!(app.window(third).unwrap().geometry.x, -1);

    // Tabs open next to the active one (the default) …
    let views = open_texts(&world, &mut app, root, 3);
    app.0.activate(root, 0);
    app.0.execute(root, CommandId::TabNew);
    let w = app.window(root).unwrap();
    assert_eq!(w.active, 1);
    assert!(matches!(w.tabs[1].model(), Some(ViewModel::Home(_))));
    assert_eq!(
        w.tabs[0].id, views[0],
        "the untouched Home tab was replaced by the first view"
    );
    // … or last.
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tabs]\nopen_new_tab_next_to_active = false\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    open_texts(&world, &mut app, root, 2);
    app.0.activate(root, 0);
    app.0.execute(root, CommandId::TabNew);
    let w = app.window(root).unwrap();
    assert_eq!(w.active, 2);
    assert!(matches!(w.tabs[2].model(), Some(ViewModel::Home(_))));
    world.settle(&mut app);
}

#[test]
fn opens_go_to_the_focused_window_or_a_new_one() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    app.0.execute(root, CommandId::WindowNew);
    let second = windows(&app)[1];
    app.focus(root);
    assert_eq!(app.open_target(), OpenTarget::Window(root));
    app.focus(second);
    assert_eq!(app.open_target(), OpenTarget::Window(second));
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    assert_eq!(app.window_of(view), Some(second));
    world.settle(&mut app);

    // `open_sessions_in = "window"`: a new window (unless the focused one holds only an untouched Home tab).
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tabs]\nopen_sessions_in = \"window\"\nopen_child_sessions_in = \"window\"\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    assert_eq!(app.open_target(), OpenTarget::Window(root));
    let first = world.open(&mut app, &l, &r);
    assert_eq!(app.window_of(first), Some(root));
    assert_eq!(app.open_target(), OpenTarget::NewWindow);
    let (a, b) = folder_pair(&world);
    let folder = world.open(&mut app, &a, &b);
    let w = app.window_of(folder).unwrap();
    assert_ne!(w, root);
    world.settle(&mut app);
    // FC-062 Open of a pair goes to a new window with `open_child_sessions_in = "window"`.
    let before = app.windows().len();
    let tab = app.window(w).unwrap().active;
    let pair = {
        let crate::window::TabContent::Live(ViewModel::Folder(m)) =
            &mut app.window_mut(w).unwrap().tabs[tab].content
        else {
            panic!("folder")
        };
        m.execute(&world.service, CommandId::ExpandAll);
        let index = m.rows().iter().position(|row| {
            let Some(p) = m.prepared() else { return false };
            let read = p.model().read();
            read.node(row.node)
                .is_some_and(|n| !n.is_folder() && n.left.is_some() && n.right.is_some())
        });
        index.expect("a file pair")
    };
    if let crate::window::TabContent::Live(ViewModel::Folder(m)) =
        &mut app.window_mut(w).unwrap().tabs[tab].content
    {
        m.select(pair);
    }
    app.0.execute(w, CommandId::Open);
    assert_eq!(app.windows().len(), before + 1);
    world.settle(&mut app);
}

#[test]
fn duplicate_tab_runs_its_own_view() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    // Home copies its fields.
    if let Some(ViewModel::Home(h)) = app.active_tab_mut().and_then(|t| t.model_mut()) {
        h.panel.left = "/a".into();
        h.panel.right = "/b".into();
    }
    app.0.execute(root, CommandId::TabDuplicate);
    let w = app.window(root).unwrap();
    assert_eq!(w.tabs.len(), 2);
    assert_eq!(w.active, 1);
    let Some(ViewModel::Home(copy)) = w.tabs[1].model() else {
        panic!("home")
    };
    assert_eq!(
        (copy.panel.left.as_str(), copy.panel.right.as_str()),
        ("/a", "/b")
    );
    assert_ne!(w.tabs[0].id, w.tabs[1].id);
    // A comparison: a second view with its own job, right of the source.
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let index = app.window(root).unwrap().index_of(view).unwrap();
    app.0.activate(root, index);
    app.0.execute(root, CommandId::TabDuplicate);
    let w = app.window(root).unwrap();
    let dup = w.tabs[index + 1].id;
    assert_ne!(dup, view);
    assert_eq!(w.active, index + 1);
    assert!(
        app.busy()
            || matches!(app.tab(dup).and_then(|t| t.model()), Some(ViewModel::Text(m)) if m.response().is_some())
    );
    world.settle(&mut app);
    for v in [view, dup] {
        let Some(ViewModel::Text(m)) = app.tab(v).and_then(|t| t.model()) else {
            panic!("text")
        };
        assert!(m.response().is_some(), "{v:?} compared");
    }
    assert!(
        world.shared.list().iter().any(|i| i.view == dup),
        "the duplicate is listed"
    );
}

#[test]
fn moving_tabs_between_windows() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    // Refused with one tab.
    assert!(!app.0.enabled(root, CommandId::TabMoveToNewWindow));
    assert!(
        app.0
            .execute(root, CommandId::TabMoveToNewWindow)
            .is_empty()
    );
    assert!(app.0.move_tab(root, 0, None, None).is_empty());
    assert_eq!(app.windows().len(), 1);
    let views = open_texts(&world, &mut app, root, 3);
    world.settle(&mut app);
    // Move to a new window: the view is unchanged.
    app.0.activate(root, 1);
    let effects = app.0.execute(root, CommandId::TabMoveToNewWindow);
    let second = windows(&app)[1];
    assert!(effects.contains(&Effect::OpenViewport(second)));
    assert_eq!(app.window(second).unwrap().tabs[0].id, views[1]);
    assert_eq!(tab_count(&app, root), 2);
    let Some(ViewModel::Text(m)) = app.tab(views[1]).and_then(|t| t.model()) else {
        panic!("text")
    };
    assert!(m.response().is_some(), "the comparison moved with its tab");
    // Move to another window (the submenu's argument): appended and active there.
    assert!(app.0.enabled(root, CommandId::TabMoveToWindow));
    app.0.activate(root, 0);
    app.0.execute(
        root,
        Invocation::with(CommandId::TabMoveToWindow, CommandArg::Window(second)),
    );
    let w2 = app.window(second).unwrap();
    assert_eq!(
        w2.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
        [views[1], views[0]]
    );
    assert_eq!(w2.active, 1);
    // A drag reorders inside the bar.
    app.0.move_tab(second, 1, Some(second), Some(0));
    assert_eq!(app.window(second).unwrap().tabs[0].id, views[0]);
    // A window that loses its last tab by a move closes (no Home tab).
    let effects = app.0.move_tab(root, 0, Some(second), None);
    assert_eq!(windows(&app), [second]);
    assert!(effects.contains(&Effect::Geometry(
        second,
        app.window(second).unwrap().geometry.clone()
    )));
    assert_eq!(tab_count(&app, second), 3);
    // View ids stay unique across windows.
    let mut ids: Vec<_> = app
        .windows()
        .iter()
        .flat_map(|w| w.tabs.iter().map(|t| t.id))
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
}

#[test]
fn close_others_and_last_tab_rules() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    open_texts(&world, &mut app, root, 3);
    world.settle(&mut app);
    // With the warning: a prompt; Cancel closes nothing; Yes closes the others.
    app.0.execute(root, CommandId::TabCloseOthers);
    assert_eq!(prompt_text(&app, root).as_deref(), Some("Close 2 tabs?"));
    app.dialog_button(root, ButtonRole::Cancel);
    assert_eq!(tab_count(&app, root), 3);
    assert_eq!(dialogs(&app, root), 0);
    let keep = app.window(root).unwrap().tabs[1].id;
    app.0.activate(root, 1);
    app.0.execute(root, CommandId::TabCloseOthers);
    app.dialog_button(root, ButtonRole::Yes);
    let w = app.window(root).unwrap();
    assert_eq!(w.tabs.iter().map(|t| t.id).collect::<Vec<_>>(), [keep]);
    // Closing the last tab: a Home tab replaces it (the default).
    app.0.execute(root, CommandId::CloseTab);
    assert!(matches!(
        app.window(root).unwrap().tabs[0].model(),
        Some(ViewModel::Home(_))
    ));
    // Without the warning, Close Other Tabs acts at once; with `closing_last_tab_closes_window` the window
    // closes.
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tabs]\nwarn_when_closing_multiple_tabs = false\nclosing_last_tab_closes_window = true\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    open_texts(&world, &mut app, root, 3);
    app.0.execute(root, CommandId::TabCloseOthers);
    assert_eq!(tab_count(&app, root), 1);
    assert_eq!(dialogs(&app, root), 0);
    app.0.execute(root, CommandId::WindowNew);
    let second = windows(&app)[1];
    let effects = app.0.execute(second, CommandId::CloseTab);
    assert_eq!(windows(&app), [root]);
    assert!(effects.contains(&Effect::CloseViewport(second)));
    // The last tab of the last window: the process ends.
    let effects = app.0.execute(root, CommandId::CloseTab);
    assert!(effects.contains(&Effect::Exit));
    assert!(app.quitting());
}

#[test]
fn closing_windows_swaps_the_root_and_warns() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    open_texts(&world, &mut app, root, 2);
    app.0.execute(root, CommandId::WindowNew);
    let second = windows(&app)[1];
    // More than one tab: a prompt; Cancel keeps everything.
    assert!(app.close_window(root).is_empty());
    assert_eq!(prompt_text(&app, root).as_deref(), Some("Close 2 tabs?"));
    app.dialog_button(root, ButtonRole::Cancel);
    assert_eq!(windows(&app), [root, second]);
    // Yes: the root closes, the second window moves into the root viewport.
    app.close_window(root);
    let effects = app.dialog_button(root, ButtonRole::Yes);
    assert_eq!(windows(&app), [second]);
    let g = app.window(second).unwrap().geometry.clone();
    assert_eq!(
        effects,
        vec![Effect::Geometry(second, g), Effect::CloseViewport(second)]
    );
    assert_eq!(app.root(), second);
    world.settle(&mut app);

    // Exit: the warning by window count, then by tab count.
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    app.0.execute(root, CommandId::WindowNew);
    app.0.execute(root, CommandId::Exit);
    assert_eq!(prompt_text(&app, root).as_deref(), Some("Close 2 windows?"));
    app.dialog_key(root, &sc("Esc"));
    assert!(!app.quitting());
    let second = windows(&app)[1];
    app.close_window(second);
    open_texts(&world, &mut app, root, 2);
    app.0.execute(root, CommandId::Exit);
    assert_eq!(prompt_text(&app, root).as_deref(), Some("Close 2 tabs?"));
    let effects = app.dialog_key(root, &sc("Enter"));
    assert!(effects.contains(&Effect::Exit));
    assert!(app.quitting());
    app.flush(Duration::from_secs(10));
}

#[test]
fn the_tab_bar_hides_with_one_tab_when_asked() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tabs]\nhide_tab_bar_if_single_tab = true\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    assert!(!app.tab_bar_visible(root));
    app.0.execute(root, CommandId::TabNew);
    assert!(app.tab_bar_visible(root));
    let world = World::new();
    let app = world.app(None);
    assert!(app.tab_bar_visible(app.root()));
}

/// A waited view moved to another window still reports its D-23 outcome once.
#[test]
fn a_waited_view_moved_to_another_window_reports_once() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = world.app(Some(handle));
    let root = app.root();
    let timeout = Duration::from_secs(10);
    let specs = vec![left.display().to_string(), right.display().to_string()];
    let wait_endpoint = endpoint.clone();
    let waiting = std::thread::spawn(move || {
        let mut client = IpcClient::connect(&wait_endpoint, "test", timeout).expect("connect");
        let (id, response) = client
            .request(Request::OpenView {
                kind: ViewKind::TextCompare,
                specs,
                options: OpenOptions::default(),
                wait: true,
            })
            .expect("open");
        let Response::ViewOpened { view } = response else {
            panic!("{response:?}");
        };
        let closed = client.wait_view_closed(id).expect("closed");
        (view, closed)
    });
    let began = Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(Duration::from_millis(5));
    }
    world.settle(&mut app);
    app.0.execute(root, CommandId::TabNew);
    let view = world.shared.list()[0].view;
    let index = app.window(root).unwrap().index_of(view).unwrap();
    app.0.move_tab(root, index, None, None);
    let second = app.window_of(view).unwrap();
    assert_ne!(second, root);
    assert!(world.shared.is_waited(view));
    app.0.execute(second, CommandId::CloseTab);
    let (opened, (closed, result, _code)) = waiting.join().expect("waiter");
    assert_eq!((opened, closed), (view, view));
    assert_eq!(result.close_reason(), Some(CloseReason::User));
    assert!(!world.shared.contains(view), "reported once, then gone");
    app.shutdown();
}

// ---- A4.2 layout memory ---------------------------------------------------------------------------------------

#[test]
fn layout_is_written_on_close_and_read_on_the_next_start() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (l, r) = text_pair(&world);
    let (a, b) = folder_pair(&world);
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    app.0.execute(root, CommandId::DisplayFontIncrease);
    app.0.execute(root, CommandId::DisplayFontIncrease);
    app.0.execute(root, CommandId::ShowThumbnail);
    app.0.execute(root, CommandId::ShowToolbar);
    world.open(&mut app, &a, &b);
    world.settle(&mut app);
    app.0.execute(root, CommandId::ShowLog);
    let mut g = app.window(root).unwrap().geometry.clone();
    (g.x, g.y, g.width, g.height) = (40, 30, 1000, 640);
    app.set_geometry(root, g, true);
    // Exit writes `[window]` and the layout of the views.
    app.shutdown();
    app.flush(Duration::from_secs(10));
    let state = std::fs::read_to_string(world.path("config/state.toml")).expect("state.toml");
    golden("state-after-exit.toml", &state);
    // The next start reads it: a text view takes the font size and the thumbnail, the folder view its log.
    let mut app = world.app(None);
    let root = app.root();
    assert_eq!(
        (
            app.window(root).unwrap().geometry.width,
            app.window(root).unwrap().geometry.height
        ),
        (1000, 640)
    );
    let view = world.open(&mut app, &l, &r);
    let Some(ViewModel::Text(m)) = app.tab(view).and_then(|t| t.model()) else {
        panic!("text")
    };
    assert_eq!(m.font_px(), 15);
    assert!(!m.show_thumbnail());
    assert!(
        !app.tab(view).unwrap().toolbar,
        "the toolbar stays hidden for text views"
    );
    let folder = world.open(&mut app, &a, &b);
    let Some(ViewModel::Folder(m)) = app.tab(folder).and_then(|t| t.model()) else {
        panic!("folder")
    };
    assert!(m.show_log());
    assert!(app.tab(folder).unwrap().toolbar);
    world.settle(&mut app);
}

#[test]
fn the_layout_falls_back_to_the_theme_and_defaults() {
    // A layout fixture with a bad type and the theme's editor size.
    let world = World::with_files(&[
        (
            "state.toml",
            "schema_version = 1\n[layout.text-compare]\nfont_size = \"big\"\nthumbnail = false\n",
        ),
        (
            "theme.toml",
            "schema_version = 1\n[fonts.editor]\nsize = 16.0\n",
        ),
    ]);
    let mut app = world.app(None);
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    let Some(ViewModel::Text(m)) = app.tab(view).and_then(|t| t.model()) else {
        panic!("text")
    };
    assert_eq!(m.font_px(), 16, "the theme's editor size");
    assert!(!m.show_thumbnail());
    let root = app.root();
    app.0.execute(root, CommandId::DisplayFontIncrease);
    app.0.execute(root, CommandId::DisplayFontReset);
    let Some(ViewModel::Text(m)) = app.tab(view).and_then(|t| t.model()) else {
        panic!("text")
    };
    assert_eq!(m.font_px(), 16, "reset goes back to the editor size");
    world.settle(&mut app);
}

// ---- A4.3 commands, menus, toolbars ---------------------------------------------------------------------------

const COMMANDS_FIXTURE: &str = "schema_version = 1\n\
[view.text-compare]\nshow_all_in_menu = false\n\
[view.text-compare.command.Reload]\nshortcuts = [\"Ctrl+R\"]\nmenu = false\n\
[view.text-compare.command.ShowThumbnail]\nshortcuts = []\n\
[view.folder-compare.command.ExpandAll]\nmenu = false\n\
[view.folder-compare.command.SuppressFilters]\nshortcuts = [\"Alt+S\"]\n\
[view.home.command.About]\nmenu = false\n";

fn menu_goldens(world: &World, suffix: &str) {
    let mut app = world.app(None);
    let root = app.root();
    golden(&format!("menus-home{suffix}.txt"), &menus_text(&app, root));
    let (l, r) = text_pair(world);
    let (a, b) = folder_pair(world);
    world.open(&mut app, &a, &b);
    world.settle(&mut app);
    golden(
        &format!("menus-folder-compare{suffix}.txt"),
        &menus_text(&app, root),
    );
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    golden(
        &format!("menus-text-compare{suffix}.txt"),
        &menus_text(&app, root),
    );
}

#[test]
fn menu_goldens_per_view_type() {
    menu_goldens(&World::new(), "");
    menu_goldens(
        &World::with_files(&[("commands.toml", COMMANDS_FIXTURE)]),
        "-custom",
    );
}

#[test]
fn tab_context_menu_entries() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    app.0.execute(root, CommandId::TabNew);
    app.0.execute(root, CommandId::WindowNew);
    let others = app.other_windows(root);
    assert_eq!(others.len(), 1);
    let checked = |_: CommandId| None;
    let mc = MenuContext {
        tables: app.tables(),
        scope: Scope::Home,
        ctx: app.0.ctx(root),
        checked: &checked,
        workspaces: &[],
    };
    let text = render_text(&[crate::menus::Menu {
        title: crate::menus::MenuTitle::Session,
        items: tab_menu(&mc, 0, &others),
    }]);
    golden("tab-context-menu.txt", &text);
}

#[test]
fn the_toolbar_menu_writes_commands_toml_and_hides_the_toolbar() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (a, b) = folder_pair(&world);
    world.open(&mut app, &a, &b);
    world.settle(&mut app);
    let items = crate::menus::toolbar_menu(app.tables(), Scope::Folder);
    assert_eq!(items.len(), 6, "three modes, large buttons, options, hide");
    assert_eq!(items[4], crate::menus::ToolbarMenuItem::Options);
    app.set_display_filter_mode(root, DisplayFilterMode::Toggles);
    app.set_large_buttons(root, true);
    world.settle(&mut app);
    let text = std::fs::read_to_string(world.path("config/commands.toml")).expect("written");
    assert!(text.contains("[view.folder-compare]"), "{text}");
    assert!(text.contains("display_filter_mode = \"toggles\""), "{text}");
    assert!(text.contains("large_buttons = true"), "{text}");
    assert_eq!(
        app.tables().view(Scope::Folder).custom.display_filter_mode,
        DisplayFilterMode::Toggles
    );
    let control = crate::menus::display_filter(app.tables(), Scope::Folder);
    assert!(control.buttons.is_empty() && !control.toggles.is_empty());
    // Hide Toolbar toggles the view's toolbar and `[layout.folder-compare] toolbar`.
    assert_eq!(app.0.checked(root, CommandId::ShowToolbar), Some(true));
    app.0.execute(root, CommandId::ShowToolbar);
    assert_eq!(app.0.checked(root, CommandId::ShowToolbar), Some(false));
    assert_eq!(
        app.state().layout_value("folder-compare", "toolbar"),
        Some(hikasa_session::settings::state::LayoutValue::Bool(false))
    );
}

// ---- A4.4 dialogs ---------------------------------------------------------------------------------------------

#[test]
fn prompts_keys_stacking_and_do_not_ask_again() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    open_texts(&world, &mut app, root, 3);
    world.settle(&mut app);
    // About is a dialog; F1 shows the dialog's topic.
    app.0.execute(root, CommandId::About);
    assert_eq!(dialogs(&app, root), 1);
    app.dialog_key(root, &sc("F1"));
    assert_eq!(app.0.help(root), Some("main-window"));
    // The window's commands take no key and are disabled while a dialog is open.
    assert!(app.0.ctx(root).dialog_open);
    assert!(!app.0.enabled(root, CommandId::CloseTab));
    assert_eq!(
        app.dispatch_key(root, &sc("Ctrl+W"), crate::commands::FocusArea::Root),
        None
    );
    assert!(app.0.execute(root, CommandId::CloseTab).is_empty());
    assert_eq!(tab_count(&app, root), 3);
    // Esc closes About.
    app.dialog_key(root, &sc("Esc"));
    assert_eq!(dialogs(&app, root), 0);
    assert_eq!(
        app.dispatch_key(root, &sc("Ctrl+W"), crate::commands::FocusArea::Root),
        Some(CommandId::CloseTab)
    );
    // Stacking: a prompt, then About on top; the top one gets the keys.
    app.0.execute(root, CommandId::TabCloseOthers);
    app.window_mut(root)
        .unwrap()
        .dialogs
        .push(Box::new(crate::dialog::AboutDialog));
    assert_eq!(dialogs(&app, root), 2);
    app.dialog_key(root, &sc("Enter"));
    assert_eq!(dialogs(&app, root), 1, "Enter closed About only");
    assert_eq!(tab_count(&app, root), 3);
    // "Do not ask again" with Yes: the tabs close and the option is written.
    app.set_do_not_ask(root, true);
    app.dialog_key(root, &sc("Enter"));
    assert_eq!(tab_count(&app, root), 1);
    assert!(!app.options().tabs.warn_when_closing_multiple_tabs);
    world.settle(&mut app);
    let options = std::fs::read_to_string(world.path("config/options.toml")).expect("written");
    assert!(
        options.contains("warn_when_closing_multiple_tabs = false"),
        "{options}"
    );
    // The next Close Other Tabs does not ask.
    open_texts(&world, &mut app, root, 2);
    app.0.execute(root, CommandId::TabCloseOthers);
    assert_eq!(dialogs(&app, root), 0);
    assert_eq!(tab_count(&app, root), 1);
    world.settle(&mut app);
}

// ---- A4.5 theme and fonts -------------------------------------------------------------------------------------

#[test]
fn the_theme_follows_the_os_and_its_files() {
    let world = World::new();
    let mut app = world.app(None);
    assert_eq!(app.scheme().name, "light");
    assert_eq!(app.set_os_dark(true), vec![Effect::ApplyTheme]);
    assert_eq!(app.scheme().name, "dark");
    assert!(app.set_os_dark(true).is_empty(), "not per frame");
    assert_eq!(app.set_os_dark(false), vec![Effect::ApplyTheme]);
    // A changed theme.toml re-applies once.
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    world.write(
        "config/themes/mine.toml",
        "schema_version = 1\nbase = \"dark\"\n[folder]\nsame = \"#123456\"\n",
    );
    world.write(
        "config/theme.toml",
        "schema_version = 1\nlight_scheme = \"mine\"\n",
    );
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Config(c)] if c.len() == 2),
        "{events:?}"
    );
    let mut effects = app.on_watch(events);
    effects.extend(world.settle(&mut app));
    assert_eq!(
        effects.iter().filter(|e| **e == Effect::ApplyTheme).count(),
        1,
        "{effects:?}"
    );
    assert_eq!(app.scheme().name, "mine");
    let pal = crate::ui::theme::Palette::from_scheme_base(app.scheme(), app.scheme_base());
    assert!(pal.dark, "the base of mine.toml");
    assert_eq!(pal.c_same, egui::Color32::from_rgb(0x12, 0x34, 0x56));
    let notices = &app.window(app.root()).unwrap().notices;
    assert!(
        notices
            .all()
            .iter()
            .any(|n| n.text.starts_with("Theme reloaded"))
    );
    assert!(watcher.poll_once().expect("polled").is_empty());
    assert!(
        world
            .settle(&mut app)
            .iter()
            .all(|e| *e != Effect::ApplyTheme)
    );
}

#[test]
fn a_font_family_gives_one_notice_and_the_built_in_font() {
    let world = World::with_files(&[(
        "theme.toml",
        "schema_version = 1\n[fonts.editor]\nfamily = \"Nowhere Mono\"\nsize = 14.0\n[fonts.hex]\nfamily = \"Nowhere Mono\"\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    let notices: Vec<_> = app
        .window(root)
        .unwrap()
        .notices
        .all()
        .iter()
        .filter(|n| n.text.contains("Nowhere Mono"))
        .cloned()
        .collect();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, NoticeKind::Warning);
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    let Some(ViewModel::Text(m)) = app.tab(view).and_then(|t| t.model()) else {
        panic!("text")
    };
    assert_eq!(m.font_px(), 14, "the editor size reaches the panes");
    world.settle(&mut app);
}

// ---- A4.7 status, jobs, messages, log -------------------------------------------------------------------------

#[test]
fn the_job_indicator_aggregates_and_cancels_per_view() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (a, b) = folder_pair(&world);
    // Not polled yet: both views are preparing.
    let first = world.open(&mut app, &a, &b);
    let second = world.open(&mut app, &a, &b);
    let jobs = app.jobs(root);
    assert_eq!(jobs.len(), 2, "{jobs:?}");
    let summary = crate::status::JobSummary::of(&jobs);
    assert_eq!(summary.count, 2);
    app.cancel_jobs(first);
    let jobs = app.jobs(root);
    assert_eq!(jobs.iter().map(|j| j.view).collect::<Vec<_>>(), [second]);
    // Stop cancels the active view.
    assert!(app.0.enabled(root, CommandId::Stop));
    app.0.execute(root, CommandId::Stop);
    assert!(app.jobs(root).is_empty());
    world.settle(&mut app);
}

#[test]
fn the_message_panel_shows_dismisses_and_does_not_show_again() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let view = app.active_tab().unwrap().id;
    app.drop_paths(root, DropTarget::Window, vec![PathBuf::from("/a")]);
    let m = app.tab(view).unwrap().message.clone().expect("a message");
    assert_eq!(
        (m.kind, m.text.as_str()),
        (MessageKind::Info, DROP_ONE_MORE)
    );
    app.dismiss_message(view, false);
    assert!(app.tab(view).unwrap().message.is_none());
    app.set_message(
        view,
        crate::status::Message {
            do_not_show: Some("next_difference.show_message_panel"),
            ..crate::status::Message::info("No more differences.")
        },
    );
    app.dismiss_message(view, true);
    assert!(!app.options().next_difference.show_message_panel);
    world.settle(&mut app);
    let options = std::fs::read_to_string(world.path("config/options.toml")).expect("written");
    assert!(options.contains("show_message_panel = false"), "{options}");
}

#[test]
fn show_log_and_the_log_book() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (a, b) = folder_pair(&world);
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &a, &b);
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    // Finished jobs are summaries with the tab caption as their source.
    let entries: Vec<_> = app.log().entries().cloned().collect();
    assert!(entries.iter().any(|e| e.source.contains("left.c") && e.kind == hikasa_session::log::LogKind::Summary), "{entries:?}");
    assert!(
        entries.iter().any(|e| e.text.contains("compared in")),
        "{entries:?}"
    );
    let index = app.window(root).unwrap().index_of(view).unwrap();
    app.0.activate(root, index);
    assert_eq!(app.0.checked(root, CommandId::ShowLog), Some(false));
    app.0.execute(root, CommandId::ShowLog);
    assert_eq!(app.0.checked(root, CommandId::ShowLog), Some(true));
    app.clear_log();
    assert!(app.log().is_empty());
}

/// Warnings of a folder comparison appear as log entries with their codes.
#[cfg(unix)]
#[test]
fn folder_warnings_reach_the_log_with_their_codes() {
    use std::os::unix::fs::PermissionsExt;
    if hikasa_session::capture_env().uid == Some(0) {
        return; // root reads every folder
    }
    let world = World::new();
    let mut app = world.app(None);
    world.write("w/l/locked/a.txt", "a");
    world.write("w/r/locked/a.txt", "a");
    let locked = world.path("w/l/locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    world.open(&mut app, &world.path("w/l"), &world.path("w/r"));
    world.settle(&mut app);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let warned: Vec<_> = app
        .log()
        .entries()
        .filter(|e| e.kind == hikasa_session::log::LogKind::Warning)
        .map(|e| e.text.clone())
        .collect();
    assert!(
        warned.iter().any(|t| t.starts_with("[entry_error]")),
        "{warned:?}"
    );
}

// ---- A4.8 polling and notices ---------------------------------------------------------------------------------

#[test]
fn settings_files_changed_on_disk_raise_notices_and_reload() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    // options.toml created: reloaded (later calls see it), info notice.
    world.write(
        "config/options.toml",
        "schema_version = 1\n[text_editing]\nnumber_of_context_lines = 9\n",
    );
    let events = watcher.poll_once().expect("polled");
    app.on_watch(events);
    world.settle(&mut app);
    assert_eq!(world.service.options().context_lines, 9);
    assert_eq!(app.options().text_editing.number_of_context_lines, 9);
    let text = |app: &TestApp, key: &str| {
        app.window(root)
            .unwrap()
            .notices
            .get(key)
            .map(|n| (n.kind, n.text.clone()))
    };
    assert_eq!(
        text(&app, "settings:options.toml"),
        Some((
            NoticeKind::Info,
            "Settings reloaded: options.toml changed on disk.".into()
        ))
    );
    assert!(
        app.log()
            .entries()
            .any(|e| e.text.starts_with("Settings reloaded"))
    );
    // A problem: a warning naming the key reset to its default.
    world.write(
        "config/options.toml",
        "schema_version = 1\n[tweaks.metrics]\nmax_mru_items = 0\n",
    );
    app.on_watch(watcher.poll_once().expect("polled"));
    world.settle(&mut app);
    let (kind, message) = text(&app, "settings:options.toml").expect("a notice");
    assert_eq!(kind, NoticeKind::Warning);
    assert!(message.contains("max_mru_items"), "{message}");
    // commands.toml: the tables are rebuilt.
    world.write(
        "config/commands.toml",
        "schema_version = 1\n[view.home.command.About]\nshortcuts = [\"Ctrl+F12\"]\n",
    );
    app.on_watch(watcher.poll_once().expect("polled"));
    let effects = world.settle(&mut app);
    assert!(effects.contains(&Effect::ApplyCommands));
    assert_eq!(
        app.dispatch_key(root, &sc("Ctrl+F12"), crate::commands::FocusArea::Root),
        Some(CommandId::About)
    );
    // Removed: defaults apply.
    std::fs::remove_file(world.path("config/commands.toml")).expect("rm");
    app.on_watch(watcher.poll_once().expect("polled"));
    world.settle(&mut app);
    assert_eq!(
        app.dispatch_key(root, &sc("Ctrl+F12"), crate::commands::FocusArea::Root),
        None
    );
    assert!(
        text(&app, "settings:commands.toml")
            .expect("notice")
            .1
            .contains("removed: defaults apply")
    );
    // A format file: the registry is reloaded; a comparison started later uses it, a running one keeps its own.
    let running = world.service.registry();
    world
        .service
        .format_new("w4test", None, Some("W4 test"), Some("*.w4t"), None)
        .expect("format");
    let events = watcher.poll_once().expect("polled");
    app.on_watch(events);
    world.settle(&mut app);
    assert!(world.service.registry().find("w4test").is_some());
    assert!(running.find("w4test").is_none());
    assert!(
        text(&app, "settings:w4test.toml")
            .is_some_and(|(_, t)| t.contains("open comparisons use the new formats when reloaded"))
    );
    // filters.toml: a refreshed snapshot, an info notice. state.toml: nothing.
    world.write("config/filters.toml", "schema_version = 1\n");
    world.write(
        "config/state.toml",
        "schema_version = 1\nlast_session = \"x\"\n",
    );
    app.on_watch(watcher.poll_once().expect("polled"));
    world.settle(&mut app);
    assert!(text(&app, "settings:filters.toml").is_some());
    assert!(text(&app, "settings:state.toml").is_none());
    // The GUI's own writes raise nothing (acknowledged before the job returns).
    app.set_large_buttons(root, true);
    world.settle(&mut app);
    assert!(world.path("config/commands.toml").exists());
    let before = app.window(root).unwrap().notices.len();
    let events = watcher.poll_once().expect("polled");
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(app.window(root).unwrap().notices.len(), before);
    app.shutdown();
    app.flush(Duration::from_secs(10));
    assert!(watcher.poll_once().is_none(), "Stop ends the watcher");
}

#[test]
fn input_files_changed_on_disk() {
    for auto in [true, false] {
        let options = format!(
            "schema_version = 1\n[tweaks.file_operations]\nautomatically_reload = {auto}\n"
        );
        let world = World::with_files(&[("options.toml", &options)]);
        let mut app = world.app(None);
        let root = app.root();
        let (mut watcher, link) = Watcher::new(world.service.clone());
        app.set_watch(link);
        let (l, r) = text_pair(&world);
        let view = world.open(&mut app, &l, &r);
        world.settle(&mut app);
        assert!(
            watcher.poll_once().expect("polled").is_empty(),
            "tracked from now"
        );
        std::fs::write(&l, "changed\n").expect("write");
        let events = watcher.poll_once().expect("polled");
        assert!(
            matches!(events.as_slice(), [WatchEvent::Inputs(c)] if c.len() == 1),
            "{events:?}"
        );
        app.on_watch(events);
        let key = format!("input:{}:{}", view.0, l.display());
        let notice = app
            .window(root)
            .unwrap()
            .notices
            .get(&key)
            .cloned()
            .expect("a notice");
        if auto {
            assert_eq!(notice.kind, NoticeKind::Info);
            assert!(app.busy(), "reloading");
        } else {
            assert_eq!(notice.kind, NoticeKind::Warning);
            assert_eq!(notice.actions, vec![NoticeAction::ReloadView(view)]);
            assert!(!app.busy());
            app.notice_action(root, &key, NoticeAction::ReloadView(view));
            assert!(app.busy(), "Reload reloads");
        }
        world.settle(&mut app);
        let Some(ViewModel::Text(m)) = app.tab(view).and_then(|t| t.model()) else {
            panic!("text")
        };
        assert_eq!(
            m.response()
                .expect("compared")
                .comparison
                .document(hikasa_core::Side::Left)
                .line_count(),
            1
        );
        // Removed: a warning.
        std::fs::remove_file(&r).expect("rm");
        app.on_watch(watcher.poll_once().expect("polled"));
        let key = format!("input:{}:{}", view.0, r.display());
        let notice = app
            .window(root)
            .unwrap()
            .notices
            .get(&key)
            .cloned()
            .expect("a notice");
        assert_eq!(notice.kind, NoticeKind::Warning);
        assert!(notice.text.ends_with("was removed or renamed."));
        world.settle(&mut app);
    }
    // `check_for_files_changed_on_disk = false`: nothing is watched.
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tweaks.file_operations]\ncheck_for_files_changed_on_disk = false\n",
    )]);
    let mut app = world.app(None);
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let (l, r) = text_pair(&world);
    world.open(&mut app, &l, &r);
    world.settle(&mut app);
    std::fs::write(&l, "changed\n").expect("write");
    assert!(watcher.poll_once().expect("polled").is_empty());
}

// ---- A4.9 drops -----------------------------------------------------------------------------------------------

#[test]
fn drops_fill_fields_open_comparisons_or_explain() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (l, r) = text_pair(&world);
    let home = |app: &TestApp| match app.active_tab().and_then(|t| t.model()) {
        Some(ViewModel::Home(h)) => (h.panel.left.clone(), h.panel.right.clone()),
        _ => panic!("home"),
    };
    app.drop_paths(
        root,
        DropTarget::PathField(FieldId::HomeRight),
        vec![l.clone()],
    );
    assert_eq!(home(&app), (String::new(), l.display().to_string()));
    app.drop_paths(
        root,
        DropTarget::PathField(FieldId::HomeLeft),
        vec![r.clone(), l.clone()],
    );
    assert_eq!(
        home(&app),
        (r.display().to_string(), l.display().to_string())
    );
    // Bytes-only drops carry no path: nothing happens.
    app.drop_paths(root, DropTarget::Window, Vec::new());
    assert!(app.active_tab().unwrap().message.is_none());
    // Three paths: the message.
    app.drop_paths(
        root,
        DropTarget::Window,
        vec![l.clone(), r.clone(), l.clone()],
    );
    assert_eq!(
        app.active_tab()
            .unwrap()
            .message
            .as_ref()
            .map(|m| m.text.as_str()),
        Some(DROP_ONE_OR_TWO)
    );
    // Two paths: a comparison of the inferred kind, like the command line.
    app.drop_paths(root, DropTarget::Window, vec![l.clone(), r.clone()]);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    let (a, b) = folder_pair(&world);
    app.drop_paths(root, DropTarget::Window, vec![a, b]);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Folder);
}

// ---- W4a review fixes ------------------------------------------------------------------------------------------

/// M1: the watch counts the views of an input: closing one of two views of a pair (a duplicate, or two opens of
/// the same pair) keeps the input watched; a second open keeps a change not polled yet.
#[test]
fn inputs_stay_watched_while_a_view_of_them_is_open() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let (l, r) = text_pair(&world);
    let first = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    assert!(watcher.poll_once().expect("polled").is_empty());
    // A change, then the same pair opens again before the watch polled: the change is still reported.
    std::fs::write(&l, "changed once\n").expect("write");
    let second = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Inputs(c)] if c.len() == 1),
        "{events:?}"
    );
    // Closing one of the two keeps the input watched for the other.
    let index = app.window(root).unwrap().index_of(first).expect("open");
    app.0.close_tab(root, index, CloseReason::User);
    assert_eq!(app.tracked_inputs().len(), 2, "the second view's inputs");
    std::fs::write(&l, "changed twice\n").expect("write");
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Inputs(c)] if c.len() == 1),
        "{events:?}"
    );
    app.on_watch(events);
    let key = format!("input:{}:{}", second.0, l.display());
    assert!(app.window(root).unwrap().notices.get(&key).is_some());
    // A duplicate, then the duplicate closes: still watched.
    let index = app.window(root).unwrap().index_of(second).expect("open");
    app.0.activate(root, index);
    app.0.execute(root, CommandId::TabDuplicate);
    world.settle(&mut app);
    assert_eq!(app.tracked_inputs().len(), 4);
    app.0.execute(root, CommandId::CloseTab);
    assert_eq!(app.tracked_inputs().len(), 2);
    watcher.poll_once().expect("polled");
    std::fs::write(&r, "right changed\n").expect("write");
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Inputs(c)] if c.len() == 1),
        "{events:?}"
    );
    // The last view closes: nothing is watched.
    let index = app.window(root).unwrap().index_of(second).expect("open");
    app.0.close_tab(root, index, CloseReason::User);
    assert!(app.tracked_inputs().is_empty());
    watcher.poll_once().expect("polled");
    assert!(watcher.tracked().is_empty(), "{:?}", watcher.tracked());
}

/// A4.8: blobs (standard input) are never tracked by the GUI's watch.
#[test]
fn blob_inputs_are_not_tracked() {
    let world = World::with_init(|init| {
        init.stdin = Some(hikasa_session::StdinSource::new(None, || {
            Ok(b"from stdin\n".to_vec())
        }));
    });
    let mut app = world.app(None);
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let (_, r) = text_pair(&world);
    world.open(&mut app, Path::new("-"), &r);
    world.settle(&mut app);
    assert!(watcher.poll_once().expect("polled").is_empty());
    assert_eq!(watcher.tracked(), vec![r.display().to_string()]);
}

/// M2: a reload of an outside edit that ends after the window's own edit was made never reverts that edit in
/// memory; both end up applied.
#[test]
fn a_reload_never_reverts_a_newer_edit_of_the_window() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let large = |app: &TestApp| app.tables().view(app.scope()).custom.large_buttons;
    assert!(!large(&app));
    // An outside edit of commands.toml (reload R), then the window's toolbar change (write S) at once.
    world.write(
        "config/commands.toml",
        "schema_version = 1\n[view.home.command.About]\nshortcuts = [\"Ctrl+F12\"]\n",
    );
    app.on_watch(watcher.poll_once().expect("polled"));
    app.set_large_buttons(root, true);
    assert!(large(&app), "the window follows at once");
    let began = Instant::now();
    loop {
        for msg in world.mailbox.drain() {
            app.handle(msg);
        }
        app.handle(crate::app::UiMsg::Tick);
        assert!(large(&app), "the edit was reverted by an older snapshot");
        if !app.busy() {
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(large(&app));
    assert_eq!(
        app.dispatch_key(root, &sc("Ctrl+F12"), crate::commands::FocusArea::Root),
        Some(CommandId::About),
        "the outside edit applies too"
    );
    assert!(
        app.window(root)
            .unwrap()
            .notices
            .get("settings:commands.toml")
            .is_some()
    );
}

/// M3/L6: the window's own write of `options.toml` reaches the service (later comparisons) at once.
#[test]
fn the_windows_own_options_write_reloads_the_service() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    app.0.execute(root, CommandId::TabNew);
    app.0.execute(root, CommandId::TabNew);
    app.0.execute(root, CommandId::TabCloseOthers);
    app.set_do_not_ask(root, true);
    app.dialog_button(root, ButtonRole::Yes);
    world.settle(&mut app);
    assert!(!app.options().tabs.warn_when_closing_multiple_tabs);
    assert!(
        !world
            .service
            .settings_snapshot()
            .unwrap()
            .options
            .value()
            .tabs
            .warn_when_closing_multiple_tabs
    );
    // The service's own options were reloaded by the job (D-119).
    let loaded = world.service.loaded_settings();
    assert!(loaded.ignored_keys.is_empty());
}

/// M5: a log file that cannot be written stops the saving with one notice: no loop of failures through the log;
/// changed settings resume it.
#[test]
fn a_failing_log_file_stops_saving_once() {
    let world = World::new();
    let blocker = world.write("work/blocker", "a file");
    let bad = blocker.join("log.txt");
    world.write(
        "config/options.toml",
        &format!(
            "schema_version = 1\n[tweaks.log_panel]\nsave_to_file = {:?}\n",
            bad.display().to_string()
        ),
    );
    let mut app = world.app(None);
    let root = app.root();
    app.push_log("first");
    world.settle(&mut app);
    let notices = app.window(root).unwrap().notices.len();
    assert!(app.window(root).unwrap().notices.get("log:save").is_some());
    let entries = app.log().len();
    // More lines: nothing is saved, no job starts, no new notice.
    for i in 0..5 {
        app.push_log(&format!("more {i}"));
        app.handle(crate::app::UiMsg::Tick);
        assert_eq!(app.log_jobs(), 0);
    }
    world.settle(&mut app);
    assert_eq!(app.window(root).unwrap().notices.len(), notices);
    assert_eq!(app.log().len(), entries + 5);
    // The settings change (a valid file): saving resumes.
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let good = world.path("work/log.txt");
    world.write(
        "config/options.toml",
        &format!(
            "schema_version = 1\n[tweaks.log_panel]\nsave_to_file = {:?}\n",
            good.display().to_string()
        ),
    );
    app.on_watch(watcher.poll_once().expect("polled"));
    world.settle(&mut app);
    app.push_log("saved again");
    world.settle(&mut app);
    let text = std::fs::read_to_string(&good).expect("the log file");
    assert!(text.contains("saved again"), "{text}");
}

/// M5: the tracing layer posts `hikasa::*` debug events only while `debug_messages` is on.
#[test]
fn debug_events_reach_the_mailbox_only_with_debug_messages() {
    use tracing_subscriber::layer::SubscriberExt as _;
    let world = World::new();
    let mut app = world.app(None);
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
    app.set_debug_flag(Arc::clone(&flag));
    assert!(
        !flag.load(std::sync::atomic::Ordering::Relaxed),
        "synced: off"
    );
    let subscriber = tracing_subscriber::registry().with(crate::LogForward {
        mailbox: world.mailbox.clone(),
        debug: Arc::clone(&flag),
    });
    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(target: "hikasa::test", "off");
        assert!(world.mailbox.is_empty());
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::debug!(target: "hikasa::test", "on");
    });
    assert_eq!(world.mailbox.drain().len(), 1);
}

/// M6: the log lines reach the file in order (one append at a time).
#[test]
fn log_lines_are_saved_in_order() {
    let world = World::new();
    let file = world.path("work/ordered.log");
    world.write(
        "config/options.toml",
        &format!(
            "schema_version = 1\n[tweaks.log_panel]\nsave_to_file = {:?}\n",
            file.display().to_string()
        ),
    );
    let mut app = world.app(None);
    for i in 0..60 {
        app.push_log(&format!("line {i:03}"));
        app.handle(crate::app::UiMsg::Tick);
        assert!(app.log_jobs() <= 1);
    }
    world.settle(&mut app);
    let text = std::fs::read_to_string(&file).expect("the log file");
    let found: Vec<&str> = text
        .lines()
        .filter_map(|l| l.split("test: ").nth(1))
        .collect();
    let expected: Vec<String> = (0..60).map(|i| format!("line {i:03}")).collect();
    assert_eq!(found, expected);
}

/// L4: "Do not ask again" is written before the window closes or the program quits.
#[test]
fn do_not_ask_comes_before_the_close() {
    use crate::dialog::{DialogModel, DialogStep, Followup, Prompt};
    let world = World::new();
    let mut prompt = Prompt::confirm("Exit", "Close 2 tabs?", Some("x.y"), vec![Followup::Exit]);
    prompt.ticked = true;
    assert_eq!(
        prompt.on_button(ButtonRole::Yes, &world.service),
        DialogStep::CloseWith(vec![Followup::DisableOption("x.y"), Followup::Exit])
    );
}

/// L10: the notices about a view go when it closes.
#[test]
fn notices_of_a_closed_view_go_with_it() {
    let world = World::with_files(&[(
        "options.toml",
        "schema_version = 1\n[tweaks.file_operations]\nautomatically_reload = false\n",
    )]);
    let mut app = world.app(None);
    let root = app.root();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    app.set_watch(link);
    let (l, r) = text_pair(&world);
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    watcher.poll_once().expect("polled");
    std::fs::write(&l, "changed\n").expect("write");
    app.on_watch(watcher.poll_once().expect("polled"));
    let key = format!("input:{}:{}", view.0, l.display());
    assert!(app.window(root).unwrap().notices.get(&key).is_some());
    let index = app.window(root).unwrap().index_of(view).expect("open");
    app.0.close_tab(root, index, CloseReason::User);
    assert!(app.window(root).unwrap().notices.get(&key).is_none());
}

/// L13: a reorder inside one window counts the drop position before the move.
#[test]
fn reordering_tabs_counts_positions_before_the_move() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let views = open_texts(&world, &mut app, root, 3);
    let order = |app: &TestApp| -> Vec<_> {
        app.window(root)
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.id)
            .collect()
    };
    // [A, B, C]: A dropped before C → [B, A, C].
    app.0.move_tab(root, 0, Some(root), Some(2));
    assert_eq!(order(&app), [views[1], views[0], views[2]]);
    assert_eq!(app.window(root).unwrap().active, 1);
    // B dropped at the end → [A, C, B].
    app.0.move_tab(root, 0, Some(root), Some(3));
    assert_eq!(order(&app), [views[0], views[2], views[1]]);
    // C dropped at the front → [C, A, B].
    app.0.move_tab(root, 1, Some(root), Some(0));
    assert_eq!(order(&app), [views[2], views[0], views[1]]);
    world.settle(&mut app);
}

/// L3: a window's dialogs are polled once per frame, whatever its number of tabs.
#[test]
fn dialogs_are_polled_once_per_window() {
    use crate::dialog::{ButtonRole as Role, DialogButton, DialogModel, DialogStep};
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counting(Arc<AtomicUsize>);
    impl DialogModel for Counting {
        fn title(&self) -> String {
            "Counting".into()
        }
        fn help_topic(&self) -> &'static str {
            "main-window"
        }
        fn buttons(&self) -> Vec<DialogButton> {
            Vec::new()
        }
        fn on_button(&mut self, _: Role, _: &hikasa_session::Service) -> DialogStep {
            DialogStep::Close
        }
        fn poll(&mut self, _: &hikasa_session::Service) -> bool {
            self.0.fetch_add(1, Ordering::SeqCst);
            false
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    app.0.execute(root, CommandId::TabNew);
    app.0.execute(root, CommandId::TabNew);
    let polls = Arc::new(AtomicUsize::new(0));
    app.window_mut(root)
        .unwrap()
        .dialogs
        .push(Box::new(Counting(Arc::clone(&polls))));
    app.handle(crate::app::UiMsg::Tick);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
}

/// L8: a user binding of `Ctrl+=` (or `Ctrl+0`, `Ctrl+-`) wins over the keypad alias egui cannot tell apart.
#[test]
fn ctrl_equals_bindings_fire() {
    let world = World::with_files(&[(
        "commands.toml",
        "schema_version = 1\n[view.home.command.About]\nshortcuts = [\"Ctrl+=\", \"Ctrl+0\"]\n",
    )]);
    let app = world.app(None);
    let root = app.root();
    for key in [egui::Key::Equals, egui::Key::Num0] {
        let candidates = crate::ui::keys::shortcuts_of(key, egui::Modifiers::CTRL);
        assert_eq!(
            app.dispatch_keys(root, &candidates, crate::commands::FocusArea::Root),
            Some(CommandId::About),
            "{key:?}"
        );
    }
}

/// D-116: the error of a drop that cannot be opened reaches the log book.
#[test]
fn drop_errors_reach_the_log() {
    let world = World::new();
    let mut app = world.app(None);
    let root = app.root();
    let missing = world.path("work/missing-a");
    let other = world.path("work/missing-b");
    app.drop_paths(root, DropTarget::Window, vec![missing, other]);
    world.settle(&mut app);
    assert!(
        app.log()
            .entries()
            .any(|e| e.kind == hikasa_session::log::LogKind::Error && e.source == "drop"),
        "{:?}",
        app.log()
            .entries()
            .map(|e| e.text.clone())
            .collect::<Vec<_>>()
    );
}

/// D-117: the first-run format files are the program's own writes: no poll reports them, however the writes and the
/// polls interleave (they run on one thread); a file someone else adds afterwards is reported as usual.
#[test]
fn the_first_run_format_files_raise_no_notice() {
    use crate::startup::Deferred;
    use crate::watch::WatchEvent;
    // The cause: written behind the watch's back, each file is a change on disk.
    let world = World::new();
    let (mut watcher, _link) = Watcher::new(world.service.clone());
    hikasa_session::FormatStore::new(world.service.config())
        .materialize()
        .expect("materialize");
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Config(c)] if c.len() > 1),
        "{events:?}"
    );

    let world = World::new();
    let (mut watcher, link) = Watcher::new(world.service.clone());
    let mut app = world.app(None);
    world.settle(&mut app);
    let mut deferred = Deferred::materialize_formats(&world.service, Some(link));
    deferred.start().expect("started").join().expect("sent");
    let events = watcher.poll_once().expect("polled");
    assert_eq!(events, [], "the program's own files");
    assert!(world.path("config/formats/order.toml").is_file());
    assert_eq!(app.on_watch(events), []);
    world.settle(&mut app);
    let root = app.root();
    let now = app.window_mut(root).map(|w| w.notices.visible(0.0).len());
    assert_eq!(now, Some(0), "no notice");
    assert!(watcher.poll_once().expect("polled").is_empty(), "nor later");

    world.write("config/formats/mine.toml", "x = 1\n");
    let events = watcher.poll_once().expect("polled");
    assert!(
        matches!(events.as_slice(), [WatchEvent::Config(_)]),
        "{events:?}"
    );
}
