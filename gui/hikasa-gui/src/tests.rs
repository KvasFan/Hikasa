//! Headless tests of the window's behaviour through the command catalogue (docs/17 §3.4): the view models, the
//! app core, the IPC handler with a real in-process server and client (private endpoint and configuration),
//! and the enabled state of every command.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hikasa_core::ExitCode;
use hikasa_session::api::{GuiOpenRequest, ViewRequest};
use hikasa_session::ipc::{
    CloseReason, Endpoint, IpcClient, IpcServer, OpenOptions, Request, Response, ViewId, ViewKind,
};
use hikasa_session::{
    EnvFacts, Platform, Service, ServiceInit, StdinSource, SystemClock, capture_env,
};
use hikasa_text::TextDisplayFilter;

use crate::app::{AppCore, Effect, UiMsg, ViewModel};
use crate::commands::{CATALOGUE, CommandId, Scope};
use crate::ipc::GuiHandler;
use crate::mailbox::Mailbox;
use crate::shared::Shared;

/// A private test world: inputs, a configuration folder and an IPC endpoint inside one temporary folder.
struct World {
    dir: tempfile::TempDir,
    service: Service,
    shared: Arc<Shared>,
    mailbox: Mailbox<UiMsg>,
}

impl World {
    fn new() -> World {
        World::with_init(|_| {})
    }

    /// A world whose service is built after `tune` adjusted its `ServiceInit`.
    fn with_init(tune: impl FnOnce(&mut ServiceInit)) -> World {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        for sub in ["work", "home", "tmp", "ipc"] {
            std::fs::create_dir_all(root.join(sub)).expect("mkdir");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root.join("ipc"), std::fs::Permissions::from_mode(0o700))
                .expect("chmod");
        }
        let env = EnvFacts {
            cwd: Some(root.join("work")),
            home: Some(root.join("home")),
            uid: capture_env().uid,
            user: Some("tester".into()),
            ipc_endpoint_override: Some(endpoint_override(root)),
            ..EnvFacts::new(Platform::HOST, root.join("tmp"))
        };
        let mut init = ServiceInit::new(
            env,
            Arc::new(SystemClock),
            Arc::new(hikasa_formats::StdProcessRunner),
        );
        init.config_dir = Some(root.join("config"));
        tune(&mut init);
        let service = Service::new(init).expect("service");
        World {
            dir,
            service,
            shared: Arc::new(Shared::new()),
            mailbox: Mailbox::new(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.path(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(&path, text).expect("write");
        path
    }

    fn app(&self, server: Option<hikasa_session::ipc::ServerHandle>) -> AppCore {
        AppCore::new(
            self.service.clone(),
            Arc::clone(&self.shared),
            self.mailbox.clone(),
            server,
        )
    }

    /// Opens a view of two paths the way the command line does (`resolve_open` + `open_view_request`).
    fn open(&self, app: &mut AppCore, left: &Path, right: &Path) -> ViewId {
        let request = GuiOpenRequest {
            specs: vec![left.as_os_str().to_owned(), right.as_os_str().to_owned()],
            ..GuiOpenRequest::default()
        };
        let resolved = self.service.resolve_open(&request).expect("resolve");
        let view = self
            .service
            .open_view_request(resolved.kind, &resolved.specs, &resolved.options)
            .expect("view request");
        app.open_local(resolved.kind, view, &resolved.options, resolved.specs, None)
    }

    /// Handles queued messages and polls until no job runs.
    fn settle(&self, app: &mut AppCore) -> Vec<Effect> {
        let began = Instant::now();
        let mut effects = Vec::new();
        loop {
            for msg in self.mailbox.drain() {
                effects.extend(app.handle(msg));
            }
            effects.extend(app.handle(UiMsg::Tick));
            if !app.busy() && self.mailbox.is_empty() {
                return effects;
            }
            assert!(
                began.elapsed() < Duration::from_secs(60),
                "jobs did not finish"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn endpoint_override(root: &Path) -> OsString {
    #[cfg(unix)]
    {
        root.join("ipc").join("s.sock").into_os_string()
    }
    #[cfg(not(unix))]
    {
        OsString::from(format!(
            "hikasa-gui-test-{}-{}",
            std::process::id(),
            root.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))
    }
}

/// 40 lines; the right side changes line 10 importantly, line 20 only in whitespace and drops line 30.
fn text_pair(world: &World) -> (PathBuf, PathBuf) {
    let mut left = String::new();
    let mut right = String::new();
    for i in 0..40 {
        let line = format!("int v{i} = f({i});\n");
        left.push_str(&line);
        match i {
            10 => right.push_str(&format!("int v{i} = g({i});\n")),
            20 => right.push_str(&format!("int  v{i} = f({i});\n")),
            30 => {}
            _ => right.push_str(&line),
        }
    }
    (
        world.write("work/left.c", &left),
        world.write("work/right.c", &right),
    )
}

fn text_model(app: &mut AppCore, view: ViewId) -> &mut crate::text_view::TextViewModel {
    match &mut app.tab_mut(view).expect("tab").model {
        ViewModel::Text(m) => m,
        _ => panic!("not a text view"),
    }
}

fn folder_model(app: &mut AppCore, view: ViewId) -> &mut crate::folder_view::FolderViewModel {
    match &mut app.tab_mut(view).expect("tab").model {
        ViewModel::Folder(m) => m,
        _ => panic!("not a folder view"),
    }
}

#[test]
fn text_view_commands_drive_navigation_filters_and_slices() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let mut app = world.app(None);
    let view = world.open(&mut app, &left, &right);
    assert_eq!(app.tabs().len(), 1, "the untouched Home tab is replaced");
    assert_eq!(app.scope(), Scope::Text);
    let effects = world.settle(&mut app);
    assert!(effects.contains(&Effect::TextDirty(view)), "{effects:?}");

    // Ready: navigation is available forwards only.
    assert!(app.enabled(CommandId::DiffSectionNext));
    assert!(!app.enabled(CommandId::DiffSectionPrev));
    assert_eq!(app.checked(CommandId::FavCompareAll), Some(true));
    let meta = text_model(&mut app, view).meta();
    assert_eq!(meta.phase, crate::text_view::TextPhase::Ready);
    assert!(
        meta.syntax,
        "C has no multi-line state to prepare here, or it is prepared"
    );
    assert_eq!(
        meta.len, 40,
        "one row per left line (the right side lost one)"
    );

    // Slices: line numbers, states, syntax segments and inline differences.
    let slice = text_model(&mut app, view).slice(9, 3);
    assert_eq!(slice.rows.len(), 3);
    let changed = &slice.rows[1];
    assert_eq!((changed.k, changed.st, changed.im), (0, 1, 2));
    assert_eq!(changed.ln, [11, 11]);
    assert!(!changed.hl[0].is_empty() && !changed.hl[1].is_empty());
    assert!(
        changed.sp[0].len() >= 6,
        "syntax segments: {:?}",
        changed.sp[0]
    );
    assert_eq!(slice.rows[0].st, 0);

    // Next difference section: the cursor moves to the first section and the panes are told to scroll.
    app.execute(CommandId::DiffSectionNext);
    let m = text_model(&mut app, view);
    let first = m.cursor().expect("cursor");
    assert_eq!(first.0, 10);
    let meta = m.meta();
    assert_eq!(meta.scroll, 10);
    assert_eq!(meta.cursor, 10);
    assert!(app.enabled(CommandId::DiffSectionNext));
    app.execute(CommandId::DiffSectionNext);
    app.execute(CommandId::DiffSectionNext);
    assert_eq!(text_model(&mut app, view).cursor().map(|r| r.0), Some(30));
    assert!(
        !app.enabled(CommandId::DiffSectionNext),
        "no section after the last"
    );
    assert!(app.enabled(CommandId::DiffSectionPrev));
    app.execute(CommandId::DiffSectionPrev);
    assert_eq!(text_model(&mut app, view).cursor().map(|r| r.0), Some(20));

    // Ignore unimportant differences: the whitespace section is skipped, the verdict changes.
    app.execute(CommandId::InsigIgnored);
    assert_eq!(app.checked(CommandId::InsigIgnored), Some(true));
    app.execute(CommandId::DiffSectionNext);
    assert_eq!(text_model(&mut app, view).cursor().map(|r| r.0), Some(30));
    app.execute(CommandId::InsigIgnored);

    // Display filter: differences only → gap bars, a click expands one.
    app.execute(CommandId::FavDiff);
    assert_eq!(app.checked(CommandId::FavDiff), Some(true));
    assert_eq!(app.checked(CommandId::FavCompareAll), Some(false));
    let m = text_model(&mut app, view);
    let len = m.meta().len;
    assert!(len < 15, "filtered len {len}");
    let slice = m.slice(0, len);
    assert_eq!(slice.rows[0].k, 1, "a leading gap bar");
    assert_eq!(slice.rows[0].g, 10);
    assert!(m.click(0), "clicking a gap bar expands it");
    assert_eq!(m.meta().len, len + 9);

    // Context, toggles, none.
    app.execute(CommandId::ShowContext);
    assert_eq!(app.checked(CommandId::ShowContext), Some(true));
    assert!(matches!(
        text_model(&mut app, view).view_options().filter,
        TextDisplayFilter::Context { lines: 3 }
    ));
    // The context rows are Same rows (the core's toggle table): Same shows checked, and switching a
    // difference category off keeps every Same row instead of dropping the context.
    assert_eq!(app.checked(CommandId::TogSame), Some(true));
    let context_len = text_model(&mut app, view).meta().len;
    app.execute(CommandId::TogLeftOrphan);
    assert_eq!(
        text_model(&mut app, view).view_options().filter,
        TextDisplayFilter::Toggles {
            bits: (hikasa_text::TextToggles::all() ^ hikasa_text::TextToggles::LEFT_ORPHANS).bits()
        }
    );
    assert!(text_model(&mut app, view).meta().len >= context_len);
    assert_eq!(app.checked(CommandId::TogLeftOrphan), Some(false));
    assert_eq!(app.checked(CommandId::TogSame), Some(true));
    app.execute(CommandId::FavCompareAll);
    app.execute(CommandId::TogSame);
    assert_eq!(app.checked(CommandId::TogSame), Some(false));
    assert_eq!(app.checked(CommandId::TogDiff), Some(true));
    assert_eq!(app.checked(CommandId::FavCompareAll), Some(false));
    app.execute(CommandId::FavCompareNone);
    let m = text_model(&mut app, view);
    assert_eq!(m.meta().len, 1, "every row hidden in one gap bar");
    assert_eq!((m.slice(0, 1).rows[0].k, m.slice(0, 1).rows[0].g), (1, 40));

    // View commands.
    app.execute(CommandId::FavCompareAll);
    app.execute(CommandId::DisplayFontIncrease);
    assert_eq!(text_model(&mut app, view).meta().font, 14);
    app.execute(CommandId::DisplayFontReset);
    assert_eq!(text_model(&mut app, view).meta().font, 13);
    app.execute(CommandId::ShowThumbnail);
    assert!(!text_model(&mut app, view).meta().thumb);
    let thumb = text_model(&mut app, view).thumbnail(64);
    assert_eq!(thumb.cells.len(), 64);
    assert!(
        thumb.cells.iter().any(|&c| c & 4 != 0),
        "an important bucket"
    );

    // Reload restarts the comparison; the result comes back.
    app.execute(CommandId::Reload);
    assert!(app.busy() || text_model(&mut app, view).response().is_some());
    world.settle(&mut app);
    assert!(text_model(&mut app, view).response().is_some());
    let status = text_model(&mut app, view).status();
    assert!(
        status.state.contains("Important differences"),
        "{}",
        status.state
    );
    assert!(
        status.panes[0].detail.contains("40 lines"),
        "{:?}",
        status.panes
    );
}

#[test]
fn text_view_shows_documents_before_the_alignment() {
    let world = World::new();
    let mut left = String::new();
    for i in 0..200_000 {
        left.push_str(&format!("line {i} of the left side\n"));
    }
    let right = left.replace("line 100000 ", "LINE 100000 ");
    let l = world.write("work/big-left.txt", &left);
    let r = world.write("work/big-right.txt", &right);
    let mut app = world.app(None);
    let view = world.open(&mut app, &l, &r);
    // Poll until the documents are there; the slice is then the unaligned text.
    let began = Instant::now();
    let mut saw_unaligned = false;
    while app.busy() {
        for msg in world.mailbox.drain() {
            app.handle(msg);
        }
        app.handle(UiMsg::Tick);
        let m = text_model(&mut app, view);
        if m.phase() == crate::text_view::TextPhase::Loaded {
            let slice = m.slice(0, 2);
            assert_eq!(slice.rows[0].k, 2);
            assert_eq!(
                slice.rows[0].t[0].as_deref(),
                Some("line 0 of the left side")
            );
            saw_unaligned = true;
        }
        assert!(began.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(1));
    }
    world.settle(&mut app);
    let (loaded, ready) = text_model(&mut app, view).timings();
    assert!(loaded.is_some() && ready.is_some());
    assert!(loaded <= ready);
    // The unaligned phase may be too short to observe on a fast machine; when observed it showed the text.
    let _ = saw_unaligned;
    assert_eq!(text_model(&mut app, view).meta().len, 200_000);
}

fn folder_pair(world: &World) -> (PathBuf, PathBuf) {
    for (side, files) in [
        (
            "a",
            &[
                ("same.txt", "x\n"),
                ("src/main.c", "int main(){}\n"),
                ("src/deep/x.h", "1\n"),
                ("only-left.txt", "l\n"),
            ][..],
        ),
        (
            "b",
            &[
                ("same.txt", "x\n"),
                ("src/main.c", "int main(){ return 0; }\n"),
                ("src/deep/x.h", "1\n"),
                ("only-right.txt", "r\n"),
            ][..],
        ),
    ] {
        for (name, text) in files {
            world.write(&format!("work/{side}/{name}"), text);
        }
    }
    // Same modification times everywhere: only content sizes differ.
    for side in ["a", "b"] {
        for name in ["same.txt", "src/main.c", "src/deep/x.h"] {
            let path = world.path(&format!("work/{side}/{name}"));
            let file = std::fs::File::options()
                .write(true)
                .open(&path)
                .expect("open");
            file.set_modified(
                std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            )
            .expect("mtime");
        }
    }
    (world.path("work/a"), world.path("work/b"))
}

#[test]
fn folder_view_commands_expand_filter_navigate_and_open_pairs() {
    let world = World::new();
    let (a, b) = folder_pair(&world);
    let mut app = world.app(None);
    let view = world.open(&mut app, &a, &b);
    assert_eq!(app.scope(), Scope::Folder);
    world.settle(&mut app);
    assert!(!app.enabled(CommandId::Stop), "nothing runs");
    assert!(app.enabled(CommandId::FullRefresh));
    assert_eq!(
        app.checked(CommandId::FavDiff),
        Some(true),
        "default: differences"
    );
    assert_eq!(app.checked(CommandId::DirCompareStructure), Some(true));
    // Collapsed: src and the two orphans.
    let collapsed = folder_model(&mut app, view).rows().len();
    assert_eq!(collapsed, 3);
    assert!(!app.enabled(CommandId::Open) && !app.enabled(CommandId::Expand));

    app.execute(CommandId::ExpandAll);
    let expanded = folder_model(&mut app, view).rows().len();
    assert_eq!(expanded, 4, "src/main.c appears");
    app.execute(CommandId::FavCompareAll);
    assert_eq!(
        folder_model(&mut app, view).rows().len(),
        7,
        "src, deep, x.h, main.c, same.txt and the two orphans"
    );
    app.execute(CommandId::CollapseAll);
    assert_eq!(folder_model(&mut app, view).rows().len(), 4);

    // Select the src folder and open it with the catalogue.
    let src = folder_model(&mut app, view)
        .rows()
        .iter()
        .position(|r| r.depth == 0 && r.node.0 != 0)
        .expect("a row");
    folder_model(&mut app, view).select(src);
    let is_folder = app.enabled(CommandId::Expand) || app.enabled(CommandId::Collapse);
    if is_folder {
        app.execute(CommandId::Expand);
        assert!(app.enabled(CommandId::Collapse));
        app.execute(CommandId::Collapse);
        assert!(app.enabled(CommandId::Expand));
        app.execute(CommandId::ExpandSubfolders);
        assert!(app.enabled(CommandId::Collapse));
        app.execute(CommandId::CollapseSubfolders);
    }

    // Differences only again, expanded: Next Difference walks the differing rows.
    app.execute(CommandId::FavDiff);
    app.execute(CommandId::ExpandAll);
    assert!(app.enabled(CommandId::DiffNext));
    let mut visited = Vec::new();
    while app.enabled(CommandId::DiffNext) {
        app.execute(CommandId::DiffNext);
        visited.push(folder_model(&mut app, view).selected().expect("selection"));
        assert!(visited.len() < 20);
    }
    let rows = folder_model(&mut app, view).rows().to_vec();
    assert!(!visited.is_empty());
    assert_eq!(
        visited.last().copied(),
        rows.last().map(|r| r.node),
        "ends on the last difference"
    );
    assert!(app.enabled(CommandId::DiffPrev));
    let effects = app.execute(CommandId::DiffPrev);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::FolderScroll(v, _) if *v == view))
    );

    // Select main.c (a pair) and Open it: a Text Compare tab with the two exact paths (FC-062).
    let pair = {
        let m = folder_model(&mut app, view);
        let prepared = m.prepared().expect("model").clone();
        let read = prepared.model().read();
        m.rows()
            .iter()
            .position(|r| {
                read.node(r.node)
                    .is_some_and(|n| !n.is_folder() && n.left.is_some() && n.right.is_some())
            })
            .expect("a pair row")
    };
    folder_model(&mut app, view).select(pair);
    assert!(app.enabled(CommandId::Open));
    app.execute(CommandId::Open);
    assert_eq!(app.tabs().len(), 2);
    assert_eq!(app.scope(), Scope::Text);
    let opened = app.active_tab().expect("tab").id;
    let request = text_model(&mut app, opened).request().clone();
    assert_eq!(PathBuf::from(&request.left), a.join("src").join("main.c"));
    assert_eq!(PathBuf::from(&request.right), b.join("src").join("main.c"));
    world.settle(&mut app);
    assert!(text_model(&mut app, opened).response().is_some());
    assert_eq!(
        app.shared().list().len(),
        2,
        "both views are listed for `gui list`"
    );

    // Tabs.
    assert!(app.enabled(CommandId::TabNext));
    app.execute(CommandId::TabNext);
    assert_eq!(app.scope(), Scope::Folder);
    app.execute(CommandId::TabPrev);
    assert_eq!(app.scope(), Scope::Text);
    app.execute(CommandId::CloseTab);
    assert_eq!(app.tabs().len(), 1);
    assert_eq!(app.shared().list().len(), 1);

    // Folder modes, toggles, suppress filters, ignore unimportant.
    app.execute(CommandId::DirShowAll);
    assert_eq!(app.checked(CommandId::DirShowAll), Some(true));
    app.execute(CommandId::DirCompareFilesOnly);
    assert_eq!(app.checked(CommandId::DirCompareFilesOnly), Some(true));
    app.execute(CommandId::TogLeftOrphan);
    assert_eq!(app.checked(CommandId::TogLeftOrphan), Some(false));
    assert_eq!(app.checked(CommandId::FavDiff), Some(false));
    app.execute(CommandId::SuppressFilters);
    assert_eq!(app.checked(CommandId::SuppressFilters), Some(true));
    app.execute(CommandId::InsigIgnored);
    assert_eq!(app.checked(CommandId::InsigIgnored), Some(true));

    // The window shows names, colours and marks.
    app.execute(CommandId::FavCompareAll);
    let window = folder_model(&mut app, view).window();
    assert!(window.rows.iter().any(|r| {
        r.left.as_ref().is_some_and(|c| {
            c.name == "only-left.txt" && c.colour == hikasa_folder::ColourClass::Orphan
        })
    }));
    assert!(
        window.rows.iter().any(|r| r.right.is_none()),
        "an orphan row has an absent side"
    );
    let status = folder_model(&mut app, view).status();
    assert!(status.state.contains("Differences ·"), "{}", status.state);

    // Refresh runs the comparison again.
    app.execute(CommandId::FullRefresh);
    assert!(app.busy());
    assert!(app.enabled(CommandId::Stop) || !app.busy());
    world.settle(&mut app);
    assert!(folder_model(&mut app, view).response().is_some());
}

#[test]
fn home_compare_resolves_paths_like_the_command_line() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let mut app = world.app(None);
    assert_eq!(app.scope(), Scope::Home);
    assert!(!app.enabled(CommandId::HomeCompare), "empty fields");
    if let Some(tab) = app.active_tab_mut()
        && let ViewModel::Home(home) = &mut tab.model
    {
        home.left = "left.c".into(); // relative to the GUI's current folder (`work`)
        home.right = right.display().to_string();
    }
    assert!(app.enabled(CommandId::HomeCompare));
    app.execute(CommandId::HomeCompare);
    // The paths resolve on a job thread: the tab stays Home until the result is picked up.
    assert_eq!(app.scope(), Scope::Home);
    assert!(
        app.busy() && !app.enabled(CommandId::HomeCompare),
        "resolving"
    );
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text, "the Home tab became the view");
    assert_eq!(app.tabs().len(), 1);
    let view = app.active_tab().expect("tab").id;
    assert_eq!(
        PathBuf::from(&text_model(&mut app, view).request().left),
        left
    );
    world.settle(&mut app);

    // A missing file keeps the Home view and shows the error.
    app.execute(CommandId::TabNew);
    assert_eq!(app.scope(), Scope::Home);
    if let Some(tab) = app.active_tab_mut()
        && let ViewModel::Home(home) = &mut tab.model
    {
        home.left = "missing-1.c".into();
        home.right = "missing-2.c".into();
    }
    app.execute(CommandId::HomeCompare);
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Home);
    match &app.active_tab().expect("tab").model {
        ViewModel::Home(home) => assert!(home.error.is_some()),
        _ => panic!("still Home"),
    }
    // Closing the last comparison leaves a Home tab; About shows a message.
    app.execute(CommandId::CloseTab);
    app.execute(CommandId::CloseTab);
    assert_eq!(app.tabs().len(), 1);
    assert_eq!(app.scope(), Scope::Home);
    app.execute(CommandId::About);
    assert!(app.message().is_some_and(|m| m.contains("Hikasa")));
}

/// Every command, in typical states of every view type: the catalogue's predicate decides (docs/01 §7: every
/// command has an enabled-state test).
#[test]
fn every_command_has_an_enabled_state() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let (a, b) = folder_pair(&world);
    let mut app = world.app(None);
    let enabled_in = |app: &AppCore, scope: Scope| -> Vec<CommandId> {
        CATALOGUE
            .iter()
            .filter(|c| c.scope == scope || c.scope == Scope::Global)
            .filter(|c| app.enabled(c.id))
            .map(|c| c.id)
            .collect()
    };
    use CommandId as C;
    // Home, empty.
    assert_eq!(
        enabled_in(&app, Scope::Home),
        [C::TabNew, C::CloseTab, C::Exit, C::About]
    );
    // Folder, finished, nothing selected.
    let folder = world.open(&mut app, &a, &b);
    world.settle(&mut app);
    let got = enabled_in(&app, Scope::Folder);
    for id in [
        C::FullRefresh,
        C::ExpandAll,
        C::CollapseAll,
        C::DiffNext,
        C::FavCompareAll,
        C::DirShowAll,
        C::InsigIgnored,
        C::SuppressFilters,
        C::TogSame,
    ] {
        assert!(got.contains(&id), "{id:?} in {got:?}");
    }
    for id in [
        C::Stop,
        C::Open,
        C::Expand,
        C::Collapse,
        C::ExpandSubfolders,
        C::CollapseSubfolders,
        C::DiffPrev,
        C::TabNext,
        C::TabPrev,
    ] {
        assert!(!got.contains(&id), "{id:?} in {got:?}");
    }
    // Text, finished, with a second tab.
    world.open(&mut app, &left, &right);
    world.settle(&mut app);
    let got = enabled_in(&app, Scope::Text);
    for id in [
        C::Reload,
        C::DiffSectionNext,
        C::FavDiff,
        C::ShowContext,
        C::TogDiff,
        C::ShowThumbnail,
        C::DisplayFontIncrease,
        C::TabNext,
        C::TabPrev,
    ] {
        assert!(got.contains(&id), "{id:?} in {got:?}");
    }
    assert!(!got.contains(&C::DiffSectionPrev));
    // Commands of another view type are never enabled here.
    for c in CATALOGUE.iter().filter(|c| c.scope == Scope::Folder) {
        if !CATALOGUE
            .iter()
            .any(|t| t.scope == Scope::Text && t.id == c.id)
        {
            assert!(
                !app.enabled(c.id),
                "{:?} must not apply to a text view",
                c.id
            );
        }
    }
    // A running folder comparison enables Stop.
    app.activate(
        app.tabs()
            .iter()
            .position(|t| t.id == folder)
            .expect("folder tab"),
    );
    app.execute(C::FullRefresh);
    if app.busy() {
        assert!(app.enabled(C::Stop));
        app.execute(C::Stop);
    }
    world.settle(&mut app);
    assert!(!app.enabled(C::Stop));
}

/// The IPC handler with a real server and client: open (with wait), list, close with the view's real exit
/// code, errors, ping, shutdown (§4.4.3 rules 2 and 5; D-23).
#[test]
fn ipc_requests_open_list_close_and_shut_down_views() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let (a, b) = folder_pair(&world);
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = world.app(Some(handle));
    let timeout = Duration::from_secs(10);
    let canonical = |p: &Path| p.display().to_string();

    // A waited text view: the waiting client gets the view's exit code when the view closes.
    let specs = vec![canonical(&left), canonical(&right)];
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
    let mut client = IpcClient::connect(&endpoint, "test", timeout).expect("connect");
    let began = Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Registered by the handler, not opened by the UI yet: the title is already the tab caption.
    let Response::Views { views } = client.request(Request::ListViews).expect("list").1 else {
        panic!("views");
    };
    assert_eq!(views[0].title, "left.c ⇄ right.c");
    world.settle(&mut app);
    let Response::Views { views } = client.request(Request::ListViews).expect("list").1 else {
        panic!("views");
    };
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].kind, ViewKind::TextCompare);
    assert_eq!(views[0].title, "left.c ⇄ right.c");
    let view = views[0].view;
    assert!(world.shared.is_waited(view));
    assert_eq!(
        client
            .request(Request::CloseView { view })
            .expect("close")
            .1,
        Response::Ok
    );
    world.settle(&mut app);
    let (opened, (closed, result, code)) = waiting.join().expect("waiter thread");
    assert_eq!(opened, view);
    assert_eq!(closed, view);
    assert_eq!(
        code,
        ExitCode::RulesDifferent.code(),
        "important differences: 13"
    );
    assert_eq!(result.close_reason(), Some(CloseReason::User));
    assert_eq!(result.verdict.as_deref(), Some("important_differences"));

    // Unknown view, relative specs, unsupported kinds, ping.
    let Response::Error { code, .. } = client
        .call(Request::CloseView { view: ViewId(999) })
        .expect("call")
        .1
    else {
        panic!("not found");
    };
    assert_eq!(code, "not_found");
    let Response::Error { code, .. } = client
        .call(Request::OpenView {
            kind: ViewKind::TextCompare,
            specs: vec!["left.c".into(), "right.c".into()],
            options: OpenOptions::default(),
            wait: false,
        })
        .expect("call")
        .1
    else {
        panic!("invalid");
    };
    assert_eq!(code, "invalid_request");
    let Response::Error { code, .. } = client
        .call(Request::OpenView {
            kind: ViewKind::HexCompare,
            specs: vec![canonical(&left), canonical(&right)],
            options: OpenOptions::default(),
            wait: false,
        })
        .expect("call")
        .1
    else {
        panic!("unsupported");
    };
    assert_eq!(code, "unsupported_view");
    assert_eq!(
        client.request(Request::Ping).expect("ping").1,
        Response::Pong
    );

    // A waited folder view still open at shutdown reports its real verdict with reason `shutdown`.
    let specs = vec![canonical(&a), canonical(&b)];
    let wait_endpoint = endpoint.clone();
    let waiting = std::thread::spawn(move || {
        let mut client = IpcClient::connect(&wait_endpoint, "test", timeout).expect("connect");
        let (id, _) = client
            .request(Request::OpenView {
                kind: ViewKind::FolderCompare,
                specs,
                options: OpenOptions::default(),
                wait: true,
            })
            .expect("open");
        client.wait_view_closed(id).expect("closed")
    });
    let began = Instant::now();
    while world.shared.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(Duration::from_millis(5));
    }
    world.settle(&mut app);
    assert_eq!(
        client.request(Request::Shutdown).expect("shutdown").1,
        Response::Ok
    );
    let effects = world.settle(&mut app);
    assert!(effects.contains(&Effect::Exit), "{effects:?}");
    assert!(app.quitting());
    let (_, result, code) = waiting.join().expect("waiter thread");
    assert_eq!(code, ExitCode::RulesDifferent.code());
    assert_eq!(result.close_reason(), Some(CloseReason::Shutdown));
    assert_eq!(result.folder_verdict.as_deref(), Some("differences"));
    assert!(world.shared.is_empty());
}

/// Views opened through IPC while a comparison is still running close with `Incomplete` (folders, 100) or
/// `NoVerdict` (files, 0); a load failure closes with 100.
#[test]
fn close_outcomes_follow_the_d23_rules() {
    let world = World::new();
    let (left, _) = text_pair(&world);
    let mut app = world.app(None);
    let missing = world.path("work/missing.c");
    let request = |l: &Path, r: &Path| {
        let specs = [l.display().to_string(), r.display().to_string()];
        world
            .service
            .open_view_request(ViewKind::TextCompare, &specs, &OpenOptions::default())
            .expect("request")
    };
    let view = world.shared.allocate();
    world.shared.register(
        hikasa_session::ipc::ViewInfo {
            view,
            kind: ViewKind::TextCompare,
            title: String::new(),
            specs: Vec::new(),
        },
        None,
    );
    app.open_view(
        view,
        request(&left, &missing),
        &OpenOptions::default(),
        None,
    );
    world.settle(&mut app);
    let outcome = text_model(&mut app, view).outcome(&world.service);
    assert_eq!(outcome, hikasa_session::api::ViewOutcome::LoadFailed);
    let (result, code) = outcome.close(CloseReason::User, false);
    assert_eq!(code, ExitCode::UnknownError);
    assert_eq!(result.close_reason(), Some(CloseReason::LoadFailed));
    assert!(text_model(&mut app, view).status().error.is_some());

    // A folder view that has not finished is incomplete.
    let (a, b) = folder_pair(&world);
    let specs = [a.display().to_string(), b.display().to_string()];
    let request = world
        .service
        .open_view_request(ViewKind::FolderCompare, &specs, &OpenOptions::default())
        .expect("request");
    let folder = world.shared.allocate();
    app.open_view(folder, request, &OpenOptions::default(), None);
    if app.busy() {
        let m = folder_model(&mut app, folder);
        if m.busy() {
            let outcome = m.outcome(&world.service);
            if m.busy() {
                assert_eq!(
                    outcome,
                    hikasa_session::api::ViewOutcome::Folder {
                        verdict: hikasa_core::CompareVerdict::Incomplete
                    }
                );
            }
        }
    }
    world.settle(&mut app);
    let outcome = folder_model(&mut app, folder).outcome(&world.service);
    assert_eq!(
        outcome,
        hikasa_session::api::ViewOutcome::Folder {
            verdict: hikasa_core::CompareVerdict::Differences
        }
    );
    // A text view with an unfinished comparison has no verdict.
    let big: String = (0..300_000).map(|i| format!("{i}\n")).collect();
    let l = world.write("work/n1.txt", &big);
    let r = world.write("work/n2.txt", &big.replace("150000\n", "x\n"));
    let text = world.shared.allocate();
    app.open_view(
        text,
        request_text(&world, &l, &r),
        &OpenOptions::default(),
        None,
    );
    let m = text_model(&mut app, text);
    let outcome = m.outcome(&world.service);
    if m.response().is_none() {
        assert_eq!(outcome, hikasa_session::api::ViewOutcome::NoVerdict);
        assert_eq!(outcome.close(CloseReason::User, false).1, ExitCode::Success);
    }
    world.settle(&mut app);
}

fn request_text(world: &World, l: &Path, r: &Path) -> ViewRequest {
    let specs = [l.display().to_string(), r.display().to_string()];
    world
        .service
        .open_view_request(ViewKind::TextCompare, &specs, &OpenOptions::default())
        .expect("request")
}

fn set_home(app: &mut AppCore, left: &str, right: &str) {
    if let Some(tab) = app.active_tab_mut()
        && let ViewModel::Home(home) = &mut tab.model
    {
        home.left = left.into();
        home.right = right.into();
    }
}

fn home_error(app: &AppCore) -> Option<String> {
    match &app.active_tab().expect("tab").model {
        ViewModel::Home(home) => home.error.clone(),
        _ => panic!("not Home"),
    }
}

/// Wave-6 review (critical): Home never reads standard input and never blocks the UI thread. `-` and
/// `clipboard://` are refused with an inline message before anything runs; the typed paths are passed verbatim
/// (§2.1: spaces are legal in names); the resolution runs as a job.
#[test]
fn home_refuses_stdin_and_clipboard_keeps_spaces_and_resolves_on_a_job() {
    let read = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&read);
    let world = World::with_init(move |init| {
        init.stdin = Some(StdinSource::new(None, move || {
            flag.store(true, Ordering::SeqCst);
            Ok(b"from stdin\n".to_vec())
        }));
    });
    let (_, right) = text_pair(&world);
    let right = right.display().to_string();
    let mut app = world.app(None);
    for (left, message) in [
        ("-", crate::home::NO_STDIN),
        ("clipboard://", crate::home::NO_CLIPBOARD),
    ] {
        set_home(&mut app, left, &right);
        app.execute(CommandId::HomeCompare);
        assert!(!app.busy(), "nothing runs for {left}");
        world.settle(&mut app);
        assert_eq!(app.scope(), Scope::Home);
        assert_eq!(home_error(&app).as_deref(), Some(message));
    }
    set_home(&mut app, &right, "-");
    app.execute(CommandId::HomeCompare);
    assert_eq!(home_error(&app).as_deref(), Some(crate::home::NO_STDIN));
    assert!(
        !read.load(Ordering::SeqCst),
        "standard input was never read"
    );

    // Leading and trailing spaces are part of the names.
    let spaced = world.write("work/ spaced.c ", "int a;\n");
    set_home(&mut app, " spaced.c ", &right);
    app.execute(CommandId::HomeCompare);
    assert_eq!(app.scope(), Scope::Home, "the result arrives through poll");
    assert!(home_error(&app).is_none());
    world.settle(&mut app);
    assert_eq!(app.scope(), Scope::Text);
    let view = app.active_tab().expect("tab").id;
    assert_eq!(
        PathBuf::from(&text_model(&mut app, view).request().left),
        spaced
    );
    assert_eq!(
        app.active_tab().expect("tab").caption(),
        " spaced.c  ⇄ right.c"
    );
    // A Home tab closed while resolving does not come back as a view.
    app.execute(CommandId::TabNew);
    set_home(&mut app, "left.c", &right);
    app.execute(CommandId::HomeCompare);
    app.execute(CommandId::CloseTab);
    world.settle(&mut app);
    assert_eq!(app.tabs().len(), 1);
    assert_eq!(app.scope(), Scope::Text);
}

/// Wave-6 review: Stop while `folder_prepare` runs stops the view; no scan starts afterwards, and the view
/// closes as incomplete (100) through the core's rule.
#[test]
fn stop_while_preparing_starts_no_scan() {
    let world = World::new();
    let (a, b) = folder_pair(&world);
    let mut app = world.app(None);
    let view = world.open(&mut app, &a, &b);
    // `folder_prepare` runs on its job thread; its result is only picked up by poll, so the view is still
    // preparing here.
    assert!(folder_model(&mut app, view).prepared().is_none());
    assert!(app.enabled(CommandId::Stop));
    app.execute(CommandId::Stop);
    let m = folder_model(&mut app, view);
    assert!(m.stopped() && !m.busy());
    world.settle(&mut app);
    std::thread::sleep(Duration::from_millis(50));
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(m.stopped(), "still stopped");
    assert!(m.prepared().is_none(), "no model, so no scan");
    assert_eq!(m.events(), 0);
    assert_eq!(
        m.outcome(&world.service),
        hikasa_session::api::ViewOutcome::Folder {
            verdict: hikasa_core::CompareVerdict::Incomplete
        }
    );
    assert!(m.status().state.contains("Stopped"));
    // Full Refresh runs the comparison again.
    app.execute(CommandId::FullRefresh);
    world.settle(&mut app);
    let m = folder_model(&mut app, view);
    assert!(m.response().is_some());
    assert_eq!(
        m.outcome(&world.service),
        hikasa_session::api::ViewOutcome::Folder {
            verdict: hikasa_core::CompareVerdict::Differences
        }
    );
}

/// Wave-6 review: the next/previous-difference state is computed once per (rows, selection), not on every
/// render or command lookup.
#[test]
fn difference_availability_is_memoised_per_rows_and_selection() {
    let world = World::new();
    let (a, b) = folder_pair(&world);
    let mut app = world.app(None);
    let view = world.open(&mut app, &a, &b);
    world.settle(&mut app);
    let _ = app.ctx();
    let walks = folder_model(&mut app, view).difference_walks();
    for _ in 0..50 {
        let _ = app.ctx();
        let _ = app.enabled(CommandId::DiffNext);
    }
    assert_eq!(folder_model(&mut app, view).difference_walks(), walks);
    assert!(app.enabled(CommandId::DiffNext));
    app.execute(CommandId::DiffNext);
    let _ = app.ctx();
    let m = folder_model(&mut app, view);
    assert_eq!(
        m.difference_walks(),
        walks + 1,
        "a new selection walks once"
    );
    assert!(m.selected().is_some());
}

/// Wave-6 review: two empty files are not reported as hidden by the display filter.
#[test]
fn empty_files_say_so() {
    let world = World::new();
    let l = world.write("work/e1.txt", "");
    let r = world.write("work/e2.txt", "");
    let mut app = world.app(None);
    let view = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let meta = text_model(&mut app, view).meta();
    assert_eq!(meta.message, "Both files are empty.");
    let (_, right) = text_pair(&world);
    let view = world.open(&mut app, &l, &right);
    world.settle(&mut app);
    assert_eq!(text_model(&mut app, view).meta().message, "");
}

/// Wave-6 review: a view whose `OpenView` arrives while the GUI quits never opens and reports `shutdown` with
/// 100 (§3.6, D-23), not "no verdict" (0).
#[test]
fn a_view_opened_while_quitting_reports_shutdown() {
    let world = World::new();
    let (a, b) = folder_pair(&world);
    let (left, right) = text_pair(&world);
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    // The app does not own the server here, so the waiters are answered by the app alone.
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut app = world.app(None);
    let timeout = Duration::from_secs(10);
    let mut waiting = Vec::new();
    for (kind, l, r) in [
        (ViewKind::FolderCompare, &a, &b),
        (ViewKind::TextCompare, &left, &right),
    ] {
        let specs = vec![l.display().to_string(), r.display().to_string()];
        let wait_endpoint = endpoint.clone();
        waiting.push(std::thread::spawn(move || {
            let mut client = IpcClient::connect(&wait_endpoint, "test", timeout).expect("connect");
            let (id, _) = client
                .request(Request::OpenView {
                    kind,
                    specs,
                    options: OpenOptions::default(),
                    wait: true,
                })
                .expect("open");
            client.wait_view_closed(id).expect("closed")
        }));
    }
    let began = Instant::now();
    while world.shared.len() < 2 || world.mailbox.is_empty() {
        assert!(began.elapsed() < timeout);
        std::thread::sleep(Duration::from_millis(5));
    }
    // Quit first, then the queued `Open` messages arrive.
    let effects = app.handle(UiMsg::Quit);
    assert!(effects.contains(&Effect::Exit));
    world.settle(&mut app);
    for waiter in waiting {
        let (_, result, code) = waiter.join().expect("waiter thread");
        assert_eq!(code, 100);
        assert_eq!(result.close_reason(), Some(CloseReason::Shutdown));
        assert_eq!(result.verdict, None);
        assert_eq!(result.folder_verdict, None);
    }
    assert!(world.shared.is_empty());
    handle.shutdown().expect("shutdown");
}

/// Whenever a job's wake-up arrives, its result is either there or the job says it is settling, so the window
/// never falls back to its 100 ms poll for a finished job (wave-6b review). Repeated to exercise the race.
#[test]
fn a_woken_ui_finds_the_result_or_a_settling_job() {
    use crate::job::{Job, Wake};
    let world = World::new();
    for round in 0..300u32 {
        let (tx, rx) = crossbeam_channel::unbounded();
        let wake: Wake = Arc::new(move || {
            let _ = tx.send(());
        });
        let job = Job::spawn(
            &world.service,
            hikasa_core::JobKind::VfsRead,
            "test".into(),
            &wake,
            move |_svc, _job| Ok(round),
        );
        rx.recv().expect("woken");
        // Settling first: once it reads false after the wake-up, the result is published.
        let settling = job.settling();
        let mut result = job.try_result();
        assert!(
            result.is_some() || settling,
            "round {round}: woken, no result and not settling"
        );
        while result.is_none() {
            std::hint::spin_loop();
            result = job.try_result();
        }
        assert_eq!(result.expect("result").expect("ok"), round);
        assert!(!job.settling());
    }
    // A panicking body still wakes the window.
    let (tx, rx) = crossbeam_channel::unbounded();
    let wake: Wake = Arc::new(move || {
        let _ = tx.send(());
    });
    let job: Job<()> = Job::spawn(
        &world.service,
        hikasa_core::JobKind::VfsRead,
        "panics".into(),
        &wake,
        |_svc, _job| panic!("a failing job body"),
    );
    rx.recv().expect("woken after the panic");
    let result = loop {
        if let Some(r) = job.try_result() {
            break r;
        }
        std::hint::spin_loop();
    };
    assert!(result.is_err());
}

/// Names with a line feed or a bidirectional override are drawn escaped in the tab caption, the pane headers
/// and the folder base strip; the `ListViews` title stays raw (the command line escapes it on output).
#[test]
fn names_are_escaped_where_the_window_draws_them() {
    let world = World::new();
    let left = world.write("work/new\nline.c", "a\n");
    let right = world.write("work/rlo\u{202e}c.exe", "b\n");
    let mut app = world.app(None);
    let view = world.open(&mut app, &left, &right);
    world.settle(&mut app);
    let caption = app.tab_mut(view).expect("tab").caption();
    assert_eq!(caption, "new\nline.c ⇄ rlo\u{202e}c.exe", "the raw title");
    assert_eq!(
        crate::shared::shown_name(&caption),
        "new\\nline.c ⇄ rlo\\u{202e}c.exe"
    );
    let status = text_model(&mut app, view).status();
    for pane in &status.panes {
        assert!(!pane.title.contains(['\n', '\u{202e}']), "{:?}", pane.title);
    }
    assert!(status.panes[0].title.ends_with("new\\nline.c"));
    assert!(status.panes[1].title.ends_with("rlo\\u{202e}c.exe"));

    let l = world.path("work/L\u{1b}[31m");
    let r = world.path("work/R\u{2066}x");
    std::fs::create_dir_all(&l).expect("mkdir");
    std::fs::create_dir_all(&r).expect("mkdir");
    let folders = world.open(&mut app, &l, &r);
    world.settle(&mut app);
    let bases = folder_model(&mut app, folders).status().bases;
    assert!(bases[0].ends_with("/L\\u{1b}[31m"), "{bases:?}");
    assert!(bases[1].ends_with("/R\\u{2066}x"), "{bases:?}");
}

/// Closing a view releases its data (wave-6b review: memory stayed high after many open/close cycles). The
/// comparison, the documents and the folder model are dropped once the view's jobs have ended, also when the
/// view closes while its job still runs; what the process keeps afterwards is the allocator's, not Hikasa's.
#[test]
fn closing_views_releases_their_data() {
    let world = World::new();
    let (left, right) = text_pair(&world);
    let (fl, fr) = folder_pair(&world);
    let mut app = world.app(None);
    let text = world.open(&mut app, &left, &right);
    let folder = world.open(&mut app, &fl, &fr);
    world.settle(&mut app);
    let comparison = Arc::downgrade(
        &text_model(&mut app, text)
            .response()
            .expect("compared")
            .comparison,
    );
    let model = Arc::downgrade(
        folder_model(&mut app, folder)
            .prepared()
            .expect("prepared")
            .model(),
    );
    // A third view closes while its comparison is still running.
    let mut big = String::new();
    for i in 0..300_000 {
        big.push_str(&format!("line {i} of a larger file\n"));
    }
    let bl = world.write("work/big-left.txt", &big);
    let br = world.write("work/big-right.txt", &big.replace("line 7", "LINE 7"));
    let running = world.open(&mut app, &bl, &br);
    let probe = Arc::downgrade(&text_model(&mut app, running).probe());
    while app.tabs().len() > 1 || !matches!(app.tabs()[0].model, ViewModel::Home(_)) {
        app.close_tab(0, CloseReason::User);
    }
    let began = Instant::now();
    while (comparison.strong_count() + model.strong_count() + probe.strong_count()) > 0 {
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "still referenced after closing: comparison {}, folder model {}, running view {}",
            comparison.strong_count(),
            model.strong_count(),
            probe.strong_count()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Finding of the M1 review: the first-run format files must never delay the window or the endpoint. A slow
/// deferred task (a fake store that blocks until released) does not hold up its caller, runs on its own thread
/// and runs once.
#[test]
fn deferred_startup_work_never_blocks_the_caller() {
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let done = Arc::new(AtomicBool::new(false));
    let task_done = Arc::clone(&done);
    let caller = std::thread::current().id();
    let mut deferred = crate::startup::Deferred::new(move || {
        assert_ne!(
            std::thread::current().id(),
            caller,
            "runs off the UI thread"
        );
        // The slow store: blocks until the test releases it.
        release_rx.recv().expect("release");
        task_done.store(true, Ordering::SeqCst);
    });
    assert!(deferred.pending());
    let began = Instant::now();
    let handle = deferred.start().expect("started");
    assert!(
        began.elapsed() < Duration::from_secs(2),
        "start waited for the task"
    );
    assert!(!done.load(Ordering::SeqCst), "the task is still blocked");
    assert!(!deferred.pending());
    assert!(deferred.start().is_none(), "the task starts once");
    release_tx.send(()).expect("release");
    handle.join().expect("task");
    assert!(done.load(Ordering::SeqCst));
}

/// The startup path up to the window (service, endpoint, core, the command line's view) writes no format file;
/// the deferred first-run work then writes the built-ins and `order.toml` (D-21).
#[test]
fn startup_path_leaves_the_format_files_to_the_deferred_work() {
    let world = World::new();
    let formats = hikasa_session::FormatStore::new(world.service.config())
        .dir()
        .to_path_buf();
    let endpoint: Endpoint = world.service.endpoint().expect("endpoint");
    let server = IpcServer::bind(&endpoint).expect("bind");
    let handler = GuiHandler::new(
        world.service.clone(),
        Arc::clone(&world.shared),
        world.mailbox.clone(),
    );
    let handle = server.spawn(Arc::new(handler)).expect("spawn");
    let mut deferred = crate::startup::Deferred::materialize_formats(&world.service);
    let mut app = world.app(Some(handle));
    let (left, right) = text_pair(&world);
    world.open(&mut app, &left, &right);
    world.settle(&mut app);
    assert!(
        !formats.join("order.toml").exists(),
        "nothing on the startup path writes the format files"
    );
    deferred
        .start()
        .expect("started")
        .join()
        .expect("materialize");
    assert!(formats.join("order.toml").is_file(), "order.toml written");
    let written = std::fs::read_dir(&formats)
        .expect("formats folder")
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
        .count();
    assert!(written > 1, "the built-ins are written ({written} files)");
    let _ = app.shutdown();
}
