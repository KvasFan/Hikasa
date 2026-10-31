//! Sessions of views in the app core (M2 W4b, §1.4.9–§1.4.12; D-120…D-126): the queue of store jobs, the close flow
//! (D-84) around every close path, saving, auto-save, opening sessions, the Home tree's commands, the Session
//! Settings results and workspaces.
//!
//! Every read or write of a session or workspace file is a *store job*: they run one at a time, in order, like the
//! settings queue, so a Save never races an auto-save of the same view and a refresh of the tree sees the writes
//! before it. A job's body receives the view's auto-saved slot as it is when the job starts.

use std::collections::{HashMap, VecDeque};

use hikasa_core::{JobKind, Warning};
use hikasa_session::api::{
    ScopeWritesDto, SessionEdit, SessionListDto, SessionTarget, SessionView, ViewRequest,
    WorkspaceListDto, WorkspaceLoadDto,
};
use hikasa_session::ipc::{CloseReason, ViewId, ViewInfo, ViewKind};
use hikasa_session::log::LogKind;
use hikasa_session::sessions::{
    AutoId, CloseAction, NamedEntry, OpaqueTable, ParentLink, Session, SessionName, SessionRef,
    SessionType, apply_folder_display, apply_text_display, folder_display_table, is_dirty,
    layout_table, parse_folder, text_display_table,
};
use hikasa_session::settings::Shortcut;
use hikasa_session::settings::state::{LayoutValue, MruList};
use hikasa_session::workspaces::Workspace;
use hikasa_session::{ApiError, Service};

use super::{AppCore, CoreJob, Effect, OpenTarget};
use crate::commands::{CommandId, Scope};
use crate::dialog::{ButtonRole, Followup, Prompt};
use crate::home::{HomeModel, PanelMode, PanelTarget, RenameEdit, TreeNode};
use crate::job::Job;
use crate::notify::{Notice, NoticeKind};
use crate::session_dialog::{
    EditTarget, NameDialog, NamePurpose, SaveSessionDialog, SaveSource, SessionAction,
    SessionSettingsDialog, SettingsResult, TreeTarget,
};
use crate::status::Message;
use crate::view_session::{
    AUTOSAVE_PERIOD, ViewCapture, ViewOrigin, ViewSession, merged_with_file, same_autosaved,
    same_session,
};
use crate::window::{CommandArg, Invocation, Tab, TabContent, ViewModel, WindowId};
use crate::workspace::{
    CapturedTab, CapturedWindow, ManageWorkspacesDialog, SaveWorkspaceDialog, pending_tab,
    register_tabs,
};

/// What the sessions tree and the menus show, read by a store job.
#[derive(Debug, Clone, PartialEq)]
pub struct Catalog {
    /// The sessions tree.
    pub sessions: SessionListDto,
    /// The workspaces.
    pub workspaces: WorkspaceListDto,
    /// The formats: id, name.
    pub formats: Vec<(String, String)>,
}

impl Default for Catalog {
    fn default() -> Catalog {
        Catalog {
            sessions: SessionListDto {
                folders: Vec::new(),
                sessions: Vec::new(),
                defaults: Vec::new(),
                autosaved: Vec::new(),
                warnings: Vec::new(),
            },
            workspaces: WorkspaceListDto {
                workspaces: Vec::new(),
                warnings: Vec::new(),
            },
            formats: Vec::new(),
        }
    }
}

/// A session ready to open.
#[derive(Debug, Clone)]
pub struct OpenedSession {
    /// The view of the session.
    pub view: SessionView,
    /// Where it came from.
    pub origin: ViewOrigin,
    /// The session as loaded.
    pub loaded: Option<Session>,
    /// The file is locked.
    pub locked: bool,
    /// The session (`view`, `display`, the parent link, the description).
    pub session: Session,
    /// The tab caption (`None`: the input names).
    pub title: Option<String>,
    /// The view starts dirty (Home opened it with other specs than the file's).
    pub dirty: bool,
    /// The named file as read.
    pub file: Option<Session>,
    /// The auto-saved file is locked.
    pub slot_locked: bool,
}

/// A named session (D-120): `loaded` = the effective session, `locked` from the file.
pub fn load_named(service: &Service, reference: &SessionRef) -> Result<OpenedSession, ApiError> {
    let file = service.session_load(reference)?.value;
    let actual = service
        .session_store()
        .resolve(reference)
        .unwrap_or_else(|| reference.clone());
    let session = service.session_effective(&actual)?;
    let view = service.session_view_of(&session, actual.to_string())?;
    Ok(OpenedSession {
        view,
        title: Some(actual.to_string()),
        origin: ViewOrigin::Named(actual),
        loaded: Some(session.clone()),
        locked: file.locked,
        session,
        dirty: false,
        file: Some(file),
        slot_locked: false,
    })
}

/// An auto-saved session (its slot is the view's, D-83) or, with `as_new`, a copy of it.
fn load_autosaved(service: &Service, id: &AutoId, as_new: bool) -> Result<OpenedSession, ApiError> {
    let mut session = service.session_load_autosaved(id)?.value;
    service.session_resolve_parent(&mut session)?;
    let view = service.session_view_of(&session, id.to_string())?;
    // A locked file cannot take the view's changes: they are compared with what it holds (the close asks).
    let slot_locked = session.locked && !as_new;
    Ok(OpenedSession {
        view,
        origin: if as_new {
            ViewOrigin::Unnamed
        } else {
            ViewOrigin::AutoSaved(id.clone())
        },
        loaded: slot_locked.then(|| session.clone()),
        locked: false,
        session,
        title: None,
        dirty: false,
        file: None,
        slot_locked,
    })
}

/// A session value with origin `Unnamed`.
fn load_value(service: &Service, mut session: Session) -> Result<OpenedSession, ApiError> {
    session.locked = false;
    let view = service.session_view_of(&session, String::new())?;
    Ok(OpenedSession {
        view,
        origin: ViewOrigin::Unnamed,
        loaded: None,
        locked: false,
        session,
        title: None,
        dirty: false,
        file: None,
        slot_locked: false,
    })
}

/// Where a job's opened session goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenHow {
    /// A new view at a target.
    Target(OpenTarget),
    /// A pending tab turns into its view.
    Pending(ViewId),
    /// The view's tab takes it (`SessionsCombo`).
    ReplaceView(ViewId),
}

/// What a store job returns.
pub enum StoreDone {
    /// The tree and the menus.
    Catalog(Box<Catalog>),
    /// Files changed; the tree refreshes.
    Written,
    /// Nothing changed.
    Nothing,
    /// An auto-save ended.
    AutoSaved {
        /// The view.
        view: ViewId,
        /// Its slot (`None`: off).
        slot: Option<AutoId>,
        /// The session saved (`None`: nothing was written).
        saved: Option<Box<Session>>,
        /// The session differs but the file is locked (D-84): not written.
        blocked: bool,
    },
    /// A view's session was saved.
    Saved {
        /// The view.
        view: ViewId,
        /// The file.
        reference: SessionRef,
        /// The session.
        session: Box<Session>,
        /// The file as it is now.
        file: Box<Session>,
        /// The view's slot was deleted.
        slot_gone: bool,
        /// What went wrong after the save.
        note: Option<String>,
    },
    /// Named sessions were renamed or moved: the open views and pending tabs follow.
    Moved(Vec<(SessionRef, SessionRef)>),
    /// Named sessions were deleted: their views become unnamed.
    Deleted(Vec<SessionRef>),
    /// A job found that the user must be asked.
    Ask(WindowId, Box<Prompt>),
    /// The workspace of the startup does not exist (D-125: a warning and Home).
    StartupMissing(String),
    /// A view's lock changed.
    Locked(ViewId, bool),
    /// A session is ready to open.
    Opened(Box<OpenedSession>, OpenHow),
    /// A pending tab or a `SessionsCombo` could not be opened.
    OpenFailed(ViewId, String),
    /// A child view (FC-062) is ready.
    Child(Box<OpenedSession>, OpenTarget),
    /// A workspace was read; `then` loads it.
    Workspace(Box<WorkspaceLoadDto>),
    /// The Session Settings dialog can open.
    Dialog(WindowId, Box<SessionSettingsDialog>),
    /// A view takes changed specs or rules (§1.4.10).
    Reopen {
        /// The view.
        view: ViewId,
        /// Its request.
        opened: Box<SessionView>,
        /// The session now.
        session: Box<Session>,
        /// `is_dirty(loaded, session)`.
        dirty: bool,
    },
    /// The panel of a Home tab takes a session.
    Panel(ViewId, PanelTarget, Option<Box<Session>>),
    /// The panel's copy becomes this session (Factory Defaults).
    PanelFill(ViewId, Box<Session>),
}

type StoreBody = Box<dyn FnOnce(&Service, Option<AutoId>) -> Result<StoreDone, ApiError> + Send>;

/// What a store job is to the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// Any job.
    Plain,
    /// An auto-save: dropped when it starts if its view became named meanwhile.
    AutoSave,
    /// A write a close waits for.
    Close,
}

/// A store job waiting for its turn.
pub struct StoreWork {
    title: String,
    /// The view whose slot the body receives; its error also goes to the view.
    view: Option<ViewId>,
    kind: StoreKind,
    body: StoreBody,
}

/// What happens after a close flow ended.
pub enum Then {
    /// Close these tabs; `user`: the last tab may close its window.
    Tabs {
        /// The window.
        window: WindowId,
        /// The tabs.
        views: Vec<ViewId>,
        /// A user's close.
        user: bool,
    },
    /// Close a window.
    Window(WindowId),
    /// Quit.
    Exit,
    /// `SessionHome`.
    Home(ViewId),
    /// `SessionsCombo`.
    Session(ViewId, SessionRef),
    /// Load a workspace; `ids` when the IPC handler registered its tabs.
    Workspace(Box<WorkspaceLoadDto>, Option<Vec<Vec<Option<ViewId>>>>),
}

/// The decision about a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decided {
    Nothing,
    AutoSave,
    ViewState,
    Save {
        force: bool,
    },
    /// Yes for a view whose auto-saved file is locked: a new auto-saved file takes the changes.
    SaveCopy,
}

/// A close running through its views (D-84): every question is asked before anything happens, so a Cancel leaves
/// everything as it was.
pub struct CloseFlow {
    queue: VecDeque<ViewId>,
    decided: Vec<(ViewId, Decided)>,
    asking: Option<ViewId>,
    then: Then,
    /// Every view of the flow.
    views: Vec<ViewId>,
    /// Every question is answered; the writes start when no view's settings are still being applied.
    answered: bool,
    /// The saves the close waits for: it happens when they all succeeded.
    saving: Vec<ViewId>,
    /// The saves that failed.
    failed: Vec<(ViewId, String)>,
}

/// What the Home management panel can do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    /// Save the panel's session.
    Save,
    /// Save As….
    SaveAs,
    /// The panel's copy becomes the defaults (written on Save).
    FactoryDefaults,
    /// Revert.
    Cancel,
    /// Open.
    Open,
    /// Open as New.
    OpenAsNew,
    /// Delete.
    Delete,
    /// Lock or unlock.
    SetLocked(bool),
}

const CLOSE_TOPIC: &str = "main-window";

impl AppCore {
    // ---- the store queue -------------------------------------------------------------------------------------

    /// Queues a store job; it starts when none runs.
    pub(super) fn store(
        &mut self,
        title: &str,
        view: Option<ViewId>,
        body: impl FnOnce(&Service, Option<AutoId>) -> Result<StoreDone, ApiError> + Send + 'static,
    ) {
        self.store_as(title, view, StoreKind::Plain, body);
    }

    fn store_as(
        &mut self,
        title: &str,
        view: Option<ViewId>,
        kind: StoreKind,
        body: impl FnOnce(&Service, Option<AutoId>) -> Result<StoreDone, ApiError> + Send + 'static,
    ) {
        self.queued_store.push_back(StoreWork {
            title: title.to_owned(),
            view,
            kind,
            body: Box::new(body),
        });
        self.next_store_job();
    }

    /// Asks for a refresh of the tree and the menus (the last job of the queue).
    pub fn refresh_sessions(&mut self) {
        self.refresh_wanted = true;
        self.next_store_job();
    }

    /// An auto-save that must not run any more: its view became named while it was queued (D-120: a named view
    /// has no auto-saved file).
    fn autosave_is_moot(&mut self, view: ViewId) -> bool {
        if self.named_after_close.contains(&view) {
            return true;
        }
        let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) else {
            return false;
        };
        if vs.named() {
            vs.autosaving = false;
            return true;
        }
        false
    }

    /// Starts the next store job when none runs.
    pub(super) fn next_store_job(&mut self) {
        if self.store_running {
            return;
        }
        let work = loop {
            match self.queued_store.pop_front() {
                Some(work)
                    if work.kind == StoreKind::AutoSave
                        && work.view.is_some_and(|v| self.autosave_is_moot(v)) => {}
                Some(work) => break work,
                None if self.refresh_wanted => {
                    self.refresh_wanted = false;
                    break StoreWork {
                        title: "read the sessions".to_owned(),
                        view: None,
                        kind: StoreKind::Plain,
                        body: Box::new(|svc, _| {
                            let formats = svc
                                .format_list()
                                .formats
                                .into_iter()
                                .map(|f| (f.id, f.name))
                                .collect();
                            Ok(StoreDone::Catalog(Box::new(Catalog {
                                sessions: svc.session_list(None, true, true)?,
                                workspaces: svc.workspace_list(),
                                formats,
                            })))
                        }),
                    };
                }
                None => return,
            }
        };
        let slot = work.view.and_then(|v| self.slots.get(&v).cloned());
        let body = work.body;
        let job = Job::spawn(
            &self.service,
            JobKind::VfsWrite,
            work.title,
            &self.wake(),
            move |svc, _| body(svc, slot),
        );
        self.store_running = true;
        self.jobs.push(CoreJob::Store(job, work.view, work.kind));
    }

    /// The result of a store job.
    pub(super) fn store_finished(
        &mut self,
        result: Result<StoreDone, ApiError>,
        view: Option<ViewId>,
        kind: StoreKind,
    ) -> Vec<Effect> {
        let failure = result.as_ref().err().map(ToString::to_string);
        let mut effects = match result {
            Ok(done) => self.store_applied(done),
            Err(e) => {
                self.store_failed(view, &e.to_string());
                Vec::new()
            }
        };
        if let (StoreKind::Close, Some(view)) = (kind, view) {
            effects.extend(self.close_save_ended(view, failure));
        }
        if self
            .flow
            .as_ref()
            .is_some_and(|f| f.answered && f.saving.is_empty())
        {
            effects.extend(self.finish_close());
        }
        if self.queued_store.is_empty() {
            // No job can ask for a closed view's slot any more.
            let live: std::collections::HashSet<ViewId> = self.all_views().into_iter().collect();
            self.slots.retain(|v, _| live.contains(v));
            self.named_after_close.clear();
        }
        effects
    }

    fn store_applied(&mut self, done: StoreDone) -> Vec<Effect> {
        match done {
            StoreDone::Catalog(catalog) => {
                let time = self.service.clock().now();
                self.log
                    .push_warnings(&time, "sessions", &catalog.sessions.warnings);
                self.catalog = std::sync::Arc::from(catalog);
                let catalog = std::sync::Arc::clone(&self.catalog);
                for tab in self.windows.iter_mut().flat_map(|w| w.tabs.iter_mut()) {
                    if let Some(ViewModel::Home(home)) = tab.model_mut() {
                        home.load(&catalog.sessions);
                    }
                }
            }
            StoreDone::Nothing => {}
            StoreDone::Written => self.refresh_sessions(),
            StoreDone::AutoSaved {
                view,
                slot,
                saved,
                blocked,
            } => {
                if saved.is_some() {
                    match slot {
                        Some(id) => self.slots.insert(view, id),
                        None => self.slots.remove(&view),
                    };
                }
                if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                    vs.autosaving = false;
                    if blocked {
                        vs.slot_locked = true;
                        vs.dirty = true;
                    }
                    if let Some(session) = saved {
                        vs.last_autosaved = Some(*session);
                        vs.slot_locked = false;
                        if !vs.named() {
                            vs.dirty = false;
                        }
                    }
                }
                self.refresh_sessions();
            }
            StoreDone::Saved {
                view,
                reference,
                session,
                file,
                slot_gone,
                note,
            } => {
                if slot_gone {
                    self.slots.remove(&view);
                }
                if let Some(tab) = self.tab_mut(view) {
                    if let Some(vs) = tab.session.as_mut() {
                        vs.origin = ViewOrigin::Named(reference.clone());
                        vs.loaded = Some(*session);
                        vs.locked = file.locked;
                        vs.seen = Some(*file);
                        vs.dirty = false;
                        vs.explicit_save = false;
                        vs.slot_locked = false;
                        tab.title = Some(reference.to_string());
                        let caption = tab.caption();
                        self.shared.set_title(view, caption);
                    }
                } else {
                    self.named_after_close.insert(view);
                }
                if let Some(text) = note {
                    let window = self.focused();
                    self.notify(
                        window,
                        Notice::new(format!("session:{text}"), NoticeKind::Warning, text),
                        "sessions",
                        LogKind::Warning,
                    );
                }
                self.refresh_sessions();
            }
            StoreDone::Moved(moved) => {
                self.sessions_moved(&moved);
                self.refresh_sessions();
            }
            StoreDone::Deleted(gone) => {
                self.sessions_deleted(&gone);
                self.refresh_sessions();
            }
            StoreDone::Ask(window, prompt) => {
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(prompt);
                }
            }
            StoreDone::StartupMissing(name) => {
                let window = self.focused();
                let text = format!("The startup workspace `{name}` does not exist.");
                self.notify(
                    window,
                    Notice::new("workspace:startup", NoticeKind::Warning, text),
                    "workspace",
                    LogKind::Warning,
                );
            }
            StoreDone::Locked(view, locked) => {
                if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                    vs.locked = locked;
                }
                self.refresh_sessions();
            }
            StoreDone::Opened(opened, how) => return self.finish_open(*opened, how),
            StoreDone::Child(opened, target) => {
                let before = self.window_ids();
                self.finish_open(*opened, OpenHow::Target(target));
                return self.new_viewports(&before);
            }
            StoreDone::OpenFailed(view, text) => self.open_failed(view, text),
            StoreDone::Workspace(load) => {
                let views = self.all_views();
                return self.start_close(views, Then::Workspace(load, None));
            }
            StoreDone::Dialog(window, dialog) => {
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(dialog);
                }
            }
            StoreDone::Reopen {
                view,
                opened,
                session,
                dirty,
            } => self.reopen(view, *opened, *session, dirty),
            StoreDone::Panel(view, target, session) => {
                if let Some(ViewModel::Home(home)) = self.tab_mut(view).and_then(|t| t.model_mut())
                {
                    home.panel.show(target, session.map(|s| *s));
                }
            }
            StoreDone::PanelFill(view, session) => {
                if let Some(ViewModel::Home(home)) = self.tab_mut(view).and_then(|t| t.model_mut())
                {
                    home.panel.fill(Some(*session));
                }
            }
        }
        Vec::new()
    }

    fn store_failed(&mut self, view: Option<ViewId>, text: &str) {
        if let Some(tab) = view.and_then(|v| self.tab_mut(v)) {
            if let Some(vs) = tab.session.as_mut() {
                vs.autosaving = false;
                vs.settings_pending = false;
            }
            match tab.model_mut() {
                Some(ViewModel::Home(home)) => {
                    home.panel.loading = false;
                    home.error = Some(text.to_owned());
                }
                _ => tab.message = Some(Message::error(text.to_owned())),
            }
        }
        let window = self.focused();
        self.notify(
            window,
            Notice::new(
                format!("session:{text}"),
                NoticeKind::Error,
                text.to_owned(),
            ),
            "sessions",
            LogKind::Error,
        );
    }

    /// Open views follow a rename (case-folded): their origin, parent link and caption, a pending tab's reference.
    fn sessions_moved(&mut self, moved: &[(SessionRef, SessionRef)]) {
        let map: HashMap<String, &SessionRef> = moved
            .iter()
            .map(|(from, to)| (from.to_string().to_lowercase(), to))
            .collect();
        let to_of = |r: &SessionRef| map.get(&r.to_string().to_lowercase()).copied();
        let mut captions = Vec::new();
        for tab in self.windows.iter_mut().flat_map(|w| w.tabs.iter_mut()) {
            let mut changed = false;
            if let Some(vs) = tab.session.as_mut() {
                if let Some(to) = vs.reference().and_then(to_of) {
                    vs.origin = ViewOrigin::Named(to.clone());
                    tab.title = Some(to.to_string());
                    changed = true;
                }
                if let Some(link) = vs.parent.as_mut() {
                    let target = SessionRef::parse(&link.session).ok();
                    if let Some(to) = target.as_ref().and_then(to_of) {
                        link.session = to.to_string();
                    }
                }
            }
            if let TabContent::Pending(p) = &mut tab.content
                && let Some(to) = p.reference.as_ref().and_then(to_of)
            {
                p.reference = Some(to.clone());
                p.caption = to.to_string();
                changed = true;
            }
            if changed {
                captions.push((tab.id, tab.caption()));
            }
        }
        for (id, caption) in captions {
            self.shared.set_title(id, caption);
        }
    }

    /// Views of deleted sessions lose their file: origin `Unnamed`, so their close auto-saves (D-84).
    fn sessions_deleted(&mut self, gone: &[SessionRef]) {
        let gone: std::collections::HashSet<String> =
            gone.iter().map(|r| r.to_string().to_lowercase()).collect();
        let hit = |r: &SessionRef| gone.contains(&r.to_string().to_lowercase());
        let mut captions = Vec::new();
        for tab in self.windows.iter_mut().flat_map(|w| w.tabs.iter_mut()) {
            let mut changed = false;
            if let Some(vs) = tab.session.as_mut()
                && vs.reference().is_some_and(hit)
            {
                vs.origin = ViewOrigin::Unnamed;
                vs.loaded = None;
                vs.seen = None;
                vs.locked = false;
                vs.dirty = false;
                vs.explicit_save = false;
                tab.title = None;
                changed = true;
            }
            if let TabContent::Pending(p) = &mut tab.content
                && p.reference.as_ref().is_some_and(hit)
            {
                p.reference = None;
                p.caption = hikasa_session::sessions::label(&p.session);
                changed = true;
            }
            if changed {
                captions.push((tab.id, tab.caption()));
            }
        }
        for (id, caption) in captions {
            self.shared.set_title(id, caption);
        }
    }

    // ---- what the views hold ---------------------------------------------------------------------------------

    /// The session of a view, as the window knows it.
    pub fn view_session(&self, view: ViewId) -> Option<&ViewSession> {
        self.tab(view).and_then(|t| t.session.as_ref())
    }

    /// The auto-saved file of a view.
    #[cfg(test)]
    pub fn view_slot(&self, view: ViewId) -> Option<&AutoId> {
        self.slots.get(&view)
    }

    /// The workspaces, by name.
    pub fn workspace_names(&self) -> Vec<String> {
        self.catalog
            .workspaces
            .workspaces
            .iter()
            .map(|w| w.name.clone())
            .collect()
    }

    /// The named sessions of a view type (`SessionsCombo`).
    pub fn sessions_of(&self, scope: Scope) -> Vec<String> {
        let Some(view) = scope.view_name() else {
            return Vec::new();
        };
        self.catalog
            .sessions
            .sessions
            .iter()
            .filter(|s| s.kind == view)
            .map(|s| s.reference.clone())
            .collect()
    }

    fn all_views(&self) -> Vec<ViewId> {
        self.windows
            .iter()
            .flat_map(|w| w.tabs.iter().map(|t| t.id))
            .collect()
    }

    /// The window's `view` and `display` tables of a tab.
    fn tables_of(tab: &Tab) -> (OpaqueTable, OpaqueTable) {
        let mut keys: Vec<(&str, LayoutValue)> = vec![("toolbar", LayoutValue::Bool(tab.toolbar))];
        let display = match tab.model() {
            Some(ViewModel::Text(m)) => {
                keys.extend(m.layout_keys());
                text_display_table(m.view_options())
            }
            Some(ViewModel::Folder(m)) => {
                keys.extend(m.layout_keys());
                folder_display_table(m.display_options())
            }
            _ => OpaqueTable::new(),
        };
        (layout_table(&keys), display)
    }

    /// What a job needs to build the current session of a tab (D-105); `None` for Home and pending tabs.
    fn capture(tab: &Tab) -> Option<ViewCapture> {
        let request = match tab.model()? {
            ViewModel::Text(m) => m.duplicate().0,
            ViewModel::Folder(m) => m.duplicate().0,
            ViewModel::Home(_) => return None,
        };
        let (view, display) = AppCore::tables_of(tab);
        let vs = tab.session.as_ref();
        Some(ViewCapture {
            request,
            view,
            display,
            parent: vs.and_then(|s| s.parent.clone()),
            description: vs.map(|s| s.description.clone()).unwrap_or_default(),
        })
    }

    fn auto_save_on(&self) -> bool {
        self.options().tweaks.metrics.max_auto_saved_sessions > 0
    }

    // ---- close flow ------------------------------------------------------------------------------------------

    /// Starts a close: each view's close action in turn; a question stops until it is answered; a Cancel ends the
    /// flow with nothing closed. Without a question everything happens now.
    pub(super) fn start_close(&mut self, views: Vec<ViewId>, then: Then) -> Vec<Effect> {
        if self.flow.is_some() {
            // One close at a time: a second one waits for the user's next try; a workspace says so.
            if let Then::Workspace(load, ids) = then {
                if let Some(ids) = ids {
                    self.cancel_registered(ids);
                }
                let window = self.focused();
                let text = format!(
                    "The workspace {} was not loaded: a close is still running.",
                    load.name
                );
                self.notify(
                    window,
                    Notice::new("workspace:busy", NoticeKind::Warning, text),
                    "workspace",
                    LogKind::Warning,
                );
            }
            return Vec::new();
        }
        if matches!(then, Then::Exit) {
            self.exit_workspace_save();
        }
        self.flow = Some(CloseFlow {
            views: views.clone(),
            queue: views.into(),
            decided: Vec::new(),
            asking: None,
            then,
            answered: false,
            saving: Vec::new(),
            failed: Vec::new(),
        });
        self.advance_close()
    }

    /// The top dialog of a window went (its OS close button): a close asking there ends as a Cancel.
    pub(super) fn cancel_flow(&mut self, window: WindowId) -> Vec<Effect> {
        let asking = self.flow.as_ref().and_then(|f| f.asking);
        if asking.is_some_and(|v| self.window_of(v) == Some(window)) {
            return self.close_answer(ButtonRole::Cancel);
        }
        Vec::new()
    }

    fn cancel_registered(&mut self, ids: Vec<Vec<Option<ViewId>>>) {
        for view in ids.into_iter().flatten().flatten() {
            let (result, code) =
                hikasa_session::api::ViewOutcome::close_unopened(CloseReason::User);
            self.shared.close(view, result, code);
        }
    }

    fn advance_close(&mut self) -> Vec<Effect> {
        let auto = self.auto_save_on();
        loop {
            let next = self.flow.as_mut().and_then(|f| f.queue.pop_front());
            let Some(view) = next else {
                break;
            };
            let Some(tab) = self.tab(view) else {
                continue;
            };
            let Some(vs) = &tab.session else {
                continue;
            };
            if vs.decided {
                continue;
            }
            let action = vs.close_action(auto);
            let decided = match action {
                CloseAction::Nothing => Decided::Nothing,
                CloseAction::AutoSave => Decided::AutoSave,
                CloseAction::SaveViewState => Decided::ViewState,
                CloseAction::AskSaveChanges | CloseAction::AskSaveLocked => {
                    return self.ask_close(view, action);
                }
            };
            if let Some(flow) = self.flow.as_mut() {
                flow.decided.push((view, decided));
            }
        }
        self.finish_close()
    }

    fn ask_close(&mut self, view: ViewId, action: CloseAction) -> Vec<Effect> {
        let Some(window) = self.window_of(view) else {
            return Vec::new();
        };
        if let Some(index) = self.window(window).and_then(|w| w.index_of(view)) {
            self.activate(window, index);
        }
        let name = self.tab(view).map(Tab::caption).unwrap_or_default();
        let answer = |role| vec![Followup::Session(SessionAction::CloseAnswer(role))];
        let prompt = if action == CloseAction::AskSaveLocked {
            Prompt::ask(
                name,
                "Save changes to locked session?",
                ButtonRole::Yes,
                vec![
                    (ButtonRole::Yes, "Yes", answer(ButtonRole::Yes)),
                    (ButtonRole::No, "No", answer(ButtonRole::No)),
                ],
                CLOSE_TOPIC,
            )
        } else {
            Prompt::ask(
                name,
                "Save changes to session?",
                ButtonRole::Yes,
                vec![
                    (ButtonRole::Yes, "Yes", answer(ButtonRole::Yes)),
                    (ButtonRole::No, "No", answer(ButtonRole::No)),
                    (ButtonRole::Cancel, "Cancel", answer(ButtonRole::Cancel)),
                ],
                CLOSE_TOPIC,
            )
        };
        if let Some(flow) = self.flow.as_mut() {
            flow.asking = Some(view);
        }
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(prompt));
        }
        // The prompt may be in a window the user is not looking at: an Exit must not look hung.
        self.focus(window);
        vec![Effect::Raise(window)]
    }

    /// A flow ended without its close: nothing more happens, the views go on (a later close asks again).
    fn end_flow(&mut self, then: Then) {
        match then {
            Then::Exit => self.exit_saved = false,
            Then::Workspace(_, Some(ids)) => self.cancel_registered(ids),
            _ => {}
        }
    }

    fn close_answer(&mut self, role: ButtonRole) -> Vec<Effect> {
        if role == ButtonRole::Cancel {
            if let Some(flow) = self.flow.take() {
                self.end_flow(flow.then);
            }
            return Vec::new();
        }
        let asking = self.flow.as_mut().and_then(|f| f.asking.take());
        if let Some(view) = asking {
            let session = self.view_session(view);
            let locked = session.is_some_and(|v| v.locked);
            let named = session.is_some_and(ViewSession::named);
            let decided = match (role, named) {
                (ButtonRole::Yes, true) => Decided::Save { force: locked },
                (ButtonRole::Yes, false) => Decided::SaveCopy,
                _ => Decided::Nothing,
            };
            if let Some(flow) = self.flow.as_mut() {
                flow.decided.push((view, decided));
            }
        }
        self.advance_close()
    }

    /// Every question was answered: the writes are queued. The close happens now, or, when the user asked for
    /// a save, once every save succeeded: a save that fails keeps its tab, so "keep my changes" is never a silent
    /// loss. While a view's settings are being applied everything waits (`dirty` and the capture are not final).
    fn finish_close(&mut self) -> Vec<Effect> {
        let Some(flow) = self.flow.as_mut() else {
            return Vec::new();
        };
        flow.answered = true;
        let decided = std::mem::take(&mut flow.decided);
        let waits = decided
            .iter()
            .any(|(v, _)| self.view_session(*v).is_some_and(|s| s.settings_pending));
        if waits {
            if let Some(flow) = self.flow.as_mut() {
                flow.decided = decided;
            }
            return Vec::new();
        }
        let mut saving = Vec::new();
        for (view, decided) in decided {
            if self.run_decision(view, decided) {
                saving.push(view);
            }
        }
        if saving.is_empty() {
            return match self.flow.take() {
                Some(flow) => self.run_then(flow.then),
                None => Vec::new(),
            };
        }
        if let Some(flow) = self.flow.as_mut() {
            flow.saving = saving;
        }
        Vec::new()
    }

    /// A save of the close ended. The last one decides: the close happens, or it ends as a Cancel and the tab
    /// that could not be saved stays, with the reason in a prompt.
    fn close_save_ended(&mut self, view: ViewId, failure: Option<String>) -> Vec<Effect> {
        let Some(flow) = self.flow.as_mut() else {
            return Vec::new();
        };
        flow.saving.retain(|v| *v != view);
        if let Some(text) = failure {
            flow.failed.push((view, text));
        }
        if !flow.saving.is_empty() {
            return Vec::new();
        }
        let Some(flow) = self.flow.take() else {
            return Vec::new();
        };
        if flow.failed.is_empty() {
            return self.run_then(flow.then);
        }
        for view in &flow.views {
            if let Some(vs) = self.tab_mut(*view).and_then(|t| t.session.as_mut()) {
                vs.decided = false;
            }
        }
        self.end_flow(flow.then);
        let mut effects = Vec::new();
        for (view, text) in flow.failed {
            let Some(window) = self.window_of(view) else {
                continue;
            };
            if let Some(index) = self.window(window).and_then(|w| w.index_of(view)) {
                self.activate(window, index);
            }
            let prompt = Prompt::ask(
                "Save Session",
                format!(
                    "The changes could not be saved, so the tab stays open: {text}\nUse Save Session As to keep them."
                ),
                ButtonRole::Ok,
                vec![(ButtonRole::Ok, "OK", Vec::new())],
                CLOSE_TOPIC,
            );
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(prompt));
            }
            self.focus(window);
            effects.push(Effect::Raise(window));
        }
        effects
    }

    /// Runs a decision; true when the close waits for its result.
    fn run_decision(&mut self, view: ViewId, decided: Decided) -> bool {
        let Some(tab) = self.tab_mut(view) else {
            return false;
        };
        let Some(vs) = tab.session.as_mut() else {
            return false;
        };
        vs.decided = true;
        let vs = vs.clone();
        let capture = self.tab(view).and_then(AppCore::capture);
        let Some(capture) = capture else {
            return false;
        };
        self.run_decided(view, &vs, capture, decided)
    }

    fn run_decided(
        &mut self,
        view: ViewId,
        vs: &ViewSession,
        capture: ViewCapture,
        d: Decided,
    ) -> bool {
        match d {
            Decided::Nothing => false,
            Decided::AutoSave => {
                self.queue_autosave(view, capture, None);
                false
            }
            Decided::ViewState => {
                if let Some(reference) = vs.reference().cloned() {
                    let seen = vs.seen.clone();
                    self.store("save the view state", None, move |svc, _| {
                        let (current, _) = capture.session(svc)?;
                        // A file that went or was replaced meanwhile is not this view's to touch.
                        let mut file = match svc.session_load(&reference) {
                            Ok(file) => file.value,
                            Err(ApiError::NotFound(_)) => return Ok(StoreDone::Nothing),
                            Err(e) => return Err(e),
                        };
                        if file.locked
                            || (file.view == current.view && file.display == current.display)
                            || seen.is_some_and(|s| !same_session(&s, &file))
                        {
                            return Ok(StoreDone::Nothing);
                        }
                        file.view = current.view;
                        file.display = current.display;
                        svc.session_save(&reference, &file, false)?;
                        Ok(StoreDone::Written)
                    });
                }
                false
            }
            Decided::Save { force } => match vs.reference().cloned() {
                Some(reference) => {
                    let seen = vs.seen.clone();
                    self.queue_save(view, reference, capture, force, seen, StoreKind::Close);
                    true
                }
                None => false,
            },
            Decided::SaveCopy => {
                self.store_as(
                    "save a copy",
                    Some(view),
                    StoreKind::Close,
                    move |svc, _| {
                        let (current, _) = capture.session(svc)?;
                        let slot = svc.session_autosave(None, &current)?;
                        Ok(StoreDone::AutoSaved {
                            view,
                            slot,
                            saved: Some(Box::new(current)),
                            blocked: false,
                        })
                    },
                );
                true
            }
        }
    }

    /// A close that cannot ask (D-120): every question is answered No. Called for each view that goes without
    /// having been through a flow (IPC `CloseView`, `Shutdown`, the end of a desktop session).
    pub(super) fn decide_silently(&mut self, tab: &mut Tab) {
        let Some(vs) = tab.session.as_mut() else {
            return;
        };
        if vs.decided {
            return;
        }
        vs.decided = true;
        let vs = vs.clone();
        let decided = match vs.close_action(self.auto_save_on()) {
            CloseAction::AutoSave => Decided::AutoSave,
            CloseAction::SaveViewState => Decided::ViewState,
            _ => Decided::Nothing,
        };
        if let Some(capture) = AppCore::capture(tab) {
            self.run_decided(tab.id, &vs, capture, decided);
        }
    }

    fn run_then(&mut self, then: Then) -> Vec<Effect> {
        match then {
            Then::Tabs {
                window,
                views,
                user,
            } => {
                let mut effects = Vec::new();
                for view in views {
                    let index = self.window(window).and_then(|w| w.index_of(view));
                    if let Some(index) = index {
                        effects.extend(self.remove_tab(window, index, CloseReason::User, user));
                    }
                }
                effects
            }
            Then::Window(window) => self.close_window_now(window),
            Then::Exit => self.shutdown(),
            Then::Home(view) => {
                self.replace_by_home(view);
                Vec::new()
            }
            Then::Session(view, reference) => {
                self.open_named(reference, OpenHow::ReplaceView(view));
                Vec::new()
            }
            Then::Workspace(load, ids) => self.restore_workspace(*load, ids),
        }
    }

    /// `SessionHome`: a Home tab in the view's place; returns it.
    fn replace_by_home(&mut self, view: ViewId) -> Option<ViewId> {
        let window = self.window_of(view)?;
        let index = self.window(window).and_then(|w| w.index_of(view))?;
        let home = self.new_home_model();
        let id = self.shared.allocate();
        let tab = Tab::live(id, ViewModel::Home(Box::new(home)), true);
        let mut old = {
            let w = self.window_mut(window).expect("the window exists");
            std::mem::replace(&mut w.tabs[index], tab)
        };
        self.remember_layout(&old);
        self.close_model(&mut old, CloseReason::User);
        self.refresh_sessions();
        Some(id)
    }

    /// A Home model filled from the catalog.
    pub(super) fn new_home_model(&self) -> HomeModel {
        let mut home = HomeModel::default();
        home.load(&self.catalog.sessions);
        home
    }

    // ---- saving ----------------------------------------------------------------------------------------------

    fn queue_autosave(&mut self, view: ViewId, capture: ViewCapture, last: Option<Session>) {
        self.store_as(
            "auto-save",
            Some(view),
            StoreKind::AutoSave,
            move |svc, slot| {
                let (current, _) = capture.session(svc)?;
                if last.as_ref().is_some_and(|l| same_autosaved(l, &current)) {
                    return Ok(StoreDone::AutoSaved {
                        view,
                        slot,
                        saved: None,
                        blocked: false,
                    });
                }
                // The file's identity stays (a child's Type scope writes overrides into its parent's slot).
                let file = slot
                    .as_ref()
                    .and_then(|id| svc.session_load_autosaved(id).ok())
                    .map(|l| l.value);
                if file.as_ref().is_some_and(|f| f.locked) {
                    return Ok(StoreDone::AutoSaved {
                        view,
                        slot,
                        saved: None,
                        blocked: true,
                    });
                }
                let merged = match &file {
                    Some(file) => merged_with_file(current.clone(), file),
                    None => current.clone(),
                };
                let slot = svc.session_autosave(slot.as_ref(), &merged)?;
                Ok(StoreDone::AutoSaved {
                    view,
                    slot,
                    saved: Some(Box::new(current)),
                    blocked: false,
                })
            },
        );
    }

    fn queue_save(
        &mut self,
        view: ViewId,
        reference: SessionRef,
        capture: ViewCapture,
        force: bool,
        seen: Option<Session>,
        kind: StoreKind,
    ) {
        self.store_as("save the session", Some(view), kind, move |svc, _| {
            let (current, _) = capture.session(svc)?;
            let file = svc.session_load(&reference)?.value;
            if seen.is_some_and(|s| !same_session(&s, &file)) {
                return Err(ApiError::InvalidData(format!(
                    "the session `{reference}` is no longer the one this view loaded"
                )));
            }
            let merged = merged_with_file(current.clone(), &file);
            svc.session_save(&reference, &merged, force)?;
            let file = svc.session_load(&reference).map_or(merged, |l| l.value);
            Ok(StoreDone::Saved {
                view,
                reference,
                session: Box::new(current),
                file: Box::new(file),
                slot_gone: false,
                note: None,
            })
        });
    }

    /// Seconds until the timer wants to look at a view (`None`: no view waits): an idle window schedules a frame
    /// for it.
    pub fn autosave_wake_in(&self) -> Option<f64> {
        if !self.auto_save_on() {
            return None;
        }
        self.windows
            .iter()
            .flat_map(|w| &w.tabs)
            .filter_map(|t| t.session.as_ref())
            .filter(|v| !v.named() && !v.autosaving && !v.decided)
            .map(|v| v.next_autosave_at - self.timer_now)
            .reduce(f64::min)
    }

    /// The timer (D-121): a view of origin `Unnamed` or `AutoSaved` whose period passed is looked at by a job,
    /// which saves it when its session differs from the last auto-saved one.
    pub(super) fn autosave_tick(&mut self) {
        let (now, auto) = (self.timer_now, self.auto_save_on());
        let due: Vec<ViewId> = self
            .windows
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.session.as_ref().is_some_and(|v| v.timer_due(now, auto)))
            .map(|t| t.id)
            .collect();
        for view in due {
            let capture = self.tab(view).and_then(AppCore::capture);
            let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) else {
                continue;
            };
            vs.next_autosave_at = now + AUTOSAVE_PERIOD;
            let last = vs.last_autosaved.clone();
            vs.autosaving = capture.is_some();
            if let Some(capture) = capture {
                self.queue_autosave(view, capture, last);
            }
        }
    }

    fn command_save(&mut self, window: WindowId) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let view = tab.id;
        let Some(vs) = tab.session.clone() else {
            return;
        };
        match (vs.reference().cloned(), vs.locked) {
            (Some(reference), false) => {
                if let Some(capture) = self.tab(view).and_then(AppCore::capture) {
                    self.queue_save(view, reference, capture, false, vs.seen, StoreKind::Plain);
                }
            }
            (Some(reference), true) => {
                if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                    vs.explicit_save = true;
                }
                let prompt = Prompt::ask(
                    reference.to_string(),
                    "Save changes to locked session?",
                    ButtonRole::Yes,
                    vec![
                        (
                            ButtonRole::Yes,
                            "Yes",
                            vec![Followup::Session(SessionAction::SaveLocked(view))],
                        ),
                        (ButtonRole::No, "No", Vec::new()),
                    ],
                    "dialog-save-session",
                );
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            (None, _) => self.open_save_as(window, SaveSource::View(view)),
        }
    }

    fn open_save_as(&mut self, window: WindowId, source: SaveSource) {
        let list = &self.catalog.sessions;
        let dialog = SaveSessionDialog::new(
            source,
            list.folders.clone(),
            list.sessions.iter().map(|s| s.reference.clone()).collect(),
        );
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
    }

    fn exists(&self, reference: &SessionRef) -> bool {
        let wanted = reference.to_string().to_lowercase();
        self.catalog
            .sessions
            .sessions
            .iter()
            .any(|s| s.reference.to_lowercase() == wanted)
    }

    fn save_as(
        &mut self,
        window: WindowId,
        source: SaveSource,
        reference: SessionRef,
        confirmed: bool,
    ) {
        if !confirmed && self.exists(&reference) {
            let action = SessionAction::SaveAs {
                source,
                reference: reference.clone(),
                confirmed: true,
            };
            let prompt = Prompt::ask(
                "Save Session As",
                format!("The session {reference} exists. Replace it?"),
                ButtonRole::Cancel,
                vec![
                    (ButtonRole::Yes, "Replace", vec![Followup::Session(action)]),
                    (ButtonRole::Cancel, "Cancel", Vec::new()),
                ],
                "dialog-save-session",
            );
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(prompt));
            }
            return;
        }
        match source {
            SaveSource::View(view) => {
                let Some(capture) = self.tab(view).and_then(AppCore::capture) else {
                    return;
                };
                self.store("save the session", Some(view), move |svc, slot| {
                    let reference = spelled(svc, reference);
                    let (current, _) = capture.session(svc)?;
                    let merged = match svc.session_load(&reference) {
                        Ok(file) => merged_with_file(current.clone(), &file.value),
                        Err(_) => current.clone(),
                    };
                    svc.session_save(&reference, &merged, false)?;
                    // The session is saved whatever happens to the auto-saved file: the view is named now.
                    let note = slot.as_ref().and_then(|id| {
                        svc.session_delete_autosaved(id)
                            .err()
                            .map(|e| format!("The auto-saved session {id} stays: {e}"))
                    });
                    let file = svc.session_load(&reference).map_or(merged, |l| l.value);
                    Ok(StoreDone::Saved {
                        view,
                        reference,
                        session: Box::new(current),
                        file: Box::new(file),
                        slot_gone: slot.is_some(),
                        note,
                    })
                });
            }
            SaveSource::Named(from) => {
                self.store("save the session", None, move |svc, _| {
                    let reference = spelled(svc, reference);
                    let mut session = svc.session_load(&from)?.value;
                    session.locked = false;
                    svc.session_save(&reference, &session, false)?;
                    Ok(StoreDone::Written)
                });
            }
            SaveSource::AutoSaved(id) => {
                self.store("save the session", None, move |svc, _| {
                    let reference = spelled(svc, reference);
                    let mut session = svc.session_load_autosaved(&id)?.value;
                    session.saved_at = None;
                    session.locked = false;
                    svc.session_save(&reference, &session, false)?;
                    Ok(StoreDone::Written)
                });
            }
            SaveSource::Panel(view) => {
                let Some(ViewModel::Home(home)) = self.tab(view).and_then(|t| t.model()) else {
                    return;
                };
                let Some(session) = home.panel.edited() else {
                    return;
                };
                self.store("save the session", Some(view), move |svc, _| {
                    let reference = spelled(svc, reference);
                    let mut stored = stored_specs(svc, session)?;
                    stored.locked = false;
                    svc.session_save(&reference, &stored, false)?;
                    Ok(StoreDone::Written)
                });
            }
        }
    }

    // ---- opening sessions ------------------------------------------------------------------------------------

    /// Opens a named session.
    pub fn open_named(&mut self, reference: SessionRef, how: OpenHow) {
        let target_view = match how {
            OpenHow::Pending(v) | OpenHow::ReplaceView(v) => Some(v),
            OpenHow::Target(_) => None,
        };
        self.store("open the session", None, move |svc, _| {
            match load_named(svc, &reference) {
                Ok(opened) => Ok(StoreDone::Opened(Box::new(opened), how)),
                Err(e) => match target_view {
                    Some(view) => Ok(StoreDone::OpenFailed(view, e.to_string())),
                    None => Err(e),
                },
            }
        });
    }

    /// Where an open from a Home tab goes: the tab itself (by view: the user may switch tabs meanwhile), or a new
    /// window with `open_sessions_in = "window"`.
    fn home_target(&self, window: WindowId) -> OpenHow {
        match self.active_tab(window) {
            Some(tab)
                if self.options().tabs.open_sessions_in
                    != hikasa_session::settings::options::OpenIn::Window =>
            {
                OpenHow::ReplaceView(tab.id)
            }
            _ => OpenHow::Target(OpenTarget::NewWindow),
        }
    }

    /// The state of a view from what a job read (its slot is [`AppCore::adopt_slot`]'s).
    fn view_state_of(&self, opened: &OpenedSession) -> ViewSession {
        let mut vs = ViewSession::unnamed(self.timer_now);
        vs.origin = opened.origin.clone();
        vs.loaded.clone_from(&opened.loaded);
        vs.locked = opened.locked;
        vs.dirty = opened.dirty;
        vs.parent.clone_from(&opened.session.parent);
        vs.description.clone_from(&opened.session.description);
        vs.seen.clone_from(&opened.file);
        vs.slot_locked = opened.slot_locked;
        if matches!(opened.origin, ViewOrigin::AutoSaved(_)) {
            // What the file holds: an unchanged view is not saved again.
            vs.last_autosaved = Some(opened.session.clone());
        }
        vs
    }

    /// An auto-saved view writes to the file it came from (D-83).
    fn adopt_slot(&mut self, view: ViewId, origin: &ViewOrigin) {
        if let ViewOrigin::AutoSaved(id) = origin {
            self.slots.insert(view, id.clone());
        }
    }

    /// The toolbar of a view: the session's `view` key, else the default of its type (D-108).
    fn session_toolbar(&self, session: &Session, scope: Scope) -> bool {
        session
            .view
            .get("toolbar")
            .and_then(hikasa_session::sessions::view_bool)
            .unwrap_or_else(|| self.toolbar_default(scope))
    }

    /// A session became ready: a new view, a pending tab's view, or a tab's new view.
    pub(super) fn finish_open(&mut self, mut opened: OpenedSession, how: OpenHow) -> Vec<Effect> {
        let before = self.window_ids();
        let mut inline = None;
        if let OpenHow::Pending(id) = how {
            // The tab may have closed while its session was read.
            let Some(p) = self.tab(id).and_then(Tab::pending_tab) else {
                return Vec::new();
            };
            // D-85: the workspace tab's own layout wins over the session's.
            opened
                .session
                .view
                .extend(p.view.iter().map(|(k, v)| (k.clone(), v.clone())));
            inline = (!p.inline.is_empty()).then(|| p.inline.clone());
        }
        let mut vs = self.view_state_of(&opened);
        vs.inline = inline;
        let origin = vs.origin.clone();
        let OpenedSession {
            view,
            session,
            title,
            ..
        } = opened;
        let mut request = view.request;
        match &mut request {
            ViewRequest::Text { view, .. } => apply_text_display(&session.display, view),
            ViewRequest::Folder { display, .. } => apply_folder_display(&session.display, display),
        }
        let kind = view.kind;
        let specs = view.specs;
        let model = self.build_model_with([None, None], request, &session.view);
        let toolbar = self.session_toolbar(&session, model.scope());
        match how {
            OpenHow::Pending(id) => {
                self.adopt_slot(id, &origin);
                if let Some(tab) = self.tab_mut(id) {
                    tab.content = TabContent::Live(model);
                    tab.session = Some(vs);
                    tab.title = title;
                    let caption = tab.caption();
                    self.shared.set_title(id, caption);
                    self.track_view(id);
                }
            }
            OpenHow::ReplaceView(old) => {
                let place = self
                    .window_of(old)
                    .and_then(|w| self.window(w).and_then(|m| m.index_of(old)).map(|i| (w, i)));
                let id = self.register_view(kind, specs);
                self.adopt_slot(id, &origin);
                let mut tab = Tab::live(id, model, toolbar);
                tab.session = Some(vs);
                tab.title = title;
                self.shared.set_title(id, tab.caption());
                match place {
                    Some((w, index)) => {
                        self.place_tab(tab, OpenTarget::Replace(w, index));
                    }
                    None => {
                        let target = self.open_target();
                        self.place_tab(tab, target);
                    }
                }
                self.track_view(id);
            }
            OpenHow::Target(target) => {
                let id = self.register_view(kind, specs);
                self.adopt_slot(id, &origin);
                let mut tab = Tab::live(id, model, toolbar);
                tab.session = Some(vs);
                tab.title = title;
                self.shared.set_title(id, tab.caption());
                self.place_tab(tab, target);
                self.track_view(id);
            }
        }
        self.new_viewports(&before)
    }

    /// An IPC `LoadSession` view registered by the handler (its waiter included).
    pub(super) fn open_registered(&mut self, view: ViewId, opened: OpenedSession) -> Vec<Effect> {
        let before = self.window_ids();
        let target = self.open_target();
        let window = match target {
            OpenTarget::Window(w) | OpenTarget::Replace(w, _) => w,
            OpenTarget::NewWindow => {
                let source = Some(self.focused());
                self.new_window(source)
            }
        };
        let vs = self.view_state_of(&opened);
        self.adopt_slot(view, &vs.origin);
        let OpenedSession {
            view: session_view,
            session,
            title,
            ..
        } = opened;
        let model = self.build_model_with([None, None], session_view.request, &session.view);
        let toolbar = self.session_toolbar(&session, model.scope());
        let mut tab = Tab::live(view, model, toolbar);
        tab.session = Some(vs);
        tab.title = title;
        self.shared.set_title(view, tab.caption());
        self.place_tab(tab, OpenTarget::Window(window));
        self.track_view(view);
        self.new_viewports(&before)
    }

    fn register_view(&self, kind: ViewKind, specs: Vec<String>) -> ViewId {
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
        view
    }

    fn track_view(&mut self, view: ViewId) {
        if let Some(window) = self.window_of(view) {
            let index = self.window(window).and_then(|w| w.index_of(view));
            if let Some(index) = index
                && let Some(w) = self.window_mut(window)
            {
                let tab = w.tabs.remove(index);
                self.track_inputs(&tab);
                if let Some(w) = self.window_mut(window) {
                    w.tabs.insert(index, tab);
                }
            }
        }
    }

    fn open_failed(&mut self, view: ViewId, text: String) {
        let pending = self.tab(view).is_some_and(|t| t.pending_tab().is_some());
        if pending {
            // The tab cannot start: a Home tab with the reason takes its place.
            if let Some(home) = self.replace_by_home_keeping_outcome(view) {
                self.set_message(home, Message::error(text.clone()));
            }
        } else {
            if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                vs.decided = false;
            }
            self.set_message(view, Message::error(text.clone()));
        }
        self.log_entry(LogKind::Error, "sessions", text);
    }

    fn replace_by_home_keeping_outcome(&mut self, view: ViewId) -> Option<ViewId> {
        let (result, code) = hikasa_session::api::ViewOutcome::close_unopened(CloseReason::User);
        self.shared.close(view, result, code);
        self.replace_by_home(view)
    }

    /// Starts the pending tab each window shows (D-126): only an activated tab starts its comparison.
    pub(super) fn start_active_pending(&mut self) {
        let mut start = Vec::new();
        for w in &mut self.windows {
            let Some(tab) = w.tabs.get_mut(w.active) else {
                continue;
            };
            let id = tab.id;
            if let TabContent::Pending(p) = &mut tab.content
                && !p.starting
            {
                p.starting = true;
                start.push((id, p.reference.clone(), p.session.clone()));
            }
        }
        for (view, reference, session) in start {
            match reference {
                Some(reference) => self.open_named(reference, OpenHow::Pending(view)),
                None => self.store("open the tab", None, move |svc, _| {
                    Ok(match load_value(svc, session) {
                        Ok(opened) => StoreDone::Opened(Box::new(opened), OpenHow::Pending(view)),
                        Err(e) => StoreDone::OpenFailed(view, e.to_string()),
                    })
                }),
            }
        }
    }

    /// FC-062 Open with a parent link: a child session with the effective rules (D-82).
    pub(super) fn open_child(
        &mut self,
        request: ViewRequest,
        parent: ParentLink,
        target: OpenTarget,
    ) {
        self.store("open the pair", None, move |svc, _| {
            let (mut session, _) = svc.session_for_view(
                &request,
                OpaqueTable::new(),
                OpaqueTable::new(),
                Some(parent),
            )?;
            svc.session_resolve_parent(&mut session)?;
            let mut opened = load_value(svc, session)?;
            // The display of the folder view's pair (FC-062) stays.
            if let (ViewRequest::Text { view: wanted, .. }, ViewRequest::Text { view, .. }) =
                (&request, &mut opened.view.request)
            {
                *view = wanted.clone();
            }
            Ok(StoreDone::Child(Box::new(opened), target))
        });
    }

    // ---- commands --------------------------------------------------------------------------------------------

    /// The session and tree commands (`None`: not one of them).
    pub(super) fn session_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        let mut effects = Vec::new();
        match inv.id {
            CommandId::SessionNewList => {
                self.new_home_tab(window);
                if let Some(CommandArg::SessionType(kind)) = inv.arg {
                    self.select_new(window, kind);
                }
            }
            CommandId::SessionOpen => {
                self.new_home_tab(window);
                self.refresh_sessions();
            }
            CommandId::WorkspaceLoadList => {
                if let Some(CommandArg::Workspace(name)) = inv.arg.clone() {
                    self.load_workspace(name);
                }
            }
            CommandId::WorkspaceManager => self.open_manage_workspaces(window),
            CommandId::WorkspaceSaveAs => {
                let dialog = SaveWorkspaceDialog {
                    names: self.workspace_names(),
                    ..SaveWorkspaceDialog::default()
                };
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(dialog));
                }
            }
            CommandId::SessionHome => {
                if let Some(view) = self.active_tab(window).map(|t| t.id) {
                    effects.extend(self.start_close(vec![view], Then::Home(view)));
                }
            }
            CommandId::SessionsCombo => {
                if let Some(view) = self.active_tab(window).map(|t| t.id) {
                    match inv.arg.clone() {
                        Some(CommandArg::Session(reference)) => {
                            effects.extend(
                                self.start_close(vec![view], Then::Session(view, reference)),
                            );
                        }
                        _ => {
                            if let Some(w) = self.window_mut(window) {
                                w.focus_sessions = true;
                            }
                        }
                    }
                }
            }
            CommandId::SessionSave => self.command_save(window),
            CommandId::SessionSaveAs => {
                if let Some(view) = self.active_tab(window).map(|t| t.id) {
                    self.open_save_as(window, SaveSource::View(view));
                }
            }
            CommandId::SessionEdit => self.edit_view(window),
            CommandId::SessionLocked => {
                if let Some(tab) = self.active_tab(window)
                    && let Some(vs) = &tab.session
                    && let Some(reference) = vs.reference().cloned()
                {
                    let (view, locked) = (tab.id, !vs.locked);
                    self.store("lock the session", None, move |svc, _| {
                        svc.session_lock(&reference, locked)?;
                        Ok(StoreDone::Locked(view, locked))
                    });
                }
            }
            CommandId::ShowManage | CommandId::MaximizeManage | CommandId::Find => {
                if let Some(home) = self.home_mut(window) {
                    match inv.id {
                        CommandId::ShowManage => {
                            home.panel_mode = if home.panel_mode == PanelMode::Hidden {
                                PanelMode::Shown
                            } else {
                                PanelMode::Hidden
                            };
                        }
                        CommandId::MaximizeManage => {
                            home.panel_mode = if home.panel_mode == PanelMode::Maximized {
                                PanelMode::Shown
                            } else {
                                PanelMode::Maximized
                            };
                        }
                        _ => home.focus_search = true,
                    }
                }
            }
            CommandId::TreeOpen
            | CommandId::TreeOpenAsNew
            | CommandId::TreeOpenAll
            | CommandId::TreeNewSession
            | CommandId::TreeNewFolder
            | CommandId::TreeClear
            | CommandId::TreeDelete
            | CommandId::TreeSaveAs
            | CommandId::TreeRename
            | CommandId::TreeEdit
            | CommandId::TreeExpandAll
            | CommandId::TreeCollapseAll
            | CommandId::TreeFavorite => self.tree_command(window, inv.id),
            CommandId::HomeCompare => self.home_compare(window),
            _ => return None,
        }
        Some(effects)
    }

    fn home_mut(&mut self, window: WindowId) -> Option<&mut HomeModel> {
        match self.active_tab_mut(window)?.model_mut()? {
            ViewModel::Home(home) => Some(home),
            _ => None,
        }
    }

    fn home(&self, window: WindowId) -> Option<&HomeModel> {
        match self.active_tab(window)?.model()? {
            ViewModel::Home(home) => Some(home),
            _ => None,
        }
    }

    fn select_new(&mut self, window: WindowId, kind: SessionType) {
        if let Some(home) = self.home_mut(window) {
            home.select_key(&format!("new:{kind}"));
        }
        self.load_panel(window);
    }

    /// Selects a row of the Home tree and fills the management panel from it.
    pub fn home_select(&mut self, window: WindowId, key: &str) {
        if let Some(home) = self.home_mut(window) {
            home.select_key(key);
            home.error = None;
        }
        self.load_panel(window);
    }

    /// Reads the selection into the panel (a job).
    fn load_panel(&mut self, window: WindowId) {
        let Some(view) = self.active_tab(window).map(|t| t.id) else {
            return;
        };
        let Some(home) = self.home_mut(window) else {
            return;
        };
        let node = home.selection().map(|r| r.node.clone());
        match node {
            Some(TreeNode::NewType { kind, .. }) => {
                home.panel.loading = true;
                let left = std::mem::take(&mut home.panel.left);
                self.store("read the defaults", Some(view), move |svc, _| {
                    let defaults = svc.session_defaults(kind)?;
                    let mut session = Session::new(kind, &defaults);
                    session.specs.left = left;
                    Ok(StoreDone::Panel(
                        view,
                        PanelTarget::New(kind),
                        Some(Box::new(session)),
                    ))
                });
            }
            Some(TreeNode::Named { reference, .. }) => {
                home.panel.loading = true;
                self.store("read the session", Some(view), move |svc, _| {
                    let session = svc.session_load(&reference)?.value;
                    Ok(StoreDone::Panel(
                        view,
                        PanelTarget::Named(reference),
                        Some(Box::new(session)),
                    ))
                });
            }
            Some(TreeNode::AutoSaved { id, .. }) => {
                home.panel.loading = true;
                self.store("read the session", Some(view), move |svc, _| {
                    let session = svc.session_load_autosaved(&id)?.value;
                    Ok(StoreDone::Panel(
                        view,
                        PanelTarget::AutoSaved(id),
                        Some(Box::new(session)),
                    ))
                });
            }
            _ => home.panel.show(PanelTarget::None, None),
        }
    }

    /// `HomeCompare`: Open of the selection with the panel's specs (D-124).
    fn home_compare(&mut self, window: WindowId) {
        let wake = self.wake();
        let service = self.service.clone();
        let target = self.home_target(window);
        let Some(home) = self.home_mut(window) else {
            return;
        };
        match home.panel.target.clone() {
            PanelTarget::Named(reference) if !home.panel.changed() => {
                self.open_named(reference, target);
            }
            PanelTarget::Named(reference) => {
                let Some(edited) = home.panel.edited() else {
                    return;
                };
                self.store("open the session", None, move |svc, _| {
                    let edited = stored_specs(svc, edited)?;
                    let mut opened = load_named(svc, &reference)?;
                    opened.session.specs = edited.specs;
                    opened.session.description = edited.description;
                    opened.view = svc.session_view_of(&opened.session, reference.to_string())?;
                    opened.dirty = true;
                    Ok(StoreDone::Opened(Box::new(opened), target))
                });
            }
            PanelTarget::AutoSaved(id) => {
                self.store("open the session", None, move |svc, _| {
                    let opened = load_autosaved(svc, &id, false)?;
                    Ok(StoreDone::Opened(Box::new(opened), target))
                });
            }
            PanelTarget::New(kind) => home.compare(&service, Some(kind.view_kind()), wake),
            PanelTarget::None => home.compare(&service, None, wake),
        }
    }

    /// The node of the selected row.
    fn tree_target(&self, window: WindowId) -> Option<TreeNode> {
        Some(self.home(window)?.selection()?.node.clone())
    }

    fn tree_command(&mut self, window: WindowId, id: CommandId) {
        let node = self.tree_target(window);
        let target = self.home_target(window);
        let folder_of = |node: &Option<TreeNode>| match node {
            Some(TreeNode::Folder(path)) => path.clone(),
            Some(TreeNode::Named { reference, .. }) => reference
                .folder
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join("/"),
            _ => String::new(),
        };
        match (id, node) {
            (CommandId::TreeOpen, Some(TreeNode::Named { reference, .. })) => {
                self.open_named(reference, target);
            }
            (CommandId::TreeOpen, Some(TreeNode::AutoSaved { id, .. })) => {
                self.store("open the session", None, move |svc, _| {
                    let opened = load_autosaved(svc, &id, false)?;
                    Ok(StoreDone::Opened(Box::new(opened), target))
                });
            }
            (CommandId::TreeOpen, Some(TreeNode::NewType { .. })) => self.load_panel(window),
            (CommandId::TreeOpenAsNew, Some(TreeNode::Named { reference, .. })) => {
                self.store("open the session", None, move |svc, _| {
                    let mut session = svc.session_load(&reference)?.value;
                    svc.session_resolve_parent(&mut session)?;
                    let opened = load_value(svc, session)?;
                    Ok(StoreDone::Opened(Box::new(opened), target))
                });
            }
            (CommandId::TreeOpenAsNew, Some(TreeNode::AutoSaved { id, .. })) => {
                self.store("open the session", None, move |svc, _| {
                    let opened = load_autosaved(svc, &id, true)?;
                    Ok(StoreDone::Opened(Box::new(opened), target))
                });
            }
            (CommandId::TreeOpenAll, Some(TreeNode::Folder(path))) => {
                let inside: Vec<SessionRef> = self
                    .catalog
                    .sessions
                    .sessions
                    .iter()
                    .filter(|s| s.reference.rsplit_once('/').is_some_and(|(p, _)| p == path))
                    .filter_map(|s| SessionRef::parse(&s.reference).ok())
                    .collect();
                for (k, reference) in inside.into_iter().enumerate() {
                    let how = if k == 0 {
                        target
                    } else {
                        OpenHow::Target(OpenTarget::Window(window))
                    };
                    self.open_named(reference, how);
                }
            }
            (CommandId::TreeNewSession, node) => {
                let kind = match &node {
                    Some(TreeNode::NewType { kind, .. } | TreeNode::Named { kind, .. }) => *kind,
                    _ => SessionType::FolderCompare,
                };
                let dialog = NameDialog::new(NamePurpose::Session(kind), folder_of(&node));
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(dialog));
                }
            }
            (CommandId::TreeNewFolder, node) => {
                let dialog = NameDialog::new(NamePurpose::Folder, folder_of(&node));
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(dialog));
                }
            }
            (
                CommandId::TreeClear,
                Some(TreeNode::Named {
                    reference, kind, ..
                }),
            ) => {
                self.store("clear the session", None, move |svc, _| {
                    let defaults = svc.session_defaults(kind)?;
                    let cleared = svc.session_load(&reference)?.value.clear(&defaults);
                    svc.session_save(&reference, &cleared, false)?;
                    Ok(StoreDone::Written)
                });
            }
            (CommandId::TreeDelete, Some(node)) => self.ask_delete(window, node),
            (CommandId::TreeSaveAs, Some(TreeNode::Named { reference, .. })) => {
                self.open_save_as(window, SaveSource::Named(reference));
            }
            (CommandId::TreeSaveAs, Some(TreeNode::AutoSaved { id, .. })) => {
                self.open_save_as(window, SaveSource::AutoSaved(id));
            }
            (CommandId::TreeRename, Some(_)) => {
                if let Some(home) = self.home_mut(window)
                    && let Some(row) = home.selection()
                {
                    let (key, text) = (row.key.clone(), row.caption());
                    home.rename = Some(RenameEdit { key, text });
                }
            }
            (CommandId::TreeEdit, Some(TreeNode::Named { reference, .. })) => {
                self.edit_stored(window, EditTarget::Named(reference));
            }
            (CommandId::TreeEdit, Some(TreeNode::NewType { kind, .. })) => {
                self.edit_stored(window, EditTarget::Defaults(kind));
            }
            (CommandId::TreeExpandAll, _) => {
                if let Some(home) = self.home_mut(window) {
                    home.expand_all();
                }
            }
            (CommandId::TreeCollapseAll, _) => {
                if let Some(home) = self.home_mut(window) {
                    home.collapse_all();
                }
            }
            (
                CommandId::TreeFavorite,
                Some(TreeNode::Named {
                    reference,
                    favorite,
                    ..
                }),
            ) => {
                self.store("mark the session", None, move |svc, _| {
                    svc.session_edit(
                        &SessionTarget::Named(reference),
                        &SessionEdit {
                            set: vec![("favorite".into(), (!favorite).to_string())],
                            ..SessionEdit::default()
                        },
                        false,
                    )?;
                    Ok(StoreDone::Written)
                });
            }
            _ => {}
        }
    }

    fn ask_delete(&mut self, window: WindowId, node: TreeNode) {
        let (target, text) = match node {
            TreeNode::Named {
                reference, locked, ..
            } => {
                let text = if locked {
                    format!("The session {reference} is locked. Delete it anyway?")
                } else {
                    format!("Delete the session {reference}?")
                };
                let force = locked;
                self.push_delete_prompt(window, TreeTarget::Named(reference), force, text);
                return;
            }
            TreeNode::Folder(path) => {
                // The disk, not the catalog: the prompt names what is locked there now (D-91).
                self.store("read the folder", None, move |svc, _| {
                    let inside = svc.session_folder_contents(&path)?;
                    let locked = locked_of(&inside);
                    let mut text = format!(
                        "Delete the folder {path} and the {} sessions in it?",
                        inside.len()
                    );
                    if !locked.is_empty() {
                        text.push_str(&format!(
                            " {} of them are locked and go too: {}.",
                            locked.len(),
                            locked.join(", ")
                        ));
                    }
                    let force = !locked.is_empty();
                    Ok(StoreDone::Ask(
                        window,
                        Box::new(delete_prompt(TreeTarget::Folder(path), force, text)),
                    ))
                });
                return;
            }
            TreeNode::AutoSaved { id, label, .. } => (
                TreeTarget::AutoSaved(id),
                format!("Delete the auto-saved session {label}?"),
            ),
            _ => return,
        };
        self.push_delete_prompt(window, target, false, text);
    }

    fn push_delete_prompt(
        &mut self,
        window: WindowId,
        target: TreeTarget,
        force: bool,
        text: String,
    ) {
        let prompt = delete_prompt(target, force, text);
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(prompt));
        }
    }

    /// Commits the in-place rename of the Home tree.
    pub fn commit_rename(&mut self, window: WindowId) {
        let Some(home) = self.home_mut(window) else {
            return;
        };
        let Some(edit) = home.rename.take() else {
            return;
        };
        let Some(row) = home.tree.iter().find(|r| r.key == edit.key).cloned() else {
            return;
        };
        let name = edit.text.trim().to_owned();
        if name.is_empty() || name == row.caption() {
            return;
        }
        match row.node {
            TreeNode::Named {
                reference, locked, ..
            } => {
                let mut to: Vec<String> =
                    reference.folder.iter().map(ToString::to_string).collect();
                to.push(name);
                self.tree_rename(window, TreeTarget::Named(reference), to.join("/"), locked);
            }
            TreeNode::Folder(path) => {
                let to = match path.rsplit_once('/') {
                    Some((parent, _)) => format!("{parent}/{name}"),
                    None => name,
                };
                self.tree_rename(window, TreeTarget::Folder(path), to, false);
            }
            _ => {}
        }
    }

    /// Cancels the in-place rename.
    pub fn cancel_rename(&mut self, window: WindowId) {
        if let Some(home) = self.home_mut(window) {
            home.rename = None;
        }
    }

    /// Drags a named session onto a folder (`""`: the top level).
    pub fn tree_move(&mut self, window: WindowId, reference: SessionRef, folder: &str) {
        let locked = self
            .catalog
            .sessions
            .sessions
            .iter()
            .any(|s| s.reference == reference.to_string() && s.locked);
        let to = if folder.is_empty() {
            reference.name.to_string()
        } else {
            format!("{folder}/{}", reference.name)
        };
        if to != reference.to_string() {
            self.tree_rename(window, TreeTarget::Named(reference), to, locked);
        }
    }

    fn tree_rename(&mut self, window: WindowId, target: TreeTarget, to: String, locked: bool) {
        if locked {
            let action = SessionAction::Rename {
                target,
                to: to.clone(),
                force: true,
            };
            let prompt = Prompt::ask(
                "Rename",
                format!("The session is locked. Rename it to {to} anyway?"),
                ButtonRole::Cancel,
                vec![
                    (ButtonRole::Yes, "Rename", vec![Followup::Session(action)]),
                    (ButtonRole::Cancel, "Cancel", Vec::new()),
                ],
                "view-home",
            );
            if let Some(w) = self.window_mut(window) {
                w.dialogs.push(Box::new(prompt));
            }
        } else {
            self.session_action(
                window,
                SessionAction::Rename {
                    target,
                    to,
                    force: false,
                },
            );
        }
    }

    /// The management panel's buttons.
    pub fn panel_action(&mut self, window: WindowId, action: PanelAction) -> Vec<Effect> {
        let Some(view) = self.active_tab(window).map(|t| t.id) else {
            return Vec::new();
        };
        let target = self.home_target(window);
        let Some(home) = self.home_mut(window) else {
            return Vec::new();
        };
        let panel_target = home.panel.target.clone();
        match action {
            PanelAction::Save => {
                if let (PanelTarget::Named(reference), Some(session)) =
                    (panel_target, home.panel.edited())
                {
                    self.store("save the session", Some(view), move |svc, _| {
                        // A session deleted meanwhile stays deleted.
                        svc.session_load(&reference)?;
                        let stored = stored_specs(svc, session)?;
                        svc.session_save(&reference, &stored, false)?;
                        Ok(StoreDone::Panel(
                            view,
                            PanelTarget::Named(reference.clone()),
                            Some(Box::new(svc.session_load(&reference)?.value)),
                        ))
                    });
                    self.refresh_sessions();
                }
            }
            PanelAction::SaveAs => self.open_save_as(window, SaveSource::Panel(view)),
            PanelAction::FactoryDefaults => {
                if let Some(session) = home.panel.session.clone() {
                    self.store("read the defaults", Some(view), move |svc, _| {
                        let defaults = svc.session_defaults(session.kind)?;
                        Ok(StoreDone::PanelFill(
                            view,
                            Box::new(session.clear(&defaults)),
                        ))
                    });
                }
            }
            PanelAction::Cancel => {
                let original = home.panel.original.clone();
                home.panel.fill(original);
                home.error = None;
            }
            PanelAction::Open => self.home_compare(window),
            PanelAction::OpenAsNew => {
                if let PanelTarget::AutoSaved(id) = panel_target {
                    self.store("open the session", None, move |svc, _| {
                        let opened = load_autosaved(svc, &id, true)?;
                        Ok(StoreDone::Opened(Box::new(opened), target))
                    });
                }
            }
            PanelAction::Delete => {
                if let PanelTarget::AutoSaved(id) = panel_target {
                    let label = home.selection().map(|r| r.caption()).unwrap_or_default();
                    self.push_delete_prompt(
                        window,
                        TreeTarget::AutoSaved(id),
                        false,
                        format!("Delete the auto-saved session {label}?"),
                    );
                }
            }
            PanelAction::SetLocked(locked) => {
                if let PanelTarget::Named(reference) = panel_target {
                    self.store("lock the session", Some(view), move |svc, _| {
                        svc.session_lock(&reference, locked)?;
                        Ok(StoreDone::Panel(
                            view,
                            PanelTarget::Named(reference.clone()),
                            Some(Box::new(svc.session_load(&reference)?.value)),
                        ))
                    });
                    self.refresh_sessions();
                }
            }
        }
        Vec::new()
    }

    /// Paths dropped on a Home type button.
    pub(super) fn drop_new_session(&mut self, window: WindowId, kind: SessionType, left: String) {
        if let Some(home) = self.home_mut(window) {
            home.select_key(&format!("new:{kind}"));
            home.panel.left = left;
            home.error = None;
        }
        self.load_panel(window);
    }

    /// Two paths dropped on a Home type button: that type's view; a mismatch is the panel's error.
    pub(super) fn drop_open_type(
        &mut self,
        window: WindowId,
        kind: SessionType,
        paths: [std::path::PathBuf; 2],
    ) {
        let wake = self.wake();
        let service = self.service.clone();
        if let Some(home) = self.home_mut(window) {
            let [left, right] = paths;
            home.panel.left = left.to_string_lossy().into_owned();
            home.panel.right = right.to_string_lossy().into_owned();
            home.compare(&service, Some(kind.view_kind()), wake);
        }
    }

    // ---- the Session Settings dialog -------------------------------------------------------------------------

    fn edit_view(&mut self, window: WindowId) {
        let Some(tab) = self.active_tab(window) else {
            return;
        };
        let view = tab.id;
        let Some(capture) = AppCore::capture(tab) else {
            return;
        };
        let parent = tab.session.as_ref().and_then(|v| v.parent.clone());
        let mru = self.mru_lists();
        let formats = self.catalog.formats.clone();
        self.store("read the session", Some(view), move |svc, _| {
            let (session, _) = capture.session(svc)?;
            let (folder_parent, link) = parent_facts(svc, parent.as_ref());
            Ok(StoreDone::Dialog(
                window,
                Box::new(SessionSettingsDialog::new(
                    EditTarget::View(view),
                    session,
                    folder_parent,
                    link,
                    mru,
                    formats,
                )),
            ))
        });
    }

    fn edit_stored(&mut self, window: WindowId, target: EditTarget) {
        let mru = self.mru_lists();
        let formats = self.catalog.formats.clone();
        self.store("read the session", None, move |svc, _| {
            let session = match &target {
                EditTarget::Named(r) => svc.session_load(r)?.value,
                EditTarget::Defaults(kind) => svc.session_defaults(*kind)?,
                EditTarget::View(_) => return Ok(StoreDone::Nothing),
            };
            let (folder_parent, link) = parent_facts(svc, session.parent.as_ref());
            Ok(StoreDone::Dialog(
                window,
                Box::new(SessionSettingsDialog::new(
                    target,
                    session,
                    folder_parent,
                    link,
                    mru,
                    formats,
                )),
            ))
        });
    }

    fn mru_lists(&self) -> [Vec<String>; 2] {
        [
            self.state.mru(MruList::Left).to_vec(),
            self.state.mru(MruList::Right).to_vec(),
        ]
    }

    fn settings_ok(&mut self, result: SettingsResult) {
        if result.specs_changed {
            let (left, right) = (
                result.session.specs.left.clone(),
                result.session.specs.right.clone(),
            );
            self.push_mru(MruList::Left, left);
            self.push_mru(MruList::Right, right);
        }
        match result.target.clone() {
            EditTarget::View(view) => {
                let Some(vs) = self.view_session(view).cloned() else {
                    return;
                };
                let title = vs.reference().map(ToString::to_string).unwrap_or_default();
                let loaded = vs.loaded;
                // Until the job answers the close must not see the old state (§1.4.20 W4b).
                if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                    vs.settings_pending = true;
                    if loaded.is_some() {
                        vs.dirty = true;
                    }
                }
                self.store("apply the session settings", Some(view), move |svc, _| {
                    let (_, session) = apply_rules(svc, &result)?;
                    let session = stored_specs(svc, session)?;
                    let opened = svc.session_view_of(&session, title)?;
                    let dirty = loaded.as_ref().is_some_and(|l| {
                        is_dirty(l, &session) || l.description != session.description
                    });
                    Ok(StoreDone::Reopen {
                        view,
                        opened: Box::new(opened),
                        session: Box::new(session),
                        dirty,
                    })
                });
            }
            EditTarget::Named(reference) => {
                self.store("save the session", None, move |svc, _| {
                    // A session deleted meanwhile stays deleted.
                    svc.session_load(&reference)?;
                    let (_, session) = apply_rules(svc, &result)?;
                    let session = stored_specs(svc, session)?;
                    svc.session_save(&reference, &session, false)?;
                    Ok(StoreDone::Written)
                });
            }
            EditTarget::Defaults(_) => {
                self.store("save the defaults", None, move |svc, _| {
                    let mut session = result.session.clone();
                    if let Some((_, rules)) = &result.rules {
                        session.rules = rules.clone();
                    }
                    svc.session_save_defaults(&session)?;
                    Ok(StoreDone::Written)
                });
            }
        }
    }

    /// A view takes changed specs or rules: its model is rebuilt in its tab, origin kept.
    fn reopen(&mut self, view: ViewId, opened: SessionView, session: Session, dirty: bool) {
        if self.tab(view).is_none() {
            return;
        }
        let mut request = opened.request;
        match &mut request {
            ViewRequest::Text { view, .. } => apply_text_display(&session.display, view),
            ViewRequest::Folder { display, .. } => apply_folder_display(&session.display, display),
        }
        let model = self.build_model_with([None, None], request, &session.view);
        let specs = opened.specs;
        if let Some(tab) = self.tab_mut(view) {
            if let Some(m) = tab.model_mut() {
                match m {
                    ViewModel::Text(m) => m.cancel(),
                    ViewModel::Folder(m) => m.cancel(),
                    ViewModel::Home(_) => {}
                }
            }
            tab.content = TabContent::Live(model);
            if let Some(vs) = tab.session.as_mut() {
                vs.dirty = dirty;
                vs.settings_pending = false;
                vs.description.clone_from(&session.description);
            }
            let caption = tab.caption();
            self.shared.set_title(view, caption);
            self.shared.set_specs(view, specs);
        }
        self.untrack_view(view);
        self.track_view(view);
    }

    // ---- workspaces ------------------------------------------------------------------------------------------

    /// Loads a workspace (D-125): read, then every view's close action, then the restore.
    pub fn load_workspace(&mut self, name: SessionName) {
        self.store("read the workspace", None, move |svc, _| {
            Ok(StoreDone::Workspace(Box::new(svc.workspace_load(&name)?)))
        });
    }

    /// An IPC `LoadWorkspace` (its tabs are registered with the handler's answer).
    pub(super) fn load_registered(
        &mut self,
        load: WorkspaceLoadDto,
        ids: Vec<Vec<Option<ViewId>>>,
    ) -> Vec<Effect> {
        let views = self.all_views();
        self.start_close(views, Then::Workspace(Box::new(load), Some(ids)))
    }

    fn restore_workspace(
        &mut self,
        load: WorkspaceLoadDto,
        ids: Option<Vec<Vec<Option<ViewId>>>>,
    ) -> Vec<Effect> {
        let before = self.window_ids();
        let mut effects = Vec::new();
        let ids = ids.unwrap_or_else(|| register_tabs(&self.shared, &load));
        // The open views go.
        for w in 0..self.windows.len() {
            for mut tab in std::mem::take(&mut self.windows[w].tabs) {
                self.remember_layout(&tab);
                self.close_model(&mut tab, CloseReason::User);
            }
            self.windows[w].active = 0;
        }
        let wanted = load.windows.len().max(1);
        while self.windows.len() > wanted {
            if let Some(w) = self.windows.pop() {
                effects.push(Effect::CloseViewport(w.id));
            }
        }
        let mut warnings: Vec<Warning> = load.warnings.clone();
        if load.windows.is_empty() {
            let root = self.root();
            self.new_home_tab(root);
        }
        for (i, restored) in load.windows.iter().enumerate() {
            let window = match self.windows.get(i) {
                Some(w) => w.id,
                None => self.new_window(None),
            };
            let stored = &restored.window;
            let geometry = hikasa_session::settings::state::WindowState {
                width: stored.width as i32,
                height: stored.height as i32,
                x: stored.x as i32,
                y: stored.y as i32,
                maximized: stored.maximized,
            };
            let existed = before.contains(&window);
            if let Some(w) = self.window_mut(window) {
                w.geometry = geometry.clone();
            }
            if existed {
                effects.push(Effect::Geometry(window, geometry));
            }
            for (t, tab) in restored.tabs.iter().enumerate() {
                let stored_tab = &load.workspace.windows[i].tabs[t];
                let id = ids.get(i).and_then(|w| w.get(t).copied()).flatten();
                match (pending_tab(tab, stored_tab), id) {
                    (Ok(Some(pending)), Some(id)) => {
                        let scope = crate::window::scope_of(pending.kind);
                        let toolbar = pending
                            .view
                            .get("toolbar")
                            .and_then(hikasa_session::sessions::view_bool)
                            .unwrap_or_else(|| self.toolbar_default(scope));
                        if let Some(w) = self.window_mut(window) {
                            w.tabs.push(Tab::pending(id, pending, toolbar));
                        }
                    }
                    (Err(text), _) => {
                        warnings.push(Warning {
                            code: hikasa_core::WarningCode::SessionMissing,
                            message: text,
                            spec: None,
                        });
                        self.push_home_tab(window);
                    }
                    _ => self.push_home_tab(window),
                }
            }
            if self.window(window).is_some_and(|w| w.tabs.is_empty()) {
                self.new_home_tab(window);
            }
            if let Some(w) = self.window_mut(window) {
                let last = w.tabs.len().saturating_sub(1);
                w.active = (stored.active_tab as usize).min(last);
            }
        }
        let focused = self.focused();
        for w in warnings {
            let text = format!("[{}] {}", w.code, w.message);
            self.notify(
                focused,
                Notice::new(format!("workspace:{text}"), NoticeKind::Warning, text),
                "workspace",
                LogKind::Warning,
            );
        }
        self.start_active_pending();
        effects.extend(self.new_viewports(&before));
        effects
    }

    fn push_home_tab(&mut self, window: WindowId) {
        let id = self.shared.allocate();
        let home = self.new_home_model();
        let tab = Tab::live(id, ViewModel::Home(Box::new(home)), true);
        if let Some(w) = self.window_mut(window) {
            w.tabs.push(tab);
        }
    }

    /// Every window as the save job needs it (D-125): root first, then in opening order; Home tabs are not saved.
    fn capture_workspace(&self) -> Vec<CapturedWindow> {
        self.windows
            .iter()
            .map(|w| {
                let mut tabs = Vec::new();
                let mut active = 0;
                for (i, tab) in w.tabs.iter().enumerate() {
                    if i == w.active {
                        active = tabs.len();
                    }
                    let captured = match (tab.pending_tab(), &tab.session) {
                        (Some(p), _) => Some(CapturedTab::Pending(Box::new(p.clone()))),
                        (None, Some(vs)) => match (vs.reference(), AppCore::capture(tab)) {
                            (Some(r), Some(c)) => Some(CapturedTab::Named(r.to_string(), c.view)),
                            (None, Some(c)) => Some(CapturedTab::Inline {
                                capture: Box::new(c),
                                previous: vs.inline.clone(),
                                slot: self.slots.get(&tab.id).cloned(),
                            }),
                            _ => None,
                        },
                        _ => None,
                    };
                    tabs.extend(captured);
                }
                CapturedWindow {
                    geometry: w.geometry.clone(),
                    active: u32::try_from(active.min(tabs.len().saturating_sub(1)))
                        .unwrap_or_default(),
                    tabs,
                }
            })
            .collect()
    }

    fn save_workspace(&mut self, name: SessionName) {
        let windows = self.capture_workspace();
        self.push_mru(MruList::Workspaces, name.to_string());
        self.store("save the workspace", None, move |svc, _| {
            let shortcut = svc
                .workspace_load(&name)
                .map(|l| l.workspace.shortcut)
                .unwrap_or_default();
            let mut workspace = Workspace {
                shortcut,
                windows: Vec::with_capacity(windows.len()),
            };
            for window in &windows {
                workspace.windows.push(window.window(svc)?);
            }
            svc.workspace_save(&name, &workspace)?;
            Ok(StoreDone::Written)
        });
    }

    /// `on_exit_save_workspace_as` (D-125): saved after the close warnings, before the views close.
    pub(super) fn exit_workspace_save(&mut self) {
        if self.exit_saved {
            return;
        }
        self.exit_saved = true;
        let name = self.options().startup.on_exit_save_workspace_as.clone();
        if name.is_empty() {
            return;
        }
        match SessionName::parse(&name) {
            Ok(name) => self.save_workspace(name),
            Err(e) => self.log_entry(LogKind::Warning, "workspace", e.to_string()),
        }
    }

    fn open_manage_workspaces(&mut self, window: WindowId) {
        let globals = self
            .tables
            .view(Scope::Global)
            .entries
            .iter()
            .filter(|e| e.section == "global" && e.implemented())
            .flat_map(|e| {
                e.shortcuts
                    .iter()
                    .map(|s| (s.clone(), e.caption.to_owned()))
            })
            .collect();
        let dialog = ManageWorkspacesDialog {
            rows: self.catalog.workspaces.workspaces.clone(),
            globals,
            ..ManageWorkspacesDialog::default()
        };
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
    }

    /// The workspace a key press loads (D-111 rule 3): only when no command took the key; a key without Ctrl, Alt
    /// or Cmd only at the window root (a focused control types it).
    pub fn workspace_key(
        &self,
        keys: &[Shortcut],
        at: crate::commands::FocusArea,
    ) -> Option<SessionName> {
        let key = keys.first()?;
        if at != crate::commands::FocusArea::Root && crate::commands::is_focus_bound(key) {
            return None;
        }
        self.catalog
            .workspaces
            .workspaces
            .iter()
            .find(|w| Shortcut::parse(&w.shortcut).is_ok_and(|s| keys.contains(&s)))
            .and_then(|w| SessionName::parse(&w.name).ok())
    }

    /// What runs after the first frame (D-125, §1.4.12): the workspace of the command line or of
    /// `[startup] load_workspace`; missing: a warning notice and Home.
    pub fn after_first_frame(&mut self) {
        if !self.auto_save_on() {
            self.prune_autosaved();
        }
        let Some(startup) = self.startup.take() else {
            return;
        };
        let name = match startup {
            super::Startup::Workspace(name) => name,
            super::Startup::Session(reference) => {
                let target = self.open_target();
                self.open_named(reference, OpenHow::Target(target));
                return;
            }
            super::Startup::Import(path) => {
                let window = self.focused();
                self.open_import(window, Some(&path));
                return;
            }
            super::Startup::Options => self.options().startup.load_workspace.clone(),
        };
        if name.is_empty() {
            return;
        }
        match SessionName::parse(&name) {
            Ok(name) => self.store("read the workspace", None, move |svc, _| {
                match svc.workspace_load(&name) {
                    Ok(load) => Ok(StoreDone::Workspace(Box::new(load))),
                    Err(ApiError::NotFound(_)) => Ok(StoreDone::StartupMissing(name.to_string())),
                    Err(e) => Err(e),
                }
            }),
            Err(e) => {
                let window = self.focused();
                let text = format!("The startup workspace `{name}` cannot be loaded: {e}");
                self.notify(
                    window,
                    Notice::new("workspace:startup", NoticeKind::Warning, text),
                    "workspace",
                    LogKind::Warning,
                );
            }
        }
    }

    /// Applies `max_auto_saved_sessions` to the files now (D-83: at 0 no later save would).
    pub(super) fn prune_autosaved(&mut self) {
        self.store("prune the auto-saved sessions", None, |svc, _| {
            Ok(if svc.session_prune_autosaved()?.is_empty() {
                StoreDone::Nothing
            } else {
                StoreDone::Written
            })
        });
    }

    // ---- the dialogs' actions --------------------------------------------------------------------------------

    /// A session dialog was confirmed.
    pub(super) fn session_action(
        &mut self,
        window: WindowId,
        action: SessionAction,
    ) -> Vec<Effect> {
        match action {
            SessionAction::CloseAnswer(role) => return self.close_answer(role),
            SessionAction::SaveLocked(view) => {
                if let (Some(reference), Some(capture)) = (
                    self.view_session(view).and_then(|v| v.reference().cloned()),
                    self.tab(view).and_then(AppCore::capture),
                ) {
                    let seen = self.view_session(view).and_then(|v| v.seen.clone());
                    self.queue_save(view, reference, capture, true, seen, StoreKind::Plain);
                }
            }
            SessionAction::SaveAs {
                source,
                reference,
                confirmed,
            } => self.save_as(window, source, reference, confirmed),
            SessionAction::Create { reference, kind } => {
                self.store("create the session", None, move |svc, _| {
                    svc.session_new(&reference, kind, None, None, None)?;
                    Ok(StoreDone::Written)
                });
            }
            SessionAction::NewFolder(path) => {
                self.store("create the folder", None, move |svc, _| {
                    svc.session_mkdir(&path)?;
                    Ok(StoreDone::Written)
                });
            }
            SessionAction::Delete { target, force } => {
                self.store("delete", None, move |svc, _| {
                    Ok(match &target {
                        TreeTarget::Named(r) => {
                            let actual = spelled(svc, r.clone());
                            svc.session_delete(&actual, force)?;
                            StoreDone::Deleted(vec![actual])
                        }
                        TreeTarget::Folder(path) => {
                            let inside = svc.session_folder_contents(path)?;
                            svc.session_rmdir(path, true, force)?;
                            StoreDone::Deleted(inside.into_iter().map(|e| e.reference).collect())
                        }
                        TreeTarget::AutoSaved(id) => {
                            svc.session_delete_autosaved(id)?;
                            StoreDone::Written
                        }
                    })
                });
            }
            SessionAction::Rename { target, to, force } => {
                self.store("rename", None, move |svc, _| {
                    Ok(match &target {
                        TreeTarget::Named(from) => {
                            let actual = spelled(svc, from.clone());
                            let to = SessionRef::parse(&to)?;
                            svc.session_rename(&actual, &to, force)?;
                            let to = spelled(svc, to);
                            StoreDone::Moved(vec![(actual, to)])
                        }
                        TreeTarget::Folder(path) => {
                            let inside = svc.session_folder_contents(path)?;
                            let locked = locked_of(&inside);
                            if !force && !locked.is_empty() {
                                return Ok(StoreDone::Ask(
                                    window,
                                    Box::new(rename_folder_prompt(path, &to, &locked)),
                                ));
                            }
                            svc.session_rename_folder(path, &to, force)?;
                            let depth = parse_folder(path)?.len();
                            let new = parse_folder(&to)?;
                            let moved = inside.into_iter().map(|e| {
                                let mut folder = new.clone();
                                folder.extend_from_slice(&e.reference.folder[depth..]);
                                let name = e.reference.name.clone();
                                (e.reference, SessionRef { folder, name })
                            });
                            StoreDone::Moved(moved.collect())
                        }
                        TreeTarget::AutoSaved(_) => StoreDone::Nothing,
                    })
                });
            }
            SessionAction::Settings(result) => self.settings_ok(*result),
            SessionAction::WorkspaceSave { name, confirmed } => {
                let wanted = name.to_string().to_lowercase();
                let exists = self
                    .workspace_names()
                    .iter()
                    .any(|n| n.to_lowercase() == wanted);
                if exists && !confirmed {
                    let action = SessionAction::WorkspaceSave {
                        name: name.clone(),
                        confirmed: true,
                    };
                    let prompt = Prompt::ask(
                        "Save Workspace As",
                        format!("The workspace {name} exists. Replace it?"),
                        ButtonRole::Cancel,
                        vec![
                            (ButtonRole::Yes, "Replace", vec![Followup::Session(action)]),
                            (ButtonRole::Cancel, "Cancel", Vec::new()),
                        ],
                        "dialog-workspaces",
                    );
                    if let Some(w) = self.window_mut(window) {
                        w.dialogs.push(Box::new(prompt));
                    }
                } else {
                    self.save_workspace(name);
                }
            }
            SessionAction::WorkspaceLoad(name) => self.load_workspace(name),
            SessionAction::WorkspaceRename(from, to) => {
                self.store("rename the workspace", None, move |svc, _| {
                    svc.workspace_rename(&from, &to)?;
                    Ok(StoreDone::Written)
                });
            }
            SessionAction::WorkspaceAskDelete(name) => {
                let action = SessionAction::WorkspaceDelete(name.clone());
                let prompt = Prompt::ask(
                    "Delete Workspace",
                    format!("Delete the workspace {name}?"),
                    ButtonRole::Cancel,
                    vec![
                        (ButtonRole::Yes, "Delete", vec![Followup::Session(action)]),
                        (ButtonRole::Cancel, "Cancel", Vec::new()),
                    ],
                    "dialog-workspaces",
                );
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            SessionAction::WorkspaceDelete(name) => {
                self.store("delete the workspace", None, move |svc, _| {
                    svc.workspace_delete(&name)?;
                    Ok(StoreDone::Written)
                });
            }
            SessionAction::WorkspaceShortcut(name, shortcut) => {
                self.store("set the workspace shortcut", None, move |svc, _| {
                    let mut workspace = svc.workspace_load(&name)?.workspace;
                    workspace.shortcut = shortcut;
                    svc.workspace_save(&name, &workspace)?;
                    Ok(StoreDone::Written)
                });
            }
        }
        Vec::new()
    }

    /// Pushes a value on an MRU list of `state.toml` (through `state_update`, D-108).
    fn push_mru(&mut self, list: MruList, value: String) {
        if value.is_empty() {
            return;
        }
        let max = self.options().tweaks.metrics.max_mru_items as usize;
        self.state.push_mru(list, &value, max);
        self.update_state(move |s| s.push_mru(list, &value, max));
    }
}

/// The spelling on disk of a session that exists (D-80: names are case-insensitive), else `reference` as given.
fn spelled(service: &Service, reference: SessionRef) -> SessionRef {
    service
        .session_store()
        .resolve(&reference)
        .unwrap_or(reference)
}

/// The references of the locked sessions of a listing.
fn locked_of(entries: &[NamedEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e.locked)
        .map(|e| e.reference.to_string())
        .collect()
}

fn delete_prompt(target: TreeTarget, force: bool, text: String) -> Prompt {
    let action = SessionAction::Delete { target, force };
    Prompt::ask(
        "Delete",
        text,
        ButtonRole::Cancel,
        vec![
            (ButtonRole::Yes, "Delete", vec![Followup::Session(action)]),
            (ButtonRole::Cancel, "Cancel", Vec::new()),
        ],
        "view-home",
    )
}

fn rename_folder_prompt(path: &str, to: &str, locked: &[String]) -> Prompt {
    let action = SessionAction::Rename {
        target: TreeTarget::Folder(path.to_owned()),
        to: to.to_owned(),
        force: true,
    };
    Prompt::ask(
        "Rename",
        format!(
            "The folder {path} holds {} locked sessions: {}. Rename it to {to} anyway?",
            locked.len(),
            locked.join(", ")
        ),
        ButtonRole::Cancel,
        vec![
            (ButtonRole::Yes, "Rename", vec![Followup::Session(action)]),
            (ButtonRole::Cancel, "Cancel", Vec::new()),
        ],
        "view-home",
    )
}

/// A session with its specs in stored form (D-80; a typed relative path becomes absolute).
fn stored_specs(service: &Service, mut session: Session) -> Result<Session, ApiError> {
    for slot in [&mut session.specs.left, &mut session.specs.right] {
        if !slot.is_empty() {
            *slot = service.stored_spec(slot)?.0;
        }
    }
    Ok(session)
}

/// The rules of the dialog applied with their scope: what was written, and the session.
fn apply_rules(
    service: &Service,
    result: &SettingsResult,
) -> Result<(Option<ScopeWritesDto>, Session), ApiError> {
    let mut session = result.session.clone();
    let writes = match &result.rules {
        Some((scope, rules)) => Some(service.session_apply_scope(
            *scope,
            rules,
            &mut session,
            result.parent.as_deref(),
        )?),
        None => None,
    };
    Ok((writes, session))
}

/// Whether a link names a readable folder session, and its text.
fn parent_facts(service: &Service, link: Option<&ParentLink>) -> (bool, Option<String>) {
    let Some(link) = link else {
        return (false, None);
    };
    let session = match link.session.strip_prefix("_autosaved/") {
        Some(id) => AutoId::parse(id)
            .and_then(|id| service.session_load_autosaved(&id))
            .map(|l| l.value),
        None => SessionRef::parse(&link.session)
            .and_then(|r| service.session_load(&r))
            .map(|l| l.value),
    };
    (
        session.is_ok_and(|s| s.kind.is_folder()),
        Some(link.session.clone()),
    )
}
