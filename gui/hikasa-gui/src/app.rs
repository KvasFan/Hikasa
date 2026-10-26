//! The application core: tabs, the active view, command dispatch through the catalogue, the messages of the
//! IPC handler and the jobs, and the D-23 close rules for waited views (contract §1.10, §3.7). It holds no
//! toolkit type, so the whole behaviour of the window runs headless in tests (docs/17 §3.4); the egui layer
//! (`ui`) only draws it and feeds it commands.

use std::sync::Arc;

use hikasa_session::api::{ViewOutcome, ViewRequest};
use hikasa_session::ipc::{CloseReason, OpenOptions, ServerHandle, ViewId, ViewInfo, ViewKind};
use hikasa_session::{HIKASA_VERSION, Service};

use crate::commands::{CommandId, Ctx, Scope, lookup};
use crate::folder_view::FolderViewModel;
use crate::home::HomeModel;
use crate::mailbox::Mailbox;
use crate::shared::Shared;
use crate::text_view::TextViewModel;

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
    /// Close a view (IPC `CloseView`).
    Close {
        /// The view.
        view: ViewId,
    },
    /// Quit (IPC `Shutdown`, the window's close button).
    Quit,
    /// A frame: poll the jobs and the live models.
    Tick,
    /// A job or the scan has news.
    Wake,
}

/// What the toolkit layer must do after the core handled something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// A text view must refetch its meta (data or scroll position changed).
    TextDirty(ViewId),
    /// The folder tree must scroll a row into view.
    FolderScroll(ViewId, u32),
    /// Quit the process (views are closed, the IPC server is shut down).
    Exit,
}

/// A tab's view.
pub enum ViewModel {
    /// Home.
    Home(HomeModel),
    /// Folder Compare.
    Folder(Box<FolderViewModel>),
    /// Text Compare.
    Text(Box<TextViewModel>),
}

impl ViewModel {
    /// The command scope of the view.
    pub fn scope(&self) -> Scope {
        match self {
            ViewModel::Home(_) => Scope::Home,
            ViewModel::Folder(_) => Scope::Folder,
            ViewModel::Text(_) => Scope::Text,
        }
    }
}

/// One tab.
pub struct Tab {
    /// View id (registered in [`Shared`] for comparison views).
    pub id: ViewId,
    /// The view.
    pub model: ViewModel,
}

impl Tab {
    /// Tab caption.
    pub fn caption(&self) -> String {
        match &self.model {
            ViewModel::Home(_) => "Home".into(),
            ViewModel::Folder(m) => m.caption(),
            ViewModel::Text(m) => m.caption(),
        }
    }
}

/// The application core.
pub struct AppCore {
    service: Service,
    shared: Arc<Shared>,
    mailbox: Mailbox<UiMsg>,
    server: Option<ServerHandle>,
    tabs: Vec<Tab>,
    active: usize,
    message: Option<String>,
    quitting: bool,
}

impl AppCore {
    /// The core with one Home tab.
    pub fn new(
        service: Service,
        shared: Arc<Shared>,
        mailbox: Mailbox<UiMsg>,
        server: Option<ServerHandle>,
    ) -> AppCore {
        let mut app = AppCore {
            service,
            shared,
            mailbox,
            server,
            tabs: Vec::new(),
            active: 0,
            message: None,
            quitting: false,
        };
        app.new_home_tab();
        app
    }

    /// The view registry shared with the IPC handler.
    #[cfg(test)]
    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// The tabs.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Index of the active tab.
    pub fn active(&self) -> usize {
        self.active
    }

    /// The active tab.
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    /// The active tab, mutable.
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    /// The tab of a view.
    #[cfg(test)]
    pub fn tab_mut(&mut self, view: ViewId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == view)
    }

    /// The tab of a view.
    pub fn tab(&self, view: ViewId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == view)
    }

    /// Selects a tab.
    pub fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    /// The message panel text (errors, About).
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Hides the message panel.
    pub fn dismiss_message(&mut self) {
        self.message = None;
    }

    /// True once quitting started.
    #[cfg(test)]
    pub fn quitting(&self) -> bool {
        self.quitting
    }

    /// True while a job of any view runs (the window then polls at a low rate besides the jobs' wake-ups).
    pub fn busy(&self) -> bool {
        self.tabs.iter().any(|t| match &t.model {
            ViewModel::Home(m) => m.resolving(),
            ViewModel::Folder(m) => m.busy(),
            ViewModel::Text(m) => m.busy(),
        })
    }

    /// True while a job has ended but its result is not published yet: the window looks again at once
    /// ([`crate::job::Job::settling`]).
    pub fn settling(&self) -> bool {
        self.tabs.iter().any(|t| match &t.model {
            ViewModel::Home(m) => m.settling(),
            ViewModel::Folder(m) => m.settling(),
            ViewModel::Text(m) => m.settling(),
        })
    }

    fn wake(&self) -> Arc<dyn Fn() + Send + Sync> {
        let mailbox = self.mailbox.clone();
        Arc::new(move || mailbox.post(UiMsg::Wake))
    }

    fn new_home_tab(&mut self) {
        let id = self.shared.allocate();
        self.tabs.push(Tab {
            id,
            model: ViewModel::Home(HomeModel::default()),
        });
        self.active = self.tabs.len() - 1;
    }

    /// Opens a comparison view registered under `view` (IPC, the command line, Home, FC-062). When `replace`
    /// is set, the view replaces that tab (a Home tab's Compare); otherwise it opens in a new tab, reusing an
    /// untouched lone Home tab.
    pub fn open_view(
        &mut self,
        view: ViewId,
        request: ViewRequest,
        options: &OpenOptions,
        replace: Option<usize>,
    ) {
        let titles = [options.titles[0].clone(), options.titles[1].clone()];
        let model = match request {
            ViewRequest::Text {
                request,
                view: opts,
            } => ViewModel::Text(Box::new(TextViewModel::new(
                &self.service,
                request,
                opts,
                titles,
                self.wake(),
            ))),
            ViewRequest::Folder {
                request,
                display,
                expand_all,
            } => ViewModel::Folder(Box::new(FolderViewModel::new(
                &self.service,
                request,
                display,
                expand_all,
                titles,
                self.wake(),
            ))),
        };
        let tab = Tab { id: view, model };
        self.shared.set_title(view, tab.caption());
        let untouched_home = self.tabs.len() == 1
            && matches!(&self.tabs[0].model, ViewModel::Home(h) if *h == HomeModel::default());
        match replace {
            Some(index) if index < self.tabs.len() => {
                self.tabs[index] = tab;
                self.active = index;
            }
            _ if untouched_home => {
                self.tabs[0] = tab;
                self.active = 0;
            }
            _ => {
                self.tabs.push(tab);
                self.active = self.tabs.len() - 1;
            }
        }
    }

    /// Registers and opens a view that did not come over IPC (the command line, Home, FC-062).
    pub fn open_local(
        &mut self,
        kind: ViewKind,
        request: ViewRequest,
        options: &OpenOptions,
        specs: Vec<String>,
        replace: Option<usize>,
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
        self.open_view(view, request, options, replace);
        view
    }

    /// Handles one message; returns the effects for the toolkit layer.
    pub fn handle(&mut self, msg: UiMsg) -> Vec<Effect> {
        match msg {
            UiMsg::Open {
                view,
                request,
                options,
            } => {
                if self.quitting {
                    // Too late: the view never opens; it reports `shutdown` (100), as the server does for
                    // the waiters it notifies itself (§3.6, D-23).
                    let (result, code) = ViewOutcome::close_unopened(CloseReason::Shutdown);
                    self.shared.close(view, result, code);
                } else {
                    self.open_view(view, *request, &options, None);
                }
            }
            UiMsg::Close { view } => {
                if let Some(index) = self.tabs.iter().position(|t| t.id == view) {
                    self.close_tab(index, CloseReason::User);
                } else {
                    // Registered but not opened: the view was dropped before it ran (100).
                    let (result, code) = ViewOutcome::close_unopened(CloseReason::User);
                    self.shared.close(view, result, code);
                }
            }
            UiMsg::Quit => return self.shutdown(),
            UiMsg::Tick | UiMsg::Wake => {
                self.poll();
            }
        }
        self.dirty_effects()
    }

    /// Polls every view's jobs; true when something changed. A Home tab whose Compare resolved becomes the
    /// view.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let service = self.service.clone();
        let mut opened = Vec::new();
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            changed |= match &mut tab.model {
                ViewModel::Home(m) => {
                    let (open, changed) = m.poll();
                    if let Some(open) = open {
                        opened.push((index, open));
                    }
                    changed
                }
                ViewModel::Folder(m) => m.poll(&service),
                ViewModel::Text(m) => m.poll(&service),
            };
        }
        for (index, open) in opened {
            let active = self.active;
            self.open_local(
                open.kind,
                open.request,
                &open.options,
                open.specs,
                Some(index),
            );
            // A Compare that resolved in a background tab does not steal the focus.
            self.active = active;
        }
        changed
    }

    /// `TextDirty` for every text view whose panes must refetch, `FolderScroll` for every tree that must
    /// bring a row into view.
    pub fn dirty_effects(&mut self) -> Vec<Effect> {
        self.tabs
            .iter_mut()
            .filter_map(|t| match &mut t.model {
                ViewModel::Text(m) => m.take_dirty().then_some(Effect::TextDirty(t.id)),
                ViewModel::Folder(m) => m.take_scroll().map(|row| Effect::FolderScroll(t.id, row)),
                _ => None,
            })
            .collect()
    }

    /// The command context of the active view.
    pub fn ctx(&self) -> Ctx {
        let mut ctx = Ctx {
            tabs: self.tabs.len(),
            ..Ctx::default()
        };
        if let Some(tab) = self.active_tab() {
            ctx.scope = tab.model.scope();
            match &tab.model {
                ViewModel::Home(m) => m.ctx_into(&mut ctx),
                ViewModel::Folder(m) => m.ctx_into(&mut ctx),
                ViewModel::Text(m) => m.ctx_into(&mut ctx),
            }
        }
        ctx
    }

    /// The scope of the active view.
    pub fn scope(&self) -> Scope {
        self.active_tab().map_or(Scope::Global, |t| t.model.scope())
    }

    /// True when `id` is available in the active view (its catalogue predicate holds).
    pub fn enabled(&self, id: CommandId) -> bool {
        let ctx = self.ctx();
        lookup(ctx.scope, id).is_some_and(|c| c.enabled_when.holds(&ctx))
    }

    /// The check state of a toggle or radio command in the active view.
    pub fn checked(&self, id: CommandId) -> Option<bool> {
        match &self.active_tab()?.model {
            ViewModel::Home(_) => None,
            ViewModel::Folder(m) => m.checked(id),
            ViewModel::Text(m) => m.checked(id),
        }
    }

    /// Executes a command in the active view (ignored when it is not available there).
    pub fn execute(&mut self, id: CommandId) -> Vec<Effect> {
        if !self.enabled(id) {
            return Vec::new();
        }
        let service = self.service.clone();
        match id {
            CommandId::TabNew => self.new_home_tab(),
            CommandId::CloseTab => self.close_tab(self.active, CloseReason::User),
            CommandId::TabNext => self.active = (self.active + 1) % self.tabs.len().max(1),
            CommandId::TabPrev => {
                let n = self.tabs.len().max(1);
                self.active = (self.active + n - 1) % n;
            }
            CommandId::Exit => return self.shutdown(),
            CommandId::About => {
                self.message = Some(format!(
                    "Hikasa {HIKASA_VERSION} — compare folders and files. GUI prototype (milestone 1); every \
                     function is also available from the hikasa command line."
                ));
            }
            CommandId::HomeCompare => {
                let wake = self.wake();
                if let Some(Tab {
                    model: ViewModel::Home(home),
                    ..
                }) = self.tabs.get_mut(self.active)
                {
                    home.compare(&service, wake);
                }
            }
            _ => {
                let Some(tab) = self.tabs.get_mut(self.active) else {
                    return Vec::new();
                };
                let open = match &mut tab.model {
                    ViewModel::Home(_) => None,
                    ViewModel::Text(m) => {
                        m.execute(&service, id);
                        None
                    }
                    ViewModel::Folder(m) => m.execute(&service, id).flatten(),
                };
                if let Some((request, view)) = open {
                    let specs = vec![
                        request.left.to_string_lossy().into_owned(),
                        request.right.to_string_lossy().into_owned(),
                    ];
                    self.open_local(
                        ViewKind::TextCompare,
                        ViewRequest::Text { request, view },
                        &OpenOptions::default(),
                        specs,
                        None,
                    );
                }
            }
        }
        self.dirty_effects()
    }

    /// Closes a tab: its view reports the D-23 outcome to its waiter (if any), its jobs stop. The last tab is
    /// replaced by a Home tab.
    pub fn close_tab(&mut self, index: usize, reason: CloseReason) {
        if index >= self.tabs.len() {
            return;
        }
        let mut tab = self.tabs.remove(index);
        self.close_model(&mut tab, reason);
        if self.tabs.is_empty() {
            if !self.quitting {
                self.new_home_tab();
            }
            self.active = 0;
        } else if self.active >= self.tabs.len() || self.active > index {
            self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
        }
    }

    fn close_model(&mut self, tab: &mut Tab, reason: CloseReason) {
        let service = self.service.clone();
        let outcome = match &mut tab.model {
            ViewModel::Home(_) => None,
            ViewModel::Folder(m) => {
                let outcome = m.outcome(&service);
                m.cancel();
                Some(outcome)
            }
            ViewModel::Text(m) => {
                let outcome = m.outcome(&service);
                m.cancel();
                Some(outcome)
            }
        };
        if let Some(outcome) = outcome {
            let (result, code) = outcome.close(reason, false);
            self.shared.close(tab.id, result, code);
        }
    }

    /// Quits: every view closes with reason `shutdown` and its real outcome (D-23), then the IPC server stops
    /// (it notifies any waiter left and closes the connections, §1.8.7 item 12).
    pub fn shutdown(&mut self) -> Vec<Effect> {
        if self.quitting {
            return vec![Effect::Exit];
        }
        self.quitting = true;
        while !self.tabs.is_empty() {
            let last = self.tabs.len() - 1;
            self.close_tab(last, CloseReason::Shutdown);
        }
        if let Some(server) = self.server.take()
            && let Err(e) = server.shutdown()
        {
            tracing::warn!(target: "hikasa::gui", "IPC server shutdown: {e}");
        }
        vec![Effect::Exit]
    }
}
