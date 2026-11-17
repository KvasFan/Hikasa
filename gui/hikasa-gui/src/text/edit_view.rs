//! The Text Edit view (TE-001; M2 W11b, contract §1.11.22, D-313), toolkit-free: one file in one pane, no
//! comparison. Its model is the Text Compare model in its single-pane mode ([`TextViewModel::new_edit`]): the same
//! [`super::PaneState`] machinery, editor, clipboard, save flow (side `None`), header, status, find, details and
//! navigation code runs on the left pane over the [`TextEditResponse`]'s document, so the two views never differ in
//! how a key, a save or a search behaves (§1.11.35 W11b note 1). The rows of the view are the document's lines (no
//! filter, no gap, no thumbnail); the commands that need two panes have no row in `Scope::TextEdit`.

use std::sync::Arc;

use hikasa_core::{JobKind, Side};
use hikasa_session::api::{TextEditRequest, TextEditResponse, ViewOutcome};
use hikasa_session::{ApiError, Service};
use hikasa_text::edit::EditableDocument;

use super::TextViewModel;
use crate::job::Job;

/// What a Text Edit view holds besides the pane: its request, the open job and the open document.
pub struct EditBody {
    /// The file and its options (`--ro`, `--savetarget`, `--nobackups`, the format and the encoding).
    pub request: TextEditRequest,
    /// `Service::text_edit_open` running.
    pub(crate) job: Option<Job<TextEditResponse>>,
    /// The open document with its format, spec and stamp.
    pub(crate) open: Option<TextEditResponse>,
}

impl EditBody {
    /// A body that opens `request`.
    pub fn new(request: TextEditRequest) -> EditBody {
        EditBody {
            request,
            job: None,
            open: None,
        }
    }

    /// The document (once open).
    pub fn doc(&self) -> Option<&EditableDocument> {
        self.open.as_ref().map(|o| &o.doc)
    }

    /// The document, to change.
    pub fn doc_mut(&mut self) -> Option<&mut EditableDocument> {
        self.open.as_mut().map(|o| &mut o.doc)
    }
}

impl TextViewModel {
    /// A Text Edit view of `request` (D-313): starts `text_edit_open` as a job; the first screen comes when it loads.
    pub fn new_edit(
        service: &Service,
        request: TextEditRequest,
        title: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> TextViewModel {
        let compare = hikasa_session::api::TextCompareRequest {
            left: request.spec.clone(),
            right: request.spec.clone(),
            format: request.format.clone(),
            ..hikasa_session::api::TextCompareRequest::default()
        };
        let mut model = TextViewModel::idle(
            service,
            compare,
            hikasa_text::ViewOptions::default(),
            [title, None],
            wake,
            hikasa_session::api::LiveOptions::default(),
        );
        model.edit = Some(Box::new(EditBody::new(request)));
        model.start(service);
        model
    }

    /// True for a Text Edit view (one pane, no comparison).
    pub fn is_edit(&self) -> bool {
        self.edit.is_some()
    }

    /// The view's type: `TextEdit` or `Text`.
    pub fn scope(&self) -> crate::commands::Scope {
        if self.edit.is_some() {
            crate::commands::Scope::TextEdit
        } else {
            crate::commands::Scope::Text
        }
    }

    /// The request of a Text Edit view.
    pub fn edit_request(&self) -> Option<&TextEditRequest> {
        self.edit.as_ref().map(|e| &e.request)
    }

    /// The response of the open of a Text Edit view.
    pub fn edit_response(&self) -> Option<&TextEditResponse> {
        self.edit.as_ref().and_then(|e| e.open.as_ref())
    }

    /// Starts (or restarts, Reload) the open of a Text Edit view.
    pub(super) fn start_edit(&mut self, service: &Service) {
        let wake = Arc::clone(&self.wake);
        let Some(body) = self.edit.as_mut() else {
            return;
        };
        if let Some(job) = body.job.take() {
            job.cancel();
        }
        body.open = None;
        let request = body.request.clone();
        let title = super::name_of(&request.spec.to_string_lossy());
        body.job = Some(Job::spawn(
            service,
            JobKind::TextCompare,
            title,
            &wake,
            move |svc, job| svc.text_edit_open(&request, job),
        ));
    }

    /// Polls the open of a Text Edit view; true when it landed.
    pub(super) fn poll_edit(&mut self) -> bool {
        let Some(body) = self.edit.as_mut() else {
            return false;
        };
        let Some(job) = &body.job else {
            return false;
        };
        let Some(result) = job.try_result() else {
            return false;
        };
        body.job = None;
        match result {
            Ok(resp) => {
                for w in &resp.warnings {
                    self.notes.push(crate::status::LogNote {
                        kind: hikasa_session::log::LogKind::Warning,
                        text: w.message.clone(),
                    });
                }
                if let Some(body) = self.edit.as_mut() {
                    body.open = Some(resp);
                }
                self.loaded_after = Some(self.started.elapsed());
                self.ready_after = self.loaded_after;
                self.apply_write_policy();
                if let Some(doc) = self.doc(Side::Left).cloned() {
                    self.panes[0].clamp(&doc);
                }
                self.installed();
                self.scroll = Some(0);
                self.bump();
                self.apply_pending_goto();
            }
            Err(ApiError::Cancelled) => {
                self.cancelled = true;
                self.bump();
            }
            Err(e) => {
                self.notes.push(crate::status::LogNote {
                    kind: hikasa_session::log::LogKind::Error,
                    text: e.to_string(),
                });
                self.error = Some(e.to_string());
                self.bump();
            }
        }
        true
    }

    /// The reopen of a Text Edit view (a header path Accept, *Reopen with Encoding*, a format pick; its edits dropped,
    /// the window asked first): the file is opened again with the waiting change.
    pub fn reopen_edit(&mut self, service: &Service) {
        let req = self.take_reopen_request(Side::Left);
        let format = self.reopen_format.take();
        if let Some(body) = self.edit.as_mut() {
            body.request.spec = req.left.clone();
            body.request.encoding = req.encodings.left.clone();
            if let Some(format) = format {
                body.request.format = format;
            }
        }
        self.request.left = req.left.clone();
        self.request.right = req.left;
        self.names = [
            super::display_name(&self.request.left),
            super::display_name(&self.request.left),
        ];
        self.start(service);
    }

    /// The D-23 outcome of a Text Edit view (`ViewOutcome::for_text_edit`): 0, or 100 when it failed to load.
    pub(super) fn edit_outcome(&self) -> ViewOutcome {
        ViewOutcome::for_text_edit(self.error.is_none())
    }

    /// The request that opens the same file again (`TabDuplicate`, a workspace).
    pub(super) fn edit_duplicate(&self) -> Option<hikasa_session::api::ViewRequest> {
        let body = self.edit.as_ref()?;
        let mut request = body.request.clone();
        // A Save As moved the file: the copy opens the new one.
        if let Some(spec) = body.doc().and_then(EditableDocument::location) {
            request.spec = spec.canonical().into();
        }
        Some(hikasa_session::api::ViewRequest::TextEdit { request })
    }
}
