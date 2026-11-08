//! The window's side of what a folder view's panes add in M2 W9c (§1.9.16–§1.9.18): `Copy`, `Paste`, `SelectAll` and
//! `CopyFilenames` through the GUI's own clipboard (D-252), the drops of paths and of rows (D-251) and their prompt's
//! answer, the file-manager entries (D-254), the pane context menus (D-253) and the rule tabs' openers (D-249).
//! Clipboard reads and writes and the file manager run in jobs (D-233 rule 3).

use std::ffi::OsString;
use std::path::PathBuf;

use hikasa_core::{JobKind, Side};
use hikasa_folder::NodeId;
use hikasa_session::clipboard::{
    ClipboardContent, ClipboardError, ClipboardPayload, ClipboardProvider as _,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::log::LogKind;

use super::{AppCore, CoreJob, Effect};
use crate::commands::CommandId;
use crate::dnd::{RowDrag, RowDrop, RowTarget, decide_rows};
use crate::drop_dialog::{DROP_DIALOG, DropDialog, overwrite_of, overwrite_word};
use crate::folder_view::pane::NOT_LOCAL;
use crate::job::Job;
use crate::notify::{Notice, NoticeKind};
use crate::status::Message;
use crate::window::{CommandArg, ExplorerAction, ExplorerItem, Invocation, ViewModel, WindowId};

/// What a pane's job ended with.
pub(crate) enum PaneDone {
    /// `Copy` placed its files.
    Copied,
    /// `Copy Filename` placed its lines.
    Named,
    /// `Paste` read these sources for this folder.
    Pasted {
        /// Where they go.
        into: crate::drop_dialog::DropInto,
        /// The paths or specs.
        sources: Vec<OsString>,
    },
    /// A file-manager entry ran.
    Shown,
}

/// The message of a paste whose clipboard holds no file and no spec.
pub const NOTHING_TO_PASTE: &str = "The clipboard holds no files to paste.";

/// The message of rows dragged onto a folder whose name is not valid Unicode (review F5).
pub const LOSSY_FOLDER: &str =
    "Rows cannot be dropped into a folder whose name is not valid Unicode.";

/// The sources of a clipboard text (D-252): core's [`hikasa_session::clipboard::pasted_paths`], absolute local paths
/// and `file://` URIs of this computer only (review F7).
pub(crate) fn text_sources(text: &str) -> Vec<OsString> {
    hikasa_session::clipboard::pasted_paths(text)
        .into_iter()
        .map(PathBuf::into_os_string)
        .collect()
}

impl AppCore {
    /// The W9c commands of a folder pane (D-252, D-254, D-249, D-250); `None` for every other command.
    pub(super) fn pane_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        let side = match inv.arg {
            Some(CommandArg::Side(side)) => Some(side),
            _ => None,
        };
        match inv.id {
            CommandId::Copy => self.copy_files(window, side),
            CommandId::CopyFilenames => self.copy_filenames(window),
            CommandId::Paste => self.paste(window, side),
            CommandId::SelectAll => {
                if let Some(ViewModel::Folder(m)) =
                    self.active_tab_mut(window).and_then(|t| t.model_mut())
                {
                    m.select_all();
                }
            }
            CommandId::ExplorerList => {
                let item = match inv.arg {
                    Some(CommandArg::Explorer(item)) => item,
                    _ => ExplorerItem {
                        action: ExplorerAction::ShowItem,
                        side,
                    },
                };
                self.explorer(window, item);
            }
            CommandId::Rules => self.edit_view_on(window, Some(crate::folder_tabs::COMPARISON)),
            CommandId::NameFilter => {
                self.edit_view_on(window, Some(crate::folder_tabs::NAME_FILTERS))
            }
            CommandId::NameFilterEdit => {
                if let Some(w) = self.window_mut(window) {
                    w.focus_filters = true;
                }
            }
            _ => return None,
        }
        Some(Vec::new())
    }

    /// The active folder view of a window.
    fn active_folder(&self, window: WindowId) -> Option<ViewId> {
        let tab = self.active_tab(window)?;
        matches!(tab.model(), Some(ViewModel::Folder(_))).then_some(tab.id)
    }

    /// `Copy` (D-252): the selected local items of the focus sides (or `side`), tree order, as a file list; the others
    /// are left out with a message; nothing local, nothing written.
    fn copy_files(&mut self, window: WindowId, side: Option<Side>) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        let (paths, left_out) = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.clipboard_paths(side),
            _ => return,
        };
        if left_out > 0 {
            self.set_message(view, Message::info(NOT_LOCAL));
        }
        if paths.is_empty() {
            return;
        }
        let clipboard = std::sync::Arc::clone(&self.script_clipboard);
        let ticket = clipboard.ticket();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy to the clipboard".into(),
            &self.wake(),
            move |_, _| {
                clipboard
                    .in_turn(ticket, |c| c.write_files(&paths))
                    .map_err(|e| hikasa_session::ApiError::Vfs(e.to_vfs_write()))?;
                Ok(PaneDone::Copied)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    /// `Copy Filename` (FC-063): the selected items' specs as text, one a line.
    fn copy_filenames(&mut self, window: WindowId) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        let text = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.filenames_text(cfg!(windows)),
            _ => return,
        };
        if text.is_empty() {
            return;
        }
        let clipboard = std::sync::Arc::clone(&self.script_clipboard);
        let ticket = clipboard.ticket();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy file names".into(),
            &self.wake(),
            move |_, _| {
                clipboard
                    .in_turn(ticket, |c| {
                        c.write_payload(&ClipboardPayload { text, html: None })
                    })
                    .map_err(|e| hikasa_session::ApiError::Vfs(e.to_vfs_write()))?;
                Ok(PaneDone::Named)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    /// `Paste` (D-252): the clipboard's files (else its text's lines that are specs) dropped on the pane's current
    /// folder: the drop's prompt, plan and run.
    fn paste(&mut self, window: WindowId, side: Option<Side>) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        if self.refuse_busy(view) {
            return;
        }
        let into = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.paste_into(view, side),
            _ => return,
        };
        let into = match into {
            Ok(into) => into,
            Err(message) => {
                self.set_message(view, Message::warning(message));
                return;
            }
        };
        let clipboard = std::sync::Arc::clone(&self.script_clipboard);
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "read the clipboard".into(),
            &self.wake(),
            move |_, _| {
                let sources: Vec<OsString> = match clipboard.read_files() {
                    Ok(paths) => paths.into_iter().map(PathBuf::into_os_string).collect(),
                    Err(ClipboardError::Failed(why)) => {
                        return Err(hikasa_session::ApiError::Vfs(
                            ClipboardError::Failed(why).to_vfs(),
                        ));
                    }
                    Err(_) => match clipboard.read() {
                        Ok(ClipboardContent::Text(text)) => text_sources(&text),
                        Ok(_) | Err(ClipboardError::Empty | ClipboardError::Unavailable(_)) => {
                            Vec::new()
                        }
                        Err(e) => return Err(hikasa_session::ApiError::Vfs(e.to_vfs())),
                    },
                };
                Ok(PaneDone::Pasted { into, sources })
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    /// A file-manager entry (D-254) on the current item of `item.side` (else the focus side).
    fn explorer(&mut self, window: WindowId, item: ExplorerItem) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        let target = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.explorer_item(item.side),
            _ => return,
        };
        let Some((spec, folder)) = target else {
            return;
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            item.action.caption().to_owned(),
            &self.wake(),
            move |svc, job| {
                match item.action {
                    ExplorerAction::OpenFolder => {
                        svc.open_folder_in_file_manager(&spec, folder, job)?
                    }
                    ExplorerAction::ShowItem => svc.reveal_in_file_manager(&spec, job)?,
                }
                Ok(PaneDone::Shown)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    /// A pane's job ended.
    pub(super) fn pane_done(
        &mut self,
        window: WindowId,
        view: ViewId,
        result: Result<PaneDone, hikasa_session::ApiError>,
    ) -> Vec<Effect> {
        match result {
            Ok(PaneDone::Copied | PaneDone::Named | PaneDone::Shown) => Vec::new(),
            Ok(PaneDone::Pasted { into, sources }) => {
                if sources.is_empty() {
                    self.set_message(view, Message::info(NOTHING_TO_PASTE));
                    return Vec::new();
                }
                self.open_drop(window, into, sources, false)
            }
            Err(e) => {
                let notice = Notice::new(
                    format!("pane:{}", view.0),
                    NoticeKind::Warning,
                    e.to_string(),
                );
                self.notify(window, notice, "clipboard", LogKind::Warning);
                Vec::new()
            }
        }
    }

    /// Paths dropped on a pane's path edit or header (D-251): the path edits take them, then Accept.
    pub(super) fn drop_bases(&mut self, window: WindowId, bases: Vec<(Side, String)>) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
            for (side, text) in bases {
                let k = usize::from(side == Side::Right);
                m.path_edit(k).set_text(&text);
            }
            m.accept_paths();
        }
    }

    /// Paths dropped on a pane's tree (D-251): the prompt over the plans of a copy and a move into the folder.
    pub(super) fn drop_transfer(
        &mut self,
        window: WindowId,
        side: Side,
        folder: Option<NodeId>,
        sources: Vec<OsString>,
        moving: bool,
    ) -> Vec<Effect> {
        let Some(view) = self.active_folder(window) else {
            return Vec::new();
        };
        if self.refuse_busy(view) {
            return Vec::new();
        }
        let into = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.drop_into(view, side, folder),
            _ => return Vec::new(),
        };
        match into {
            Ok(into) => self.open_drop(window, into, sources, moving),
            Err(message) => {
                self.set_message(view, Message::warning(message));
                Vec::new()
            }
        }
    }

    /// Paths dropped on a folder view where the window could not say which pane (W9c review F4: winit gives no
    /// pointer position during an OS drag): the prompt asks for the pane, the focus pane's current folder first, and
    /// offers W4's *Open as new comparison* for one or two paths. Never a silent fallback.
    pub(super) fn drop_ask(
        &mut self,
        window: WindowId,
        sources: Vec<OsString>,
        moving: bool,
    ) -> Vec<Effect> {
        let Some(view) = self.active_folder(window) else {
            return Vec::new();
        };
        if self.refuse_busy(view) {
            return Vec::new();
        }
        let (sides, first, refused) = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => (
                [Side::Left, Side::Right].map(|side| m.paste_into(view, Some(side))),
                if m.selection().focus == hikasa_folder::select::SideSet::RIGHT {
                    Side::Right
                } else {
                    Side::Left
                },
                m.move_refused(&sources),
            ),
            _ => return Vec::new(),
        };
        if !self.start_drop_prompt(view) {
            return Vec::new();
        }
        let overwrite = overwrite_of(self.state.dialog_text(DROP_DIALOG, "overwrite"));
        let dialog = DropDialog::asking(
            view,
            sides,
            first,
            sources,
            overwrite,
            moving,
            refused,
            self.wake(),
        );
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
        vec![Effect::Raise(window)]
    }

    /// The view waits for the drop's prompt (false: it is busy; the window said why).
    fn start_drop_prompt(&mut self, view: ViewId) -> bool {
        if self.refuse_busy(view) {
            return false;
        }
        match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => m.start_drop_prompt(),
            _ => false,
        }
    }

    /// Opens the drop's prompt over its view (the view waits for its answer). A source on a read-only side leaves
    /// *Copy* only (review F1).
    fn open_drop(
        &mut self,
        window: WindowId,
        into: crate::drop_dialog::DropInto,
        sources: Vec<OsString>,
        moving: bool,
    ) -> Vec<Effect> {
        let view = into.view;
        let refused = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.move_refused(&sources),
            _ => None,
        };
        if !self.start_drop_prompt(view) {
            return Vec::new();
        }
        let overwrite = overwrite_of(self.state.dialog_text(DROP_DIALOG, "overwrite"));
        let dialog = DropDialog::new(into, sources, overwrite, moving, refused, self.wake());
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
        vec![Effect::Raise(window)]
    }

    /// The drop's prompt closed: the plan of its answer runs in its view (a move checked again first); the overwrite
    /// choice is remembered; *Open as new comparison* applies W4's window rule to the paths.
    pub(super) fn drop_closed(&mut self, dialog: &mut DropDialog) -> Vec<Effect> {
        let (view, answer) = (dialog.view(), dialog.take_answer());
        if answer.is_some() {
            let word = overwrite_word(dialog.overwrite);
            self.state.set_dialog_text(DROP_DIALOG, "overwrite", word);
            self.update_state(move |s| s.set_dialog_text(DROP_DIALOG, "overwrite", word));
        }
        let service = self.service.clone();
        let refused = match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => m.drop_answered(&service, answer).err(),
            _ => None,
        };
        if let Some(message) = refused {
            self.set_message(view, Message::warning(message));
        }
        match (dialog.take_open_new(), self.window_of(view)) {
            (Some(paths), Some(window)) => self.drop_paths(
                window,
                crate::dnd::DropTarget::Window,
                paths.into_iter().map(PathBuf::from).collect(),
            ),
            _ => Vec::new(),
        }
    }

    /// Rows dragged inside the active folder view were released (D-251, §1.9.16's table): to the other side through
    /// `CopySide`/`MoveSide`, into a folder row of the other pane through Copy/Move to Folder prefilled (*relative*),
    /// else nothing. The rows must still be the selection.
    pub fn drop_rows(
        &mut self,
        window: WindowId,
        drag: &RowDrag,
        target: RowTarget,
        shift: bool,
    ) -> Vec<Effect> {
        let Some(view) = self.active_folder(window) else {
            return Vec::new();
        };
        let same = match self.tab(view).and_then(|t| t.model()) {
            Some(ViewModel::Folder(m)) => m.selection().items == drag.rows,
            _ => false,
        };
        if !same {
            return Vec::new();
        }
        if matches!(decide_rows(drag, target, shift), RowDrop::Nothing) {
            return Vec::new();
        }
        if self.refuse_busy(view) {
            return Vec::new();
        }
        match decide_rows(drag, target, shift) {
            RowDrop::Nothing => Vec::new(),
            RowDrop::ToOtherSide { from, moving } => {
                let id = if moving {
                    CommandId::MoveSide
                } else {
                    CommandId::CopySide
                };
                self.execute(window, Invocation::with(id, CommandArg::Side(from)))
            }
            RowDrop::IntoFolder {
                from,
                folder,
                moving,
            } => {
                // The folder goes to the dialog as its spec, never as text read back (review F5); a name that is not
                // valid Unicode cannot be shown or typed back, so it is refused.
                let target = match self.tab(view).and_then(|t| t.model()) {
                    Some(ViewModel::Folder(m)) => m
                        .drop_into(view, from.opposite(), Some(folder))
                        .map(|into| into.spec),
                    _ => return Vec::new(),
                };
                let target = match target {
                    Ok(spec) if spec.is_lossy() => {
                        self.set_message(view, Message::warning(LOSSY_FOLDER));
                        return Vec::new();
                    }
                    Ok(spec) => spec,
                    Err(message) => {
                        self.set_message(view, Message::warning(message));
                        return Vec::new();
                    }
                };
                let id = if moving {
                    CommandId::MoveTo
                } else {
                    CommandId::CopyTo
                };
                if !self.enabled(window, id) {
                    return Vec::new();
                }
                self.start_verb_into(window, id, from, target);
                Vec::new()
            }
        }
    }

    /// Enter in the Filters edit of the active folder view (D-250), or a preset or recent filter chosen (`mask`): core
    /// parses the text; an error stays under the field and nothing runs; else the view's name filter becomes it, the
    /// view runs again and the text goes to `mru.name_filters`.
    pub fn filters_enter(&mut self, window: WindowId, mask: Option<&str>) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        let entered = match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => {
                let edit = m.filters_edit();
                let result = match mask {
                    Some(mask) => edit.choose(mask),
                    None => edit.enter(),
                };
                result.map(|lists| (lists, edit.text.clone()))
            }
            _ => return,
        };
        let Ok((Some(lists), text)) = entered else {
            return;
        };
        if self.refuse_busy(view) {
            return;
        }
        if !text.is_empty() {
            self.push_mru(hikasa_session::settings::state::MruList::NameFilters, text);
        }
        self.reopen_changed(view, "apply the name filter", None, move |_, session| {
            match &mut session.rules {
                hikasa_session::sessions::SessionRules::Folder(rules) => {
                    rules.filters.names = lists;
                }
                hikasa_session::sessions::SessionRules::Sync(rules) => {
                    rules.folder.filters.names = lists;
                }
                _ => {}
            }
            Ok(())
        });
    }

    /// The Filters edit's drop-down (D-250): the presets of `filters.toml` (name, mask) and the recent filters.
    pub fn filters_choices(&self) -> (Vec<(String, String)>, Vec<String>) {
        (
            self.presets(),
            self.state
                .mru(hikasa_session::settings::state::MruList::NameFilters)
                .to_vec(),
        )
    }

    /// *Add To Presets* of the Filters edit (D-250): the prompt for a name of the field's filter.
    pub fn filters_add_to_presets(&mut self, window: WindowId) {
        let Some(view) = self.active_folder(window) else {
            return;
        };
        let mask = match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => {
                let edit = m.filters_edit();
                match edit.enter() {
                    Ok(_) if !edit.text.is_empty() => edit.text.clone(),
                    _ => return,
                }
            }
            _ => return,
        };
        let dialog = crate::filters_edit::PresetNameDialog::new(mask, self.settings.clone());
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
    }

    /// The context menu of a pane area of the active view (D-253), as the drawing layer builds it (tests).
    #[cfg(test)]
    pub fn pane_menu(
        &self,
        window: WindowId,
        area: crate::commands::Area,
    ) -> Vec<crate::menus::MenuItem> {
        let mut ctx = self.ctx(window);
        ctx.folder_pane = matches!(
            ctx.scope,
            crate::commands::Scope::Folder | crate::commands::Scope::Sync
        );
        let checked = |id: CommandId| self.checked(window, id);
        let names = self.workspace_names();
        let columns = self.column_menu(window);
        let apps = self.open_with_names();
        let mc = crate::menus::MenuContext {
            tables: self.tables(),
            scope: ctx.scope,
            ctx,
            checked: &checked,
            workspaces: &names,
            columns: &columns,
            open_with: &apps,
        };
        crate::menus::pane_menu(&mc, area)
    }

    /// A drop's or a paste's run ended (D-251): the failed items to the log, one notice with the summary.
    #[allow(clippy::too_many_arguments)] // an outcome and where it came from
    pub(super) fn transfer_outcome(
        &mut self,
        window: WindowId,
        view: ViewId,
        ended: Option<&str>,
        source: &str,
        caption: &str,
        outcome: &hikasa_session::api::VfsOpDto,
        moving: bool,
    ) {
        let lines = hikasa_session::api::human::vfs_op_lines(outcome);
        for line in lines.iter().filter(|l| l.starts_with("failed ")) {
            self.log_entry(LogKind::Error, source, line.clone());
        }
        let verb = if moving {
            hikasa_folder::ops::OpVerb::Move
        } else {
            hikasa_folder::ops::OpVerb::Copy
        };
        let summary =
            hikasa_session::api::folder_ops::summary_text(verb, false, &outcome.summary, None);
        let what = match ended {
            None => caption.to_owned(),
            Some(why) => format!("{caption} in {source} ({why})"),
        };
        let text = if outcome.cancelled {
            format!("{what}: stopped; {summary}.")
        } else {
            format!("{what}: {summary}.")
        };
        let kind = if outcome.ok() {
            NoticeKind::Info
        } else {
            NoticeKind::Warning
        };
        let log = if outcome.ok() {
            LogKind::Summary
        } else {
            LogKind::Warning
        };
        self.notify(
            window,
            Notice::new(format!("op:{}", view.0), kind, text),
            source,
            log,
        );
    }
}
