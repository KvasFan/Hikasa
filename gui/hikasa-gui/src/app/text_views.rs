//! The window's side of the text views' panels, search, navigation and Text Edit (M2 W11b; contract §1.11.14–
//! §1.11.24): the dialogs (Go To, Display Font, Find in Files, Info), Clear Session, the clipboard texts (Compare
//! Selection to Clipboard, Open Clipboard), Open, Edit Text File and the Text Edit opens, the session link (Next and
//! Previous Difference Files, Copy File and Open Next), the Find in Files panel (open a hit, Replace in Files) and the
//! changes on disk of the views' inputs and formats. The models are `text/`'s; this file orders their jobs and
//! dialogs.

use std::sync::Arc;

use hikasa_core::{JobKind, Reply, Side};
use hikasa_folder::view::Dir;
use hikasa_session::api::{
    CopyFileExecute, CopyFileReport, CopyFileRequest, CopyFileSteps, NamedText, ParentCompare,
    TextEditRequest, TextFindOutcome, TextFindReplacePlan, TextFindRequest, ViewRequest,
};
use hikasa_session::clipboard::ClipboardContent;
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use hikasa_session::log::LogKind;
use hikasa_session::sessions::ParentLink;
use hikasa_session::settings::state::MruList;

use super::OpenTarget;
use super::{AppCore, CoreJob, Effect, ViewModel};
use crate::browse::{BrowseMode, BrowseTarget};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel, Followup, Prompt};
use crate::job::Job;
use crate::notify::{Notice, NoticeAction, NoticeKind};
use crate::status::Message;
use crate::text::find_files::FindFilesDialog;
use crate::text::layout::DisplayFontDialog;
use crate::text::nav::GoToLineDialog;
use crate::text::{TextRequest, TextViewModel, info, link, marks, nav};
use crate::window::{CommandArg, Invocation, OpenWithItem, Tab, TabContent, WindowId};
use hikasa_session::clipboard::ClipboardProvider as _;

/// The parent comparison a link step works on: the open parent view's, or the parent compared again (D-311).
#[derive(Clone)]
pub struct ParentState {
    /// The prepared comparison.
    pub prepared: hikasa_session::api::PreparedFolderCompare,
    /// The parent's read-only sides (D-184).
    pub read_only: hikasa_folder::ops::ReadOnlySides,
    /// The child's row (`None`: the parent no longer has it).
    pub row: Option<hikasa_folder::NodeId>,
    /// The parent view's display (the next pair skips what it hides).
    pub display: hikasa_folder::DisplayOptions,
}

impl ParentState {
    fn of_compare(parent: ParentCompare) -> ParentState {
        ParentState {
            prepared: parent.prepared,
            read_only: parent.read_only,
            row: parent.row,
            // The parent's view is closed: every difference of the folder structure (D-311).
            display: hikasa_folder::DisplayOptions {
                filter: hikasa_folder::FolderDisplayFilter::All,
                folders: hikasa_folder::FolderMode::default(),
                ignore_unimportant: false,
                suppress_filters: false,
            },
        }
    }
}

/// What a job of this file hands back.
pub enum MoreDone {
    /// The window's clipboard text for Compare Selection to Clipboard (`None`: no text), with the left text.
    ClipboardText(NamedText, Option<String>),
    /// The Service read its clipboard for Open Clipboard (`Err`: no text there).
    ClipboardChecked(Side, Result<(), String>),
    /// The parent folders compared again (a step of the session link, D-311).
    Parent(Box<ParentState>, LinkStep),
    /// The steps of a copy, planned on the parent.
    CopySteps(Box<ParentState>, Side, Box<CopyFileSteps>),
    /// A copy ran.
    Copied(
        Box<CopyFileReport>,
        Box<ParentState>,
        Option<Box<hikasa_text::EditableDocument>>,
    ),
    /// A Replace in Files plan (the files open with unsaved edits left out).
    ReplacePlan(Box<TextFindReplacePlan>),
    /// A Replace in Files ran.
    Replaced(Box<TextFindOutcome>),
}

/// What a parent comparison was made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStep {
    /// Next (`true`) or previous difference files.
    Next(bool),
    /// Copy File from this side and Open Next.
    Copy(Side),
}

/// The answers of this file's prompts.
#[derive(Debug, Clone, PartialEq)]
pub enum MoreFollowup {
    /// *Copy File and Open Next*'s confirmation.
    CopyFile {
        /// The view.
        view: ViewId,
        /// Yes.
        yes: bool,
    },
    /// An answer to a Replace in Files question.
    ReplaceReply {
        /// The window.
        window: WindowId,
        /// The answer.
        reply: Reply,
    },
    /// *Reload* or *Keep mine* of a change on disk (D-315).
    Disk {
        /// The view.
        view: ViewId,
        /// The side.
        side: Side,
        /// Reload (else Keep mine).
        reload: bool,
    },
    /// *Reopen* of a format changed on disk (FF-004).
    FormatReopen(ViewId),
}

/// The session link's waiting state of a view.
pub struct LinkRun {
    /// The parent comparison of the last step (kept for the step that asked D-301 first).
    parent: Option<Box<ParentState>>,
    /// The planned copy waiting for its confirmation.
    steps: Option<(Side, Box<CopyFileSteps>)>,
}

/// A Replace in Files waiting for its questions (per window).
pub struct FilesReplace {
    plan: Box<TextFindReplacePlan>,
    driver: hikasa_session::api::ReplaceDriver,
}

impl AppCore {
    /// The W11b commands of a text view, and `EditTextFile` from any view (`None`: not one of them).
    pub(super) fn text_more_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
        confirmed: bool,
    ) -> Option<Vec<Effect>> {
        if inv.id == CommandId::EditTextFile {
            let start = self
                .state
                .mru(MruList::Files)
                .first()
                .cloned()
                .unwrap_or_default();
            let start = crate::browse::folder_of(&start);
            self.open_browse(window, BrowseMode::File, start, BrowseTarget::EditTextFile);
            return Some(Vec::new());
        }
        let view = self.active_text(window)?;
        let service = self.service.clone();
        match inv.id {
            CommandId::Info => {
                let m = self.text_model(view)?;
                let scope = m.scope();
                let dialog = crate::info_dialog::InfoDialog::with_sections(
                    scope.type_title(),
                    info::sections(m),
                    info::TEXT_INFO_TOPIC,
                    Arc::clone(&self.script_clipboard) as _,
                );
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::SessionClear => {
                let m = self.text_model(view)?;
                if !confirmed && !crate::text::save::modified_sides(m).is_empty() {
                    return Some(self.ask_unsaved_then(window, view, inv.clone()));
                }
                m.clear_session();
                let now = self.timer_now;
                if let Some(tab) = self.tab_mut(view) {
                    tab.session = Some(crate::view_session::ViewSession::unnamed(now));
                }
                self.shared.set_title(view, String::new());
            }
            CommandId::GoTo => {
                let m = self.text_model(view)?;
                let last = nav::last_line(m);
                self.push_dialog(window, Box::new(GoToLineDialog::new(view, last)));
            }
            CommandId::DisplayFont => {
                let families: Vec<String> = crate::fonts::named_families(&self.theme().fonts)
                    .into_iter()
                    .filter(|f| !f.is_empty())
                    .collect();
                let m = self.text_model(view)?;
                let dialog = DisplayFontDialog::new(
                    view,
                    families,
                    m.display().font_family.clone(),
                    m.font_px(),
                );
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::FindInFiles => {
                let m = self.text_model(view)?;
                let roots = view_folders(m);
                let text = m.find.query.text.clone();
                let dialog = FindFilesDialog::new(&self.state, roots, text);
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::BrowseMode => {
                if let Some(CommandArg::BrowseMode(mode)) = inv.arg {
                    let m = self.text_model(view)?;
                    if !m.is_edit()
                        || matches!(
                            mode,
                            hikasa_text::nav::BrowseMode::Edit | hikasa_text::nav::BrowseMode::Find
                        )
                    {
                        m.display_mut().browse_mode = mode;
                    }
                }
            }
            CommandId::BookmarkToggleList | CommandId::BookmarkGoToList => {
                if let Some(CommandArg::Bookmark(n)) = inv.arg {
                    let m = self.text_model(view)?;
                    if inv.id == CommandId::BookmarkToggleList {
                        nav::toggle_bookmark(m, n);
                    } else {
                        nav::go_to_bookmark(m, n);
                    }
                }
            }
            CommandId::ConvertList | CommandId::EndingStyleList => {}
            // The header menu's Encoding ▸ (reopen with it, D-302) and Format ▸ (a relive, D-304).
            CommandId::FileFormat => {
                let m = self.text_model(view)?;
                let side = m.focus();
                match inv.arg.clone() {
                    Some(CommandArg::Encoding(label)) => {
                        m.request_reopen(side, None, Some(Some(label)));
                    }
                    Some(CommandArg::Format(name)) => m.request_format(side, name),
                    _ => {}
                }
            }
            // Ids the folder views run as their own verbs (`Ignored`, `AlignWith`): a text view's model takes them.
            CommandId::Ignored
            | CommandId::AlignWith
            | CommandId::Isolate
            | CommandId::AnchorsClear
            | CommandId::TrimTrailWS
            | CommandId::LeadSpaceToTab
            | CommandId::TabToSpace
            | CommandId::ToWindows
            | CommandId::ToUnix
            | CommandId::ToMac => {
                self.text_model(view)?.execute(&service, inv.id);
            }
            CommandId::CompareToClipboard => {
                let left = marks::clipboard_left(self.text_model(view)?)?;
                let provider = Arc::clone(&self.script_clipboard);
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsRead,
                    "read the clipboard".into(),
                    &self.wake(),
                    move |_, _| {
                        let text = match provider.read() {
                            Ok(ClipboardContent::Text(t)) => Some(t),
                            _ => None,
                        };
                        Ok(MoreDone::ClipboardText(
                            NamedText {
                                name: left.0,
                                text: left.1,
                            },
                            text,
                        ))
                    },
                );
                self.jobs.push(CoreJob::More(window, view, job));
            }
            CommandId::OpenClipboard => {
                let side = self.text_model(view)?.focus();
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsRead,
                    "read the clipboard".into(),
                    &self.wake(),
                    move |svc, _| {
                        let checked = match svc.read_clipboard() {
                            Ok(dto) if dto.kind == "text" => Ok(()),
                            Ok(_) => Err(crate::text::clipboard::NO_TEXT.to_owned()),
                            Err(e) => Err(format!("{}: {e}", crate::text::clipboard::NO_TEXT)),
                        };
                        Ok(MoreDone::ClipboardChecked(side, checked))
                    },
                );
                self.jobs.push(CoreJob::More(window, view, job));
            }
            CommandId::Open | CommandId::OpenLocal => {
                let m = self.text_model(view)?;
                let side = m.focus();
                let current = crate::text::header::title(m, side);
                let start = crate::browse::folder_of(&current);
                self.open_browse(
                    window,
                    BrowseMode::File,
                    start,
                    BrowseTarget::TextOpen {
                        view,
                        side,
                        local: inv.id == CommandId::OpenLocal,
                    },
                );
            }
            CommandId::DiffFileNext | CommandId::DiffFilePrev => {
                return Some(self.link_step(
                    window,
                    view,
                    LinkStep::Next(inv.id == CommandId::DiffFileNext),
                    confirmed,
                ));
            }
            CommandId::CopyFileRight | CommandId::CopyFileLeft => {
                let from = if inv.id == CommandId::CopyFileRight {
                    Side::Left
                } else {
                    Side::Right
                };
                return Some(self.link_step(window, view, LinkStep::Copy(from), confirmed));
            }
            CommandId::OpenWithList => self.text_open_with(window, view, inv.arg.as_ref()),
            CommandId::AssocApp | CommandId::ExplorerList => {
                // The focus pane's file through the window's launcher (W9b's associated application and file
                // manager).
                let m = self.text_model(view)?;
                let side = m.focus();
                let spec = m.doc(side).and_then(|d| d.location()).cloned()?;
                return Some(self.launch_spec(window, view, spec, inv));
            }
            CommandId::Replace if !self.text_model(view)?.find.has_query() => {
                crate::text::find::open(self.text_model(view)?, true);
            }
            _ => {
                let _ = service;
                return None;
            }
        }
        Some(self.text_requests())
    }

    /// Open With ▸ of a text header (D-312): *Text Compare* with the other pane's file, *Text Edit*, the
    /// `openwith.toml` entries.
    fn text_open_with(&mut self, window: WindowId, view: ViewId, item: Option<&CommandArg>) {
        let Some(m) = self.text_model(view) else {
            return;
        };
        let side = m.focus();
        let spec = |s| {
            m.doc(s)
                .and_then(|d| d.location())
                .map(hikasa_core::Spec::canonical)
        };
        let (mine, other) = (spec(side), spec(crate::text::edit::other(side)));
        match item {
            Some(CommandArg::OpenWith(OpenWithItem::TextEdit)) => {
                if let Some(spec) = mine {
                    self.open_text_edit(window, spec, None);
                }
            }
            Some(CommandArg::OpenWith(OpenWithItem::TextCompare)) => {
                if let (Some(a), Some(b)) = (mine, other) {
                    let parse = |s: &str| self.service.parse_spec(std::ffi::OsStr::new(s)).ok();
                    if let (Some(a), Some(b)) = (parse(&a), parse(&b)) {
                        self.open_specs(window, [a, b], Some(ViewKind::TextCompare));
                    }
                }
            }
            _ => {}
        }
    }

    /// The associated application or the file manager of one file of a text view.
    fn launch_spec(
        &mut self,
        window: WindowId,
        view: ViewId,
        spec: hikasa_core::Spec,
        inv: &Invocation,
    ) -> Vec<Effect> {
        if inv.id == CommandId::AssocApp {
            self.launch(crate::app::ops::Launch {
                view,
                caption: "Associated Application".to_owned(),
                what: crate::app::ops::LaunchWhat::Associated,
                items: vec![(spec.clone(), spec)],
                refresh: None,
            });
            return Vec::new();
        }
        let action = match inv.arg {
            Some(CommandArg::Explorer(item)) => item.action,
            _ => crate::window::ExplorerAction::ShowItem,
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            action.caption().to_owned(),
            &self.wake(),
            move |svc, job| {
                match action {
                    crate::window::ExplorerAction::OpenFolder => {
                        svc.open_folder_in_file_manager(&spec, false, job)?;
                    }
                    crate::window::ExplorerAction::ShowItem => {
                        svc.reveal_in_file_manager(&spec, job)?
                    }
                }
                Ok(crate::app::pane::PaneDone::Shown)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
        Vec::new()
    }

    /// Opens a Text Edit view of `spec` (D-313); `caret`: the line and column (1-based) and the query a Find in Files
    /// hit puts in its Find panel. A tab of this window already showing the file is activated instead.
    pub(super) fn open_text_edit(
        &mut self,
        window: WindowId,
        spec: String,
        caret: Option<(u32, u32, u32, hikasa_text::find::FindQuery)>,
    ) -> ViewId {
        let existing = self.window(window).and_then(|w| {
            w.tabs.iter().position(|t| match t.model() {
                Some(ViewModel::Text(m)) => m.is_edit() && m.inputs()[0] == spec,
                _ => false,
            })
        });
        let view = match existing {
            Some(index) => {
                self.activate(window, index);
                self.window(window)
                    .and_then(|w| w.tabs.get(index))
                    .map(|t| t.id)
            }
            None => None,
        };
        let view = view.unwrap_or_else(|| {
            let request = ViewRequest::TextEdit {
                request: TextEditRequest {
                    spec: spec.clone().into(),
                    ..TextEditRequest::default()
                },
            };
            self.open_local(
                ViewKind::TextEdit,
                request,
                &OpenOptions::default(),
                vec![spec.clone()],
                OpenTarget::Window(window),
            )
        });
        if let Some((line, column, len, query)) = caret
            && let Some(m) = self.text_model(view)
        {
            m.pending_goto = Some((line, column, len));
            m.find.query = query;
        }
        view
    }

    /// The browse answer of Open / Open on File System (reopen the focus pane with that file, D-302 asked first) and of
    /// Edit Text File.
    pub(super) fn text_more_browsed(
        &mut self,
        window: WindowId,
        target: &BrowseTarget,
        answer: Option<String>,
    ) -> bool {
        match target {
            BrowseTarget::TextOpen { view, side, local } => {
                let Some(path) = answer else {
                    return true;
                };
                if *local
                    && !matches!(
                        self.service.parse_spec(std::ffi::OsStr::new(&path)),
                        Ok(hikasa_core::Spec::Local(_))
                    )
                {
                    self.set_message(
                        *view,
                        Message::error(
                            "Open File on File System opens a file of the file system only.",
                        ),
                    );
                    return true;
                }
                if let Some(m) = self.text_model(*view) {
                    m.request_reopen(*side, Some(path.into()), None);
                }
                true
            }
            BrowseTarget::EditTextFile => {
                if let Some(path) = answer {
                    self.open_text_edit(window, path, None);
                }
                true
            }
            BrowseTarget::FindFilesRoot => {
                if let Some(path) = answer
                    && let Some(d) = self
                        .window_mut(window)
                        .and_then(|w| w.dialogs.find_mut::<FindFilesDialog>())
                {
                    if d.look_in.last().is_some_and(|r| r.trim().is_empty()) {
                        d.look_in.pop();
                    }
                    d.look_in.push(path);
                }
                true
            }
            _ => false,
        }
    }

    /// A dialog of this file left the stack: Go To moves the caret, Display Font sets the view's font, Find in Files
    /// starts the search. True when `closed` was one of them.
    pub(super) fn text_more_dialog_closed(
        &mut self,
        window: WindowId,
        closed: &mut Box<dyn DialogModel>,
    ) -> bool {
        if let Some(d) = closed.as_any_mut().downcast_mut::<GoToLineDialog>() {
            let view = d.view;
            if let Some((line, column)) = d.take_chosen()
                && let Some(m) = self.text_model(view)
            {
                nav::go_to_line(m, line, column);
            }
            return true;
        }
        if let Some(d) = closed.as_any_mut().downcast_mut::<DisplayFontDialog>() {
            let view = d.view;
            if let Some((family, size, theme)) = d.take_chosen() {
                if let Some(m) = self.text_model(view) {
                    m.set_display_font(family.clone(), size);
                }
                if theme {
                    self.save_editor_font(family, size);
                }
            }
            return true;
        }
        if let Some(d) = closed.as_any_mut().downcast_mut::<FindFilesDialog>() {
            let mut remembered: Vec<(&'static str, String)> = d.remembered();
            if let Some(h) = self
                .window(window)
                .map(|w| w.find_files.height)
                .filter(|h| *h > 0.0)
            {
                remembered.push((crate::text::find_files::PANEL_HEIGHT_KEY, format!("{h:.0}")));
            }
            let chosen = d.take_chosen();
            let max = self.options().tweaks.metrics.max_mru_items as usize;
            for (key, value) in &remembered {
                self.state
                    .set_dialog_text(crate::text::find_files::DIALOG_ID, key, value);
            }
            if let Some(request) = chosen {
                self.state.push_mru(MruList::Find, &request.pattern, max);
                for root in &request.roots {
                    self.state
                        .push_mru(MruList::FindInFiles, &root.to_string_lossy(), max);
                }
                let pattern = request.pattern.clone();
                let roots: Vec<String> = request
                    .roots
                    .iter()
                    .map(|r| r.to_string_lossy().into_owned())
                    .collect();
                self.update_state(move |s| {
                    for (key, value) in &remembered {
                        s.set_dialog_text(crate::text::find_files::DIALOG_ID, key, value);
                    }
                    s.push_mru(MruList::Find, &pattern, max);
                    for root in &roots {
                        s.push_mru(MruList::FindInFiles, root, max);
                    }
                });
                self.start_find_files(window, request);
            }
            return true;
        }
        false
    }

    /// Starts a search in the window's results panel (a replace plan when the request has a replacement).
    pub(super) fn start_find_files(&mut self, window: WindowId, request: TextFindRequest) {
        let wake = self.wake();
        let service = self.service.clone();
        if request.replace.is_some() {
            self.replace_in_files(window, request);
            return;
        }
        // The panel's remembered height, before its first search.
        let remembered = self
            .state
            .dialog_text(
                crate::text::find_files::DIALOG_ID,
                crate::text::find_files::PANEL_HEIGHT_KEY,
            )
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(80.0, 2_000.0));
        if let Some(w) = self.window_mut(window) {
            if w.find_files.height <= 0.0
                && let Some(h) = remembered
            {
                w.find_files.height = h;
            }
            w.find_files.start(&service, &wake, request);
        }
    }

    /// *Theme's Editor font*: `theme.toml`'s `[fonts.editor]` through `settings_save` (D-305).
    fn save_editor_font(&mut self, family: Option<String>, size: u32) {
        self.start_settings_job("save the editor font", move |svc| {
            let snapshot = svc.settings_snapshot()?;
            let mut theme = snapshot.theme.value().clone();
            if let Some(family) = family {
                theme.fonts.editor.family = family;
            }
            theme.fonts.editor.size = f64::from(size) as _;
            let changes = hikasa_session::settings::SettingsChanges {
                theme: Some(theme),
                ..hikasa_session::settings::SettingsChanges::default()
            };
            let dto = svc.settings_save(&snapshot, &changes, false)?;
            Ok(dto.files.into_iter().map(|(_, p, _)| p).collect())
        });
    }

    // ---- the session link (D-311) ---------------------------------------------------------------------------

    /// The parent link of a view (its session's).
    fn parent_of(&self, view: ViewId) -> Option<ParentLink> {
        self.tab(view)?
            .session
            .as_ref()?
            .parent
            .clone()
            .filter(|p| !p.is_empty())
    }

    /// The open folder view of this process that is `link`'s parent, if any.
    fn parent_view(&self, link: &ParentLink) -> Option<ViewId> {
        self.windows.iter().flat_map(|w| &w.tabs).find_map(|t| {
            let ViewModel::Folder(m) = t.model()? else {
                return None;
            };
            m.prepared()?;
            let text = t.session.as_ref()?.link_text(self.slots.get(&t.id))?;
            (text == link.session).then_some(t.id)
        })
    }

    /// A step of the session link: with the parent folder view open, its comparison; else the parent compared again
    /// in a job (*Comparing the parent folders…*).
    fn link_step(
        &mut self,
        window: WindowId,
        view: ViewId,
        step: LinkStep,
        confirmed: bool,
    ) -> Vec<Effect> {
        let Some(link) = self.parent_of(view) else {
            return Vec::new();
        };
        // A step that asked D-301 first and was answered runs on the parent it computed.
        if confirmed
            && let Some(parent) = self.text_links.get_mut(&view).and_then(|l| l.parent.take())
        {
            return self.link_with(window, view, step, &link, parent, true);
        }
        if let Some(parent_view) = self.parent_view(&link) {
            let found = match self.tab(parent_view).and_then(Tab::model) {
                Some(ViewModel::Folder(m)) => m.prepared().cloned().map(|prepared| {
                    let read_only = m.write_policy().read_only;
                    let row = hikasa_session::api::parent_row(&prepared.model().read(), &link);
                    ParentState {
                        prepared,
                        read_only,
                        row,
                        display: *m.display_options(),
                    }
                }),
                _ => None,
            };
            if let Some(parent) = found {
                return self.link_with(window, view, step, &link, Box::new(parent), confirmed);
            }
        }
        let wake = self.wake();
        let job = Job::spawn(
            &self.service,
            JobKind::FolderCompare,
            "Comparing the parent folders…".into(),
            &wake,
            move |svc, job| {
                let parent = svc.text_parent_compare(&link, job, &hikasa_folder::NullObserver)?;
                Ok(MoreDone::Parent(
                    Box::new(ParentState::of_compare(parent)),
                    step,
                ))
            },
        );
        self.jobs.push(CoreJob::More(window, view, job));
        Vec::new()
    }

    /// A link step with the parent's comparison.
    fn link_with(
        &mut self,
        window: WindowId,
        view: ViewId,
        step: LinkStep,
        link: &ParentLink,
        parent: Box<ParentState>,
        confirmed: bool,
    ) -> Vec<Effect> {
        let iu = parent.prepared.ignore_unimportant();
        let opts = self.service.next_file_options(parent.display, iu);
        match step {
            LinkStep::Next(forward) => {
                let Some(current) = self.text_model(view).map(|m| m.request().clone()) else {
                    return Vec::new();
                };
                let dir = if forward { Dir::Forward } else { Dir::Backward };
                match link::next_pair(&parent.prepared, link, dir, &opts, &current) {
                    Err(text) => {
                        self.set_message(view, Message::info(text));
                        Vec::new()
                    }
                    Ok((_, request, moved)) => {
                        let modified = self
                            .text_model(view)
                            .is_some_and(|m| !crate::text::save::modified_sides(m).is_empty());
                        if modified && !confirmed {
                            self.text_links.insert(
                                view,
                                LinkRun {
                                    parent: Some(parent),
                                    steps: None,
                                },
                            );
                            let id = if forward {
                                CommandId::DiffFileNext
                            } else {
                                CommandId::DiffFilePrev
                            };
                            return self.ask_unsaved_then(window, view, Invocation::new(id));
                        }
                        self.reopen_pair(view, request, moved);
                        Vec::new()
                    }
                }
            }
            LinkStep::Copy(from) => {
                let Some(row) = parent.row else {
                    self.set_message(view, Message::info(link::ROW_GONE));
                    return Vec::new();
                };
                let service = self.service.clone();
                let Some(m) = self.text_model(view) else {
                    return Vec::new();
                };
                let Some(live) = m.live_view() else {
                    return Vec::new();
                };
                let docs = [live.docs.left.clone(), live.docs.right.clone()];
                let save = service.live_save_request(live, from).ok();
                let job = Job::spawn(
                    &self.service,
                    JobKind::FolderOp,
                    "plan the copy".into(),
                    &self.wake(),
                    move |svc, job| {
                        let steps = svc.text_copy_file_steps(
                            &CopyFileRequest {
                                parent: &parent.prepared,
                                row,
                                from,
                                docs: hikasa_core::Pair::new(&docs[0], &docs[1]),
                                read_only: parent.read_only,
                                next: opts,
                                save: save.as_ref(),
                            },
                            job,
                        )?;
                        Ok(MoreDone::CopySteps(parent, from, Box::new(steps)))
                    },
                );
                self.jobs.push(CoreJob::More(window, view, job));
                Vec::new()
            }
        }
    }

    /// This view reopens with the next pair (D-311), its edits dropped (D-301 was asked), the link moved to it.
    fn reopen_pair(
        &mut self,
        view: ViewId,
        request: hikasa_session::api::TextCompareRequest,
        moved: ParentLink,
    ) {
        let service = self.service.clone();
        let wake = self.wake();
        let Some(tab) = self.tab_mut(view) else {
            return;
        };
        let (opts, write) = match tab.model() {
            Some(ViewModel::Text(m)) => (m.view_options().clone(), m.write_policy()),
            _ => return,
        };
        let mut model = TextViewModel::new(&service, request, opts, [None, None], wake);
        model.set_write_policy(write);
        if let Some(ViewModel::Text(old)) = tab.model_mut() {
            old.cancel();
            model.set_tweaks(*old.tweaks());
        }
        tab.content = TabContent::Live(ViewModel::Text(Box::new(model)));
        if let Some(s) = tab.session.as_mut() {
            s.parent = Some(moved);
        }
        let caption = tab.caption();
        self.shared.set_title(view, caption);
    }

    // ---- jobs ---------------------------------------------------------------------------------------------

    /// A job of this file ended.
    pub(super) fn more_done(
        &mut self,
        window: WindowId,
        view: ViewId,
        result: Result<MoreDone, hikasa_session::ApiError>,
    ) -> Vec<Effect> {
        let done = match result {
            Ok(done) => done,
            Err(hikasa_session::ApiError::Cancelled) => return Vec::new(),
            Err(e) => {
                self.set_message(view, Message::error(e.to_string()));
                return Vec::new();
            }
        };
        match done {
            MoreDone::ClipboardText(left, right) => {
                let Some(text) = right else {
                    self.set_message(view, Message::info(crate::text::clipboard::NO_TEXT));
                    return Vec::new();
                };
                let right = NamedText {
                    name: "clipboard".to_owned(),
                    text,
                };
                let model = TextViewModel::new_texts(&self.service, left, right, self.wake());
                let id = self.shared.allocate();
                self.shared.register(
                    hikasa_session::ipc::ViewInfo {
                        view: id,
                        kind: ViewKind::TextCompare,
                        title: String::new(),
                        specs: Vec::new(),
                    },
                    None,
                );
                let toolbar = self.toolbar_default(crate::commands::Scope::Text);
                let mut tab = Tab::live(id, ViewModel::Text(Box::new(model)), toolbar);
                tab.session = Some(crate::view_session::ViewSession::unnamed(self.timer_now));
                self.shared.set_title(id, tab.caption());
                self.insert_tab(window, tab, true);
            }
            MoreDone::ClipboardChecked(side, checked) => match checked {
                Err(text) => self.set_message(view, Message::info(text)),
                Ok(()) => {
                    if let Some(m) = self.text_model(view) {
                        m.request_reopen(side, Some("clipboard://".into()), None);
                    }
                }
            },
            MoreDone::Parent(parent, step) => {
                let Some(link) = self.parent_of(view) else {
                    return Vec::new();
                };
                return self.link_with(window, view, step, &link, parent, false);
            }
            MoreDone::CopySteps(parent, from, steps) => {
                let Some(m) = self.text_model(view) else {
                    return Vec::new();
                };
                let names = [
                    crate::text::name_of(&crate::text::header::title(m, Side::Left)),
                    crate::text::name_of(&crate::text::header::title(m, Side::Right)),
                ];
                let next = steps
                    .next
                    .and_then(|n| link::pair_names(&parent.prepared, n));
                let lines = link::steps_text(&steps, [&names[0], &names[1]], from, next);
                self.text_links.insert(
                    view,
                    LinkRun {
                        parent: Some(parent),
                        steps: Some((from, steps)),
                    },
                );
                let answer = |yes| vec![Followup::More(MoreFollowup::CopyFile { view, yes })];
                let prompt = Prompt::ask(
                    "Copy File and Open Next",
                    format!("These steps run in order:\n• {}", lines.join("\n• ")),
                    ButtonRole::Yes,
                    vec![
                        (ButtonRole::Yes, "Yes", answer(true)),
                        (ButtonRole::No, "No", answer(false)),
                    ],
                    "dialog-copy-file-next",
                );
                self.push_dialog(window, Box::new(prompt));
            }
            MoreDone::Copied(report, parent, saved) => {
                if let (Some(doc), Some(m)) = (saved, self.text_model(view))
                    && let Some(rep) = report.save.as_ref()
                {
                    let _ = (doc, rep);
                    m.request_window(TextRequest::Message(Message::info("The edits were saved.")));
                }
                if !report.ok() {
                    let why = report
                        .error
                        .as_ref()
                        .map_or_else(|| "a step did not run".to_owned(), |e| e.message.clone());
                    self.set_message(view, Message::error(format!("Copy File stopped: {why}.")));
                    return Vec::new();
                }
                // The parent folder view, when open, shows the refreshed rows.
                if let Some(link) = self.parent_of(view)
                    && let Some(pv) = self.parent_view(&link)
                    && let Some(ViewModel::Folder(m)) = self.tab_mut(pv).and_then(|t| t.model_mut())
                {
                    m.rebuild();
                }
                match report.next {
                    None => self.set_message(view, Message::info(link::NO_MORE_FILES)),
                    Some(next) => {
                        let (Some(link), Some(current)) = (
                            self.parent_of(view),
                            self.text_model(view).map(|m| m.request().clone()),
                        ) else {
                            return Vec::new();
                        };
                        if let Some((_, request, moved)) =
                            link::pair_of(&parent.prepared, next, &link, &current)
                        {
                            self.reopen_pair(view, request, moved);
                        }
                    }
                }
            }
            MoreDone::ReplacePlan(plan) => return self.replace_planned(window, *plan),
            MoreDone::Replaced(outcome) => {
                let lines = outcome.replace_lines();
                for line in &lines {
                    self.log_entry(LogKind::Detail, "replace in files", line.clone());
                }
                let written: Vec<String> =
                    outcome.dto.files.iter().map(|f| f.spec.clone()).collect();
                // Views of the rewritten files without edits reload (D-118).
                self.reload_unmodified(&written);
                let text = format!(
                    "Replace in Files: {} occurrences in {} files.",
                    outcome.dto.counts.hits, outcome.dto.counts.files_matched
                );
                self.notify(
                    window,
                    Notice::new("replace-in-files".to_owned(), NoticeKind::Info, text),
                    "replace in files",
                    LogKind::Summary,
                );
            }
        }
        self.text_requests()
    }

    /// The answers of this file's prompts.
    pub(super) fn more_followup(&mut self, window: WindowId, f: MoreFollowup) -> Vec<Effect> {
        match f {
            MoreFollowup::CopyFile { view, yes } => {
                let run = self.text_links.remove(&view);
                if !yes {
                    self.set_message(view, Message::info("Nothing was copied."));
                    return Vec::new();
                }
                let Some(LinkRun {
                    parent: Some(parent),
                    steps: Some((from, steps)),
                }) = run
                else {
                    return Vec::new();
                };
                let Some(row) = parent.row else {
                    return Vec::new();
                };
                let Some(source) = self.text_model(view).and_then(|m| m.doc(from)).cloned() else {
                    return Vec::new();
                };
                let iu = parent.prepared.ignore_unimportant();
                let next = self.service.next_file_options(parent.display, iu);
                let job = Job::spawn(
                    &self.service,
                    JobKind::FolderOp,
                    "copy the file".into(),
                    &self.wake(),
                    move |svc, job| {
                        let mut source = source;
                        let report = svc.text_copy_file_execute(
                            *steps,
                            &CopyFileExecute {
                                parent: &parent.prepared,
                                row,
                                from,
                                read_only: parent.read_only,
                                next,
                                copy_mode: hikasa_folder::ops::ConfirmMode::YesToAll,
                            },
                            &mut source,
                            job,
                            &hikasa_folder::NullObserver,
                        )?;
                        Ok(MoreDone::Copied(
                            Box::new(report),
                            parent,
                            Some(Box::new(source)),
                        ))
                    },
                );
                self.jobs.push(CoreJob::More(window, view, job));
                Vec::new()
            }
            MoreFollowup::ReplaceReply { window, reply } => self.replace_reply(window, reply),
            MoreFollowup::Disk { view, side, reload } => {
                if reload {
                    // Reload asks D-301 first (the side's edits would go).
                    if let Some(m) = self.text_model(view) {
                        m.request_reopen(side, None, None);
                    }
                    return self.text_requests();
                }
                Vec::new()
            }
            MoreFollowup::FormatReopen(view) => {
                let service = self.service.clone();
                if let Some(m) = self.text_model(view) {
                    if crate::text::save::modified_sides(m).is_empty() {
                        m.execute(&service, CommandId::Reload);
                    } else if !m.is_edit() {
                        let request = m.request().clone();
                        crate::text::relive::start(
                            m,
                            &service,
                            request,
                            crate::text::relive::Cause::Rules,
                        );
                    }
                }
                Vec::new()
            }
        }
    }

    // ---- Find in Files ------------------------------------------------------------------------------------

    /// Opens hit `k` of the window's results (Enter, double-click): a Text Compare tab of this window showing the file
    /// in a pane is activated; else a Text Edit view of the file (new, or the tab showing it) with the caret at the hit
    /// and the query in its Find panel.
    pub fn open_find_hit(&mut self, window: WindowId, k: usize) {
        let Some((spec, line, column, len, query)) = self.window(window).and_then(|w| {
            let (spec, line, column, len) = w.find_files.hit_target(k)?;
            let r = w.find_files.request.as_ref()?;
            Some((
                spec,
                line,
                column,
                len,
                hikasa_text::find::FindQuery {
                    text: r.pattern.clone(),
                    regex: r.regex,
                    match_case: r.match_case,
                    whole_word: r.whole_word,
                },
            ))
        }) else {
            return;
        };
        let compare = self.window(window).and_then(|w| {
            w.tabs.iter().position(|t| match t.model() {
                Some(ViewModel::Text(m)) => !m.is_edit() && m.inputs().contains(&spec),
                _ => false,
            })
        });
        if let Some(index) = compare {
            self.activate(window, index);
            if let Some(view) = self
                .window(window)
                .and_then(|w| w.tabs.get(index))
                .map(|t| t.id)
                && let Some(m) = self.text_model(view)
            {
                let side = if m.inputs()[1] == spec {
                    Side::Right
                } else {
                    Side::Left
                };
                m.set_focus(side);
                m.pending_goto = Some((line, column, len));
                m.find.query = query;
                m.apply_pending_goto();
            }
            return;
        }
        let view = self.open_text_edit(window, spec, Some((line, column, len, query)));
        if let Some(m) = self.text_model(view) {
            m.apply_pending_goto();
        }
    }

    /// Replace in Files (D-308): the plan as a job, the files open with unsaved edits left out of it.
    pub fn replace_in_files(&mut self, window: WindowId, request: TextFindRequest) {
        let unsaved: Vec<String> = self
            .windows
            .iter()
            .flat_map(|w| &w.tabs)
            .filter_map(|t| match t.model() {
                Some(ViewModel::Text(m)) => Some(
                    [Side::Left, Side::Right]
                        .into_iter()
                        .filter(|s| m.doc(*s).is_some_and(|d| d.is_modified()))
                        .filter_map(|s| {
                            m.doc(s)
                                .and_then(|d| d.location())
                                .map(hikasa_core::Spec::canonical)
                        })
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();
        let view = self.active_tab(window).map_or(ViewId(0), |t| t.id);
        let job = Job::spawn(
            &self.service,
            JobKind::TextFind,
            "plan the replace in files".into(),
            &self.wake(),
            move |svc, job| {
                let mut sink = NoSink;
                let mut plan = svc.text_find_replace_plan(&request, &mut sink, job)?;
                plan.leave_out(&unsaved, "open with unsaved edits");
                Ok(MoreDone::ReplacePlan(Box::new(plan)))
            },
        );
        self.jobs.push(CoreJob::More(window, view, job));
    }

    /// The plan landed: its preview and questions (the operation question, then the save questions) one at a time.
    fn replace_planned(&mut self, window: WindowId, plan: TextFindReplacePlan) -> Vec<Effect> {
        let Some(driver) = self.service.text_find_replace_confirmation(&plan) else {
            let view = self.active_tab(window).map_or(ViewId(0), |t| t.id);
            self.set_message(view, Message::info("Replace in Files: no file to rewrite."));
            return Vec::new();
        };
        let preview = plan_preview(&self.service, &plan);
        if let Some(w) = self.window_mut(window) {
            w.find_files.preview = preview;
        }
        self.files_replace.insert(
            window,
            FilesReplace {
                plan: Box::new(plan),
                driver,
            },
        );
        self.replace_ask(window)
    }

    /// The next question of a Replace in Files, or its execution when none is left.
    fn replace_ask(&mut self, window: WindowId) -> Vec<Effect> {
        let Some(question) = self
            .files_replace
            .get(&window)
            .and_then(|r| r.driver.next_question().map(str::to_owned))
        else {
            let Some(FilesReplace { plan, driver }) = self.files_replace.remove(&window) else {
                return Vec::new();
            };
            let Ok(plan) = plan.answered(driver) else {
                return Vec::new();
            };
            let view = self.active_tab(window).map_or(ViewId(0), |t| t.id);
            let job = Job::spawn(
                &self.service,
                JobKind::TextFind,
                "replace in files".into(),
                &self.wake(),
                move |svc, job| {
                    Ok(MoreDone::Replaced(Box::new(
                        svc.text_find_replace_execute(plan, job)?,
                    )))
                },
            );
            self.jobs.push(CoreJob::More(window, view, job));
            return Vec::new();
        };
        let reply = |r| {
            vec![Followup::More(MoreFollowup::ReplaceReply {
                window,
                reply: r,
            })]
        };
        let preview = self
            .window(window)
            .map(|w| w.find_files.preview.join("\n"))
            .unwrap_or_default();
        let text = if preview.is_empty() {
            question
        } else {
            format!("{preview}\n\n{question}")
        };
        let prompt = Prompt::ask(
            "Replace in Files",
            text,
            ButtonRole::No,
            vec![
                (ButtonRole::Yes, "Yes", reply(Reply::Yes)),
                (ButtonRole::No, "No", reply(Reply::No)),
                (ButtonRole::Cancel, "Cancel", reply(Reply::Cancel)),
            ],
            crate::text::find_files::PANEL_TOPIC,
        );
        self.push_dialog(window, Box::new(prompt));
        Vec::new()
    }

    fn replace_reply(&mut self, window: WindowId, reply: Reply) -> Vec<Effect> {
        if let Some(r) = self.files_replace.get_mut(&window) {
            r.driver.reply(reply);
        }
        self.replace_ask(window)
    }

    /// Reloads the open views of `specs` whose documents have no edits (a Replace in Files rewrote them).
    fn reload_unmodified(&mut self, specs: &[String]) {
        let service = self.service.clone();
        for w in &mut self.windows {
            for tab in &mut w.tabs {
                if let Some(ViewModel::Text(m)) = tab.model_mut()
                    && m.inputs().iter().any(|i| specs.contains(i))
                    && crate::text::save::modified_sides(m).is_empty()
                {
                    m.execute(&service, CommandId::Reload);
                }
            }
        }
    }

    /// The panel's Copy: its selected lines on the window's clipboard.
    pub fn copy_find_files(&mut self, window: WindowId) {
        let Some(text) = self.window(window).and_then(|w| w.find_files.copy_text()) else {
            return;
        };
        let provider = Arc::clone(&self.script_clipboard);
        let ticket = provider.ticket();
        let view = self.active_tab(window).map_or(ViewId(0), |t| t.id);
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy the hits".into(),
            &self.wake(),
            move |_, _| {
                provider
                    .in_turn(ticket, |c| {
                        c.write_payload(&hikasa_session::clipboard::ClipboardPayload {
                            text,
                            html: None,
                        })
                    })
                    .map_err(|e| hikasa_session::ApiError::Vfs(e.to_vfs_write()))?;
                Ok(crate::app::pane::PaneDone::Named)
            },
        );
        self.jobs.push(CoreJob::Pane(window, view, job));
    }

    // ---- changes on disk (D-315) -------------------------------------------------------------------------

    /// An input of a text view changed on disk: a view with edits on that side gets the notice *<name> changed on
    /// disk.* with *Reload* and *Keep mine* (never a reload); true when one did (the caller then leaves the view
    /// alone).
    pub(super) fn text_changed_on_disk(
        &mut self,
        window: WindowId,
        view: ViewId,
        spec: &str,
    ) -> bool {
        let Some(m) = self.text_model(view) else {
            return false;
        };
        let sides: Vec<Side> = info::sides_of(m, spec)
            .into_iter()
            .filter(|s| info::modified(m, *s))
            .collect();
        if sides.is_empty() {
            return false;
        }
        let notices: Vec<(String, String, Side)> = sides
            .iter()
            .map(|s| {
                let (key, text) = info::changed_notice(m, *s);
                (key, text, *s)
            })
            .collect();
        for (key, text, side) in notices {
            let notice = Notice::new(format!("{}:{}", key, view.0), NoticeKind::Warning, text)
                .with_actions(vec![
                    NoticeAction::DiskReload(view, side),
                    NoticeAction::DiskKeep(view, side),
                ]);
            self.notify(window, notice, "watch", LogKind::Warning);
        }
        true
    }

    /// The format registry was reloaded with `changed` formats: a text view using one gets *File format <name> changed
    /// on disk.* with *Reopen* (FF-004).
    pub(super) fn formats_changed(&mut self, changed: &[String]) {
        let mut hits = Vec::new();
        for w in &self.windows {
            for t in &w.tabs {
                if let Some(ViewModel::Text(m)) = t.model() {
                    for name in info::formats_used(m) {
                        if changed.contains(&name) {
                            hits.push((w.id, t.id, name));
                        }
                    }
                }
            }
        }
        for (window, view, name) in hits {
            let notice = Notice::new(
                format!("format:{}:{name}", view.0),
                NoticeKind::Warning,
                format!("File format {name} changed on disk."),
            )
            .with_actions(vec![NoticeAction::FormatReopen(view)]);
            self.notify(window, notice, "formats", LogKind::Warning);
        }
    }

    /// The Find in Files panels' polling (once per frame); true when one changed.
    pub(super) fn poll_find_files(&mut self) -> bool {
        let mut changed = false;
        for w in &mut self.windows {
            changed |= w.find_files.poll();
        }
        changed
    }

    /// An action of the window's Find in Files panel (the drawing layer's).
    pub fn find_files_action(
        &mut self,
        window: WindowId,
        action: crate::ui::find_files::FilesAction,
    ) -> Vec<Effect> {
        use crate::ui::find_files::FilesAction as A;
        match action {
            A::Open(k) => self.open_find_hit(window, k),
            A::Copy => self.copy_find_files(window),
            A::Stop => {
                if let Some(w) = self.window_mut(window) {
                    w.find_files.stop();
                }
            }
            A::Clear => {
                if let Some(w) = self.window_mut(window) {
                    w.find_files.clear();
                }
            }
            A::Close => {
                if let Some(w) = self.window_mut(window) {
                    w.find_files.shown = false;
                }
            }
            A::Height(h) => {
                if let Some(w) = self.window_mut(window) {
                    w.find_files.height = h;
                }
                // Kept with the dialog's choices (`state.dialogs."find-in-files".panel_height`), written at the next
                // search.
                self.state.set_dialog_text(
                    crate::text::find_files::DIALOG_ID,
                    crate::text::find_files::PANEL_HEIGHT_KEY,
                    &format!("{h:.0}"),
                );
            }
            A::Replace(text) => {
                let request = self
                    .window(window)
                    .and_then(|w| w.find_files.request.clone());
                if let Some(mut request) = request {
                    request.replace = Some(hikasa_session::api::TextFindReplace {
                        text,
                        ..hikasa_session::api::TextFindReplace::default()
                    });
                    self.replace_in_files(window, request);
                }
            }
        }
        Vec::new()
    }
}

/// A sink that keeps nothing (the replace plan's search: its hits are the plan's).
struct NoSink;

impl hikasa_text::find::files::FilesSink for NoSink {
    fn event(&mut self, _e: hikasa_text::find::files::FilesEvent<'_>) {}
}

/// The default *Look in* of the dialog: the active view's folders (the folders of its files).
fn view_folders(m: &TextViewModel) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for spec in m.inputs() {
        if spec.is_empty() || spec == "clipboard://" {
            continue;
        }
        let folder = crate::browse::folder_of(&spec);
        let folder = folder.trim_end_matches(['/', '\\']).to_owned();
        if !folder.is_empty() && !out.contains(&folder) {
            out.push(folder);
        }
    }
    out
}

/// The preview of a Replace in Files plan: one line per file with its count (W9b's plan previews), the files left out
/// with their reason.
fn plan_preview(svc: &hikasa_session::Service, plan: &TextFindReplacePlan) -> Vec<String> {
    svc.text_find_replace_dry_run(plan.clone()).replace_lines()
}

#[cfg(test)]
impl AppCore {
    /// A dialog left the stack (tests: as the window's close of it).
    pub fn dialog_closed_for_test(&mut self, closed: Box<dyn DialogModel>) -> Vec<Effect> {
        self.dialog_closed(closed)
    }
}

#[cfg(test)]
impl AppCore {
    /// Starts a search in `window`'s panel (tests: as the dialog's Find).
    pub fn start_find_files_for_test(&mut self, window: WindowId, request: TextFindRequest) {
        self.start_find_files(window, request);
    }
}
