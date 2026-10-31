//! The application core (M1 contract §1.10; M2 W4a, §1.4.1–§1.4.8): every window ([`WindowModel`]) with its
//! tabs, dialogs and notices; command dispatch through the effective command table; the messages of the IPC
//! handler, the jobs and the watch thread; the D-23 close rules for waited views; layout memory; the log book;
//! the settings the window applies (options, `commands.toml`, the theme). It holds no toolkit type, so the whole
//! behaviour of the windows runs headless in tests (docs/17 §3.4, §1.4.16); the egui layer (`ui`) only draws it
//! and feeds it invocations.
//!
//! Disk access is a `Service` call on a job ([`crate::job::Job`]) or on the watch thread (D-105), except the
//! settings read once by [`AppCore::new`] before the first frame.

use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{JobKind, Warning};
use hikasa_session::api::{ViewOutcome, ViewRequest};
use hikasa_session::ipc::{CloseReason, OpenOptions, ServerHandle, ViewId, ViewInfo, ViewKind};
use hikasa_session::log::{LogBook, LogEntry, LogFilter, LogKind};
use hikasa_session::settings::commands::DisplayFilterMode;
use hikasa_session::settings::options::OpenIn;
use hikasa_session::settings::state::{LayoutValue, WindowState};
use hikasa_session::settings::{
    ChangeKind, ConfigChange, FileOutcome, LoadMode, Options, Scheme, SettingsChanges,
    SettingsSnapshot, Shortcut, State, ThemeSelection, parse_key,
};
use hikasa_session::{ApiError, Service};

use crate::commands::{CommandId, CommandTables, Ctx, FocusArea, HintTiming, Scope, lookup};
use crate::dialog::{
    AboutDialog, ButtonRole, DialogModel, DialogRequest, DialogStep, Followup, Prompt, SettingsBody,
};
use crate::dnd::{DropOutcome, DropTarget, FieldId, decide};
use crate::folder_view::FolderViewModel;
use crate::home::HomeOpen;
use crate::job::Job;
use crate::mailbox::Mailbox;
use crate::notify::{Notice, NoticeAction, NoticeKind};
use crate::options::OptionsDialog;
use crate::shared::Shared;
use crate::status::{JobStatus, LogNote, Message, StatusItems};
use crate::text_view::TextViewModel;
use crate::watch::{WatchCmd, WatchEvent, WatchLink};
pub use crate::window::{
    CommandArg, Invocation, Tab, TabContent, ViewModel, WindowId, WindowModel,
};
use crate::wizards::{ExportWizard, ImportWizard, PackageOffer, RestoreWizard};

mod flow;

#[cfg(test)]
pub use flow::OpenHow;
pub use flow::{Catalog, OpenedSession, PanelAction, load_named};
use flow::{CloseFlow, StoreDone, StoreKind, StoreWork, Then};

/// Messages to the UI thread.
#[derive(Debug)]
pub enum UiMsg {
    /// Open a view (IPC `OpenView`, already registered in [`Shared`] under `view`).
    Open {
        /// The view id answered to the client.
        view: ViewId,
        /// The view's request (`Service::open_view_request`).
        request: Box<ViewRequest>,
        /// Options (titles).
        options: Box<OpenOptions>,
    },
    /// Open a session view (IPC `LoadSession`, already registered in [`Shared`] under `view`).
    OpenSession {
        /// The view id answered to the client.
        view: ViewId,
        /// The session.
        opened: Box<OpenedSession>,
    },
    /// Load a workspace (IPC `LoadWorkspace`; its tabs are registered in [`Shared`]).
    LoadWorkspace {
        /// The workspace.
        load: Box<hikasa_session::api::WorkspaceLoadDto>,
        /// The view ids per window and tab.
        ids: Vec<Vec<Option<ViewId>>>,
    },
    /// Show the import wizard for a package (IPC `ImportPackage`, D-127).
    ImportPackage(PathBuf),
    /// Close a view (IPC `CloseView`).
    Close {
        /// The view.
        view: ViewId,
    },
    /// Quit (IPC `Shutdown`).
    Quit,
    /// A frame: poll the jobs and the live models.
    Tick,
    /// A job or the scan has news.
    Wake,
    /// The watch thread found changes (D-117).
    Watch(Vec<WatchEvent>),
    /// A `hikasa::*` tracing event at debug level (the log book with `debug_messages`, D-116).
    Debug(String),
}

/// What the toolkit layer must do after the core handled something.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// A text view must refetch its meta (data or scroll position changed).
    TextDirty(ViewId),
    /// The folder tree must scroll a row into view.
    FolderScroll(ViewId, u32),
    /// Quit the process (views are closed, the IPC server is shut down).
    Exit,
    /// A window was opened: show its viewport.
    OpenViewport(WindowId),
    /// A window's viewport closes (its model is gone, or moved into the root viewport).
    CloseViewport(WindowId),
    /// Apply this geometry to a window (the root swap, D-106).
    Geometry(WindowId, WindowState),
    /// The theme changed: recompute the palette, visuals and fonts (never per frame, D-113).
    ApplyTheme,
    /// The command tables were rebuilt (`commands.toml` changed or was written).
    ApplyCommands,
    /// A prompt waits in this window: bring it to the front.
    Raise(WindowId),
}

/// Where an opened view goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenTarget {
    /// A new tab of this window (an untouched lone Home tab is replaced, M1).
    Window(WindowId),
    /// Replaces tab `index` of this window (a Home tab's Compare).
    Replace(WindowId, usize),
    /// A new window.
    NewWindow,
}

/// A job of the app core itself (settings writes, reloads, drops).
enum CoreJob {
    /// `settings_save` / `settings_set` followed by a fresh snapshot (and `reload_settings` when it wrote
    /// `options.toml`); its sequence number.
    Settings(u64, Job<(SettingsSnapshot, Vec<Warning>)>),
    /// The reaction to changed settings files: `reload_settings` (when needed) and a fresh snapshot; its sequence
    /// number.
    Reload(
        u64,
        Job<(Vec<Warning>, SettingsSnapshot)>,
        Vec<ConfigChange>,
    ),
    /// `state_update` (layout, window geometry).
    State(Job<bool>),
    /// `log_append` to this file (one at a time: the lines stay in order).
    Log(String, Job<()>),
    /// The resolution of two dropped paths (opens a comparison in the window).
    Drop(WindowId, Job<HomeOpen>),
    /// A store job (`flow`): session and workspace files, one at a time.
    Store(Job<StoreDone>, Option<ViewId>, StoreKind),
}

/// What runs after the first frame (§1.4.12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    /// `[startup] load_workspace` (no view on the command line).
    Options,
    /// The workspace of the command line.
    Workspace(String),
    /// The session of the command line.
    Session(hikasa_session::sessions::SessionRef),
    /// The settings package of the command line (D-127).
    Import(PathBuf),
}

impl CoreJob {
    fn settling(&self) -> bool {
        match self {
            CoreJob::Settings(_, j) => j.settling(),
            CoreJob::Reload(_, j, _) => j.settling(),
            CoreJob::State(j) => j.settling(),
            CoreJob::Log(_, j) => j.settling(),
            CoreJob::Drop(_, j) => j.settling(),
            CoreJob::Store(j, ..) => j.settling(),
        }
    }
}

/// A settings job waiting for its turn (writes and reloads run one at a time, in order).
enum QueuedSettings {
    /// A write of the window (title, body).
    Write(String, SettingsBody),
    /// The reaction to settings files changed on disk.
    Reload(Vec<ConfigChange>),
}

/// The application core.
pub struct AppCore {
    service: Service,
    shared: Arc<Shared>,
    mailbox: Mailbox<UiMsg>,
    server: Option<ServerHandle>,
    windows: Vec<WindowModel>,
    next_window: u32,
    focus_counter: u64,
    quitting: bool,
    settings: SettingsSnapshot,
    /// `state.toml` as read at start, with the layout tables and the window of this process applied since.
    state: State,
    /// Shared with the drawing layer (no copy per frame).
    tables: Arc<CommandTables>,
    log: LogBook,
    os_dark: bool,
    scheme: Scheme,
    /// Font families already reported as unavailable (D-113: one notice per family).
    warned_families: Vec<String>,
    jobs: Vec<CoreJob>,
    /// The window's settings writes and reloads not started yet: they run one at a time, so two writes of one
    /// file never race between the stamp check and the write, and a reload never finishes after a later write.
    /// Each has a sequence number (`settings_issued` counts them).
    queued_settings: std::collections::VecDeque<(u64, QueuedSettings)>,
    /// The sequence number of the last settings job queued.
    settings_issued: u64,
    /// The sequence number of the last snapshot applied: an older one is never applied.
    settings_applied: u64,
    /// The newest snapshot not applied because a later settings job was pending (applied when that job fails).
    skipped_snapshot: Option<(u64, SettingsSnapshot)>,
    /// The inputs each text view tracks with the watch (untracked exactly once when it closes).
    tracked: std::collections::HashMap<ViewId, Vec<String>>,
    /// Saving the log failed: no more saves until the settings change (one notice, no loop through the log).
    log_save_failed: bool,
    /// `debug_messages` for the tracing layer (it posts debug events only when set, D-116).
    debug_flag: Arc<std::sync::atomic::AtomicBool>,
    watch: Option<WatchLink>,
    /// The frame time (seconds) of the last `set_now` (notices).
    now: f64,
    /// A monotonic time (seconds) for the auto-save timer: egui's frame time stands still while the window is
    /// hidden, and the timer must not.
    timer_now: f64,
    /// A store job runs.
    store_running: bool,
    /// Views that became named after their tab closed: their queued auto-save is dropped.
    named_after_close: std::collections::HashSet<ViewId>,
    /// `max_auto_saved_sessions` as last applied (a change prunes).
    auto_limit: u32,
    /// The store jobs not started yet (one at a time, in order).
    queued_store: std::collections::VecDeque<StoreWork>,
    /// The tree and the menus want fresh data: a read after the queue drained.
    refresh_wanted: bool,
    /// What the Home tree and the menus show.
    catalog: Arc<Catalog>,
    /// The auto-saved slot of each view that has one: the only record of it, kept after the tab closed until no
    /// store job is left (a queued job reads it when it starts).
    slots: std::collections::HashMap<ViewId, hikasa_session::sessions::AutoId>,
    /// The close running through its questions.
    flow: Option<CloseFlow>,
    /// The workspace of `on_exit_save_workspace_as` was saved.
    exit_saved: bool,
    /// What runs after the first frame.
    startup: Option<Startup>,
    /// Views built so far (the G12 job counter: a pending tab builds none).
    views_built: u64,
    /// The theme of the Options dialog's working copies while it differs from the files (D-130).
    preview: Option<Preview>,
}

/// The theme preview of the Options dialog (D-130).
#[derive(Debug, Clone, PartialEq)]
struct Preview {
    /// The working scheme's base.
    base: hikasa_session::settings::theme::SchemeBase,
    /// The working `theme.toml` (fonts).
    theme: ThemeSelection,
}

/// The tab caption cut for the title of a menu entry.
const CAPTION_CHARS: usize = 60;

impl AppCore {
    /// The core with one window holding one Home tab. Reads the settings (`settings_snapshot`, leniently) and
    /// `state.toml` once, before the first frame; a failure leaves the defaults and a warning notice.
    pub fn new(
        service: Service,
        shared: Arc<Shared>,
        mailbox: Mailbox<UiMsg>,
        server: Option<ServerHandle>,
    ) -> AppCore {
        let mut problems = Vec::new();
        let settings = match service.settings_snapshot() {
            Ok(s) => s,
            Err(e) => {
                problems.push(e.to_string());
                default_snapshot(&service)
            }
        };
        let state = match service.settings_store().state(LoadMode::Lenient) {
            Ok(loaded) => loaded.value,
            Err(e) => {
                problems.push(e.to_string());
                State::default()
            }
        };
        let options = settings.options.value().clone();
        let tables = Arc::new(CommandTables::new(settings.commands.value()));
        let scheme = settings.scheme(options.appearance.theme_mode, false);
        let log = LogBook::new(
            LogFilter::from_options(&options),
            !options.tweaks.log_panel.save_to_file.is_empty(),
        );
        let auto_limit = settings
            .options
            .value()
            .tweaks
            .metrics
            .max_auto_saved_sessions;
        let mut app = AppCore {
            service,
            shared,
            mailbox,
            server,
            windows: Vec::new(),
            next_window: 1,
            focus_counter: 0,
            quitting: false,
            settings,
            state,
            tables,
            log,
            os_dark: false,
            scheme,
            warned_families: Vec::new(),
            jobs: Vec::new(),
            queued_settings: std::collections::VecDeque::new(),
            settings_issued: 0,
            settings_applied: 0,
            skipped_snapshot: None,
            tracked: std::collections::HashMap::new(),
            log_save_failed: false,
            debug_flag: Arc::new(std::sync::atomic::AtomicBool::new(
                options.tweaks.log_panel.debug_messages,
            )),
            watch: None,
            now: 0.0,
            queued_store: std::collections::VecDeque::new(),
            refresh_wanted: false,
            catalog: Arc::new(Catalog::default()),
            slots: std::collections::HashMap::new(),
            timer_now: 0.0,
            store_running: false,
            named_after_close: std::collections::HashSet::new(),
            auto_limit,
            flow: None,
            exit_saved: false,
            startup: None,
            views_built: 0,
            preview: None,
        };
        let root = app.new_window(None);
        app.new_home_tab(root);
        app.refresh_sessions();
        let mut warnings: Vec<Warning> = app.service.startup_warnings().to_vec();
        warnings.extend(app.settings.warnings());
        for w in &warnings {
            let text = format!("[{}] {}", w.code, w.message);
            app.notify(
                root,
                Notice::new(format!("startup:{text}"), NoticeKind::Warning, text.clone()),
                "settings",
                LogKind::Warning,
            );
        }
        for p in problems {
            app.notify(
                root,
                Notice::new(format!("startup:{p}"), NoticeKind::Warning, p.clone()),
                "settings",
                LogKind::Warning,
            );
        }
        app.check_fonts();
        app
    }

    // ---- accessors -----------------------------------------------------------------------------------------

    /// The view registry shared with the IPC handler.
    #[cfg(test)]
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// Every window; `windows()[0]` is drawn in the root viewport.
    pub fn windows(&self) -> &[WindowModel] {
        &self.windows
    }

    /// A window.
    pub fn window(&self, id: WindowId) -> Option<&WindowModel> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// A window, mutable.
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut WindowModel> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }

    /// The root window (drawn in eframe's root viewport).
    pub fn root(&self) -> WindowId {
        self.windows.first().map_or(WindowId(0), |w| w.id)
    }

    /// The window focused last (IPC, command-line and Home opens go there, D-106).
    pub fn focused(&self) -> WindowId {
        self.windows
            .iter()
            .max_by_key(|w| w.focused_seq)
            .map_or(WindowId(0), |w| w.id)
    }

    /// `options.toml`.
    pub fn options(&self) -> &Options {
        self.settings.options.value()
    }

    /// The effective command tables.
    pub fn tables(&self) -> &CommandTables {
        &self.tables
    }

    /// The effective command tables, shared (a new `Arc` each time they are rebuilt).
    pub fn shared_tables(&self) -> Arc<CommandTables> {
        Arc::clone(&self.tables)
    }

    /// The hint timing of the options.
    pub fn hints(&self) -> HintTiming {
        HintTiming::from_options(self.options())
    }

    /// The log book.
    pub fn log(&self) -> &LogBook {
        &self.log
    }

    /// Clears the log book (the Log panel's Clear).
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// The applied scheme.
    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// The base of the applied scheme (its chrome): a built-in's own, else its file's `base`.
    pub fn scheme_base(&self) -> hikasa_session::settings::theme::SchemeBase {
        if let Some(preview) = &self.preview {
            return preview.base;
        }
        let name = &self.scheme.name;
        Scheme::base_of(name, self.settings.schemes.get(name).map(|f| f.value()))
    }

    /// `theme.toml` (fonts).
    pub fn theme(&self) -> &ThemeSelection {
        match &self.preview {
            Some(preview) => &preview.theme,
            None => self.settings.theme.value(),
        }
    }

    /// `state.toml` as this process knows it (layout tables, the last window).
    #[cfg_attr(not(test), allow(dead_code))] // used by the tests
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Shares the flag the tracing layer reads before it posts a debug event (`debug_messages`, D-116): the core
    /// keeps it current.
    pub fn set_debug_flag(&mut self, flag: Arc<std::sync::atomic::AtomicBool>) {
        self.debug_flag = flag;
        self.sync_debug_flag();
    }

    fn sync_debug_flag(&self) {
        self.debug_flag.store(
            self.log.filter().debug,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Connects the watch thread (D-117).
    pub fn set_watch(&mut self, link: WatchLink) {
        self.watch = Some(link);
    }

    fn send_watch(&self, cmd: WatchCmd) {
        if let Some(link) = &self.watch {
            link.send(cmd);
        }
    }

    /// The frame time (seconds) for notices.
    pub fn set_now(&mut self, now: f64) {
        self.now = now;
        self.timer_now = now;
    }

    /// Sets the auto-save timer's time (monotonic seconds, also while the window is hidden).
    pub fn set_timer_now(&mut self, now: f64) {
        self.timer_now = now;
    }

    /// The OS theme (egui's system theme) changed or was read: under `follow_os` the theme is re-applied.
    pub fn set_os_dark(&mut self, dark: bool) -> Vec<Effect> {
        self.os_dark = dark;
        self.reapply_theme()
    }

    fn reapply_theme(&mut self) -> Vec<Effect> {
        if self.preview.is_some() {
            // The dialog's working copies stay on screen until it closes (D-130).
            return self.preview_step();
        }
        let scheme = self
            .settings
            .scheme(self.options().appearance.theme_mode, self.os_dark);
        if scheme == self.scheme {
            Vec::new()
        } else {
            self.scheme = scheme;
            vec![Effect::ApplyTheme]
        }
    }

    /// D-113: a non-empty font family is not available (no font discovery): one warning notice per family.
    fn check_fonts(&mut self) {
        let fonts = &self.settings.theme.value().fonts;
        let families = [
            fonts.editor.family.clone(),
            fonts.listing.family.clone(),
            fonts.hex.family.clone(),
            fonts.folder.family.clone(),
        ];
        for family in families {
            if family.is_empty() || self.warned_families.contains(&family) {
                continue;
            }
            self.warned_families.push(family.clone());
            let window = self.focused();
            let text = format!(
                "The font family \"{family}\" is not available in this version; the built-in font is used."
            );
            self.notify(
                window,
                Notice::new(format!("font:{family}"), NoticeKind::Warning, text),
                "theme",
                LogKind::Warning,
            );
        }
    }

    /// True once quitting started.
    #[cfg_attr(not(test), allow(dead_code))] // used by the tests
    pub fn quitting(&self) -> bool {
        self.quitting
    }

    /// The window of a view.
    pub fn window_of(&self, view: ViewId) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|w| w.index_of(view).is_some())
            .map(|w| w.id)
    }

    /// The tab of a view.
    pub fn tab(&self, view: ViewId) -> Option<&Tab> {
        self.windows
            .iter()
            .find_map(|w| w.tabs.iter().find(|t| t.id == view))
    }

    /// The tab of a view, mutable.
    pub fn tab_mut(&mut self, view: ViewId) -> Option<&mut Tab> {
        self.windows
            .iter_mut()
            .find_map(|w| w.tabs.iter_mut().find(|t| t.id == view))
    }

    /// The active tab of a window.
    pub fn active_tab(&self, window: WindowId) -> Option<&Tab> {
        self.window(window).and_then(WindowModel::active_tab)
    }

    /// The active tab of a window, mutable.
    pub fn active_tab_mut(&mut self, window: WindowId) -> Option<&mut Tab> {
        self.window_mut(window)
            .and_then(WindowModel::active_tab_mut)
    }

    /// What runs after the first frame (the GUI's command line, D-125).
    pub fn set_startup(&mut self, startup: Startup) {
        self.startup = Some(startup);
    }

    /// Views built so far (the G12 job counter).
    pub fn views_built(&self) -> u64 {
        self.views_built
    }

    /// Tabs not started yet.
    pub fn pending_tabs(&self) -> usize {
        self.windows
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.pending_tab().is_some())
            .count()
    }

    /// True while a job of any view or of the core runs.
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty()
            || !self.queued_settings.is_empty()
            || !self.queued_store.is_empty()
            || self.refresh_wanted
            || self.windows.iter().any(|w| w.dialogs.busy())
            || self
                .windows
                .iter()
                .flat_map(|w| &w.tabs)
                .any(|t| match t.model() {
                    Some(ViewModel::Home(m)) => m.resolving(),
                    Some(ViewModel::Folder(m)) => m.busy(),
                    Some(ViewModel::Text(m)) => m.busy(),
                    None => false,
                })
    }

    /// True while a job has ended but its result is not published yet: the window looks again at once.
    pub fn settling(&self) -> bool {
        self.jobs.iter().any(CoreJob::settling)
            || self
                .windows
                .iter()
                .flat_map(|w| &w.tabs)
                .any(|t| match t.model() {
                    Some(ViewModel::Home(m)) => m.settling(),
                    Some(ViewModel::Folder(m)) => m.settling(),
                    Some(ViewModel::Text(m)) => m.settling(),
                    None => false,
                })
    }

    fn wake(&self) -> Arc<dyn Fn() + Send + Sync> {
        let mailbox = self.mailbox.clone();
        Arc::new(move || mailbox.post(UiMsg::Wake))
    }

    // ---- windows ---------------------------------------------------------------------------------------------

    /// A new window (no tabs), focused. Size: `state.toml [window]`; position: the source window's + (32, 32)
    /// when known, else unknown.
    fn new_window(&mut self, source: Option<WindowId>) -> WindowId {
        let id = WindowId(self.next_window);
        self.next_window += 1;
        let mut geometry = self.state.window.clone();
        geometry.maximized = false;
        match source.and_then(|s| self.window(s)) {
            Some(src) if src.position_known && src.geometry.x >= 0 && src.geometry.y >= 0 => {
                geometry.x = src.geometry.x + 32;
                geometry.y = src.geometry.y + 32;
            }
            _ if source.is_some() => {
                geometry.x = -1;
                geometry.y = -1;
            }
            _ => {}
        }
        self.focus_counter += 1;
        self.windows
            .push(WindowModel::new(id, geometry, self.focus_counter));
        id
    }

    /// A window gained focus (focus order; a watch poll, D-117).
    pub fn focus(&mut self, window: WindowId) {
        if self
            .window(window)
            .is_none_or(|w| w.focused_seq == self.focus_counter)
        {
            return;
        }
        self.focus_counter += 1;
        let seq = self.focus_counter;
        if let Some(w) = self.window_mut(window) {
            w.focused_seq = seq;
        }
        self.send_watch(WatchCmd::Poll);
        self.refresh_sessions();
    }

    /// The platform's geometry of a window (read each frame).
    pub fn set_geometry(&mut self, window: WindowId, geometry: WindowState, position_known: bool) {
        if let Some(w) = self.window_mut(window) {
            w.geometry = geometry;
            w.position_known = position_known;
        }
    }

    /// The layout default of the toolbar for a view type.
    fn toolbar_default(&self, scope: Scope) -> bool {
        scope
            .view_name()
            .and_then(|v| self.state.layout_value(v, "toolbar"))
            .and_then(LayoutValue::as_bool)
            .unwrap_or(true)
    }

    fn new_home_tab(&mut self, window: WindowId) {
        let id = self.shared.allocate();
        let home = self.new_home_model();
        let tab = Tab::live(id, ViewModel::Home(Box::new(home)), true);
        self.insert_tab(window, tab, false);
    }

    /// Inserts a tab after the active one (`open_new_tab_next_to_active`) or last, and activates it. With
    /// `replace_home`, an untouched lone Home tab is replaced instead.
    fn insert_tab(&mut self, window: WindowId, tab: Tab, replace_home: bool) {
        let next_to_active = self.options().tabs.open_new_tab_next_to_active;
        let Some(w) = self.window_mut(window) else {
            return;
        };
        if replace_home && w.tabs.len() == 1 && w.tabs[0].untouched_home() {
            w.tabs[0] = tab;
            w.active = 0;
            return;
        }
        let at = if next_to_active && !w.tabs.is_empty() {
            (w.active + 1).min(w.tabs.len())
        } else {
            w.tabs.len()
        };
        w.tabs.insert(at, tab);
        w.active = at;
    }

    /// Selects a tab.
    pub fn activate(&mut self, window: WindowId, index: usize) {
        if let Some(w) = self.window_mut(window)
            && index < w.tabs.len()
        {
            w.active = index;
        }
        self.after_activation(window);
    }

    /// A tab became active: a pending one starts (D-126), Home's tree refreshes (D-124).
    fn after_activation(&mut self, window: WindowId) {
        self.start_active_pending();
        if self.scope(window) == Scope::Home {
            self.refresh_sessions();
        }
    }

    // ---- opening views ---------------------------------------------------------------------------------------

    /// The target of an open from IPC, the command line or Home (D-106): the window focused last, or a new
    /// window with `open_sessions_in = "window"` (unless the focused window holds only an untouched Home tab,
    /// which the view then replaces).
    pub fn open_target(&self) -> OpenTarget {
        let focused = self.focused();
        if self.options().tabs.open_sessions_in == OpenIn::Window
            && !self
                .window(focused)
                .is_some_and(|w| w.tabs.len() == 1 && w.tabs[0].untouched_home())
        {
            OpenTarget::NewWindow
        } else {
            OpenTarget::Window(focused)
        }
    }

    fn build_model(&mut self, request: ViewRequest, titles: [Option<String>; 2]) -> ViewModel {
        self.build_model_with(
            titles,
            request,
            &hikasa_session::sessions::OpaqueTable::new(),
        )
    }

    /// The model of a request; a session's `view` keys win over the layout of the view type (D-108).
    fn build_model_with(
        &mut self,
        titles: [Option<String>; 2],
        request: ViewRequest,
        session_view: &hikasa_session::sessions::OpaqueTable,
    ) -> ViewModel {
        self.views_built += 1;
        let overlaid;
        let layout = if session_view.is_empty() {
            &self.state
        } else {
            let mut state = self.state.clone();
            let name = match &request {
                ViewRequest::Text { .. } => Scope::Text,
                ViewRequest::Folder { .. } => Scope::Folder,
            }
            .view_name()
            .unwrap_or_default();
            state.overlay_view(name, session_view);
            overlaid = state;
            &overlaid
        };
        let editor_px = self.theme().fonts.editor.size.round().clamp(1.0, 1000.0) as u32;
        match request {
            ViewRequest::Text {
                request,
                view: opts,
            } => {
                let mut m = TextViewModel::new(&self.service, request, opts, titles, self.wake());
                m.apply_layout(layout, editor_px);
                ViewModel::Text(Box::new(m))
            }
            ViewRequest::Folder {
                request,
                display,
                expand_all,
            } => {
                let mut m = FolderViewModel::new(
                    &self.service,
                    request,
                    display,
                    expand_all,
                    titles,
                    self.wake(),
                );
                m.apply_layout(layout);
                ViewModel::Folder(Box::new(m))
            }
        }
    }

    /// Opens a comparison view registered under `view` (IPC, the command line, Home, FC-062, drops) at
    /// `target`; returns the window it went to.
    pub fn open_view(
        &mut self,
        view: ViewId,
        request: ViewRequest,
        options: &OpenOptions,
        target: OpenTarget,
    ) -> WindowId {
        let titles = [options.titles[0].clone(), options.titles[1].clone()];
        let model = self.build_model(request, titles);
        let scope = model.scope();
        let mut tab = Tab::live(view, model, self.toolbar_default(scope));
        tab.session = Some(crate::view_session::ViewSession::unnamed(self.timer_now));
        self.shared.set_title(view, tab.caption());
        self.track_inputs(&tab);
        self.place_tab(tab, target)
    }

    /// Puts a tab at `target`; returns the window.
    fn place_tab(&mut self, tab: Tab, target: OpenTarget) -> WindowId {
        match target {
            OpenTarget::Replace(window, index)
                if self.window(window).is_some_and(|w| index < w.tabs.len()) =>
            {
                let old = {
                    let w = self.window_mut(window).expect("the window exists");
                    w.active = index;
                    std::mem::replace(&mut w.tabs[index], tab)
                };
                let mut old = old;
                self.remember_layout(&old);
                self.close_model(&mut old, CloseReason::User);
                window
            }
            OpenTarget::Window(window) | OpenTarget::Replace(window, _)
                if self.window(window).is_some() =>
            {
                self.insert_tab(window, tab, true);
                window
            }
            _ => {
                let source = Some(self.focused());
                let window = self.new_window(source);
                self.insert_tab(window, tab, false);
                window
            }
        }
    }

    /// Tracks the inputs of a text view with the watch (D-118), once per view (the watch counts views).
    fn track_inputs(&mut self, tab: &Tab) {
        let Some(ViewModel::Text(m)) = tab.model() else {
            return;
        };
        if !self
            .options()
            .tweaks
            .file_operations
            .check_for_files_changed_on_disk
        {
            return;
        }
        let specs = m.inputs().to_vec();
        for spec in &specs {
            self.send_watch(WatchCmd::Track(spec.clone()));
        }
        self.tracked.insert(tab.id, specs);
    }

    /// The inputs the watch tracks for open views (a view tracked twice counts twice).
    #[cfg(test)]
    pub fn tracked_inputs(&self) -> Vec<String> {
        let mut all: Vec<String> = self.tracked.values().flatten().cloned().collect();
        all.sort();
        all
    }

    /// Pushes a log entry (tests).
    #[cfg(test)]
    pub fn push_log(&mut self, text: &str) {
        self.log_entry(LogKind::Summary, "test", text.to_owned());
    }

    /// The number of log appends running (tests: at most one).
    #[cfg(test)]
    pub fn log_jobs(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| matches!(j, CoreJob::Log(..)))
            .count()
    }

    /// Registers and opens a view that did not come over IPC (the command line, Home, FC-062, drops).
    pub fn open_local(
        &mut self,
        kind: ViewKind,
        request: ViewRequest,
        options: &OpenOptions,
        specs: Vec<String>,
        target: OpenTarget,
    ) -> ViewId {
        let view = self.shared.allocate();
        self.shared.register(
            ViewInfo {
                view,
                kind,
                title: String::new(),
                specs,
            },
            None,
        );
        self.open_view(view, request, options, target);
        view
    }

    /// The viewport effects of windows that appeared since `before` windows.
    fn new_viewports(&self, before: &[WindowId]) -> Vec<Effect> {
        self.windows
            .iter()
            .filter(|w| !before.contains(&w.id))
            .map(|w| Effect::OpenViewport(w.id))
            .collect()
    }

    pub(crate) fn window_ids(&self) -> Vec<WindowId> {
        self.windows.iter().map(|w| w.id).collect()
    }

    // ---- messages and polling --------------------------------------------------------------------------------

    /// Handles one message; returns the effects for the toolkit layer.
    pub fn handle(&mut self, msg: UiMsg) -> Vec<Effect> {
        let before = self.window_ids();
        let mut effects = Vec::new();
        match msg {
            UiMsg::Open {
                view,
                request,
                options,
            } => {
                if self.quitting {
                    // Too late: the view never opens; it reports `shutdown` (100), as the server does for the
                    // waiters it notifies itself (§3.6, D-23).
                    let (result, code) = ViewOutcome::close_unopened(CloseReason::Shutdown);
                    self.shared.close(view, result, code);
                } else {
                    let target = self.open_target();
                    self.open_view(view, *request, &options, target);
                }
            }
            UiMsg::OpenSession { view, opened } => {
                if self.quitting {
                    let (result, code) = ViewOutcome::close_unopened(CloseReason::Shutdown);
                    self.shared.close(view, result, code);
                } else {
                    effects.extend(self.open_registered(view, *opened));
                }
            }
            UiMsg::LoadWorkspace { load, ids } => {
                if self.quitting {
                    for view in ids.into_iter().flatten().flatten() {
                        let (result, code) = ViewOutcome::close_unopened(CloseReason::Shutdown);
                        self.shared.close(view, result, code);
                    }
                } else {
                    effects.extend(self.load_registered(*load, ids));
                }
            }
            UiMsg::ImportPackage(path) => {
                if !self.quitting {
                    let window = self.focused();
                    let shown = self.open_import(window, Some(&path));
                    effects.push(Effect::Raise(shown));
                }
            }
            UiMsg::Close { view } => {
                if let Some(window) = self.window_of(view) {
                    let index = self.window(window).and_then(|w| w.index_of(view));
                    if let Some(index) = index {
                        effects.extend(self.remove_tab(window, index, CloseReason::User, false));
                    }
                } else {
                    // Registered but not opened: the view was dropped before it ran (100).
                    let (result, code) = ViewOutcome::close_unopened(CloseReason::User);
                    self.shared.close(view, result, code);
                }
            }
            UiMsg::Quit => return self.shutdown(),
            UiMsg::Tick | UiMsg::Wake => {
                self.autosave_tick();
                effects.extend(self.poll_all());
            }
            UiMsg::Watch(events) => effects.extend(self.on_watch(events)),
            UiMsg::Debug(text) => {
                if self.log.filter().debug {
                    self.log_entry(LogKind::Debug, "debug", text);
                }
            }
        }
        effects.extend(self.new_viewports(&before));
        effects.extend(self.dirty_effects());
        effects
    }

    fn poll_all(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let service = self.service.clone();
        let mut opened: Vec<(WindowId, usize, HomeOpen)> = Vec::new();
        let mut notes: Vec<(WindowId, ViewId, bool, String, Vec<LogNote>)> = Vec::new();
        for w in &mut self.windows {
            let active = w.active;
            for (index, tab) in w.tabs.iter_mut().enumerate() {
                let caption = tab.caption();
                let id = tab.id;
                match tab.model_mut() {
                    Some(ViewModel::Home(m)) => {
                        let (open, done) = m.poll();
                        if let Some(open) = open {
                            opened.push((w.id, index, open));
                        } else if done && let Some(e) = &m.error {
                            // Shown in the Home form itself: a log entry, no notice.
                            notes.push((
                                w.id,
                                id,
                                true,
                                "Home".to_owned(),
                                vec![LogNote {
                                    kind: LogKind::Error,
                                    text: e.clone(),
                                }],
                            ));
                        }
                    }
                    Some(ViewModel::Folder(m)) => {
                        m.poll(&service);
                        let n = m.take_notes();
                        if !n.is_empty() {
                            notes.push((w.id, id, index == active, caption, n));
                        }
                    }
                    Some(ViewModel::Text(m)) => {
                        m.poll(&service);
                        let n = m.take_notes();
                        if !n.is_empty() {
                            notes.push((w.id, id, index == active, caption, n));
                        }
                    }
                    None => {}
                }
            }
        }
        for (window, index, open) in opened {
            let active = self.window(window).map(|w| w.active);
            self.open_local(
                open.kind,
                open.request,
                &open.options,
                open.specs,
                OpenTarget::Replace(window, index),
            );
            // A Compare that resolved in a background tab does not steal the focus.
            if let (Some(active), Some(w)) = (active, self.window_mut(window)) {
                w.active = active;
            }
        }
        for (window, view, active, caption, list) in notes {
            for note in list {
                if note.kind == LogKind::Error && !active {
                    let text = format!("{caption}: {}", note.text);
                    let notice = Notice::new(format!("job:{}", view.0), NoticeKind::Error, text)
                        .with_actions(vec![NoticeAction::ShowTab(view)]);
                    self.push_notice(window, notice);
                }
                self.log_entry(note.kind, &caption, note.text);
            }
        }
        effects.extend(self.poll_jobs());
        // Once per window and frame, also in a window without tabs; after the jobs, so a dialog sees the result of
        // a settings job in the frame the job ended.
        for w in &mut self.windows {
            w.dialogs.poll(&service);
        }
        effects.extend(self.dialog_requests());
        self.flush_log();
        effects
    }

    /// `TextDirty` for every text view whose panes must refetch, `FolderScroll` for every tree that must
    /// bring a row into view.
    pub fn dirty_effects(&mut self) -> Vec<Effect> {
        self.windows
            .iter_mut()
            .flat_map(|w| w.tabs.iter_mut())
            .filter_map(|t| {
                let id = t.id;
                match t.model_mut() {
                    Some(ViewModel::Text(m)) => m.take_dirty().then_some(Effect::TextDirty(id)),
                    Some(ViewModel::Folder(m)) => {
                        m.take_scroll().map(|row| Effect::FolderScroll(id, row))
                    }
                    _ => None,
                }
            })
            .collect()
    }

    // ---- commands --------------------------------------------------------------------------------------------

    /// The command context of a window's active view.
    pub fn ctx(&self, window: WindowId) -> Ctx {
        let Some(w) = self.window(window) else {
            return Ctx::default();
        };
        let mut ctx = Ctx {
            tabs: w.tabs.len(),
            windows: self.windows.len(),
            dialog_open: !w.dialogs.is_empty(),
            ..Ctx::default()
        };
        if let Some(tab) = w.active_tab() {
            ctx.scope = tab.scope();
            ctx.named = tab.session.as_ref().is_some_and(|s| s.named());
            match tab.model() {
                Some(ViewModel::Home(m)) => m.ctx_into(&mut ctx),
                Some(ViewModel::Folder(m)) => m.ctx_into(&mut ctx),
                Some(ViewModel::Text(m)) => m.ctx_into(&mut ctx),
                None => {}
            }
        }
        ctx
    }

    /// The scope of a window's active view.
    pub fn scope(&self, window: WindowId) -> Scope {
        self.active_tab(window).map_or(Scope::Global, Tab::scope)
    }

    /// True when `id` is shown and available in the window's active view.
    pub fn enabled(&self, window: WindowId, id: CommandId) -> bool {
        let ctx = self.ctx(window);
        !ctx.dialog_open
            && crate::commands::shown(&self.tables, ctx.scope, id)
            && lookup(ctx.scope, id).is_some_and(|r| r.enabled_when.holds(&ctx))
    }

    /// The check state of a toggle or radio command in the window's active view.
    pub fn checked(&self, window: WindowId, id: CommandId) -> Option<bool> {
        let tab = self.active_tab(window)?;
        if id == CommandId::ShowToolbar {
            return (tab.scope() != Scope::Home).then_some(tab.toolbar);
        }
        if id == CommandId::SessionLocked {
            return tab.session.as_ref().map(|s| s.locked);
        }
        match tab.model()? {
            ViewModel::Home(h) => match id {
                CommandId::ShowManage => Some(h.panel_mode != crate::home::PanelMode::Hidden),
                CommandId::MaximizeManage => {
                    Some(h.panel_mode == crate::home::PanelMode::Maximized)
                }
                _ => None,
            },
            ViewModel::Folder(m) => m.checked(id),
            ViewModel::Text(m) => m.checked(id),
        }
    }

    /// The command a key press runs in a window (D-111).
    pub fn dispatch_key(
        &self,
        window: WindowId,
        key: &Shortcut,
        at: FocusArea,
    ) -> Option<CommandId> {
        let ctx = self.ctx(window);
        self.tables.dispatch(ctx.scope, &ctx, key, at)
    }

    /// The command a key press runs in a window (D-111): each candidate of the press in turn (the key as typed,
    /// then its keypad alias), first in the focused area `at`, then at the window root.
    pub fn dispatch_keys(
        &self,
        window: WindowId,
        keys: &[Shortcut],
        at: FocusArea,
    ) -> Option<CommandId> {
        keys.iter().find_map(|key| {
            (at != FocusArea::Root)
                .then(|| self.dispatch_key(window, key, at))
                .flatten()
                .or_else(|| self.dispatch_key(window, key, FocusArea::Root))
        })
    }

    /// The workspace shortcut of a key press no command took (D-111 rule 3).
    pub fn dispatch_workspace(
        &self,
        window: WindowId,
        keys: &[Shortcut],
        at: FocusArea,
    ) -> Option<Invocation> {
        if self.window(window).is_none_or(|w| !w.dialogs.is_empty()) {
            return None;
        }
        self.workspace_key(keys, at)
            .map(|name| Invocation::with(CommandId::WorkspaceLoadList, CommandArg::Workspace(name)))
    }

    /// Executes a command in a window (ignored when it is not available there).
    pub fn execute(&mut self, window: WindowId, inv: impl Into<Invocation>) -> Vec<Effect> {
        let inv = inv.into();
        // A command on another tab (the tab context menu) runs in that tab's context.
        if let Some(CommandArg::Tab(index)) = inv.arg
            && !matches!(inv.id, CommandId::TabNew)
        {
            self.activate(window, index);
        }
        if !self.enabled(window, inv.id) {
            return Vec::new();
        }
        self.run(window, inv, false)
    }

    /// Runs a command; `confirmed` when a prompt was answered (no warning again).
    fn run(&mut self, window: WindowId, inv: Invocation, confirmed: bool) -> Vec<Effect> {
        let before = self.window_ids();
        let mut effects = Vec::new();
        let service = self.service.clone();
        if let Some(done) = self.session_command(window, &inv) {
            effects.extend(done);
            self.start_active_pending();
            effects.extend(self.new_viewports(&before));
            effects.extend(self.dirty_effects());
            return effects;
        }
        match inv.id {
            CommandId::TabNew => {
                self.new_home_tab(window);
                self.refresh_sessions();
            }
            CommandId::WindowNew => {
                let id = self.new_window(Some(window));
                self.new_home_tab(id);
            }
            CommandId::CloseTab => {
                if let Some(view) = self.active_tab(window).map(|t| t.id) {
                    effects.extend(self.start_close(
                        vec![view],
                        Then::Tabs {
                            window,
                            views: vec![view],
                            user: true,
                        },
                    ));
                }
            }
            CommandId::TabNext | CommandId::TabPrev => {
                if let Some(w) = self.window_mut(window) {
                    let n = w.tabs.len().max(1);
                    w.active = if inv.id == CommandId::TabNext {
                        (w.active + 1) % n
                    } else {
                        (w.active + n - 1) % n
                    };
                }
                self.after_activation(window);
            }
            CommandId::TabDuplicate => self.duplicate(window),
            CommandId::TabMoveToNewWindow => {
                let index = self.window(window).map_or(0, |w| w.active);
                effects.extend(self.move_tab(window, index, None, None));
            }
            CommandId::TabMoveToWindow => {
                if let Some(CommandArg::Window(to)) = inv.arg {
                    let index = self.window(window).map_or(0, |w| w.active);
                    effects.extend(self.move_tab(window, index, Some(to), None));
                }
            }
            CommandId::TabCloseOthers => effects.extend(self.close_others(window, confirmed)),
            CommandId::Exit => return self.exit(window, confirmed),
            CommandId::About => {
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(AboutDialog));
                }
            }
            CommandId::ProgramOptions => {
                let page = match inv.arg {
                    Some(CommandArg::Page(page)) => Some(page),
                    _ => None,
                };
                effects.extend(self.open_options(window, page));
            }
            CommandId::Export => {
                let wizard = ExportWizard::new(&self.service, self.wake());
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(wizard));
                }
            }
            CommandId::Import => {
                let path = match &inv.arg {
                    Some(CommandArg::Paths(paths)) => paths.first().cloned(),
                    _ => None,
                };
                self.open_import(window, path.as_deref());
            }
            CommandId::Restore => {
                let wizard = RestoreWizard::new(self.wake());
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(wizard));
                }
            }
            CommandId::HelpContents => self.show_help(window, hikasa_session::help::INDEX_TOPIC),
            CommandId::ContextHelp => {
                let topic = self.scope(window).help_topic();
                self.show_help(window, topic);
            }
            CommandId::ShowToolbar => {
                let scope = self.scope(window);
                if let Some(tab) = self.active_tab_mut(window) {
                    tab.toolbar = !tab.toolbar;
                    let on = tab.toolbar;
                    if let Some(view) = scope.view_name() {
                        self.state
                            .set_layout_value(view, "toolbar", LayoutValue::Bool(on));
                    }
                }
            }
            id => {
                let open = {
                    let Some(tab) = self.active_tab_mut(window) else {
                        return Vec::new();
                    };
                    match tab.model_mut() {
                        Some(ViewModel::Text(m)) => {
                            m.execute(&service, id);
                            None
                        }
                        Some(ViewModel::Folder(m)) => m.execute(&service, id).flatten(),
                        _ => None,
                    }
                };
                if let Some((request, view)) = open {
                    let parent = self.child_link(window, &request);
                    let target = if self.options().tabs.open_child_sessions_in == OpenIn::Window {
                        OpenTarget::NewWindow
                    } else {
                        OpenTarget::Window(window)
                    };
                    if let Some(parent) = parent {
                        self.open_child(ViewRequest::Text { request, view }, parent, target);
                        return effects;
                    }
                    let specs = vec![
                        request.left.to_string_lossy().into_owned(),
                        request.right.to_string_lossy().into_owned(),
                    ];
                    self.open_local(
                        ViewKind::TextCompare,
                        ViewRequest::Text { request, view },
                        &OpenOptions::default(),
                        specs,
                        target,
                    );
                }
            }
        }
        self.start_active_pending();
        effects.extend(self.new_viewports(&before));
        effects.extend(self.dirty_effects());
        effects
    }

    /// The parent link of a pair opened from the folder view of `window` (FC-062, TC-052): the view's reference
    /// (named or auto-saved) and the pair's paths relative to its bases; `None` for a view without a file.
    fn child_link(
        &self,
        window: WindowId,
        request: &hikasa_session::api::TextCompareRequest,
    ) -> Option<hikasa_session::sessions::ParentLink> {
        let tab = self.active_tab(window)?;
        let session = tab.session.as_ref()?.link_text(self.slots.get(&tab.id))?;
        let Some(ViewModel::Folder(m)) = tab.model() else {
            return None;
        };
        let [left, right] = m.relative_pair(request)?;
        Some(hikasa_session::sessions::ParentLink {
            session,
            left,
            right,
        })
    }

    /// `TabDuplicate` (D-107): a new view of the same request with its own job, right of the source tab; a Home
    /// tab copies its fields.
    fn duplicate(&mut self, window: WindowId) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let copy = match tab.model() {
            Some(ViewModel::Home(h)) => Err(h.copy_fields()),
            Some(ViewModel::Text(m)) => {
                let (request, titles) = m.duplicate();
                Ok((ViewKind::TextCompare, request, titles, m.inputs().to_vec()))
            }
            Some(ViewModel::Folder(m)) => {
                let (request, titles) = m.duplicate();
                Ok((ViewKind::FolderCompare, request, titles, m.specs()))
            }
            None => return,
        };
        let toolbar = tab.toolbar;
        let now = self.now;
        // Right of the source, whatever `open_new_tab_next_to_active` says.
        let at = self.window(window).map_or(0, |w| w.active + 1);
        let tab = match copy {
            Err(home) => Tab::live(
                self.shared.allocate(),
                ViewModel::Home(Box::new(home)),
                toolbar,
            ),
            Ok((kind, request, titles, specs)) => {
                let view = self.shared.allocate();
                self.shared.register(
                    ViewInfo {
                        view,
                        kind,
                        title: String::new(),
                        specs,
                    },
                    None,
                );
                let model = self.build_model(request, titles);
                let mut tab = Tab::live(view, model, toolbar);
                tab.toolbar = toolbar;
                tab.session = Some(crate::view_session::ViewSession::unnamed(now));
                self.shared.set_title(view, tab.caption());
                self.track_inputs(&tab);
                tab
            }
        };
        if let Some(w) = self.window_mut(window) {
            let at = at.min(w.tabs.len());
            w.tabs.insert(at, tab);
            w.active = at;
        }
    }

    /// Moves tab `tab` of `from` to window `to` (`None`: a new window) at `index` (`None`: last), unchanged
    /// (view, jobs, waiter, layout), and activates it. Inside the same window it reorders. A window whose last
    /// tab moves away closes (no Home tab, no prompt).
    pub fn move_tab(
        &mut self,
        from: WindowId,
        tab: usize,
        to: Option<WindowId>,
        index: Option<usize>,
    ) -> Vec<Effect> {
        let before = self.window_ids();
        let mut effects = Vec::new();
        let Some(src) = self.window(from) else {
            return effects;
        };
        if tab >= src.tabs.len() {
            return effects;
        }
        if to == Some(from) {
            if let Some(w) = self.window_mut(from) {
                let moved = w.tabs.remove(tab);
                // `index` is a position in the bar before the move: right of the tab it shifts by one.
                let at = match index {
                    Some(i) if i > tab => i - 1,
                    Some(i) => i,
                    None => w.tabs.len(),
                }
                .min(w.tabs.len());
                w.tabs.insert(at, moved);
                w.active = at;
            }
            return effects;
        }
        if to.is_none() && src.tabs.len() < 2 {
            // Moving the only tab to a new window changes nothing.
            return effects;
        }
        if to.is_some_and(|t| self.window(t).is_none()) {
            return effects;
        }
        let moved = {
            let w = self.window_mut(from).expect("the source exists");
            let moved = w.tabs.remove(tab);
            if w.active >= w.tabs.len() || w.active > tab {
                w.active = w
                    .active
                    .saturating_sub(1)
                    .min(w.tabs.len().saturating_sub(1));
            }
            moved
        };
        let target = match to {
            Some(t) => t,
            None => self.new_window(Some(from)),
        };
        if let Some(w) = self.window_mut(target) {
            let at = index.unwrap_or(w.tabs.len()).min(w.tabs.len());
            w.tabs.insert(at, moved);
            w.active = at;
        }
        self.focus(target);
        if self.window(from).is_some_and(|w| w.tabs.is_empty()) {
            effects.extend(self.close_window_now(from));
        }
        self.start_active_pending();
        effects.extend(self.new_viewports(&before));
        effects
    }

    /// `TabCloseOthers` (D-107): with `warn_when_closing_multiple_tabs` and two or more tabs to close, a prompt
    /// first.
    fn close_others(&mut self, window: WindowId, confirmed: bool) -> Vec<Effect> {
        let Some(w) = self.window(window) else {
            return Vec::new();
        };
        let others = w.tabs.len().saturating_sub(1);
        if others == 0 {
            return Vec::new();
        }
        if !confirmed && others >= 2 && self.options().tabs.warn_when_closing_multiple_tabs {
            let prompt = Prompt::confirm(
                "Close Tabs",
                format!("Close {others} tabs?"),
                Some(WARN_TABS),
                vec![Followup::Run(Invocation::new(CommandId::TabCloseOthers))],
            );
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(prompt));
            }
            return Vec::new();
        }
        let keep = self.window(window).map(|w| w.tabs[w.active].id);
        let views: Vec<ViewId> = self
            .window(window)
            .map(|w| {
                w.tabs
                    .iter()
                    .map(|t| t.id)
                    .filter(|v| Some(*v) != keep)
                    .collect()
            })
            .unwrap_or_default();
        self.start_close(
            views.clone(),
            Then::Tabs {
                window,
                views,
                user: false,
            },
        )
    }

    /// Closes the tab `index` of a window: its view reports the D-23 outcome to its waiter (if any), its jobs
    /// stop. The last tab: with `user` and `closing_last_tab_closes_window` the window closes, else a Home tab
    /// replaces it.
    #[cfg_attr(not(test), allow(dead_code))] // used by the tests
    pub fn close_tab(
        &mut self,
        window: WindowId,
        index: usize,
        reason: CloseReason,
    ) -> Vec<Effect> {
        self.remove_tab(window, index, reason, reason == CloseReason::User)
    }

    fn remove_tab(
        &mut self,
        window: WindowId,
        index: usize,
        reason: CloseReason,
        user: bool,
    ) -> Vec<Effect> {
        let Some(w) = self.window(window) else {
            return Vec::new();
        };
        if index >= w.tabs.len() {
            return Vec::new();
        }
        if user
            && w.tabs.len() == 1
            && self.options().tabs.closing_last_tab_closes_window
            && !self.quitting
        {
            return self.close_window_now(window);
        }
        let mut tab = {
            let w = self.window_mut(window).expect("the window exists");
            w.tabs.remove(index)
        };
        self.remember_layout(&tab);
        self.close_model(&mut tab, reason);
        let quitting = self.quitting;
        let empty = self.window(window).is_some_and(|w| w.tabs.is_empty());
        if empty && !quitting {
            self.new_home_tab(window);
        }
        if let Some(w) = self.window_mut(window) {
            if w.tabs.is_empty() {
                w.active = 0;
            } else if w.active >= w.tabs.len() || w.active > index {
                w.active = w.active.saturating_sub(1).min(w.tabs.len() - 1);
            }
        }
        self.after_activation(window);
        Vec::new()
    }

    fn close_model(&mut self, tab: &mut Tab, reason: CloseReason) {
        self.decide_silently(tab);
        let service = self.service.clone();
        let id = tab.id;
        if tab.pending_tab().is_some() {
            // Never started: `close_unopened` semantics (D-126).
            let (result, code) = ViewOutcome::close_unopened(reason);
            self.shared.close(id, result, code);
        }
        let outcome = match tab.model_mut() {
            Some(ViewModel::Folder(m)) => {
                let outcome = m.outcome(&service);
                m.cancel();
                Some(outcome)
            }
            Some(ViewModel::Text(m)) => {
                let outcome = m.outcome(&service);
                m.cancel();
                Some(outcome)
            }
            _ => None,
        };
        for spec in self.tracked.remove(&id).unwrap_or_default() {
            self.send_watch(WatchCmd::Untrack(spec));
        }
        if let Some(outcome) = outcome {
            let (result, code) = outcome.close(reason, false);
            self.shared.close(id, result, code);
        }
        // The notices about the view go with it.
        for w in &mut self.windows {
            w.notices.forget_view(id);
        }
    }

    /// Keeps the layout keys of a closing view in memory (the last closed view of a type wins, D-108).
    fn remember_layout(&mut self, tab: &Tab) {
        let scope = tab.scope();
        let Some(view) = scope.view_name() else {
            return;
        };
        if scope != Scope::Home {
            self.state
                .set_layout_value(view, "toolbar", LayoutValue::Bool(tab.toolbar));
        }
        let keys = match tab.model() {
            Some(ViewModel::Text(m)) => m.layout_keys(),
            Some(ViewModel::Folder(m)) => m.layout_keys(),
            _ => Vec::new(),
        };
        for (key, value) in keys {
            self.state.set_layout_value(view, key, value);
        }
    }

    /// Stops tracking a view's inputs.
    fn untrack_view(&mut self, view: ViewId) {
        for spec in self.tracked.remove(&view).unwrap_or_default() {
            self.send_watch(WatchCmd::Untrack(spec));
        }
    }

    /// Runs `change` on `state.toml` with `state_update` on a job (D-108).
    fn update_state(&mut self, change: impl FnMut(&mut State) + Send + 'static) {
        let mut change = change;
        let path = self
            .service
            .settings_store()
            .path(hikasa_session::settings::FileId::State);
        let watch = self.watch.clone();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsWrite,
            "save the state".into(),
            &self.wake(),
            move |svc, _| {
                let written = svc.state_update(&mut change)?;
                if let Some(link) = watch {
                    link.send(WatchCmd::Acknowledge(vec![path]));
                }
                Ok(written)
            },
        );
        self.jobs.push(CoreJob::State(job));
    }

    /// Writes `[window]` (from `geometry`) and every `[layout.<view>]` this process knows (D-108), on a job.
    fn persist_state(&mut self, geometry: Option<WindowState>) {
        if let Some(g) = geometry {
            self.state.window = g;
        }
        let window = self.state.window.clone();
        let layout = self.state.layout.clone();
        let path = self
            .service
            .settings_store()
            .path(hikasa_session::settings::FileId::State);
        let watch = self.watch.clone();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsWrite,
            "save the window layout".into(),
            &self.wake(),
            move |svc, _| {
                let written = svc.state_update(&mut |s: &mut State| {
                    s.window = window.clone();
                    for (view, table) in &layout {
                        let entry = s.layout.entry(view.clone()).or_default();
                        for (k, v) in table {
                            entry.insert(k.clone(), v.clone());
                        }
                    }
                })?;
                if let Some(link) = watch {
                    link.send(WatchCmd::Acknowledge(vec![path]));
                }
                Ok(written)
            },
        );
        self.jobs.push(CoreJob::State(job));
    }

    /// The OS close button or `Exit` of one window (D-106): with `warn_when_closing_multiple_tabs` and more than
    /// one tab a prompt first; then each tab closes; the last window ends the process; closing `windows[0]` while
    /// others are open moves the next window into the root viewport.
    pub fn close_window(&mut self, window: WindowId) -> Vec<Effect> {
        let Some(w) = self.window(window) else {
            return Vec::new();
        };
        if w.dialogs.busy() {
            // Cancel is not offered either: the write ends first, then the dialog reports it.
            self.refuse_close(window);
            return Vec::new();
        }
        if !w.dialogs.is_empty() {
            // The top dialog closes with Cancel first (D-112): the window stays, what lies under it is not lost.
            let mut effects = Vec::new();
            if let Some(closed) = self.window_mut(window).and_then(|w| w.dialogs.pop()) {
                effects.extend(self.dialog_closed(closed));
            }
            effects.extend(self.cancel_flow(window));
            return effects;
        }
        let tabs = self.window(window).map_or(0, |w| w.tabs.len());
        if tabs > 1 && self.options().tabs.warn_when_closing_multiple_tabs {
            let prompt = Prompt::confirm(
                "Close Window",
                format!("Close {tabs} tabs?"),
                Some(WARN_TABS),
                vec![Followup::CloseWindow],
            );
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(prompt));
            }
            return Vec::new();
        }
        self.close_window_confirmed(window)
    }

    /// A dialog's job runs (an import, a restore, an Apply): stopping now would leave it half done with nobody to
    /// report it, so the window stays and says why.
    fn refuse_close(&mut self, window: WindowId) {
        self.notify(
            window,
            Notice::new(
                "dialog-busy",
                NoticeKind::Warning,
                "A settings dialog is still working: close it when it has finished.",
            ),
            "settings",
            LogKind::Warning,
        );
    }

    /// The window whose dialog's job runs, if any.
    fn busy_dialog(&self) -> Option<WindowId> {
        self.windows.iter().find(|w| w.dialogs.busy()).map(|w| w.id)
    }

    /// The close actions of a window's tabs, then it closes (the last window: the exit).
    fn close_window_confirmed(&mut self, window: WindowId) -> Vec<Effect> {
        if self.window(window).is_some_and(|w| w.dialogs.busy()) {
            self.refuse_close(window);
            return Vec::new();
        }
        let views = self.window(window).map_or_else(Vec::new, |w| {
            w.tabs.iter().map(|t| t.id).collect::<Vec<_>>()
        });
        if self.windows.len() == 1 {
            self.start_close(views, Then::Exit)
        } else {
            self.start_close(views, Then::Window(window))
        }
    }

    /// Closes a window without asking.
    pub(crate) fn close_window_now(&mut self, window: WindowId) -> Vec<Effect> {
        let Some(index) = self.index_of(window) else {
            return Vec::new();
        };
        if self.windows.len() == 1 {
            return self.shutdown();
        }
        let mut w = self.windows.remove(index);
        let geometry = w.geometry.clone();
        for mut tab in std::mem::take(&mut w.tabs) {
            self.remember_layout(&tab);
            self.close_model(&mut tab, CloseReason::User);
        }
        self.persist_state(Some(geometry));
        // A dialog that goes with the window ends like any other (the Options dialog's theme preview).
        let mut effects = Vec::new();
        while let Some(closed) = w.dialogs.pop() {
            effects.extend(self.dialog_closed(closed));
        }
        if index == 0 {
            // The root swap: the next window is drawn in the root viewport from now on.
            let root = &self.windows[0];
            effects.push(Effect::Geometry(root.id, root.geometry.clone()));
            effects.push(Effect::CloseViewport(root.id));
        } else {
            effects.push(Effect::CloseViewport(window));
        }
        effects
    }

    /// `Exit` (D-106): with `warn_when_closing_multiple_windows` and more than one window, else with
    /// `warn_when_closing_multiple_tabs` and more than one tab, a prompt first; then quits.
    fn exit(&mut self, window: WindowId, confirmed: bool) -> Vec<Effect> {
        if let Some(busy) = self.busy_dialog() {
            self.refuse_close(busy);
            return Vec::new();
        }
        if !confirmed {
            let tabs: usize = self.windows.iter().map(|w| w.tabs.len()).sum();
            let windows = self.windows.len();
            let o = &self.options().tabs;
            let prompt = if windows > 1 && o.warn_when_closing_multiple_windows {
                Some(Prompt::confirm(
                    "Exit",
                    format!("Close {windows} windows?"),
                    Some(WARN_WINDOWS),
                    vec![Followup::Exit],
                ))
            } else if tabs > 1 && o.warn_when_closing_multiple_tabs {
                Some(Prompt::confirm(
                    "Exit",
                    format!("Close {tabs} tabs?"),
                    Some(WARN_TABS),
                    vec![Followup::Exit],
                ))
            } else {
                None
            };
            if let Some(prompt) = prompt {
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
                return Vec::new();
            }
        }
        self.exit_confirmed()
    }

    /// The exit workspace, then every close action (a Cancel stops the exit), then the shutdown (D-106).
    fn exit_confirmed(&mut self) -> Vec<Effect> {
        if let Some(busy) = self.busy_dialog() {
            self.refuse_close(busy);
            return Vec::new();
        }
        let views: Vec<ViewId> = self
            .windows
            .iter()
            .flat_map(|w| w.tabs.iter().map(|t| t.id))
            .collect();
        self.start_close(views, Then::Exit)
    }

    /// Quits without asking (IPC `Shutdown`, a confirmed Exit, the end of the desktop session): every view closes
    /// with reason `shutdown` and its real outcome (D-23), the layout is written, then the IPC server stops (it
    /// notifies any waiter left and closes the connections, §1.8.7 item 12).
    pub fn shutdown(&mut self) -> Vec<Effect> {
        if self.quitting {
            return vec![Effect::Exit];
        }
        self.exit_workspace_save();
        self.quitting = true;
        let geometry = self.window(self.focused()).map(|w| w.geometry.clone());
        for wi in 0..self.windows.len() {
            while let Some(last) = self.windows[wi].tabs.len().checked_sub(1) {
                let mut tab = self.windows[wi].tabs.remove(last);
                self.remember_layout(&tab);
                self.close_model(&mut tab, CloseReason::Shutdown);
            }
        }
        self.persist_state(geometry);
        self.send_watch(WatchCmd::Stop);
        if let Some(server) = self.server.take()
            && let Err(e) = server.shutdown()
        {
            tracing::warn!(target: "hikasa::gui", "IPC server shutdown: {e}");
        }
        vec![Effect::Exit]
    }

    /// Waits (at most `limit`) for the core's own jobs (the layout write at exit, log appends). A settings write
    /// (an import, a restore: it deletes first) is not cut off by `limit`: a shutdown nobody can refuse (IPC, the
    /// end of the desktop session) gives it [`SETTINGS_FLUSH`] and logs an error if it is still running.
    pub fn flush(&mut self, limit: std::time::Duration) {
        let began = std::time::Instant::now();
        self.flush_log();
        self.next_settings_job();
        self.next_store_job();
        loop {
            let writing = !self.queued_settings.is_empty()
                || self.jobs.iter().any(|j| matches!(j, CoreJob::Settings(..)));
            let wait = (!self.jobs.is_empty() && began.elapsed() < limit)
                || (writing && began.elapsed() < SETTINGS_FLUSH);
            if !wait {
                if writing {
                    tracing::error!(
                        target: "hikasa::gui",
                        "a settings write was still running at exit: it may be incomplete"
                    );
                }
                return;
            }
            self.poll_jobs();
            if !self.jobs.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    // ---- help, dialogs, messages, notices --------------------------------------------------------------------

    /// The help topic shown in a window's help panel.
    #[cfg_attr(not(test), allow(dead_code))] // used by the tests
    pub fn help(&self, window: WindowId) -> Option<&'static str> {
        self.window(window).and_then(|w| w.help)
    }

    /// Shows the topic `id` in a window's help panel (an unknown id shows the contents).
    pub fn show_help(&mut self, window: WindowId, id: &str) {
        let topic = hikasa_session::help::topic(id)
            .or_else(|| hikasa_session::help::topic(hikasa_session::help::INDEX_TOPIC));
        if let Some(w) = self.window_mut(window) {
            w.help = topic.map(|t| t.id);
        }
    }

    /// Hides a window's help panel.
    pub fn close_help(&mut self, window: WindowId) {
        if let Some(w) = self.window_mut(window) {
            w.help = None;
        }
    }

    /// A button of the window's top dialog was pressed.
    pub fn dialog_button(&mut self, window: WindowId, role: ButtonRole) -> Vec<Effect> {
        let service = self.service.clone();
        let step = match self.window_mut(window).and_then(|w| w.dialogs.top_mut()) {
            Some(top) => top.on_button(role, &service),
            None => return Vec::new(),
        };
        let mut effects = Vec::new();
        match step {
            DialogStep::Stay => {}
            DialogStep::Close => {
                if let Some(closed) = self.window_mut(window).and_then(|w| w.dialogs.pop()) {
                    effects.extend(self.dialog_closed(closed));
                }
            }
            DialogStep::CloseWith(followups) => {
                if let Some(closed) = self.window_mut(window).and_then(|w| w.dialogs.pop()) {
                    effects.extend(self.dialog_closed(closed));
                }
                for f in followups {
                    effects.extend(self.followup(window, f));
                }
            }
        }
        effects.extend(self.dialog_requests());
        effects
    }

    // ---- the settings dialogs (W4c) ----------------------------------------------------------------------------

    /// Opens the Options dialog on `page` (`None`: the last page used, `state.dialogs.options.page`). One at a time:
    /// a dialog open in another window is raised instead.
    fn open_options(&mut self, window: WindowId, page: Option<&'static str>) -> Vec<Effect> {
        if let Some(other) = self
            .windows
            .iter()
            .find(|w| w.dialogs.holds::<OptionsDialog>())
            .map(|w| w.id)
        {
            return vec![Effect::Raise(other)];
        }
        let remembered = self.state.dialog_text("options", "page").map(str::to_owned);
        let dialog = OptionsDialog::new(
            self.settings.clone(),
            page.or(remembered.as_deref()),
            self.workspace_names(),
        );
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
        Vec::new()
    }

    /// "Toolbar Options…" (UI-004): the commands page with the view preselected.
    pub fn open_commands_page(&mut self, window: WindowId, view: &str) {
        self.run(
            window,
            Invocation::with(CommandId::ProgramOptions, CommandArg::Page("commands")),
            true,
        );
        if let Some(d) = self
            .window_mut(window)
            .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
            && let Some((name, _)) = crate::options::commands_page::SHOWN_VIEWS
                .iter()
                .find(|(v, _)| *v == view)
        {
            d.select_view(name);
        }
    }

    /// Opens the Import wizard, prefilled with `path`, and returns its window. One at a time: an open wizard takes
    /// the path on its first step, is replaced after its result, asks in between and refuses (with a notice) while
    /// a job runs: a package is never dropped or imported in place of another in silence.
    fn open_import(&mut self, window: WindowId, path: Option<&std::path::Path>) -> WindowId {
        let wake = self.wake();
        let open = self
            .windows
            .iter()
            .find(|w| w.dialogs.holds::<ImportWizard>())
            .map(|w| w.id);
        let Some(owner) = open else {
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(ImportWizard::new(path, wake)));
            }
            return window;
        };
        let (Some(path), Some(wizard)) = (
            path,
            self.window_mut(owner)
                .and_then(|w| w.dialogs.find_mut::<ImportWizard>()),
        ) else {
            return owner;
        };
        let current = wizard.path.clone();
        match wizard.offer(path) {
            PackageOffer::Taken => {}
            PackageOffer::Fresh => *wizard = ImportWizard::new(Some(path), wake),
            PackageOffer::Ask => {
                let prompt = Prompt::ask(
                    "Import Settings",
                    format!(
                        "The Import wizard is on {current}. Switch to {}? The choices made so far are lost.",
                        path.display()
                    ),
                    ButtonRole::Cancel,
                    vec![
                        (
                            ButtonRole::Yes,
                            "Switch",
                            vec![Followup::SwitchPackage(path.to_path_buf())],
                        ),
                        (ButtonRole::Cancel, "Keep this package", Vec::new()),
                    ],
                    "wizard-import",
                );
                if let Some(w) = self.window_mut(owner) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            PackageOffer::Refused => self.package_refused(owner, path),
        }
        owner
    }

    fn package_refused(&mut self, window: WindowId, path: &std::path::Path) {
        let text = format!(
            "The Import wizard is working: {} was not opened. Open it when the wizard has finished.",
            path.display()
        );
        self.notify(
            window,
            Notice::new("import-busy", NoticeKind::Warning, text),
            "settings",
            LogKind::Warning,
        );
    }

    /// The fields of a window's dialog changed (each frame of the Options dialog): the preview (D-130).
    pub fn dialog_edited(&mut self, window: WindowId) -> Vec<Effect> {
        let mut effects = self.preview_step_of(window);
        effects.extend(self.dialog_requests());
        effects
    }

    fn preview_step_of(&mut self, window: WindowId) -> Vec<Effect> {
        let edited = self
            .window_mut(window)
            .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
            .is_some_and(|d| std::mem::take(&mut d.theme_edited));
        if edited {
            self.preview_step()
        } else {
            Vec::new()
        }
    }

    /// Shows the theme of the Options dialog's working copies when it differs from what is on screen; one
    /// `ApplyTheme` per step (D-130).
    fn preview_step(&mut self) -> Vec<Effect> {
        let os_dark = self.os_dark;
        let working = self.windows.iter_mut().find_map(|w| {
            w.dialogs
                .find_mut::<OptionsDialog>()
                .map(|d| (d.working_scheme(os_dark), d.theme.clone()))
        });
        let Some(((scheme, base), theme)) = working else {
            return Vec::new();
        };
        let changed = self.scheme != scheme || self.scheme_base() != base || *self.theme() != theme;
        self.scheme = scheme;
        self.preview = Some(Preview { base, theme });
        if changed {
            vec![Effect::ApplyTheme]
        } else {
            Vec::new()
        }
    }

    /// A dialog left its stack: the Options dialog's preview ends with the applied theme, its page is remembered.
    fn dialog_closed(&mut self, closed: Box<dyn DialogModel>) -> Vec<Effect> {
        let Some(options) = closed.as_any().downcast_ref::<OptionsDialog>() else {
            return Vec::new();
        };
        let page = options.page;
        if self.state.dialog_text("options", "page") != Some(page) {
            self.state.set_dialog_text("options", "page", page);
            self.update_state(move |s| s.set_dialog_text("options", "page", page));
        }
        let previewing = self.preview.take();
        let mut effects = self.reapply_theme();
        if effects.is_empty() && previewing.is_some_and(|p| p.theme != *self.settings.theme.value())
        {
            effects.push(Effect::ApplyTheme);
        }
        self.check_fonts();
        effects
    }

    /// What the dialogs of every window asked for: writes on the settings queue, prompts, closes.
    fn dialog_requests(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        for window in self.window_ids() {
            effects.extend(self.preview_step_of(window));
            let Some(w) = self.window_mut(window) else {
                continue;
            };
            let requests = w.dialogs.take_requests();
            // Highest index first: a close never shifts a dialog still to be served.
            for (index, request) in requests.into_iter().rev() {
                match request {
                    DialogRequest::Settings { title, body } => {
                        self.queue_settings(QueuedSettings::Write(title, body));
                    }
                    DialogRequest::Ask(prompt) => {
                        if let Some(w) = self.window_mut(window) {
                            w.dialogs.push(prompt);
                        }
                        effects.push(Effect::Raise(window));
                    }
                    DialogRequest::Close => {
                        if let Some(w) = self.window_mut(window) {
                            let closed = w.dialogs.remove(index);
                            effects.extend(self.dialog_closed(closed));
                        }
                    }
                    DialogRequest::Refresh => self.refresh_sessions(),
                }
            }
        }
        effects
    }

    /// A key in a window with an open dialog (D-112): F1 shows the dialog's topic, Esc and Enter press their
    /// buttons; other keys are the dialog's own.
    pub fn dialog_key(&mut self, window: WindowId, key: &Shortcut) -> Vec<Effect> {
        let Some(top) = self.window_mut(window).and_then(|w| w.dialogs.top_mut()) else {
            return Vec::new();
        };
        if top.wants_keys() {
            top.on_key(key);
            return self.dialog_edited(window);
        }
        if key.key == "F1" && !key.ctrl && !key.alt && !key.shift && !key.cmd {
            let topic = top.help_topic();
            self.show_help(window, topic);
            return Vec::new();
        }
        match top.on_key(key) {
            Some(role) => self.dialog_button(window, role),
            None => self.dialog_requests(),
        }
    }

    /// Sets the "Do not ask again" check box of the window's top prompt.
    pub fn set_do_not_ask(&mut self, window: WindowId, ticked: bool) {
        if let Some(top) = self.window_mut(window).and_then(|w| w.dialogs.top_mut())
            && let Some(prompt) = top.as_any_mut().downcast_mut::<Prompt>()
        {
            prompt.ticked = ticked;
        }
    }

    fn followup(&mut self, window: WindowId, f: Followup) -> Vec<Effect> {
        match f {
            Followup::Run(inv) => {
                if self.enabled(window, inv.id) {
                    self.run(window, inv, true)
                } else {
                    Vec::new()
                }
            }
            Followup::CloseWindow => self.close_window_confirmed(window),
            Followup::Exit => self.exit_confirmed(),
            Followup::Session(action) => self.session_action(window, action),
            Followup::DisableOption(key) => {
                self.set_option_false(key);
                Vec::new()
            }
            Followup::Options(answer) => {
                if let Some(d) = self
                    .window_mut(window)
                    .and_then(|w| w.dialogs.find_mut::<OptionsDialog>())
                {
                    d.answer(answer);
                }
                self.dialog_requests()
            }
            Followup::SwitchPackage(path) => {
                let wake = self.wake();
                let wizard = self
                    .window_mut(window)
                    .and_then(|w| w.dialogs.find_mut::<ImportWizard>());
                match wizard {
                    Some(wizard) if wizard.busy() => self.package_refused(window, &path),
                    Some(wizard) => *wizard = ImportWizard::new(Some(&path), wake),
                    None => {}
                }
                Vec::new()
            }
            Followup::WizardOverwrite => {
                let service = self.service.clone();
                if let Some(d) = self
                    .window_mut(window)
                    .and_then(|w| w.dialogs.find_mut::<ExportWizard>())
                {
                    d.overwrite(&service);
                }
                Vec::new()
            }
        }
    }

    /// Writes `options.<key> = false` with `settings_set` (the CLI's path) on a job, then takes a fresh snapshot.
    fn set_option_false(&mut self, key: &'static str) {
        let path = self
            .service
            .settings_store()
            .path(hikasa_session::settings::FileId::Options);
        self.start_settings_job("set an option", move |svc| {
            let key = parse_key(&format!("options.{key}"))?;
            svc.settings_set(&key, "false", false)?;
            Ok(vec![path])
        });
        // The window follows at once (the snapshot arrives with the job).
        let mut options = self.options().clone();
        set_bool(&mut options, key, false);
        *self.settings.options.value_mut() = options;
    }

    /// Starts a settings write: `body` writes and returns the written paths; the job acknowledges them with the
    /// watch thread, reloads the service's settings when it wrote `options.toml` (D-119) and takes a fresh
    /// snapshot.
    fn start_settings_job(
        &mut self,
        title: &str,
        body: impl FnOnce(&Service) -> Result<Vec<PathBuf>, ApiError> + Send + 'static,
    ) {
        self.queue_settings(QueuedSettings::Write(title.to_owned(), Box::new(body)));
    }

    /// Queues a settings job with the next sequence number and starts it when none runs.
    fn queue_settings(&mut self, job: QueuedSettings) {
        self.settings_issued += 1;
        self.queued_settings.push_back((self.settings_issued, job));
        self.next_settings_job();
    }

    /// Starts the next queued settings write or reload when none runs (one at a time, in sequence order).
    fn next_settings_job(&mut self) {
        if self
            .jobs
            .iter()
            .any(|j| matches!(j, CoreJob::Settings(..) | CoreJob::Reload(..)))
        {
            return;
        }
        let Some((seq, queued)) = self.queued_settings.pop_front() else {
            return;
        };
        let watch = self.watch.clone();
        match queued {
            QueuedSettings::Write(title, body) => {
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsWrite,
                    title,
                    &self.wake(),
                    move |svc, _| {
                        let written = body(svc)?;
                        if let Some(link) = &watch {
                            link.send(WatchCmd::Acknowledge(written.clone()));
                        }
                        // The window's own write of `options.toml` reaches later calls and jobs too (D-119).
                        let reload = written.iter().any(|p| {
                            matches!(file_class(p), FileClass::Options | FileClass::Formats)
                        });
                        let warnings = if reload {
                            svc.reload_settings()
                        } else {
                            Vec::new()
                        };
                        Ok((svc.settings_snapshot()?, warnings))
                    },
                );
                self.jobs.push(CoreJob::Settings(seq, job));
            }
            QueuedSettings::Reload(changes) => {
                let reload = changes.iter().any(|c| {
                    matches!(file_class(&c.path), FileClass::Options | FileClass::Formats)
                });
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsRead,
                    "reload the settings".into(),
                    &self.wake(),
                    move |svc, _| {
                        let warnings = if reload {
                            svc.reload_settings()
                        } else {
                            Vec::new()
                        };
                        Ok((warnings, svc.settings_snapshot()?))
                    },
                );
                self.jobs.push(CoreJob::Reload(seq, job, changes));
            }
        }
    }

    /// A settings job `seq` ended with `snapshot`: applied when it is the newest job queued (a later write takes
    /// its own, newer snapshot, so an in-memory edit waiting for that write is never reverted), else kept for the
    /// case that the later job fails. Never older than the snapshot applied last.
    fn settings_result(&mut self, seq: u64, snapshot: SettingsSnapshot) -> Vec<Effect> {
        if seq <= self.settings_applied {
            return Vec::new();
        }
        if seq < self.settings_issued {
            self.skipped_snapshot = Some((seq, snapshot));
            return Vec::new();
        }
        self.skipped_snapshot = None;
        self.settings_applied = seq;
        self.apply_snapshot(snapshot)
    }

    /// A settings job `seq` failed: when it was the newest, the snapshot skipped for it applies (the window
    /// drops its own edit the disk did not take).
    fn settings_failed(&mut self, seq: u64) -> Vec<Effect> {
        if seq < self.settings_issued {
            return Vec::new();
        }
        match self.skipped_snapshot.take() {
            Some((s, snapshot)) if s > self.settings_applied => {
                self.settings_applied = s;
                self.apply_snapshot(snapshot)
            }
            _ => Vec::new(),
        }
    }

    /// The toolbar's context menu (UI-004): the display filter mode of the window's view type, written to
    /// `commands.toml` with `settings_save` on a job.
    pub fn set_display_filter_mode(&mut self, window: WindowId, mode: DisplayFilterMode) {
        self.change_commands(window, move |c| c.display_filter_mode = mode);
    }

    /// The toolbar's context menu: "Large buttons with text labels".
    pub fn set_large_buttons(&mut self, window: WindowId, on: bool) {
        self.change_commands(window, move |c| c.large_buttons = on);
    }

    fn change_commands(
        &mut self,
        window: WindowId,
        change: impl Fn(&mut hikasa_session::settings::ViewCustomization) + Send + 'static,
    ) {
        let Some(view) = self.scope(window).view_name() else {
            return;
        };
        let mut commands = self.settings.commands.value().clone();
        change(commands.view.entry(view.to_owned()).or_default());
        // The window follows at once; the file is written on a job, which applies the change to the file as it
        // is then (a write of an earlier change may still be on its way).
        *self.settings.commands.value_mut() = commands.clone();
        self.tables = Arc::new(CommandTables::new(&commands));
        self.start_settings_job("save the toolbar", move |svc| {
            // Another write of the window may land in between: take the file again and retry (twice).
            let mut attempt = 0;
            loop {
                attempt += 1;
                let snapshot = svc.settings_snapshot()?;
                let mut commands = snapshot.commands.value().clone();
                change(commands.view.entry(view.to_owned()).or_default());
                let changes = SettingsChanges {
                    commands: Some(commands),
                    ..SettingsChanges::default()
                };
                let out = svc.settings_save(&snapshot, &changes, false)?;
                let mut written = Vec::new();
                let mut conflict = None;
                for (_, path, outcome) in out.files {
                    match outcome {
                        FileOutcome::Written | FileOutcome::Unchanged | FileOutcome::Deleted => {
                            written.push(path);
                        }
                        FileOutcome::Conflict => conflict = Some(path),
                        FileOutcome::Invalid(p) => {
                            return Err(ApiError::InvalidArgument(
                                p.iter()
                                    .map(|p| format!("{}: {}", p.key, p.message))
                                    .collect::<Vec<_>>()
                                    .join("; "),
                            ));
                        }
                        FileOutcome::Failed(e) => return Err(ApiError::Internal(e)),
                    }
                }
                match conflict {
                    None => return Ok(written),
                    Some(_) if attempt < 3 => {}
                    Some(path) => {
                        return Err(ApiError::Session(hikasa_session::SessionError::Conflict {
                            path: path.display().to_string(),
                        }));
                    }
                }
            }
        });
    }

    /// Hides the message panel of a view; with `do_not_show`, its option key is set to `false` too.
    pub fn dismiss_message(&mut self, view: ViewId, do_not_show: bool) {
        let key = self
            .tab_mut(view)
            .and_then(|t| t.message.take())
            .and_then(|m| m.do_not_show);
        if do_not_show && let Some(key) = key {
            self.set_option_false(key);
        }
    }

    /// Shows a message in a view's message panel.
    pub fn set_message(&mut self, view: ViewId, message: Message) {
        if let Some(tab) = self.tab_mut(view) {
            tab.message = Some(message);
        }
    }

    fn push_notice(&mut self, window: WindowId, notice: Notice) {
        let now = self.now;
        let window = if self.window(window).is_some() {
            window
        } else {
            self.focused()
        };
        if let Some(w) = self.window_mut(window) {
            w.notices.push(notice, now);
        }
    }

    /// Pushes a notice and its log entry (D-117: each notice is also a log entry).
    fn notify(&mut self, window: WindowId, notice: Notice, source: &str, kind: LogKind) {
        self.log_entry(kind, source, notice.text.clone());
        self.push_notice(window, notice);
    }

    /// A button of a notice was pressed.
    pub fn notice_action(
        &mut self,
        window: WindowId,
        key: &str,
        action: NoticeAction,
    ) -> Vec<Effect> {
        let Some(action) = self
            .window_mut(window)
            .and_then(|w| w.notices.act(key, action))
        else {
            return Vec::new();
        };
        match action {
            NoticeAction::ReloadView(view) => {
                let service = self.service.clone();
                if let Some(Tab {
                    content: TabContent::Live(ViewModel::Text(m)),
                    ..
                }) = self.tab_mut(view)
                {
                    m.execute(&service, CommandId::Reload);
                }
            }
            NoticeAction::ShowTab(view) => {
                if let Some(w) = self.window_of(view) {
                    let index = self.window(w).and_then(|x| x.index_of(view));
                    if let Some(index) = index {
                        self.activate(w, index);
                    }
                    self.focus(w);
                }
            }
            // §1.4.7: the unapplied edits of the file go; the Options dialog reads it again.
            NoticeAction::ReloadSettings => {
                for w in &mut self.windows {
                    if let Some(d) = w.dialogs.find_mut::<OptionsDialog>()
                        && let Some(file) = d.notified.remove(key)
                    {
                        d.reload_files(vec![file]);
                    }
                }
                let effects = self.dialog_requests();
                return effects;
            }
            NoticeAction::OpenOptions => return self.open_options(window, None),
            // The dialog keeps its old stamp: the next Apply meets `Conflict` (D-128).
            NoticeAction::KeepMine | NoticeAction::Dismiss => {}
        }
        self.dirty_effects()
    }

    // ---- the log book ----------------------------------------------------------------------------------------

    fn log_entry(&mut self, kind: LogKind, source: &str, text: String) {
        let time = self.service.clock().now();
        self.log.push(LogEntry {
            time,
            kind,
            source: source.to_owned(),
            text,
        });
    }

    /// Appends the lines not saved yet to `save_to_file` (on a job). One append runs at a time: lines pushed
    /// meanwhile wait in the book and go with the next one, in order.
    fn flush_log(&mut self) {
        if self.log_save_failed || self.jobs.iter().any(|j| matches!(j, CoreJob::Log(..))) {
            return;
        }
        let file = self.options().tweaks.log_panel.save_to_file.clone();
        if file.is_empty() {
            return;
        }
        let lines = self.log.take_unsaved();
        if lines.is_empty() {
            return;
        }
        let path = match hikasa_core::Spec::parse_os(
            std::ffi::OsStr::new(&file),
            &self.service.spec_context(),
        ) {
            Ok(hikasa_core::Spec::Local(p)) => p.into_path_buf(),
            _ => PathBuf::from(&file),
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsWrite,
            "save the log".into(),
            &self.wake(),
            move |svc, _| svc.log_append(&path, &lines),
        );
        self.jobs.push(CoreJob::Log(file, job));
    }

    /// Saving the log failed: saving stops (the lines are dropped) until the settings change, with one warning
    /// notice. Nothing goes through tracing, so the failure never feeds the book it failed to save.
    fn stop_log_saving(&mut self, file: &str, e: &ApiError) {
        self.log_save_failed = true;
        let options = self.options().clone();
        self.log
            .set_filter(LogFilter::from_options(&options), false);
        let window = self.focused();
        self.notify(
            window,
            Notice::new(
                "log:save",
                NoticeKind::Warning,
                format!("Cannot save the log to {file}: {e}. The log is not saved until the settings change."),
            ),
            "log",
            LogKind::Warning,
        );
    }

    // ---- the core's jobs, the watch --------------------------------------------------------------------------

    fn poll_jobs(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut pending = Vec::new();
        for job in std::mem::take(&mut self.jobs) {
            match job {
                CoreJob::Settings(seq, j) => match j.try_result() {
                    None => pending.push(CoreJob::Settings(seq, j)),
                    Some(Ok((snapshot, warnings))) => {
                        effects.extend(self.settings_result(seq, snapshot));
                        let time = self.service.clock().now();
                        self.log.push_warnings(&time, "settings", &warnings);
                    }
                    Some(Err(e)) => {
                        let window = self.focused();
                        self.notify(
                            window,
                            Notice::new("settings:write", NoticeKind::Error, e.to_string()),
                            "settings",
                            LogKind::Error,
                        );
                        effects.extend(self.settings_failed(seq));
                    }
                },
                CoreJob::Reload(seq, j, changes) => match j.try_result() {
                    None => pending.push(CoreJob::Reload(seq, j, changes)),
                    Some(Ok((warnings, snapshot))) => {
                        effects.extend(self.settings_result(seq, snapshot));
                        self.reload_notices(&changes, &warnings);
                    }
                    Some(Err(e)) => {
                        let window = self.focused();
                        self.notify(
                            window,
                            Notice::new("settings:reload", NoticeKind::Error, e.to_string()),
                            "settings",
                            LogKind::Error,
                        );
                        effects.extend(self.settings_failed(seq));
                    }
                },
                CoreJob::State(j) => match j.try_result() {
                    None => pending.push(CoreJob::State(j)),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        self.log_entry(
                            LogKind::Error,
                            "layout",
                            format!("Cannot save the window layout: {e}"),
                        );
                    }
                },
                CoreJob::Log(file, j) => match j.try_result() {
                    None => pending.push(CoreJob::Log(file, j)),
                    Some(Ok(())) => {}
                    Some(Err(e)) => self.stop_log_saving(&file, &e),
                },
                CoreJob::Store(j, view, kind) => match j.try_result() {
                    None => pending.push(CoreJob::Store(j, view, kind)),
                    Some(result) => {
                        self.store_running = false;
                        effects.extend(self.store_finished(result, view, kind));
                    }
                },
                CoreJob::Drop(window, j) => match j.try_result() {
                    None => pending.push(CoreJob::Drop(window, j)),
                    Some(Ok(open)) => {
                        let before = self.window_ids();
                        let target = if self.window(window).is_some() {
                            OpenTarget::Window(window)
                        } else {
                            self.open_target()
                        };
                        self.open_local(open.kind, open.request, &open.options, open.specs, target);
                        effects.extend(self.new_viewports(&before));
                    }
                    Some(Err(e)) => {
                        self.log_entry(LogKind::Error, "drop", e.to_string());
                        if let Some(view) = self.active_tab(window).map(|t| t.id) {
                            self.set_message(view, Message::error(e.to_string()));
                        }
                    }
                },
            }
        }
        // Jobs started while applying results stay.
        pending.append(&mut self.jobs);
        self.jobs = pending;
        self.next_settings_job();
        self.next_store_job();
        self.flush_log();
        effects
    }

    /// Applies a fresh snapshot: the command tables, the theme, the log filter (D-117 reactions).
    fn apply_snapshot(&mut self, snapshot: SettingsSnapshot) -> Vec<Effect> {
        let mut effects = Vec::new();
        let commands_changed = snapshot.commands.value() != self.settings.commands.value();
        if snapshot.options.value() != self.settings.options.value() {
            // Changed settings may have fixed the log file: saving resumes.
            self.log_save_failed = false;
        }
        self.settings = snapshot;
        if commands_changed {
            self.tables = Arc::new(CommandTables::new(self.settings.commands.value()));
            effects.push(Effect::ApplyCommands);
        }
        let options = self.options().clone();
        let limit = options.tweaks.metrics.max_auto_saved_sessions;
        if limit != self.auto_limit {
            self.auto_limit = limit;
            self.prune_autosaved();
        }
        self.log.set_filter(
            LogFilter::from_options(&options),
            !options.tweaks.log_panel.save_to_file.is_empty() && !self.log_save_failed,
        );
        self.sync_debug_flag();
        // An Options dialog follows every file it has no unapplied edits for (§1.4.7).
        let settings = self.settings.clone();
        for w in &mut self.windows {
            if let Some(d) = w.dialogs.find_mut::<OptionsDialog>() {
                d.follow(&settings);
            }
        }
        effects.extend(self.reapply_theme());
        self.check_fonts();
        effects
    }

    /// The reactions of §1.4.7 to what the watch found.
    pub fn on_watch(&mut self, events: Vec<WatchEvent>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for event in events {
            match event {
                WatchEvent::Config(changes) => self.on_config(changes),
                WatchEvent::Inputs(changes) => effects.extend(self.on_inputs(changes)),
            }
        }
        effects
    }

    fn on_config(&mut self, changes: Vec<ConfigChange>) {
        let changes: Vec<ConfigChange> = changes
            .into_iter()
            .filter(|c| file_class(&c.path) != FileClass::State)
            .collect();
        if changes.is_empty() {
            return;
        }
        // Through the settings queue: a reload never runs beside the window's own writes.
        self.queue_settings(QueuedSettings::Reload(changes));
    }

    /// The notices of a settings reload (in the window focused last).
    fn reload_notices(&mut self, changes: &[ConfigChange], warnings: &[Warning]) {
        let window = self.focused();
        for change in changes {
            let name = change
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let how = match change.kind {
                ChangeKind::Removed => "was removed: defaults apply".to_owned(),
                ChangeKind::Created | ChangeKind::Changed => "changed on disk".to_owned(),
            };
            let class = file_class(&change.path);
            // §1.4.7: a file the Options dialog has unapplied edits for.
            let key = format!("settings:{name}");
            let file = file_id(&change.path);
            let edited = self.windows.iter_mut().find_map(|w| {
                let d = w.dialogs.find_mut::<OptionsDialog>()?;
                let file = file.clone().filter(|f| d.has_edits(f))?;
                d.notified.insert(key.clone(), file);
                Some(())
            });
            if edited.is_some() {
                let text =
                    format!("{name} {how}, and the Options dialog has unapplied edits of it.");
                self.notify(
                    window,
                    Notice::new(key, NoticeKind::Warning, text)
                        .with_actions(vec![NoticeAction::ReloadSettings, NoticeAction::KeepMine]),
                    "settings",
                    LogKind::Warning,
                );
                continue;
            }
            let (text, kind) = match class {
                FileClass::Options => {
                    let problems: Vec<&Warning> = warnings
                        .iter()
                        .filter(|w| w.message.contains(&name))
                        .collect();
                    if problems.is_empty() {
                        (
                            format!("Settings reloaded: {name} {how}."),
                            NoticeKind::Info,
                        )
                    } else {
                        let keys = problems
                            .iter()
                            .map(|w| w.message.clone())
                            .collect::<Vec<_>>()
                            .join("; ");
                        (
                            format!("Settings reloaded: {name} {how}; reset to defaults: {keys}"),
                            NoticeKind::Warning,
                        )
                    }
                }
                FileClass::Commands => (
                    format!("Toolbars, shortcuts and menus reloaded: {name} {how}."),
                    NoticeKind::Info,
                ),
                FileClass::Theme => (format!("Theme reloaded: {name} {how}."), NoticeKind::Info),
                FileClass::Formats => (
                    format!(
                        "File formats reloaded: {name} {how}; open comparisons use the new formats when reloaded."
                    ),
                    NoticeKind::Info,
                ),
                FileClass::Other | FileClass::State => (
                    format!("Settings reloaded: {name} {how}."),
                    NoticeKind::Info,
                ),
            };
            let log_kind = if kind == NoticeKind::Info {
                LogKind::Summary
            } else {
                LogKind::Warning
            };
            let notice = Notice::new(key, kind, text);
            let notice = if kind == NoticeKind::Warning {
                notice.with_actions(vec![NoticeAction::OpenOptions])
            } else {
                notice
            };
            self.notify(window, notice, "settings", log_kind);
        }
        for w in warnings {
            self.log_entry(
                LogKind::Warning,
                "settings",
                format!("[{}] {}", w.code, w.message),
            );
        }
    }

    /// D-118: an input of a text view changed or disappeared.
    fn on_inputs(&mut self, changes: Vec<(String, ChangeKind)>) -> Vec<Effect> {
        let o = &self.options().tweaks.file_operations;
        if !o.check_for_files_changed_on_disk {
            return Vec::new();
        }
        let auto = o.automatically_reload;
        let service = self.service.clone();
        let mut notices: Vec<(WindowId, Notice)> = Vec::new();
        for (spec, kind) in changes {
            for w in &mut self.windows {
                for tab in &mut w.tabs {
                    let id = tab.id;
                    let Some(ViewModel::Text(m)) = tab.model_mut() else {
                        continue;
                    };
                    if !m.inputs().contains(&spec) {
                        continue;
                    }
                    let name = crate::shared::shown_path(&spec);
                    let key = format!("input:{}:{spec}", id.0);
                    let notice = match kind {
                        ChangeKind::Removed => Notice::new(
                            key,
                            NoticeKind::Warning,
                            format!("{name} was removed or renamed."),
                        ),
                        // Unsaved edits arrive with W11: a view without edits reloads by itself.
                        ChangeKind::Changed | ChangeKind::Created if auto => {
                            m.execute(&service, CommandId::Reload);
                            Notice::new(
                                key,
                                NoticeKind::Info,
                                format!("{name} changed on disk and was reloaded."),
                            )
                        }
                        ChangeKind::Changed | ChangeKind::Created => Notice::new(
                            key,
                            NoticeKind::Warning,
                            format!("{name} changed on disk."),
                        )
                        .with_actions(vec![NoticeAction::ReloadView(id)]),
                    };
                    notices.push((w.id, notice));
                }
            }
        }
        for (window, notice) in notices {
            let kind = if notice.kind == NoticeKind::Info {
                LogKind::Summary
            } else {
                LogKind::Warning
            };
            self.notify(window, notice, "files", kind);
        }
        self.dirty_effects()
    }

    // ---- drag and drop ---------------------------------------------------------------------------------------

    /// Paths dropped from the OS on a target of a window (UI-008).
    pub fn drop_paths(
        &mut self,
        window: WindowId,
        target: DropTarget,
        paths: Vec<PathBuf>,
    ) -> Vec<Effect> {
        match decide(target, &paths) {
            DropOutcome::Ignore => {}
            DropOutcome::Fill(fields) => {
                if let Some(Tab {
                    content: TabContent::Live(ViewModel::Home(home)),
                    ..
                }) = self.active_tab_mut(window)
                {
                    for (field, text) in fields {
                        match field {
                            FieldId::HomeLeft => home.panel.left = text,
                            FieldId::HomeRight => home.panel.right = text,
                        }
                    }
                    home.error = None;
                }
            }
            DropOutcome::Message(text) => {
                if let Some(view) = self.active_tab(window).map(|t| t.id) {
                    self.set_message(view, Message::info(text));
                }
            }
            DropOutcome::NewSession(kind, left) => self.drop_new_session(window, kind, left),
            DropOutcome::OpenType(kind, paths) => self.drop_open_type(window, kind, paths),
            DropOutcome::Open([left, right]) => {
                let request = hikasa_session::api::GuiOpenRequest {
                    specs: vec![left.into_os_string(), right.into_os_string()],
                    ..hikasa_session::api::GuiOpenRequest::default()
                };
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsRead,
                    "open the dropped paths".into(),
                    &self.wake(),
                    move |svc, _| crate::home::resolve(svc, &request),
                );
                self.jobs.push(CoreJob::Drop(window, job));
            }
        }
        Vec::new()
    }

    // ---- status ----------------------------------------------------------------------------------------------

    /// The status bar of a window's active view.
    pub fn status(&self, window: WindowId) -> StatusItems {
        match self.active_tab(window).and_then(Tab::model) {
            Some(ViewModel::Folder(m)) => m.status_items(),
            Some(ViewModel::Text(m)) => m.status_items(),
            _ => StatusItems {
                left: "Compare two folders or two files.".to_owned(),
                right: vec![format!("Hikasa {}", hikasa_session::HIKASA_VERSION)],
            },
        }
    }

    /// The running jobs of every view of a window (the job indicator, D-115).
    pub fn jobs(&self, window: WindowId) -> Vec<JobStatus> {
        let Some(w) = self.window(window) else {
            return Vec::new();
        };
        w.tabs
            .iter()
            .flat_map(|t| match t.model() {
                Some(ViewModel::Folder(m)) => m.jobs(t.id),
                Some(ViewModel::Text(m)) => m.jobs(t.id),
                _ => Vec::new(),
            })
            .collect()
    }

    /// Cancels the jobs of one view (a row of the job indicator).
    pub fn cancel_jobs(&mut self, view: ViewId) {
        if let Some(tab) = self.tab_mut(view) {
            match tab.model_mut() {
                Some(ViewModel::Folder(m)) => m.cancel(),
                Some(ViewModel::Text(m)) => m.cancel(),
                _ => {}
            }
        }
    }

    /// True when the window shows its tab bar (hidden with `hide_tab_bar_if_single_tab` and one tab).
    pub fn tab_bar_visible(&self, window: WindowId) -> bool {
        let tabs = self.window(window).map_or(0, |w| w.tabs.len());
        !(self.options().tabs.hide_tab_bar_if_single_tab && tabs == 1)
    }

    /// The other windows of `window` with their captions (`TabMoveToWindow ▸`).
    pub fn other_windows(&self, window: WindowId) -> Vec<(WindowId, String)> {
        self.windows
            .iter()
            .filter(|w| w.id != window)
            .map(|w| {
                let caption = w
                    .active_tab()
                    .map_or_else(|| "Home".to_owned(), Tab::caption);
                let mut caption = crate::shared::shown_name(&caption);
                if caption.chars().count() > CAPTION_CHARS {
                    caption = caption.chars().take(CAPTION_CHARS - 1).collect::<String>() + "…";
                }
                (w.id, caption)
            })
            .collect()
    }
}

/// How long an exit nobody can refuse waits for a settings write (an import, a restore).
const SETTINGS_FLUSH: std::time::Duration = std::time::Duration::from_secs(60);

/// `tabs.warn_when_closing_multiple_tabs` (the prompts' "Do not ask again").
const WARN_TABS: &str = "tabs.warn_when_closing_multiple_tabs";
/// `tabs.warn_when_closing_multiple_windows`.
const WARN_WINDOWS: &str = "tabs.warn_when_closing_multiple_windows";

/// Sets one of the boolean option keys the window's prompts and messages bind (in memory).
fn set_bool(o: &mut Options, key: &str, value: bool) {
    match key {
        WARN_TABS => o.tabs.warn_when_closing_multiple_tabs = value,
        WARN_WINDOWS => o.tabs.warn_when_closing_multiple_windows = value,
        "next_difference.show_message_panel" => o.next_difference.show_message_panel = value,
        _ => {}
    }
}

/// The settings files by what a change of them triggers (§1.4.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileClass {
    Options,
    Commands,
    Theme,
    Formats,
    State,
    Other,
}

fn file_class(path: &std::path::Path) -> FileClass {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match (parent.as_str(), name.as_str()) {
        ("themes", _) => FileClass::Theme,
        ("formats", _) => FileClass::Formats,
        (_, "options.toml") => FileClass::Options,
        (_, "commands.toml") => FileClass::Commands,
        (_, "theme.toml") => FileClass::Theme,
        (_, "state.toml") => FileClass::State,
        _ => FileClass::Other,
    }
}

/// The settings file of a path (`state.toml` and the format files have none here).
fn file_id(path: &std::path::Path) -> Option<hikasa_session::settings::FileId> {
    use hikasa_session::settings::FileId;
    let stem = path.file_stem()?.to_str()?;
    let parent = path.parent().and_then(|p| p.file_name())?.to_str()?;
    match (parent, stem) {
        ("themes", name) => Some(FileId::Themes(name.to_owned())),
        ("formats", _) => None,
        (_, "options") => Some(FileId::Options),
        (_, "commands") => Some(FileId::Commands),
        (_, "theme") => Some(FileId::Theme),
        (_, "filters") => Some(FileId::Filters),
        (_, "openwith") => Some(FileId::OpenWith),
        (_, "archives") => Some(FileId::Archives),
        _ => None,
    }
}

/// A snapshot of defaults (the configuration could not be read at start).
fn default_snapshot(service: &Service) -> SettingsSnapshot {
    use hikasa_session::settings::{FileId, SettingsFile};
    let store = service.settings_store();
    let mut schemes = std::collections::BTreeMap::new();
    for name in hikasa_session::settings::theme::BUILTIN_SCHEMES {
        schemes.insert(
            name.to_owned(),
            SettingsFile::new_default(&store.path(FileId::Themes(name.to_owned()))),
        );
    }
    SettingsSnapshot {
        options: SettingsFile::new_default(&store.path(FileId::Options)),
        commands: SettingsFile::new_default(&store.path(FileId::Commands)),
        theme: SettingsFile::new_default(&store.path(FileId::Theme)),
        schemes,
        filters: SettingsFile::new_default(&store.path(FileId::Filters)),
        openwith: SettingsFile::new_default(&store.path(FileId::OpenWith)),
        archives: SettingsFile::new_default(&store.path(FileId::Archives)),
    }
}
