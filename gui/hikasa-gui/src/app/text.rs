//! The window's side of a Text Compare view's editor (M2 W11a, contract §1.11.7–§1.11.13): `Copy`, `Cut`, `Paste`
//! and the primary selection through the window's clipboard provider (jobs, D-252); `Save`, `Save All`, `Save As`
//! and `Save on File System As` (the browse dialog, the save's questions as prompts, D-300); the binary question
//! (D-297); the unsaved-edits prompts of every close (D-301); a reopen of one side and a format pick (D-304). The
//! view's model asks for these through its requests ([`TextRequest`]), taken after every call and poll.

use std::collections::HashSet;

use hikasa_core::{JobKind, Reply, Side};
use hikasa_session::clipboard::{
    ClipboardContent, ClipboardError, ClipboardPayload, ClipboardProvider as _,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::log::LogKind;

use super::{AppCore, CoreJob, Effect};
use crate::browse::{BrowseMode, BrowseTarget};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, Followup, Prompt};
use crate::job::Job;
use crate::notify::{Notice, NoticeKind};
use crate::status::Message;
use crate::text::save::{self, SaveReply, SaveTarget};
use crate::text::{TextRequest, TextViewModel, clipboard, edit, relive};
use crate::window::{Invocation, ViewModel, WindowId};

/// The help topic of the unsaved-edits prompt.
pub const UNSAVED_TOPIC: &str = "dialog-unsaved-edits";
/// The help topic of the save questions.
pub const SAVE_TOPIC: &str = "dialog-save-questions";
/// The button of *Save as UTF-8*.
pub const SAVE_UTF8: ButtonRole = ButtonRole::Other(20);
/// The buttons *Yes to All* and *No to All* of the save questions.
pub const YES_TO_ALL: ButtonRole = ButtonRole::Other(21);
/// *No to All*.
pub const NO_TO_ALL: ButtonRole = ButtonRole::Other(22);
/// *Save All* of the unsaved-edits prompt.
pub const SAVE_ALL: ButtonRole = ButtonRole::Other(23);
/// *Discard All* of the unsaved-edits prompt.
pub const DISCARD_ALL: ButtonRole = ButtonRole::Other(24);

/// What a clipboard job of a text view brought back.
pub(crate) enum TextDone {
    /// The clipboard was written.
    Copied,
    /// Text to paste into a pane (`None`: the clipboard holds no text).
    Paste(Side, Option<String>),
}

/// The answers of the text prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextFollowup {
    /// The binary question of a pane.
    Binary {
        /// The view.
        view: ViewId,
        /// The pane.
        side: Side,
        /// Yes.
        yes: bool,
    },
    /// An answer to a save question.
    SaveReply {
        /// The view.
        view: ViewId,
        /// The answer.
        reply: SaveReply,
    },
    /// An answer to the unsaved-edits prompt of a close.
    Unsaved(ButtonRole),
    /// Save the view's modified panes, then run the command (a single action that drops the edits).
    SaveThen {
        /// The view.
        view: ViewId,
        /// The command, run confirmed after the save.
        then: Invocation,
    },
    /// Reopen a side after the prompt (Discard).
    Reopen {
        /// The view.
        view: ViewId,
        /// The side.
        side: Side,
    },
}

/// The unsaved-edits state of a close flow (D-301).
#[derive(Debug, Default)]
pub struct Unsaved {
    /// The documents answered (saved or discarded).
    pub answered: HashSet<(ViewId, Side)>,
    /// *Save All* (true) or *Discard All* (false) was chosen.
    pub all: Option<bool>,
    /// The prompt asks about this document.
    pub asking: Option<(ViewId, Side)>,
    /// The close waits for the save of this document.
    pub saving: Option<(ViewId, Side)>,
    /// Prompts shown (tests: once per document).
    pub asked: u32,
}

/// What the close flow does with a view's edits.
pub enum UnsavedStep {
    /// Nothing waits: the session's question comes next.
    Continue,
    /// A prompt or a save waits.
    Wait(Vec<Effect>),
}

impl AppCore {
    /// The text view of a tab.
    fn text_model(&mut self, view: ViewId) -> Option<&mut TextViewModel> {
        match self.tab_mut(view).and_then(|t| t.model_mut()) {
            Some(ViewModel::Text(m)) => Some(m),
            _ => None,
        }
    }

    /// The active tab's text view.
    fn active_text(&self, window: WindowId) -> Option<ViewId> {
        let tab = self.active_tab(window)?;
        matches!(tab.model(), Some(ViewModel::Text(_))).then_some(tab.id)
    }

    /// The commands the window runs for a text view (§1.11.7, §1.11.9): `None` for other ids or views.
    pub(super) fn text_command(
        &mut self,
        window: WindowId,
        inv: &Invocation,
        confirmed: bool,
    ) -> Option<Vec<Effect>> {
        let view = self.active_text(window)?;
        let service = self.service.clone();
        match inv.id {
            CommandId::Copy => self.text_copy(window, view, false),
            CommandId::Cut => self.text_copy(window, view, true),
            CommandId::Paste => {
                let side = self.text_model(view)?.focus();
                self.text_paste(window, view, side, false);
            }
            CommandId::Save => {
                let m = self.text_model(view)?;
                let side = m.focus();
                match save::save_target_of(m, side) {
                    Some(target) => {
                        save::begin(m, &service, vec![side], vec![target]);
                    }
                    None => self.text_save_as(window, view, side, false),
                }
            }
            CommandId::SaveAll => {
                let m = self.text_model(view)?;
                let sides: Vec<Side> = save::modified_sides(m)
                    .into_iter()
                    .filter(|s| save::save_target_of(m, *s).is_some())
                    .collect();
                if sides.is_empty() {
                    let side = m.focus();
                    self.text_save_as(window, view, side, false);
                } else {
                    let targets = vec![SaveTarget::InPlace; sides.len()];
                    save::begin(m, &service, sides, targets);
                }
            }
            CommandId::SaveAs | CommandId::SaveAsLocal => {
                let side = self.text_model(view)?.focus();
                self.text_save_as(window, view, side, inv.id == CommandId::SaveAsLocal);
            }
            // The editor's ids that the folder views run as operations (D-299, §1.11.6).
            CommandId::CopyRight
            | CommandId::CopyLeft
            | CommandId::CopySide
            | CommandId::CopyItemRight
            | CommandId::CopyItemLeft
            | CommandId::CopyItemSide
            | CommandId::SelectAll
            | CommandId::Delete => {
                let after_copy = !matches!(inv.id, CommandId::Delete | CommandId::SelectAll)
                    && self
                        .options()
                        .next_difference
                        .go_to_next_difference_after_copy;
                let m = self.text_model(view)?;
                let before = m.generation();
                m.execute(&service, inv.id);
                if after_copy && m.generation() != before {
                    m.execute(&service, CommandId::DiffSectionNext);
                }
            }
            CommandId::Reload if !confirmed => {
                let modified = !save::modified_sides(self.text_model(view)?).is_empty();
                if !modified {
                    return None;
                }
                return Some(self.ask_unsaved_then(window, view, inv.clone()));
            }
            _ => return None,
        }
        Some(self.text_requests())
    }

    /// Copy (and Cut): the focus pane's text to the clipboard (a job); Cut then deletes it as one step.
    fn text_copy(&mut self, window: WindowId, view: ViewId, cut: bool) {
        let Some(m) = self.text_model(view) else {
            return;
        };
        let Some(text) = clipboard::copy_text(m) else {
            return;
        };
        if cut {
            clipboard::cut(m);
        }
        let provider = std::sync::Arc::clone(&self.script_clipboard);
        let ticket = provider.ticket();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "copy to the clipboard".into(),
            &self.wake(),
            move |_, _| {
                provider
                    .in_turn(ticket, |c| {
                        c.write_payload(&ClipboardPayload { text, html: None })
                    })
                    .map_err(|e| hikasa_session::ApiError::Vfs(e.to_vfs_write()))?;
                Ok(TextDone::Copied)
            },
        );
        self.jobs.push(CoreJob::Text(window, view, job));
    }

    /// Paste (or a middle click's primary paste) into `side`: the clipboard is read in a job.
    fn text_paste(&mut self, window: WindowId, view: ViewId, side: Side, primary: bool) {
        let provider = std::sync::Arc::clone(&self.script_clipboard);
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "read the clipboard".into(),
            &self.wake(),
            move |_, _| {
                if primary {
                    return Ok(TextDone::Paste(side, provider.read_primary().ok()));
                }
                match provider.read() {
                    Ok(ClipboardContent::Text(text)) => Ok(TextDone::Paste(side, Some(text))),
                    Ok(ClipboardContent::Image { .. })
                    | Err(ClipboardError::Empty | ClipboardError::Unavailable(_)) => {
                        let files = provider.read_files().unwrap_or_default();
                        let text = (!files.is_empty()).then(|| clipboard::paths_text(&files));
                        Ok(TextDone::Paste(side, text))
                    }
                    Err(e) => Err(hikasa_session::ApiError::Vfs(e.to_vfs())),
                }
            },
        );
        self.jobs.push(CoreJob::Text(window, view, job));
    }

    /// A clipboard job of a text view ended.
    pub(super) fn text_done(
        &mut self,
        window: WindowId,
        view: ViewId,
        result: Result<TextDone, hikasa_session::ApiError>,
    ) -> Vec<Effect> {
        match result {
            Ok(TextDone::Copied) => {}
            Ok(TextDone::Paste(side, text)) => {
                if let Some(m) = self.text_model(view) {
                    match text {
                        Some(text) => {
                            clipboard::paste(m, side, &text);
                        }
                        None => m.request_window(TextRequest::Message(Message::info(
                            clipboard::NO_TEXT,
                        ))),
                    }
                }
            }
            Err(e) => {
                let text = format!("The clipboard could not be used: {e}");
                self.notify(
                    window,
                    Notice::new(format!("clipboard:{}", view.0), NoticeKind::Warning, text),
                    "clipboard",
                    LogKind::Warning,
                );
            }
        }
        self.text_requests()
    }

    /// Save As (and Save on File System As) of `side`: the browse dialog in SaveFile mode, the document's name and
    /// folder offered.
    fn text_save_as(&mut self, window: WindowId, view: ViewId, side: Side, local: bool) {
        let Some(m) = self.text_model(view) else {
            return;
        };
        let current = crate::text::header::title(m, side);
        let default_name = crate::browse::file_name_of(&current);
        let start = current
            .strip_suffix(&default_name)
            .unwrap_or(&current)
            .to_owned();
        self.open_browse(
            window,
            BrowseMode::SaveFile { default_name },
            start,
            BrowseTarget::TextSaveAs { view, side, local },
        );
    }

    /// The browse dialog of a Save As answered (`None`: cancelled): the save starts with that target.
    pub(super) fn text_save_as_answered(
        &mut self,
        window: WindowId,
        view: ViewId,
        side: Side,
        local: bool,
        answer: Option<String>,
    ) {
        let service = self.service.clone();
        let Some(path) = answer else {
            // A close waiting for this Save As is cancelled.
            if self
                .flow
                .as_ref()
                .is_some_and(|f| f.unsaved.saving == Some((view, side)))
                && let Some(flow) = self.flow.take()
            {
                self.end_flow_pub(flow);
            }
            return;
        };
        let spec = match hikasa_core::spec::Spec::parse(&path, &service.spec_context()) {
            Ok(spec) => spec,
            Err(e) => {
                self.set_message(view, Message::error(format!("Not a file to save to: {e}")));
                return;
            }
        };
        if local && (spec.archive_depth() > 0 || spec.scheme_name() != "file") {
            self.set_message(
                view,
                Message::info("Save on File System As saves to a file of the file system only."),
            );
            return;
        }
        if let Some(m) = self.text_model(view) {
            save::begin(m, &service, vec![side], vec![SaveTarget::As(spec)]);
        }
        let _ = window;
        let _ = self.text_requests();
    }

    /// Takes every text view's requests and acts on them (prompts, messages, notices, the primary selection).
    pub(super) fn text_requests(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        let mut taken = Vec::new();
        for w in &mut self.windows {
            for tab in &mut w.tabs {
                let caption = tab.caption();
                if let Some(ViewModel::Text(m)) = tab.model_mut() {
                    let r = m.take_requests();
                    if !r.is_empty() {
                        taken.push((w.id, tab.id, caption, r));
                    }
                }
            }
        }
        for (window, view, caption, requests) in taken {
            for request in requests {
                effects.extend(self.text_request(window, view, &caption, request));
            }
        }
        effects
    }

    fn text_request(
        &mut self,
        window: WindowId,
        view: ViewId,
        caption: &str,
        request: TextRequest,
    ) -> Vec<Effect> {
        match request {
            TextRequest::AskBinary(side) => {
                let name = self
                    .text_model(view)
                    .map(|m| crate::text::name_of(&crate::text::header::title(m, side)))
                    .unwrap_or_default();
                let answer = |yes| vec![Followup::Text(TextFollowup::Binary { view, side, yes })];
                let prompt = Prompt::ask(
                    caption,
                    format!("{name} contains binary data. Edit anyway?"),
                    ButtonRole::No,
                    vec![
                        (ButtonRole::Yes, "Yes", answer(true)),
                        (ButtonRole::No, "No", answer(false)),
                    ],
                    "view-text-compare",
                );
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            TextRequest::Message(m) => self.set_message(view, m),
            TextRequest::Warning(text) => self.notify(
                window,
                Notice::new(format!("text:{}", view.0), NoticeKind::Warning, text),
                caption,
                LogKind::Warning,
            ),
            TextRequest::WritePrimary(text) => {
                let provider = std::sync::Arc::clone(&self.script_clipboard);
                let job = Job::spawn(
                    &self.service,
                    JobKind::VfsRead,
                    "primary selection".into(),
                    &self.wake(),
                    move |_, _| {
                        // No primary selection on this platform: nothing to do (D-298).
                        let _ = provider.write_primary(&text);
                        Ok(TextDone::Copied)
                    },
                );
                self.jobs.push(CoreJob::Text(window, view, job));
            }
            TextRequest::PastePrimary(side) => self.text_paste(window, view, side, true),
            TextRequest::SaveAsked => {
                let Some(ask) = self.text_model(view).and_then(|m| save::asking(m)) else {
                    return Vec::new();
                };
                let reply = |r| vec![Followup::Text(TextFollowup::SaveReply { view, reply: r })];
                let mut buttons = vec![
                    (ButtonRole::Yes, "Yes", reply(SaveReply::Reply(Reply::Yes))),
                    (ButtonRole::No, "No", reply(SaveReply::Reply(Reply::No))),
                ];
                if ask.remaining > 1 {
                    buttons.push((
                        YES_TO_ALL,
                        "Yes to All",
                        reply(SaveReply::Reply(Reply::YesToAll)),
                    ));
                    buttons.push((
                        NO_TO_ALL,
                        "No to All",
                        reply(SaveReply::Reply(Reply::NoToAll)),
                    ));
                }
                if ask.unmappable {
                    buttons.push((SAVE_UTF8, "Save as UTF-8", reply(SaveReply::Utf8)));
                }
                buttons.push((
                    ButtonRole::Cancel,
                    "Cancel",
                    reply(SaveReply::Reply(Reply::Cancel)),
                ));
                let text = if ask.remaining > 1 {
                    format!("{}\n({} questions left)", ask.text, ask.remaining)
                } else {
                    ask.text
                };
                let prompt = Prompt::ask("Save", text, ButtonRole::No, buttons, SAVE_TOPIC);
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            TextRequest::Saved { text, ok } => {
                let kind = if ok {
                    NoticeKind::Info
                } else {
                    NoticeKind::Warning
                };
                self.notify(
                    window,
                    Notice::new(format!("save:{}", view.0), kind, text),
                    caption,
                    if ok {
                        LogKind::Detail
                    } else {
                        LogKind::Warning
                    },
                );
            }
            TextRequest::Reopen(side) => {
                let modified = self
                    .text_model(view)
                    .and_then(|m| m.doc(side))
                    .is_some_and(hikasa_text::EditableDocument::is_modified);
                if modified {
                    return self.ask_unsaved_reopen(window, view, side, caption);
                }
                self.text_reopen(view, side);
            }
            TextRequest::Relive(cause) => {
                let service = self.service.clone();
                if let Some(m) = self.text_model(view) {
                    let request = m.request().clone();
                    relive::start(m, &service, request, cause);
                }
            }
        }
        Vec::new()
    }

    /// Reads one side again (its edits dropped) and relives the view with the other side's live document.
    fn text_reopen(&mut self, view: ViewId, side: Side) {
        let service = self.service.clone();
        if let Some(m) = self.text_model(view) {
            let request = m.take_reopen_request();
            relive::start(m, &service, request, relive::Cause::Reopen(side));
        }
    }

    /// The answers of the text prompts.
    pub(super) fn text_followup(&mut self, window: WindowId, f: TextFollowup) -> Vec<Effect> {
        let service = self.service.clone();
        match f {
            TextFollowup::Binary { view, side, yes } => {
                if let Some(m) = self.text_model(view) {
                    edit::answer_binary(m, side, yes);
                }
            }
            TextFollowup::SaveReply { view, reply } => {
                if let Some(m) = self.text_model(view) {
                    save::answer(m, &service, reply);
                }
            }
            TextFollowup::Unsaved(role) => return self.unsaved_answer(role),
            TextFollowup::SaveThen { view, then } => {
                if let Some(m) = self.text_model(view) {
                    let sides: Vec<Side> = save::modified_sides(m)
                        .into_iter()
                        .filter(|s| save::save_target_of(m, *s).is_some())
                        .collect();
                    let targets = vec![SaveTarget::InPlace; sides.len()];
                    if save::begin(m, &service, sides, targets) {
                        self.text_after_save.insert(view, (window, then));
                    }
                }
            }
            TextFollowup::Reopen { view, side } => self.text_reopen(view, side),
        }
        self.text_requests()
    }

    /// A single action that would drop a view's edits (Reload): *Save changes?* Save runs the save, then the command;
    /// Discard runs it; Cancel nothing.
    fn ask_unsaved_then(
        &mut self,
        window: WindowId,
        view: ViewId,
        then: Invocation,
    ) -> Vec<Effect> {
        let caption = self.tab(view).map(|t| t.caption()).unwrap_or_default();
        let name = self.unsaved_names(view);
        let prompt = Prompt::ask(
            caption,
            format!("Save changes to {name}?"),
            ButtonRole::Yes,
            vec![
                (
                    ButtonRole::Yes,
                    "Save",
                    vec![Followup::Text(TextFollowup::SaveThen {
                        view,
                        then: then.clone(),
                    })],
                ),
                (ButtonRole::No, "Discard", vec![Followup::Run(then)]),
                (ButtonRole::Cancel, "Cancel", Vec::new()),
            ],
            UNSAVED_TOPIC,
        );
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(prompt));
        }
        Vec::new()
    }

    /// A reopen of a modified side: *Save changes?* first (Save keeps the side as saved and reopens nothing more).
    fn ask_unsaved_reopen(
        &mut self,
        window: WindowId,
        view: ViewId,
        side: Side,
        caption: &str,
    ) -> Vec<Effect> {
        let name = self
            .text_model(view)
            .map(|m| crate::text::name_of(&crate::text::header::title(m, side)))
            .unwrap_or_default();
        let prompt = Prompt::ask(
            caption,
            format!("Save changes to {name}?\nReopening the file drops its edits."),
            ButtonRole::Cancel,
            vec![
                (
                    ButtonRole::No,
                    "Discard",
                    vec![Followup::Text(TextFollowup::Reopen { view, side })],
                ),
                (ButtonRole::Cancel, "Cancel", Vec::new()),
            ],
            UNSAVED_TOPIC,
        );
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(prompt));
        }
        Vec::new()
    }

    /// The names of a view's modified documents.
    fn unsaved_names(&mut self, view: ViewId) -> String {
        let Some(m) = self.text_model(view) else {
            return String::new();
        };
        save::modified_sides(m)
            .into_iter()
            .map(|s| crate::text::name_of(&crate::text::header::title(m, s)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The close flow's step for a view's edits (D-301): one prompt per modified document, before the view's session
    /// question; a Save waits for the save.
    pub(super) fn unsaved_step(&mut self, view: ViewId) -> UnsavedStep {
        let service = self.service.clone();
        let Some(flow) = self.flow.as_ref() else {
            return UnsavedStep::Continue;
        };
        if flow.unsaved.saving.is_some() || flow.unsaved.asking.is_some() {
            return UnsavedStep::Wait(Vec::new());
        }
        let all = flow.unsaved.all;
        let answered = flow.unsaved.answered.clone();
        let Some(m) = self.text_model(view) else {
            return UnsavedStep::Continue;
        };
        let Some(side) = save::modified_sides(m)
            .into_iter()
            .find(|s| !answered.contains(&(view, *s)))
        else {
            return UnsavedStep::Continue;
        };
        match all {
            Some(false) => {
                if let Some(flow) = self.flow.as_mut() {
                    flow.unsaved.answered.insert((view, side));
                }
                return self.unsaved_step(view);
            }
            Some(true) => return self.unsaved_save(view, side, &service),
            None => {}
        }
        let Some(window) = self.window_of(view) else {
            return UnsavedStep::Continue;
        };
        if let Some(index) = self.window(window).and_then(|w| w.index_of(view)) {
            self.activate(window, index);
        }
        let caption = self.tab(view).map(|t| t.caption()).unwrap_or_default();
        let name = self
            .text_model(view)
            .map(|m| crate::text::name_of(&crate::text::header::title(m, side)))
            .unwrap_or_default();
        let more = self.unsaved_more(view, side);
        let answer = |role| vec![Followup::Text(TextFollowup::Unsaved(role))];
        let mut buttons = vec![
            (ButtonRole::Yes, "Save", answer(ButtonRole::Yes)),
            (ButtonRole::No, "Discard", answer(ButtonRole::No)),
        ];
        if more {
            buttons.push((SAVE_ALL, "Save All", answer(SAVE_ALL)));
            buttons.push((DISCARD_ALL, "Discard All", answer(DISCARD_ALL)));
        }
        buttons.push((ButtonRole::Cancel, "Cancel", answer(ButtonRole::Cancel)));
        let prompt = Prompt::ask(
            caption,
            format!("Save changes to {name}?"),
            ButtonRole::Yes,
            buttons,
            UNSAVED_TOPIC,
        );
        if let Some(flow) = self.flow.as_mut() {
            flow.unsaved.asking = Some((view, side));
            flow.unsaved.asked += 1;
        }
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(prompt));
        }
        self.focus(window);
        UnsavedStep::Wait(vec![Effect::Raise(window)])
    }

    /// More modified documents wait in this close after `(view, side)`.
    fn unsaved_more(&self, view: ViewId, side: Side) -> bool {
        let Some(flow) = self.flow.as_ref() else {
            return false;
        };
        let pending = |v: ViewId| -> Vec<Side> {
            match self.tab(v).and_then(|t| t.model()) {
                Some(ViewModel::Text(m)) => save::modified_sides(m),
                _ => Vec::new(),
            }
        };
        flow.views
            .iter()
            .flat_map(|v| pending(*v).into_iter().map(move |s| (*v, s)))
            .any(|k| k != (view, side) && !flow.unsaved.answered.contains(&k))
    }

    /// Starts the save of a document of a close (Save As for a document without location); the close waits.
    fn unsaved_save(
        &mut self,
        view: ViewId,
        side: Side,
        service: &hikasa_session::Service,
    ) -> UnsavedStep {
        if let Some(flow) = self.flow.as_mut() {
            flow.unsaved.saving = Some((view, side));
        }
        let Some(m) = self.text_model(view) else {
            return UnsavedStep::Continue;
        };
        match save::save_target_of(m, side) {
            Some(target) => {
                if !save::begin(m, service, vec![side], vec![target]) {
                    // Refused (blocked): the close is cancelled.
                    if let Some(flow) = self.flow.take() {
                        self.end_flow_pub(flow);
                    }
                    let _ = self.text_requests();
                    return UnsavedStep::Wait(Vec::new());
                }
            }
            None => {
                if let Some(window) = self.window_of(view) {
                    self.text_save_as(window, view, side, false);
                }
            }
        }
        let effects = self.text_requests();
        UnsavedStep::Wait(effects)
    }

    /// An answer of the unsaved-edits prompt.
    fn unsaved_answer(&mut self, role: ButtonRole) -> Vec<Effect> {
        let service = self.service.clone();
        let Some(asking) = self.flow.as_mut().and_then(|f| f.unsaved.asking.take()) else {
            return Vec::new();
        };
        let (view, side) = asking;
        match role {
            ButtonRole::Cancel => {
                if let Some(flow) = self.flow.take() {
                    self.end_flow_pub(flow);
                }
                return Vec::new();
            }
            ButtonRole::No => {
                if let Some(flow) = self.flow.as_mut() {
                    flow.unsaved.answered.insert((view, side));
                }
            }
            r if r == DISCARD_ALL => {
                if let Some(flow) = self.flow.as_mut() {
                    flow.unsaved.answered.insert((view, side));
                    flow.unsaved.all = Some(false);
                }
            }
            r if r == SAVE_ALL || r == ButtonRole::Yes => {
                if r == SAVE_ALL
                    && let Some(flow) = self.flow.as_mut()
                {
                    flow.unsaved.all = Some(true);
                }
                if let UnsavedStep::Wait(effects) = self.unsaved_save(view, side, &service) {
                    return effects;
                }
            }
            _ => {}
        }
        self.advance_close_pub()
    }

    /// Each frame: a close waiting for a save learns how it ended; a command waiting for a save runs.
    pub(super) fn text_poll(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some((view, side)) = self.flow.as_ref().and_then(|f| f.unsaved.saving) {
            let ended = self.text_model(view).and_then(save::take_ended);
            match ended {
                Some(save::Ended::Saved) => {
                    if let Some(flow) = self.flow.as_mut() {
                        flow.unsaved.saving = None;
                        flow.unsaved.answered.insert((view, side));
                    }
                    effects.extend(self.advance_close_pub());
                }
                Some(save::Ended::NotSaved) => {
                    // A failed, declined or cancelled save cancels the whole close (D-301).
                    if let Some(flow) = self.flow.take() {
                        self.end_flow_pub(flow);
                    }
                }
                None => {}
            }
        }
        let waiting: Vec<ViewId> = self.text_after_save.keys().copied().collect();
        for view in waiting {
            let ended = self.text_model(view).and_then(save::take_ended);
            match ended {
                Some(save::Ended::Saved) => {
                    if let Some((window, inv)) = self.text_after_save.remove(&view) {
                        effects.extend(self.run_confirmed(window, inv));
                    }
                }
                Some(save::Ended::NotSaved) => {
                    self.text_after_save.remove(&view);
                }
                None => {}
            }
        }
        effects.extend(self.text_requests());
        effects
    }
}
