//! The window's side of the File Formats manager, the format editor and the text rule tabs (M2 W11c; contract
//! §1.11.25–§1.11.27): opening them (`FileFormatManager`, `FileFormat` without an argument, `Rules` of a text view,
//! *Edit Grammar…*, the manager's *Edit…*), routing their prompts' answers and browse dialogs back to them, the *Reopen*
//! notices of the formats they wrote, and the text views' OK of Session Settings: the view relives with its edits kept
//! (§1.11.13), a pane whose encoding or file changed reopens.

use hikasa_core::Side;
use hikasa_session::api::{SessionView, ViewRequest};
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::Session;

use super::{AppCore, ViewModel};
use crate::browse::BrowseTarget;
use crate::formats::editor::FormatEditor;
use crate::formats::{FileFormatManager, FormatsFollowup};
use crate::session_dialog::TextFacts;
use crate::window::WindowId;

impl AppCore {
    /// Tools ▸ *File Formats…* (D-318): the manager over the registry as it is now.
    pub(super) fn open_format_manager(&mut self, window: WindowId) {
        let manager = FileFormatManager::new(&self.service, self.wake());
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(manager));
        }
    }

    /// The format editor of `id` (an id or a name) over the window's dialogs.
    pub(super) fn open_format_editor(&mut self, window: WindowId, id: &str) {
        let editor = FormatEditor::open(&self.service, id, self.wake());
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(editor));
        }
    }

    /// `FileFormat` without an argument (the toolbar's *Format*, D-316): the editor of the focus pane's format (a
    /// built-in: its override is written, D-75).
    pub(super) fn edit_focus_format(&mut self, window: WindowId) {
        let Some(id) = self.focus_format(window) else {
            return;
        };
        self.open_format_editor(window, &id);
    }

    /// The format of the active text view's focus pane: its id (the name when the format has none).
    fn focus_format(&self, window: WindowId) -> Option<String> {
        let tab = self.active_tab(window)?;
        let Some(ViewModel::Text(m)) = tab.model() else {
            return None;
        };
        if let Some(r) = m.edit_response() {
            return Some(
                r.format
                    .id
                    .as_ref()
                    .map_or_else(|| r.format.name.clone(), |id| id.as_str().to_owned()),
            );
        }
        let r = m.response()?;
        let f = match m.focus() {
            Side::Right => r.right_format.as_ref().unwrap_or(&r.format),
            _ => &r.format,
        };
        Some(
            f.id.as_ref()
                .map_or_else(|| f.name.clone(), |id| id.as_str().to_owned()),
        )
    }

    /// What the active text view tells its rule tabs (§1.11.25): its panes' formats, the focus pane's, the engine *Auto*
    /// chose last.
    pub(super) fn text_facts(&self, window: WindowId) -> TextFacts {
        let mut facts = TextFacts::default();
        let Some(tab) = self.active_tab(window) else {
            return facts;
        };
        let Some(ViewModel::Text(m)) = tab.model() else {
            return facts;
        };
        facts.view_formats = crate::text::info::formats_used(m);
        facts.focus_format = self.focus_format(window);
        facts.auto_engine = m
            .comparison()
            .map(|c| crate::text::engine_name(c.algorithm()).to_owned());
        facts
    }

    /// An answer of the manager's or an editor's prompts.
    pub(super) fn formats_followup(&mut self, window: WindowId, f: FormatsFollowup) {
        let service = self.service.clone();
        let Some(w) = self.window_mut(window) else {
            return;
        };
        match f {
            FormatsFollowup::Conflict { id, answer } => {
                if let Some(e) = w.dialogs.find_mut_where::<FormatEditor>(|e| e.id == id) {
                    e.conflict(&service, answer);
                }
            }
            FormatsFollowup::Item { id, done } => {
                if let Some(e) = w.dialogs.find_mut_where::<FormatEditor>(|e| e.id == id) {
                    e.item_done(*done);
                }
            }
            other => {
                if let Some(m) = w.dialogs.find_mut::<FileFormatManager>() {
                    m.answer(other);
                }
            }
        }
    }

    /// The browse dialog's answer for the manager or an editor; true when it was theirs.
    pub(super) fn formats_browsed(
        &mut self,
        window: WindowId,
        target: &BrowseTarget,
        answer: Option<String>,
    ) -> bool {
        let service = self.service.clone();
        let Some(w) = self.window_mut(window) else {
            return false;
        };
        match target {
            BrowseTarget::FormatImport => {
                if let (Some(path), Some(m)) = (answer, w.dialogs.find_mut::<FileFormatManager>()) {
                    m.import(path.into(), false);
                }
                true
            }
            BrowseTarget::FormatExport(_) => {
                if let (Some(path), Some(m)) = (answer, w.dialogs.find_mut::<FileFormatManager>()) {
                    m.export(path.into());
                }
                true
            }
            BrowseTarget::FormatSample(id) => {
                if let (Some(path), Some(e)) = (
                    answer,
                    w.dialogs.find_mut_where::<FormatEditor>(|e| &e.id == id),
                ) {
                    e.load_sample(&service, path.into());
                }
                true
            }
            _ => false,
        }
    }

    /// OK of Session Settings on a text view (§1.11.25): the view keeps its documents and edits. A changed file or
    /// encoding reopens that pane (asking D-301 first when it has edits); new rules or formats relive the comparison
    /// (§1.11.13). A Text Edit view reopens its file with a new format or encoding. False when the view is no text view
    /// with documents (the caller rebuilds it).
    pub(super) fn relive_text(
        &mut self,
        view: ViewId,
        opened: &SessionView,
        session: &Session,
        dirty: bool,
    ) -> bool {
        let ViewRequest::Text { request, .. } = &opened.request else {
            return false;
        };
        let service = self.service.clone();
        let Some(tab) = self.tab_mut(view) else {
            return false;
        };
        let Some(ViewModel::Text(m)) = tab.model_mut() else {
            return false;
        };
        if m.live_view().is_none() && m.edit_response().is_none() {
            return false;
        }
        let old = m.request().clone();
        let mut write = m.write_policy();
        write.read_only = hikasa_folder::ops::ReadOnlySides {
            left: session.specs.read_only_left,
            right: session.specs.read_only_right,
        };
        m.set_write_policy(write);
        if m.is_edit() {
            // One pane: a format pick or an encoding reopens the file (D-301 asked by the window).
            let format = request.format.clone();
            if format != old.format {
                m.request_format(Side::Left, format);
            } else if request.encodings.left != old.encodings.left {
                m.request_reopen(Side::Left, None, Some(request.encodings.left.clone()));
            }
        } else {
            let modified = crate::text::save::modified_sides(m);
            for (side, spec, enc, old_spec, old_enc) in [
                (
                    Side::Left,
                    &request.left,
                    &request.encodings.left,
                    &old.left,
                    &old.encodings.left,
                ),
                (
                    Side::Right,
                    &request.right,
                    &request.encodings.right,
                    &old.right,
                    &old.encodings.right,
                ),
            ] {
                if spec != old_spec || enc != old_enc {
                    m.request_reopen(
                        side,
                        (spec != old_spec).then(|| spec.clone()),
                        (enc != old_enc).then(|| enc.clone()),
                    );
                    // A pane without edits reopens without a question: the view's request (what a session saved
                    // meanwhile holds) takes its file and encoding now.
                    if !modified.contains(&side) {
                        let r = m.request_mut();
                        match side {
                            Side::Right => {
                                r.right.clone_from(spec);
                                r.encodings.right.clone_from(enc);
                            }
                            _ => {
                                r.left.clone_from(spec);
                                r.encodings.left.clone_from(enc);
                            }
                        }
                    }
                }
            }
            let relive = request.base_rules != old.base_rules
                || request.rules_file != old.rules_file
                || request.rules_patch != old.rules_patch
                || request.format != old.format
                || request.formats != old.formats;
            if relive {
                // The other fields of the request as they are now; the anchors and ignored lines are the view's.
                let mut next = m.request().clone();
                next.base_rules.clone_from(&request.base_rules);
                next.rules_file.clone_from(&request.rules_file);
                next.rules_patch.clone_from(&request.rules_patch);
                next.format.clone_from(&request.format);
                next.formats.clone_from(&request.formats);
                // One pane's own format changed: its editing options follow (tab size, a new characters-per-line
                // limit re-wraps it, D-304).
                let left = next.formats.left.as_ref().or(next.format.as_ref());
                let right = next.formats.right.as_ref().or(next.format.as_ref());
                let was_left = old.formats.left.as_ref().or(old.format.as_ref());
                let was_right = old.formats.right.as_ref().or(old.format.as_ref());
                let cause = match (left != was_left, right != was_right) {
                    (true, false) => crate::text::relive::Cause::Format(Side::Left, left.cloned()),
                    (false, true) => {
                        crate::text::relive::Cause::Format(Side::Right, right.cloned())
                    }
                    _ => crate::text::relive::Cause::Rules,
                };
                // The view's request holds the new rules at once (a session saved meanwhile has them).
                *m.request_mut() = next.clone();
                crate::text::relive::start(m, &service, next, cause);
            }
        }
        if let Some(vs) = tab.session.as_mut() {
            vs.dirty = dirty;
            vs.settings_pending = false;
            vs.description.clone_from(&session.description);
        }
        true
    }
}
