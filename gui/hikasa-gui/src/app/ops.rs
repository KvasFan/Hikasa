//! The verbs of a folder view from the window's side (M2 W9b, D-244…D-246): each command of §1.9.13 starts on the
//! active view with what the window knows (the remembered choices, `include_hidden_by_default`, the pane of a context
//! menu), its dialog goes on the window's stack, and the dialogs that leave come back to their view
//! (`AppCore::dialog_closed`).

use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::ConfirmMode;
use hikasa_folder::select::ExcludeWhat;
use hikasa_folder::sync::SyncOptions;
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
                    self.open_specs(window, pair.to_vec(), None);
                } else if let Some((first, folder)) = m.compare_to_first() {
                    let mode = if folder {
                        BrowseMode::Folder
                    } else {
                        BrowseMode::File
                    };
                    let start = crate::browse::folder_of(&first);
                    self.open_browse(window, mode, start, BrowseTarget::CompareTo(view));
                }
            }
            CommandId::OpenWithList => match inv.arg {
                Some(CommandArg::OpenWith(OpenWithItem::TextCompare)) => {
                    if let Some(pair) = m.compare_to_pair() {
                        self.open_specs(window, pair.to_vec(), Some(ViewKind::TextCompare));
                    }
                }
                Some(CommandArg::OpenWith(OpenWithItem::FolderCompare)) => {
                    if let Some(pair) = m.compare_to_pair() {
                        self.open_specs(window, pair.to_vec(), Some(ViewKind::FolderCompare));
                    }
                }
                Some(CommandArg::OpenWith(OpenWithItem::App(index))) => {
                    let items = m.selected_specs();
                    let parents = m.selected_parents();
                    let refresh = refresh_app
                        .get(index)
                        .copied()
                        .unwrap_or(false)
                        .then_some(parents);
                    let job = Job::spawn(
                        &service,
                        JobKind::VfsRead,
                        caption.clone(),
                        &wake,
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
                    );
                    self.jobs.push(CoreJob::Launch(view, caption, refresh, job));
                }
                _ => {}
            },
            CommandId::AssocApp => {
                let items = m.selected_specs();
                if items.is_empty() {
                    return;
                }
                let job = Job::spawn(
                    &service,
                    JobKind::VfsRead,
                    caption.clone(),
                    &wake,
                    move |svc, job| {
                        for (spec, _) in &items {
                            svc.open_associated(spec, job)?;
                        }
                        Ok(())
                    },
                );
                self.jobs.push(CoreJob::Launch(view, caption, None, job));
            }
            _ => {}
        }
    }

    fn push_dialog(&mut self, window: WindowId, dialog: Box<dyn crate::dialog::DialogModel>) {
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(dialog);
        }
    }

    /// Opens a view of `specs` (resolved as two dropped paths are; `kind` forces the view type) in `window`.
    pub(super) fn open_specs(
        &mut self,
        window: WindowId,
        specs: Vec<String>,
        kind: Option<ViewKind>,
    ) {
        let request = hikasa_session::api::GuiOpenRequest {
            specs: specs.into_iter().map(std::ffi::OsString::from).collect(),
            kind,
            ..hikasa_session::api::GuiOpenRequest::default()
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "open the items".into(),
            &self.wake(),
            move |svc, _| crate::home::resolve(svc, &request),
        );
        self.jobs.push(CoreJob::Drop(window, job));
    }

    /// Compare To's second item came from the browse dialog: the first is the view's selected item.
    pub(super) fn compare_to_browsed(&mut self, window: WindowId, view: ViewId, second: String) {
        let first = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.compare_to_first(),
            _ => None,
        };
        if let Some((first, _)) = first {
            self.open_specs(window, vec![first, second], None);
        }
    }

    /// An application of Open With (or the associated one) ended: a launch error is a warning notice with the core
    /// message; with `refresh_when_finished` the rows' folders are refreshed.
    pub(super) fn launched(
        &mut self,
        view: ViewId,
        caption: &str,
        refresh: Option<Vec<hikasa_folder::NodeId>>,
        result: Result<(), hikasa_session::ApiError>,
    ) {
        let window = self.window_of(view).unwrap_or_else(|| self.focused());
        match result {
            Ok(()) => {
                if let Some(nodes) = refresh {
                    let service = self.service.clone();
                    if let Some(ViewModel::Folder(m)) =
                        self.tab_mut(view).and_then(|t| t.model_mut())
                    {
                        m.refresh_after_launch(&service, nodes);
                    }
                }
            }
            Err(e) => {
                let notice = Notice::new(
                    format!("launch:{}", view.0),
                    NoticeKind::Warning,
                    format!("{caption}: {e}"),
                );
                self.notify(window, notice, caption, LogKind::Warning);
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
                    for (list, new) in [
                        (&mut names.exclude_files, &files),
                        (&mut names.exclude_folders, &folders),
                    ] {
                        for mask in new {
                            if !list.contains(mask) {
                                list.push(mask.clone());
                            }
                        }
                    }
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
                    ViewRequest::Text { .. } => return,
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
