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
use hikasa_text::LineState;
use hikasa_text::edit::EditableDocument;
use hikasa_text::spans::DocumentSpans;

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
    /// The syntax spans of the document's format (W11b review: a Text Edit view draws syntax colours).
    pub(crate) spans: Option<Arc<DocumentSpans>>,
    /// The lines' tokenizer start states with the document's generation they were computed for (a grammar with
    /// multi-line items; computed on a worker after the open and after each edit, the older ones drawn meanwhile).
    pub(crate) states: Option<(u64, Arc<Vec<LineState>>)>,
    /// The worker computing them, with the generation it runs on.
    pub(crate) states_job: Option<(u64, Job<Vec<LineState>>)>,
}

impl EditBody {
    /// A body that opens `request`.
    pub fn new(request: TextEditRequest) -> EditBody {
        EditBody {
            request,
            job: None,
            open: None,
            spans: None,
            states: None,
            states_job: None,
        }
    }

    /// The syntax spans of `line` as the slice's segments (`None`: no colours yet, or none for this line).
    pub(crate) fn line_spans(&self, line: u32) -> Option<Vec<hikasa_text::ClassifiedSpan>> {
        let spans = self.spans.as_ref()?;
        let doc = self.doc()?;
        let state = if spans.needs_states() {
            let (_, states) = self.states.as_ref()?;
            states
                .get(line as usize)
                .copied()
                .unwrap_or(LineState::Normal)
        } else {
            LineState::Normal
        };
        Some(spans.spans(doc.document(), hikasa_text::LineIdx(line), state))
    }

    /// True when the spans drawn are the document's of now (no start states needed, or computed for this generation).
    pub fn spans_current(&self) -> bool {
        match (&self.spans, self.doc()) {
            (Some(spans), Some(doc)) => {
                !spans.needs_states()
                    || self
                        .states
                        .as_ref()
                        .is_some_and(|(g, _)| *g == doc.generation())
            }
            _ => false,
        }
    }

    /// Starts the start states' worker when the document changed since the last ones (a running worker of an older
    /// generation is cancelled); takes a finished worker's states. True when new states landed.
    pub(crate) fn poll_states(
        &mut self,
        service: &Service,
        wake: &Arc<dyn Fn() + Send + Sync>,
    ) -> bool {
        let mut landed = false;
        if let Some((g, job)) = &self.states_job
            && let Some(result) = job.try_result()
        {
            let g = *g;
            self.states_job = None;
            if let Ok(states) = result {
                self.states = Some((g, Arc::new(states)));
                landed = true;
            }
        }
        let (Some(spans), Some(doc)) = (self.spans.clone(), self.doc().cloned()) else {
            return landed;
        };
        let generation = doc.generation();
        if !spans.needs_states()
            || self.states.as_ref().is_some_and(|(g, _)| *g == generation)
            || self
                .states_job
                .as_ref()
                .is_some_and(|(g, _)| *g == generation)
        {
            return landed;
        }
        if let Some((_, job)) = self.states_job.take() {
            job.cancel();
        }
        let job = Job::spawn(
            service,
            JobKind::TextCompare,
            "syntax spans".into(),
            wake,
            move |_svc, job| {
                spans
                    .start_states(doc.document(), &job.cancel)
                    .map_err(|_| ApiError::Cancelled)
            },
        );
        self.states_job = Some((generation, job));
        landed
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
            // A reopen (a format pick) keeps the encoding the view was opened with.
            encodings: hikasa_core::Pair {
                left: request.encoding.clone(),
                right: None,
            },
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
        if let Some((_, job)) = body.states_job.take() {
            job.cancel();
        }
        body.open = None;
        body.spans = None;
        body.states = None;
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
                    body.spans = Some(Arc::new(DocumentSpans::new(Arc::clone(&resp.compiled))));
                    body.states = None;
                    if let Some((_, job)) = body.states_job.take() {
                        job.cancel();
                    }
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
        self.request.encodings.left.clone_from(&req.encodings.left);
        self.request.left = req.left.clone();
        self.request.right = req.left;
        self.names = [
            super::display_name(&self.request.left),
            super::display_name(&self.request.left),
        ];
        self.start(service);
    }

    /// Session Settings' OK on a Text Edit view (§1.11.25, W11c review fix F2): a new file, format or encoding than
    /// the view's opens the file again (the window asks D-301 first when it has edits); else the document and its
    /// edits stay.
    pub fn session_changed_edit(&mut self, request: &TextEditRequest) {
        let Some(old) = self.edit_request() else {
            return;
        };
        let spec = (request.spec != old.spec).then(|| request.spec.clone());
        let encoding = (request.encoding != old.encoding).then(|| request.encoding.clone());
        let format = (request.format != old.format).then(|| request.format.clone());
        if spec.is_none() && encoding.is_none() && format.is_none() {
            return;
        }
        if format.is_some() {
            self.reopen_format = format;
        }
        self.request_reopen(Side::Left, spec, encoding);
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
