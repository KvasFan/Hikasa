//! The verbs of a folder view from the window's side (M2 W9b, D-244…D-246): each command of §1.9.13 starts on the
//! active view with what the window knows (the remembered choices, `include_hidden_by_default`, the pane of a context
//! menu), its dialog goes on the window's stack, and the dialogs that leave come back to their view
//! (`AppCore::dialog_closed`).

use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::ConfirmMode;
use hikasa_folder::select::ExcludeWhat;
use hikasa_folder::sync::SyncOptions;
use hikasa_session::ApiError;
use hikasa_session::api::ViewRequest;
use hikasa_session::api::{
    OpenTarget, OpenWithRequest, QuickMethod, parse_content_method, parse_exclude_what,
};
use hikasa_session::ipc::{ViewId, ViewKind};
use hikasa_session::log::LogKind;
use hikasa_session::sessions::{SessionRules, SessionType};
use hikasa_session::settings::options::SyncConfirmations;

use super::{AppCore, CoreJob};
use crate::browse::{BrowseMode, BrowseTarget};
use crate::commands::CommandId;
use crate::content_dialogs::{
    COMPARE_CONTENTS_DIALOG, CompareContentsDialog, EXCLUDE_DIALOG, ExcludeDialog,
    QUICK_COMPARE_DIALOG, QuickCompareDialog,
};
use crate::dialog::{Followup, Prompt};
use crate::folder_view::verbs::{VerbOptions, VerbStart};
use crate::job::Job;
use crate::notify::{Notice, NoticeKind};
use crate::window::{CommandArg, Invocation, OpenWithItem, ViewModel, WindowId};

impl AppCore {
    /// `CopyPrompt`, `CopySide`, `MoveRight`, `MoveLeft`, `MovePrompt`, `MoveSide`, `MoveBoth`, `CopyTo`, `MoveTo`,
    /// `Rename`, `Touch`, `NewFolder` on the active folder view (§1.9.13).
    pub(super) fn start_verb(&mut self, window: WindowId, inv: &Invocation) {
        self.start_verb_with(window, inv, None);
    }

    /// Copy/Move to Folder of rows dragged from `from` onto a folder row of the other pane (M2 W9c, D-251): the dialog
    /// prefilled with that folder, structure *relative*, its preview first (D-244's flow).
    pub(super) fn start_verb_into(
        &mut self,
        window: WindowId,
        id: CommandId,
        from: Side,
        folder: hikasa_core::Spec,
    ) {
        self.start_verb_with(window, &Invocation::new(id), Some((folder, from)));
    }

    fn start_verb_with(
        &mut self,
        window: WindowId,
        inv: &Invocation,
        into: Option<(hikasa_core::Spec, Side)>,
    ) {
        let service = self.service.clone();
        let scope = self.scope(window);
        let caption = self.tables.caption(scope, inv.id);
        let include_hidden = self.options().file_operations.include_hidden_by_default;
        let arg = match inv.arg {
            Some(CommandArg::Side(side)) => Some(side),
            _ => None,
        };
        let state = self.state.clone();
        let options = VerbOptions {
            arg,
            state: &state,
            include_hidden,
            windows: cfg!(windows),
            into,
        };
        let started = {
            let Some(tab) = self.active_tab_mut(window) else {
                return;
            };
            let view = tab.id;
            match tab.model_mut() {
                Some(ViewModel::Folder(m)) => {
                    m.set_include_hidden(include_hidden);
                    m.start_verb(&service, view, inv.id, &caption, options)
                }
                _ => VerbStart::Nothing,
            }
        };
        if let (VerbStart::Dialog(dialog), Some(w)) = (started, self.window_mut(window)) {
            w.dialogs.push(dialog);
        }
    }

    /// The inline Rename's Enter (the edit of `ui/folder.rs`).
    pub fn commit_inline_rename(&mut self, window: WindowId) {
        let service = self.service.clone();
        let scope = self.scope(window);
        let caption = self.tables.caption(scope, CommandId::Rename);
        if let Some(tab) = self.active_tab_mut(window) {
            let view = tab.id;
            if let Some(ViewModel::Folder(m)) = tab.model_mut() {
                m.commit_inline_rename(&service, view, &caption);
            }
        }
    }

    /// The pane side a `…Side` command of a context menu carries (`None` from the menu bar).
    #[allow(dead_code)]
    pub(super) fn side_arg(inv: &Invocation) -> Option<Side> {
        match inv.arg {
            Some(CommandArg::Side(side)) => Some(side),
            _ => None,
        }
    }
}

/// The view of two specs (Compare To, Open With ▸ *Text Compare* / *Folder Compare*): the kind inferred or forced, the
/// request of `open_view_request` with the specs' exact OS strings (review F9).
fn open_pair(
    svc: &hikasa_session::Service,
    specs: &[hikasa_core::Spec; 2],
    kind: Option<ViewKind>,
) -> Result<crate::home::HomeOpen, ApiError> {
    let kind = hikasa_session::api::infer_view_kind(svc, specs, kind)?;
    let options = hikasa_session::ipc::OpenOptions::default();
    let texts = vec![specs[0].canonical(), specs[1].canonical()];
    let mut request = svc.open_view_request(kind, &texts, &options)?;
    let [left, right] = specs.each_ref().map(hikasa_session::api::spec_os_string);
    match &mut request {
        ViewRequest::Text { request, .. } => {
            request.left = left;
            request.right = right;
        }
        ViewRequest::Folder { request, .. } | ViewRequest::Sync { request, .. } => {
            request.left = left;
            request.right = right;
        }
        ViewRequest::Table { request, .. } => {
            request.left = left;
            request.right = right;
        }
        ViewRequest::TextEdit { request } => request.spec = left,
    }
    Ok(crate::home::HomeOpen {
        kind,
        request,
        options,
        specs: texts,
    })
}

/// Exclude: `new` joins `list`, each mask once (a mask already there, or twice in `new`, is not added again).
pub(crate) fn add_masks(list: &mut Vec<String>, new: &[String]) {
    for mask in new {
        if !list.contains(mask) {
            list.push(mask.clone());
        }
    }
}

/// More selected items than this ask before Open With or the associated application starts them (review F5).
pub const LAUNCH_ASK: usize = 10;

/// What a launch starts.
#[derive(Debug, Clone, PartialEq)]
pub enum LaunchWhat {
    /// An application of `openwith.toml` (its index and description).
    App(usize, String),
    /// Each item's associated application (FC-062).
    Associated,
}

/// A launch of Open With or the associated application, kept by its question until Yes.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    /// The view.
    pub view: ViewId,
    /// The command's caption.
    pub caption: String,
    /// The application.
    pub what: LaunchWhat,
    /// The items with their bases, in tree order.
    pub items: Vec<(hikasa_core::Spec, hikasa_core::Spec)>,
    /// The rows' folders to refresh when the application ends (`refresh_when_finished`), by path.
    pub refresh: Option<Vec<String>>,
}

impl AppCore {
    /// `Ignored`, `Exclude`, `Compare`, `QuickCompare`, `CompareTo`, `OpenWithList`, `AssocApp` (§1.9.13).
    pub(super) fn content_verb(&mut self, window: WindowId, inv: &Invocation) {
        let service = self.service.clone();
        let wake = self.wake();
        let include_hidden = self.options().file_operations.include_hidden_by_default;
        let remembered = |dialog, key| self.state.dialog_text(dialog, key).map(str::to_owned);
        let exclude_what = remembered(EXCLUDE_DIALOG, "what");
        let contents_method = remembered(COMPARE_CONTENTS_DIALOG, "method");
        let contents_show = remembered(COMPARE_CONTENTS_DIALOG, "show_results");
        let quick_method = remembered(QUICK_COMPARE_DIALOG, "method");
        let refresh_app = |index: usize| {
            self.settings
                .openwith
                .value()
                .app
                .get(index)
                .is_some_and(|app| app.refresh_when_finished)
        };
        let refresh_app: Vec<bool> = (0..self.settings.openwith.value().app.len())
            .map(refresh_app)
            .collect();
        let app_names: Vec<String> = self
            .settings
            .openwith
            .value()
            .app
            .iter()
            .map(|a| a.description.clone())
            .collect();
        let scope = self.scope(window);
        let caption = self.tables.caption(scope, inv.id);
        let Some(tab) = self.active_tab_mut(window) else {
            return;
        };
        let view = tab.id;
        let Some(ViewModel::Folder(m)) = tab.model_mut() else {
            return;
        };
        match inv.id {
            CommandId::Ignored => {
                if m.toggle_ignored(&service)
                    && let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut())
                {
                    vs.dirty = true;
                }
            }
            CommandId::Exclude => {
                let Some((model, select)) = m.selection_request(include_hidden) else {
                    return;
                };
                let what = exclude_what
                    .as_deref()
                    .and_then(|w| parse_exclude_what(w).ok())
                    .unwrap_or(ExcludeWhat::Items);
                let dialog = ExcludeDialog::new(view, model, select, what, wake);
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::Compare => {
                let Some((model, select)) = m.selection_request(include_hidden) else {
                    return;
                };
                let method = contents_method
                    .as_deref()
                    .and_then(|w| parse_content_method(w).ok())
                    .unwrap_or_else(|| m.content_method());
                let show = contents_show.as_deref() != Some("false");
                let dialog =
                    CompareContentsDialog::new(&service, view, model, select, method, show, wake);
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::QuickCompare => {
                let Some((model, select)) = m.selection_request(include_hidden) else {
                    return;
                };
                let iu = m.display_options().ignore_unimportant;
                let method = quick_method
                    .as_deref()
                    .and_then(QuickMethod::parse)
                    .filter(|q| matches!(q, QuickMethod::Binary | QuickMethod::Rules))
                    .unwrap_or(QuickMethod::Rules);
                let dialog = QuickCompareDialog::new(view, model, select, method, iu, wake);
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::CompareTo => {
                if let Some(pair) = m.compare_to_pair() {
                    self.open_specs(window, pair, None);
                } else if let Some((first, folder)) = m.compare_to_first() {
                    let mode = if folder {
                        BrowseMode::Folder
                    } else {
                        BrowseMode::File
                    };
                    let start = crate::browse::folder_of(&first.canonical());
                    self.open_browse(window, mode, start, BrowseTarget::CompareTo(view));
                }
            }
            CommandId::OpenWithList => match inv.arg {
                Some(CommandArg::OpenWith(OpenWithItem::TextCompare)) => {
                    if let Some(pair) = m.compare_to_pair() {
                        self.open_specs(window, pair, Some(ViewKind::TextCompare));
                    }
                }
                Some(CommandArg::OpenWith(OpenWithItem::FolderCompare)) => {
                    if let Some(pair) = m.compare_to_pair() {
                        self.open_specs(window, pair, Some(ViewKind::FolderCompare));
                    }
                }
                // FC-062 (Table Compare phase 3): the two selected files in a Table Compare view.
                // M2 W11b (D-313): each selected file in a Text Edit view.
                Some(CommandArg::OpenWith(OpenWithItem::TextEdit)) => {
                    let specs: Vec<String> = m
                        .selected_specs()
                        .into_iter()
                        .map(|(spec, _)| spec.canonical())
                        .collect();
                    for spec in specs {
                        self.open_text_edit(window, spec, None);
                    }
                }
                Some(CommandArg::OpenWith(OpenWithItem::TableCompare)) => {
                    if let Some(pair) = m.compare_to_pair() {
                        self.open_specs(window, pair, Some(ViewKind::TableCompare));
                    }
                }
                Some(CommandArg::OpenWith(OpenWithItem::App(index))) => {
                    let items = m.selected_specs();
                    // Paths, not ids: the model may be renumbered before the application ends (review F10).
                    let refresh = refresh_app
                        .get(index)
                        .copied()
                        .unwrap_or(false)
                        .then(|| m.selected_parent_paths());
                    let name = app_names.get(index).cloned().unwrap_or_default();
                    self.launch_or_ask(
                        window,
                        Launch {
                            view,
                            caption,
                            what: LaunchWhat::App(index, name),
                            items,
                            refresh,
                        },
                    );
                }
                _ => {}
            },
            CommandId::AssocApp => {
                let items = m.selected_specs();
                self.launch_or_ask(
                    window,
                    Launch {
                        view,
                        caption,
                        what: LaunchWhat::Associated,
                        items,
                        refresh: None,
                    },
                );
            }
            _ => {}
        }
    }

    /// Open With an application or the associated one (review F5): more than [`LAUNCH_ASK`] items ask first, with the
    /// count; then the launch job.
    fn launch_or_ask(&mut self, window: WindowId, launch: Launch) {
        let count = launch.items.len();
        if count == 0 {
            return;
        }
        if count <= LAUNCH_ASK {
            self.launch(launch);
            return;
        }
        let with = match &launch.what {
            LaunchWhat::App(_, name) => format!("with {name}"),
            LaunchWhat::Associated => "with their associated applications".to_owned(),
        };
        let prompt = Prompt::confirm(
            launch.caption.clone(),
            format!("Open {count} items {with}?"),
            None,
            vec![Followup::Launch(Box::new(launch))],
        );
        self.push_dialog(window, Box::new(prompt));
    }

    /// Starts the items one after the other in a job: a Stop or the first that cannot start ends it (the error names
    /// it); folders open as files do.
    pub(crate) fn launch(&mut self, launch: Launch) {
        let Launch {
            view,
            caption,
            what,
            items,
            refresh,
        } = launch;
        let job = match what {
            LaunchWhat::App(index, _) => Job::spawn(
                &self.service,
                JobKind::VfsRead,
                caption.clone(),
                &self.wake(),
                move |svc, job| {
                    let target = OpenTarget::App(index);
                    let request = OpenWithRequest {
                        target: target.clone(),
                        items,
                        line: None,
                    };
                    let commands = svc.open_with_commands(&request, job)?;
                    let waits = svc.open_with_waits(&target)?;
                    svc.open_with_run(&commands, waits, job)
                },
            ),
            LaunchWhat::Associated => Job::spawn(
                &self.service,
                JobKind::VfsRead,
                caption.clone(),
                &self.wake(),
                move |svc, job| {
                    let count = items.len();
                    for (i, (spec, _)) in items.iter().enumerate() {
                        job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                        svc.open_associated(spec, job)
                            .map_err(|e| match count - i - 1 {
                                0 => e,
                                1 => ApiError::InvalidArgument(format!(
                                    "{e}; the item after it was not opened"
                                )),
                                rest => ApiError::InvalidArgument(format!(
                                    "{e}; the {rest} items after it were not opened"
                                )),
                            })?;
                    }
                    Ok(())
                },
            ),
        };
        self.jobs.push(CoreJob::Launch(view, caption, refresh, job));
    }

    pub(super) fn push_dialog(
        &mut self,
        window: WindowId,
        dialog: Box<dyn crate::dialog::DialogModel>,
    ) {
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(dialog);
        }
    }

    /// Opens a view of two items of the model (`kind` forces the view type, else it is inferred as for two dropped
    /// paths) in `window`. The view's request takes the specs as OS strings (`spec_os_string`): a name that is not valid
    /// Unicode reaches the view as it is on disk (review F9), where the IPC-shaped open refuses it.
    pub(super) fn open_specs(
        &mut self,
        window: WindowId,
        specs: [hikasa_core::Spec; 2],
        kind: Option<ViewKind>,
    ) {
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "open the items".into(),
            &self.wake(),
            move |svc, _| open_pair(svc, &specs, kind),
        );
        self.jobs.push(CoreJob::Drop(window, job));
    }

    /// Compare To's second item came from the browse dialog: the first is the view's selected item.
    pub(super) fn compare_to_browsed(&mut self, window: WindowId, view: ViewId, second: String) {
        let first = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.compare_to_first(),
            _ => None,
        };
        let Some((first, _)) = first else {
            return;
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "open the items".into(),
            &self.wake(),
            move |svc, _| {
                let second = svc.parse_spec(std::ffi::OsStr::new(&second))?;
                open_pair(svc, &[first, second], None)
            },
        );
        self.jobs.push(CoreJob::Drop(window, job));
    }

    /// An application of Open With (or the associated one) ended: a launch error is a warning notice with the core
    /// message; with `refresh_when_finished` the rows' folders are refreshed.
    pub(super) fn launched(
        &mut self,
        view: ViewId,
        caption: &str,
        refresh: Option<Vec<String>>,
        result: Result<(), hikasa_session::ApiError>,
    ) {
        let window = self.window_of(view).unwrap_or_else(|| self.focused());
        match result {
            Ok(()) => {
                if let Some(paths) = refresh {
                    let service = self.service.clone();
                    if let Some(ViewModel::Folder(m)) =
                        self.tab_mut(view).and_then(|t| t.model_mut())
                    {
                        m.refresh_after_launch(&service, &paths);
                    }
                }
            }
            Err(hikasa_session::ApiError::Cancelled) => {}
            // The first item that could not start stopped the rest (review F5): an error notice that names it.
            Err(e) => {
                let notice = Notice::new(
                    format!("launch:{}", view.0),
                    NoticeKind::Error,
                    format!("{caption}: {e}"),
                );
                self.notify(window, notice, caption, LogKind::Error);
            }
        }
    }

    /// A content dialog left the stack: Exclude's masks join the view's name filters and the view runs again;
    /// Compare Contents asks first with `confirm_content_compare`, then compares; Quick Compare's *Open* opens its row.
    /// True when `closed` was one of them.
    pub(super) fn content_dialog_closed(
        &mut self,
        closed: &mut Box<dyn crate::dialog::DialogModel>,
    ) -> bool {
        if let Some(d) = closed.as_any_mut().downcast_mut::<ExcludeDialog>() {
            let word = d.what_word();
            self.state.set_dialog_text(EXCLUDE_DIALOG, "what", word);
            self.update_state(move |s| s.set_dialog_text(EXCLUDE_DIALOG, "what", word));
            if let Some((files, folders)) = d.take_chosen() {
                let view = d.view;
                self.reopen_changed(view, "Exclude", None, move |_, session| {
                    let rules = match &mut session.rules {
                        SessionRules::Folder(rules) => rules,
                        SessionRules::Sync(rules) => &mut rules.folder,
                        _ => return Ok(()),
                    };
                    let names = &mut rules.filters.names;
                    add_masks(&mut names.exclude_files, &files);
                    add_masks(&mut names.exclude_folders, &folders);
                    Ok(())
                });
            }
            return true;
        }
        if let Some(d) = closed.as_any_mut().downcast_mut::<CompareContentsDialog>() {
            let view = d.view;
            if let Some((method, show, count)) = d.take_chosen() {
                let (word, shown) = (method.as_str(), if show { "true" } else { "false" });
                self.state
                    .set_dialog_text(COMPARE_CONTENTS_DIALOG, "method", word);
                self.state
                    .set_dialog_text(COMPARE_CONTENTS_DIALOG, "show_results", shown);
                self.update_state(move |s| {
                    s.set_dialog_text(COMPARE_CONTENTS_DIALOG, "method", word);
                    s.set_dialog_text(COMPARE_CONTENTS_DIALOG, "show_results", shown);
                });
                let window = self.window_of(view).unwrap_or_else(|| self.focused());
                if self.options().file_operations.confirm_content_compare {
                    let message = format!(
                        "Compare the contents of {} file pairs ({} bytes)?",
                        count.pairs, count.bytes
                    );
                    let prompt = Prompt::confirm(
                        "Compare Contents",
                        message,
                        Some("file_operations.confirm_content_compare"),
                        vec![Followup::Contents {
                            view,
                            method,
                            show_results: show,
                        }],
                    );
                    self.push_dialog(window, Box::new(prompt));
                } else {
                    self.start_contents(view, method, show);
                }
            }
            return true;
        }
        if let Some(d) = closed.as_any_mut().downcast_mut::<QuickCompareDialog>() {
            let word = d.method_word();
            self.state
                .set_dialog_text(QUICK_COMPARE_DIALOG, "method", word);
            self.update_state(move |s| s.set_dialog_text(QUICK_COMPARE_DIALOG, "method", word));
            if let Some(node) = d.take_open() {
                let view = d.view;
                if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
                    m.select_node(node);
                }
                if let Some(window) = self.window_of(view) {
                    let _ = self.execute(window, Invocation::new(CommandId::Open));
                }
            }
            return true;
        }
        false
    }

    /// Runs Compare Contents on the view (after the question, or without it).
    pub(super) fn start_contents(
        &mut self,
        view: ViewId,
        method: hikasa_core::ContentMethod,
        show: bool,
    ) {
        let service = self.service.clone();
        let window = self.window_of(view).unwrap_or_else(|| self.focused());
        let caption = self.tables.caption(self.scope(window), CommandId::Compare);
        if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
            m.start_contents(&service, view, &caption, method, show);
        }
    }
}

impl AppCore {
    /// The applications of `openwith.toml` by description (Open With ▸).
    pub fn open_with_names(&self) -> Vec<String> {
        self.settings
            .openwith
            .value()
            .app
            .iter()
            .map(|app| app.description.clone())
            .collect()
    }
}

impl AppCore {
    /// The Synchronize submenu, Run Sync, the overrides, the conflicts, Sync/Compare Base Folders (§1.9.14, §1.9.15).
    pub(super) fn sync_command(&mut self, window: WindowId, inv: &Invocation) {
        let service = self.service.clone();
        let scope = self.scope(window);
        let caption = self.tables.caption(scope, inv.id);
        let mode = match self.options().file_operations.sync_confirmations {
            SyncConfirmations::Prompt => ConfirmMode::Prompt,
            SyncConfirmations::YesToAll => ConfirmMode::YesToAll,
            SyncConfirmations::NoToAll => ConfirmMode::NoToAll,
        };
        let defaults = self.sync_defaults();
        let Some(tab) = self.active_tab_mut(window) else {
            return;
        };
        let view = tab.id;
        let Some(ViewModel::Folder(m)) = tab.model_mut() else {
            return;
        };
        let mut dirty = false;
        match inv.id {
            id if crate::folder_view::sync::preset_of(id).is_some() => {
                if let Some(dialog) = m.start_sync_dialog(view, id, &caption, &defaults, mode) {
                    self.push_dialog(window, dialog);
                }
                return;
            }
            CommandId::Run => {
                m.run_sync(&service, view, &caption);
            }
            id if crate::folder_view::sync::override_of(id).is_some() => {
                if let Some(action) = crate::folder_view::sync::override_of(id) {
                    dirty = m.override_selected(&service, action);
                }
            }
            CommandId::ConflictNext | CommandId::ConflictPrev => {
                m.conflict_step(inv.id == CommandId::ConflictNext);
            }
            CommandId::SyncBaseView | CommandId::CompareBaseView => {
                let (request, _) = m.duplicate();
                let specs = m.specs();
                let (compare, display) = match request {
                    ViewRequest::Folder {
                        request, display, ..
                    }
                    | ViewRequest::Sync {
                        request, display, ..
                    } => (request, display),
                    ViewRequest::Text { .. }
                    | ViewRequest::Table { .. }
                    | ViewRequest::TextEdit { .. } => return,
                };
                let (kind, request) = if inv.id == CommandId::SyncBaseView {
                    (
                        ViewKind::FolderSync,
                        ViewRequest::Sync {
                            request: compare,
                            options: defaults,
                            allow_empty_source: false,
                            display,
                        },
                    )
                } else {
                    (
                        ViewKind::FolderCompare,
                        ViewRequest::Folder {
                            request: compare,
                            display,
                            expand_all: false,
                        },
                    )
                };
                self.open_local(
                    kind,
                    request,
                    &hikasa_session::ipc::OpenOptions::default(),
                    specs,
                    crate::app::OpenTarget::Window(window),
                );
                return;
            }
            _ => return,
        }
        if dirty && let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
            vs.dirty = true;
        }
    }

    /// The sync options of the `folder-sync` defaults (Synchronize ▸, Sync Base Folders).
    pub(super) fn sync_defaults(&self) -> SyncOptions {
        match self
            .service
            .session_defaults(SessionType::FolderSync)
            .map(|s| s.rules)
        {
            Ok(SessionRules::Sync(rules)) => rules.sync,
            _ => SyncOptions::default(),
        }
    }

    /// The Synchronize preview left the stack: Run hands its plan to the view, *Plan again* runs the command again.
    /// `None` when `closed` was not it.
    pub(super) fn sync_dialog_closed(
        &mut self,
        closed: &mut Box<dyn crate::dialog::DialogModel>,
    ) -> Option<Vec<crate::app::Effect>> {
        let d = closed
            .as_any_mut()
            .downcast_mut::<crate::folder_view::sync::SyncDialog>()?;
        let (view, started, again, command) = (d.view, d.take_started(), d.plan_again(), d.command);
        let service = self.service.clone();
        if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
            m.sync_dialog_closed(&service, view, started);
        }
        // *Plan again* (review F11): the command runs once more on the comparison as it stands now.
        if again && let Some(window) = self.window_of(view) {
            return Some(self.execute(window, crate::window::Invocation::new(command)));
        }
        Some(Vec::new())
    }
}

impl AppCore {
    /// The preset of a Folder Sync view's header (FS-001): the plan is made again; the session becomes dirty.
    pub fn sync_preset(&mut self, view: ViewId, preset: hikasa_folder::sync::SyncPreset) {
        let service = self.service.clone();
        let changed = match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => m.set_sync_preset(&service, preset),
            _ => false,
        };
        if changed && let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
            vs.dirty = true;
        }
    }
}
