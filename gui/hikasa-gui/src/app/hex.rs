//! The window's side of a Hex Compare view (M2 W13a, contract §1.13.3–§1.13.8, §1.13.17, §1.13.18): building the view
//! (the rule words on the defaults session, D-374; the defaults' layout for a view without layout words), its commands
//! that need the window (the dialogs Go To, Layout, Info, display font; the clipboard; Open, Open With, the file
//! manager; Rules; Clear Session; Next/Previous Difference Files of a child view), what its dialogs leave when they
//! close, and the view's requests (clipboard writes, message panel lines, notices).

use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::{JobKind, Side};
use hikasa_session::api::{HexCompareRequest, HexLayoutWords, ViewRequest};
use hikasa_session::clipboard::{ClipboardPayload, ClipboardProvider as _};
use hikasa_session::ipc::{OpenOptions, ViewId, ViewKind};
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_session::settings::state::MruList;

use super::pane::PaneDone;
use super::{AppCore, CoreJob, Effect, OpenTarget, ViewModel};
use crate::browse::{BrowseMode, BrowseTarget};
use crate::commands::CommandId;
use crate::hex::layout::{LayoutChoice, LayoutDialog};
use crate::hex::nav::{GOTO_DIALOG, HexGoToDialog};
use crate::hex::values::ValuesChoice;
use crate::hex::{HexRequest, HexViewModel};
use crate::job::Job;
use crate::notify::{Notice, NoticeAction, NoticeKind};
use crate::status::Message;
use crate::window::{CommandArg, Invocation, OpenWithItem, Tab, WindowId};

use crate::hex::values::VALUES_COPY;

impl AppCore {
    /// Builds a Hex Compare view: the rule words on the defaults session (D-374: a word typed wins); a view without
    /// layout words takes the defaults session's `[view]` and `[display]` (the Layout dialog's *Use for new Hex
    /// Compare views*); then the layout of the view type and the session's view state.
    pub(super) fn build_hex(
        &self,
        mut request: HexCompareRequest,
        mut rows: hikasa_hex::HexLayout,
        mut filter: Option<hikasa_hex::DisplayFilter>,
        titles: [Option<String>; 2],
        layout: &State,
        editor_px: u32,
    ) -> HexViewModel {
        request.rules = self.service.hex_rules_on_defaults(&request.rules);
        let mut state = layout.clone();
        if request.layout.is_empty()
            && let Ok(defaults) = self
                .service
                .session_defaults(hikasa_session::sessions::SessionType::HexCompare)
        {
            let words =
                hikasa_session::api::hex::hex_layout_words_of(&defaults.view, &defaults.display);
            if let Ok((l, d, _)) = hikasa_session::api::hex_layout_of(&words, filter) {
                rows = l;
                filter = d;
            }
            request.layout = HexLayoutWords {
                display: None,
                ..words
            };
            // The defaults' view state under the view type's layout memory (a session's own keys win later).
            if !defaults.view.is_empty() {
                state.overlay_view(crate::hex::VIEW, &defaults.view);
            }
        }
        let mut m = HexViewModel::new(&self.service, request, rows, filter, titles, self.wake());
        m.apply_layout(&state, editor_px);
        m.set_format_names(self.hex_format_names());
        m
    }

    /// The enabled Hex formats' names (a hex header's Format ▸, D-360).
    pub fn hex_format_names(&self) -> Vec<String> {
        self.service
            .format_list()
            .formats
            .into_iter()
            .filter(|f| f.enabled && f.kind == "hex")
            .map(|f| f.name)
            .collect()
    }

    /// The model of a hex view.
    pub(super) fn hex_model(&mut self, view: ViewId) -> Option<&mut HexViewModel> {
        match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Hex(m)) => Some(m),
            _ => None,
        }
    }

    /// The active Hex Compare view of a window.
    pub(super) fn active_hex(&self, window: WindowId) -> Option<ViewId> {
        let tab = self.active_tab(window)?;
        matches!(tab.model(), Some(ViewModel::Hex(_))).then_some(tab.id)
    }

    /// Opens a hex pair of a folder view's row (FC-062, D-369): a child session of the folder view when it has a link.
    pub(super) fn open_hex_pair(
        &mut self,
        pair: &hikasa_session::api::TextCompareRequest,
        parent: Option<hikasa_session::sessions::ParentLink>,
        target: OpenTarget,
        child: Option<(ViewId, crate::folder_view::ChildTicket)>,
        parent_view: ViewId,
    ) {
        let specs = vec![
            pair.left.to_string_lossy().into_owned(),
            pair.right.to_string_lossy().into_owned(),
        ];
        let request = match self.service.open_view_request(
            ViewKind::HexCompare,
            &specs,
            &OpenOptions::default(),
        ) {
            Ok(ViewRequest::Hex {
                mut request,
                layout,
                filter,
            }) => {
                request.left = pair.left.clone();
                request.right = pair.right.clone();
                request.format_overrides = pair.format_overrides.clone();
                ViewRequest::Hex {
                    request,
                    layout,
                    filter,
                }
            }
            Ok(other) => other,
            Err(e) => {
                self.set_message(parent_view, Message::error(e.to_string()));
                return;
            }
        };
        if let Some(parent) = parent {
            self.open_child(request, parent, target, child);
            return;
        }
        let opened = self.open_local(
            ViewKind::HexCompare,
            request,
            &OpenOptions::default(),
            specs,
            target,
        );
        if let Some((parent_view, ticket)) = child {
            self.adopt_child(opened, parent_view, ticket);
        }
    }

    /// The commands a hex view handles with the window (`None`: not one of them, or no hex view is active).
    pub(super) fn hex_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
    ) -> Option<Vec<Effect>> {
        let view = self.active_hex(window)?;
        let service = self.service.clone();
        match inv.id {
            CommandId::Copy
                if matches!(inv.arg, Some(CommandArg::Page(VALUES_COPY)))
                    || self.hex_model(view).is_some_and(|m| m.values_focus) =>
            {
                let text = self
                    .hex_model(view)?
                    .value_lines()
                    .map(|(t, _)| t.clone())?;
                self.hex_clipboard(window, view, text, "The values were copied.".to_owned());
            }
            CommandId::Info => {
                let m = self.hex_model(view)?;
                let response = m.response()?.clone();
                let mut opts = service.hex_settings(m.request()).ok()?.align;
                opts.mode = response.alignment.mode;
                let sections = crate::hex::info::sections(&response, &opts, m.mode);
                let dialog = crate::info_dialog::InfoDialog::with_sections(
                    crate::commands::Scope::Hex.type_title(),
                    sections,
                    crate::hex::info::INFO_TOPIC,
                    Arc::clone(&self.script_clipboard) as _,
                )
                .with_copy_text(crate::hex::info::copy_text(&response));
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::GoTo => {
                let radix = self
                    .state
                    .dialog_text(GOTO_DIALOG, "radix")
                    .and_then(crate::hex::radix_of)
                    .unwrap_or_default();
                let recent = self.state.mru(MruList::HexGoTo).to_vec();
                let side = self.hex_model(view)?.focus();
                self.push_dialog(
                    window,
                    Box::new(HexGoToDialog::new(view, side, radix, recent)),
                );
            }
            CommandId::Layout => {
                let m = self.hex_model(view)?;
                let d = m.display();
                let choice = LayoutChoice {
                    layout: d.layout,
                    addresses: d.addresses,
                    panes: d.panes,
                    context: d.context,
                };
                self.push_dialog(window, Box::new(LayoutDialog::new(view, choice)));
            }
            CommandId::AlignModeList => {
                if let Some(CommandArg::AlignMode(mode)) = inv.arg {
                    self.hex_model(view)?.set_mode(mode);
                    if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                        vs.dirty = true;
                    }
                }
            }
            CommandId::LittleEndianList | CommandId::BigEndianList => {
                let little = inv.id == CommandId::LittleEndianList;
                let m = self.hex_model(view)?;
                match inv.arg {
                    Some(CommandArg::Values(ValuesChoice::Radix(r))) => m.set_values_radix(r),
                    Some(CommandArg::Values(ValuesChoice::Show(l))) => {
                        let v = m.display().values.toggled(l);
                        m.set_values(v);
                    }
                    _ => {
                        let v = m.display().values.toggled(little);
                        m.set_values(v);
                    }
                }
            }
            CommandId::Rules => self.edit_view_on(window, Some(crate::hex_tabs::ALIGNMENT)),
            CommandId::DisplayFont => {
                // The panes draw with the theme's monospace font (D-160): the view's dialog sets the size.
                let px = self.hex_model(view)?.display().font_px;
                let dialog = crate::text::layout::DisplayFontDialog::size_only(view, px);
                self.push_dialog(window, Box::new(dialog));
            }
            CommandId::Open | CommandId::OpenLocal => {
                let m = self.hex_model(view)?;
                let side = m.focus();
                let current = m.inputs()[crate::hex::index(side)].clone();
                let start = crate::browse::folder_of(&current);
                self.open_browse(
                    window,
                    BrowseMode::File,
                    start,
                    BrowseTarget::HexOpen { view, side },
                );
            }
            CommandId::OpenClipboard => {
                let m = self.hex_model(view)?;
                let side = m.focus();
                let spec = OsString::from("clipboard://");
                match side {
                    Side::Right => m.replace_specs(None, Some(spec)),
                    _ => m.replace_specs(Some(spec), None),
                }
                self.hex_mark_dirty(view);
            }
            CommandId::FileFormat => {
                let m = self.hex_model(view)?;
                let side = m.focus();
                if let Some(CommandArg::Format(name)) = inv.arg.clone() {
                    m.set_format(side, name);
                    self.hex_mark_dirty(view);
                }
            }
            CommandId::OpenWithList => self.hex_open_with(window, view, inv.arg.as_ref()),
            CommandId::AssocApp | CommandId::ExplorerList => {
                let m = self.hex_model(view)?;
                let side = match inv.arg {
                    Some(CommandArg::Explorer(item)) => item.side.unwrap_or(m.focus()),
                    _ => m.focus(),
                };
                let raw = OsString::from(&m.inputs()[crate::hex::index(side)]);
                let spec = self.service.parse_spec(&raw).ok()?;
                return Some(self.launch_spec(window, view, spec, inv));
            }
            CommandId::Swap => self.swap_sides(view),
            CommandId::CompareParents => {
                let r = self.hex_model(view)?.request().clone();
                self.open_parents(window, [r.left, r.right]);
            }
            CommandId::SessionClear => {
                self.hex_model(view)?.clear();
                self.hex_mark_dirty(view);
            }
            CommandId::DiffFileNext | CommandId::DiffFilePrev => {
                self.hex_link_step(window, view, inv.id == CommandId::DiffFileNext);
            }
            CommandId::FindNext | CommandId::FindPrev => {
                let m = self.hex_model(view)?;
                if m.find.finder().is_none() {
                    m.open_find();
                } else {
                    m.execute(&service, inv.id);
                }
            }
            id => {
                if !self.hex_model(view)?.execute(&service, id) {
                    return None;
                }
            }
        }
        Some(Vec::new())
    }

    /// Marks a hex view's session dirty (a rule or a spec changed in the view): the tab's title and the watched
    /// inputs follow the specs.
    fn hex_mark_dirty(&mut self, view: ViewId) {
        self.mark_session_dirty(view);
        let caption = self.tab(view).map(Tab::caption);
        if let Some(caption) = caption {
            self.shared.set_title(view, caption);
        }
        self.untrack_view(view);
        self.track_view(view);
    }

    /// Open With ▸ of a hex pane or header: *Text Compare* of the two files, *Text Edit* of the focus side's file, the
    /// `openwith.toml` applications on the focus side's file.
    fn hex_open_with(&mut self, window: WindowId, view: ViewId, item: Option<&CommandArg>) {
        let Some(m) = self.hex_model(view) else {
            return;
        };
        let side = m.focus();
        let inputs = m.inputs();
        let mine = inputs[crate::hex::index(side)].clone();
        match item {
            Some(CommandArg::OpenWith(OpenWithItem::TextEdit)) => {
                self.open_text_edit(window, mine, None);
            }
            Some(CommandArg::OpenWith(OpenWithItem::TextCompare)) => {
                let parse = |s: &str| self.service.parse_spec(std::ffi::OsStr::new(s)).ok();
                if let (Some(a), Some(b)) = (parse(&inputs[0]), parse(&inputs[1])) {
                    self.open_specs(window, [a, b], Some(ViewKind::TextCompare));
                }
            }
            Some(CommandArg::OpenWith(OpenWithItem::App(index))) => {
                let Ok(spec) = self.service.parse_spec(std::ffi::OsStr::new(&mine)) else {
                    return;
                };
                let name = self
                    .open_with_names()
                    .get(*index)
                    .cloned()
                    .unwrap_or_default();
                self.launch(super::ops::Launch {
                    view,
                    caption: name.clone(),
                    what: super::ops::LaunchWhat::App(*index, name),
                    items: vec![(spec.clone(), spec)],
                    refresh: None,
                });
            }
            _ => {}
        }
    }

    /// Next/Previous Difference Files of a child view (D-358): the parent folder view's next difference pair, or (that
    /// view closed) the parent compared again in a job as the text views do (D-311); a pair of this view's kind opens
    /// in this view, another in a new tab of its kind beside it (open default); the open parent view's row selection
    /// follows.
    fn hex_link_step(&mut self, window: WindowId, view: ViewId, forward: bool) {
        let Some(link) = self.parent_of(view) else {
            return;
        };
        if let Some(parent_view) = self.parent_view(&link) {
            let found = match self.tab(parent_view).and_then(Tab::model) {
                Some(ViewModel::Folder(m)) => m.prepared().cloned().map(|prepared| {
                    let row = hikasa_session::api::parent_row(&prepared.model().read(), &link);
                    super::text_views::ParentState {
                        prepared,
                        read_only: m.write_policy().read_only,
                        row,
                        display: *m.display_options(),
                    }
                }),
                _ => None,
            };
            if let Some(parent) = found {
                self.hex_link_with(window, view, forward, &link, &parent);
                return;
            }
        }
        let job = Job::spawn(
            &self.service,
            JobKind::FolderCompare,
            "Comparing the parent folders…".into(),
            &self.wake(),
            move |svc, job| {
                let parent = svc.text_parent_compare(&link, job, &hikasa_folder::NullObserver)?;
                Ok(super::text_views::MoreDone::Parent(
                    Box::new(super::text_views::ParentState::of_compare(parent)),
                    super::text_views::LinkStep::Next(forward),
                ))
            },
        );
        self.jobs.push(CoreJob::More(window, view, job));
    }

    /// A link step of a hex view with the parent's comparison.
    pub(super) fn hex_link_with(
        &mut self,
        window: WindowId,
        view: ViewId,
        forward: bool,
        link: &hikasa_session::sessions::ParentLink,
        parent: &super::text_views::ParentState,
    ) {
        let opts = self
            .service
            .next_file_options(parent.display, parent.prepared.ignore_unimportant());
        let Some(current) = self.hex_model(view).map(|m| m.request().clone()) else {
            return;
        };
        let probe = hikasa_session::api::TextCompareRequest {
            left: current.left.clone(),
            right: current.right.clone(),
            ..hikasa_session::api::TextCompareRequest::default()
        };
        let dir = if forward {
            hikasa_folder::view::Dir::Forward
        } else {
            hikasa_folder::view::Dir::Backward
        };
        let (node, pair, moved) =
            match crate::text::link::next_pair(&parent.prepared, link, dir, &opts, &probe) {
                Err(text) => {
                    self.set_message(view, Message::info(text));
                    return;
                }
                Ok(found) => found,
            };
        // The open parent view's row selection follows the pair.
        if let Some(parent_view) = self.parent_view(link)
            && let Some(ViewModel::Folder(m)) =
                self.tab_mut(parent_view).and_then(|t| t.model_mut())
        {
            m.select_node(node);
        }
        let overrides = parent.prepared.format_overrides().clone();
        let kind = self
            .service
            .view_kind_of_pair(&pair.left, &pair.right, &overrides);
        if kind == ViewKind::HexCompare {
            if let Some(m) = self.hex_model(view) {
                let mut request = m.request().clone();
                request.left = pair.left.clone();
                request.right = pair.right.clone();
                request.formats = Default::default();
                m.relive(request);
            }
            if let Some(s) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
                s.parent = Some(moved);
            }
            self.hex_mark_dirty(view);
        } else {
            let parse = |s: &OsString| self.service.parse_spec(s).ok();
            if let (Some(a), Some(b)) = (parse(&pair.left), parse(&pair.right)) {
                self.open_specs(window, [a, b], Some(kind));
            }
        }
    }

    /// The browse answer of a hex view's Open (that side reopens with the file). True when `target` was one.
    pub(super) fn hex_browsed(&mut self, target: &BrowseTarget, answer: Option<String>) -> bool {
        let BrowseTarget::HexOpen { view, side } = target else {
            return false;
        };
        let Some(path) = answer else {
            return true;
        };
        if let Some(m) = self.hex_model(*view) {
            let spec = OsString::from(path);
            match side {
                Side::Right => m.replace_specs(None, Some(spec)),
                _ => m.replace_specs(Some(spec), None),
            }
        }
        self.hex_mark_dirty(*view);
        true
    }

    /// A hex dialog left the stack: Go To moves the caret; Layout applies (and writes the defaults session when asked);
    /// the display font applies. True when `closed` was one of them.
    pub(super) fn hex_dialog_closed(
        &mut self,
        closed: &mut Box<dyn crate::dialog::DialogModel>,
    ) -> bool {
        if let Some(d) = closed.as_any_mut().downcast_mut::<HexGoToDialog>() {
            let (view, radix) = (d.view, d.radix);
            self.state
                .set_dialog_text(GOTO_DIALOG, "radix", radix.as_str());
            let word = radix.as_str();
            self.update_state(move |s| s.set_dialog_text(GOTO_DIALOG, "radix", word));
            if let Some((side, query, text)) = d.take_chosen() {
                self.push_mru(MruList::HexGoTo, text);
                if let Some(m) = self.hex_model(view) {
                    m.go_to(side, query);
                }
            }
            return true;
        }
        if let Some(d) = closed.as_any_mut().downcast_mut::<LayoutDialog>() {
            let view = d.view;
            if let Some((choice, defaults)) = d.take_chosen() {
                if let Some(m) = self.hex_model(view) {
                    m.set_layout(
                        choice.layout,
                        choice.addresses,
                        choice.panes,
                        choice.context,
                    );
                }
                if defaults {
                    self.hex_layout_defaults(view, choice);
                }
            }
            return true;
        }
        if let Some(d) = closed
            .as_any_mut()
            .downcast_mut::<crate::text::layout::DisplayFontDialog>()
        {
            let view = d.view;
            if self.hex_model(view).is_none() {
                return false;
            }
            if let Some((_, size, _)) = d.take_chosen()
                && let Some(m) = self.hex_model(view)
            {
                m.set_font_px(size);
            }
            return true;
        }
        false
    }

    /// *Use for new Hex Compare views* (D-356): the layout's `[view]` keys written to the Hex Compare defaults session
    /// (the keys `session edit --set` writes).
    fn hex_layout_defaults(&mut self, view: ViewId, choice: LayoutChoice) {
        let words = crate::hex::layout::view_words(&choice);
        self.store("save the defaults", Some(view), move |svc, _| {
            svc.session_edit(
                &hikasa_session::api::SessionTarget::Defaults(
                    hikasa_session::sessions::SessionType::HexCompare,
                ),
                &hikasa_session::api::SessionEdit {
                    set: words,
                    ..Default::default()
                },
                true,
            )?;
            Ok(super::flow::StoreDone::Written)
        });
    }

    /// The hex views' requests: clipboard writes in turn, the primary selection, message panel lines (a beep without
    /// the message panel, D-309), notices with *Reload* (D-118); the Find panel's patterns to `mru.hex_find`.
    pub(super) fn hex_poll(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut asked: Vec<(WindowId, ViewId, Vec<HexRequest>, Vec<String>)> = Vec::new();
        for w in &mut self.windows {
            for tab in &mut w.tabs {
                if let Some(ViewModel::Hex(m)) = tab.model_mut() {
                    let r = m.take_requests();
                    let found = m.find.take_searched();
                    if !r.is_empty() || !found.is_empty() {
                        asked.push((w.id, tab.id, r, found));
                    }
                }
            }
        }
        let show_message = self.options().next_difference.show_message_panel;
        for (window, view, requests, found) in asked {
            for pattern in found {
                self.push_mru(MruList::HexFind, pattern);
            }
            for request in requests {
                match request {
                    HexRequest::Clipboard(text, done) => {
                        self.hex_clipboard(window, view, text, done)
                    }
                    HexRequest::Primary(text) => {
                        let provider = Arc::clone(&self.script_clipboard);
                        let job = Job::spawn(
                            &self.service,
                            JobKind::VfsRead,
                            "primary selection".into(),
                            &self.wake(),
                            move |_, _| {
                                // No primary selection on this platform: nothing to do (D-298).
                                let _ = provider.write_primary(&text);
                                Ok(PaneDone::Named)
                            },
                        );
                        self.jobs.push(CoreJob::Pane(window, view, job));
                    }
                    HexRequest::Message(m) => {
                        if !show_message && m.text == crate::hex::NO_MORE {
                            effects.push(Effect::Beep(window));
                        } else {
                            self.set_message(view, m);
                        }
                    }
                    HexRequest::Notice { text, reload } => {
                        let mut notice = Notice::new(
                            format!("hex:{}:{text}", view.0),
                            NoticeKind::Warning,
                            text,
                        );
                        if reload {
                            notice = notice.with_actions(vec![NoticeAction::ReloadView(view)]);
                        }
                        self.notify(window, notice, "hex", LogKind::Warning);
                    }
                }
            }
        }
        effects
    }

    /// Writes `text` to the window's ordered clipboard in a job; `done` is the message panel line.
    fn hex_clipboard(&mut self, window: WindowId, view: ViewId, text: String, done: String) {
        let clipboard = Arc::clone(&self.script_clipboard);
        let ticket = clipboard.ticket();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy to the clipboard".into(),
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
        self.set_message(view, Message::info(done));
    }

    /// Drops onto a hex view (UI-008, D-251): one file replaces the side it was dropped on (the focus side without a
    /// place), two files replace both. True when the view took them.
    pub(super) fn hex_drop(
        &mut self,
        view: ViewId,
        side: Option<Side>,
        paths: &[std::path::PathBuf],
    ) -> bool {
        let Some(m) = self.hex_model(view) else {
            return false;
        };
        match paths {
            [one] => {
                let side = side.unwrap_or(m.focus());
                let spec = one.clone().into_os_string();
                match side {
                    Side::Right => m.replace_specs(None, Some(spec)),
                    _ => m.replace_specs(Some(spec), None),
                }
            }
            [a, b] => m.replace_specs(
                Some(a.clone().into_os_string()),
                Some(b.clone().into_os_string()),
            ),
            _ => return false,
        }
        self.hex_mark_dirty(view);
        true
    }
}

#[cfg(test)]
impl AppCore {
    /// A list of `[mru]` (tests).
    pub fn state_mru_for_test(&self, list: MruList) -> Vec<String> {
        self.state.mru(list).to_vec()
    }
}
