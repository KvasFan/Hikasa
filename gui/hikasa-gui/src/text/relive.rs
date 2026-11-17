//! Rule and format changes that keep the edits (M2 W11a, contract §1.11.13, D-304), toolkit-free: a relive compares
//! the live documents' snapshots again with new rules, per-side formats, anchors and ignored lines
//! (`Service::text_relive_full`, a job on clones of the documents) and installs the new `LiveComparison`; the
//! documents, their undo logs, marks and bookmarks stay. A format change first sets that side's editing options
//! (`EditableDocument::set_options`: a new characters-per-line limit re-wraps; the carets follow the `LineMap`).
//!
//! *Reopen one side* (a header path Accept, *Reopen with Encoding*): that side is read again
//! (`Service::text_edit_open`) and relived with the other side's live document, so the other side keeps its edits.
//!
//! While a relive runs the documents are frozen like during a save (D-300): inputs are queued and replayed in order
//! after it lands (§1.11.35 W11a note 5). A pick back to *Detected* takes the editing options of the format the
//! comparison chose for that side.

use std::ffi::OsString;

use hikasa_core::{JobKind, Pair, Side};
use hikasa_session::api::{Relived, TextCompareRequest, TextEditRequest};
use hikasa_session::{ApiError, Service};
use hikasa_text::edit::{EditOptions, EditableDocument};

use super::{TextRequest, TextViewModel, index_of};
use crate::job::Job;
use crate::status::JobStatus;

/// What a relive was started for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cause {
    /// New rules (Session Settings' rule tabs, W11c).
    #[cfg_attr(not(test), allow(dead_code))]
    Rules,
    /// A pane's format pick (`None`: back to detection).
    Format(Side, Option<String>),
    /// That side read again (its spec or encoding changed); its edits were dropped (the window asked first).
    Reopen(Side),
}

/// The result of a relive's job.
pub struct Landed {
    relived: Relived,
    /// The new editing options of a side (a format change).
    options: Option<(Side, EditOptions)>,
    /// The reopened side's new document.
    reopened: Option<(Side, EditableDocument)>,
}

/// A running relive.
pub struct Running {
    cause: Cause,
    request: TextCompareRequest,
    job: Job<Landed>,
}

impl Running {
    /// Cancels the job.
    pub fn cancel(&self) {
        self.job.cancel();
    }

    /// True while the job winds down after a cancel.
    pub fn settling(&self) -> bool {
        self.job.settling()
    }
}

/// The editing options of a document with `format`: the format's tab size, tab-as-spaces, word characters and
/// characters-per-line limit; the settings' parts (`auto_indent`, `backspace_unindents`, the default line ending)
/// stay the document's (`edit_options`' split, §1.10.3).
fn options_of(current: &EditOptions, format: &hikasa_formats::CompiledFormat) -> EditOptions {
    let text = format.text();
    EditOptions {
        tab_size: text.tab_size,
        tab_as_spaces: text.tab_as_spaces,
        word_chars: text.word_chars.clone(),
        chars_per_line_limit: text.chars_per_line_limit,
        ..current.clone()
    }
}

/// Sets the editing options of `format` on the clone of `side` (a new CPL re-wraps it); the options set, `None` when
/// they are the document's already.
fn set_side_options(
    docs: &mut Pair<EditableDocument>,
    side: Side,
    format: &hikasa_formats::CompiledFormat,
) -> Option<(Side, EditOptions)> {
    let doc = match side {
        Side::Right => &mut docs.right,
        _ => &mut docs.left,
    };
    let opts = options_of(doc.options(), format);
    if &opts == doc.options() {
        return None;
    }
    doc.set_options(opts.clone());
    Some((side, opts))
}

/// Starts a relive of the view with `request` (rules, formats, encodings of the view's request, changed by the
/// caller). The anchors and ignored lines are the live comparison's of now. A running recompare or relive is
/// cancelled. False when the view has no documents yet.
pub fn start(
    view: &mut TextViewModel,
    service: &Service,
    mut request: TextCompareRequest,
    cause: Cause,
) -> bool {
    let Some(live) = view.view.as_ref() else {
        return false;
    };
    request.anchors = live.live.anchors().to_vec();
    request.ignored = live.live.ignored().clone();
    if let Cause::Format(side, name) = &cause {
        match side {
            Side::Right => request.formats.right = name.clone(),
            _ => request.formats.left = name.clone(),
        }
    }
    let docs = Pair {
        left: live.docs.left.clone(),
        right: live.docs.right.clone(),
    };
    if let Some((_, job)) = view.rejob.take() {
        job.cancel();
    }
    view.recompare.failed();
    if let Some(old) = view.relive.take() {
        old.cancel();
    }
    let req = request.clone();
    let job_cause = cause.clone();
    let encodings = request.encodings.clone();
    let specs = Pair {
        left: request.left.clone(),
        right: request.right.clone(),
    };
    let read_only = view.write.read_only;
    let options = view.live_options.clone();
    let job = Job::spawn(
        service,
        JobKind::TextCompare,
        "relive".into(),
        &view.wake,
        move |svc, control| {
            let mut docs = docs;
            let mut reopened = None;
            let mut new_options = None;
            match &job_cause {
                Cause::Rules => {}
                Cause::Format(side, _) => {
                    // The new format's editing options, set on the clone first (a new CPL re-wraps its lines).
                    let name = match side {
                        Side::Right => req.formats.right.clone(),
                        _ => req.formats.left.clone(),
                    }
                    .or_else(|| req.format.clone());
                    let compiled = match name {
                        Some(name) => {
                            let registry = svc.registry();
                            let entry = registry.find(&name).ok_or_else(|| {
                                ApiError::InvalidArgument(format!("no format named {name}"))
                            })?;
                            Some(
                                registry
                                    .compiled(entry.id())
                                    .map_err(|e| ApiError::InvalidArgument(e.to_string()))?,
                            )
                        }
                        None => None,
                    };
                    if let Some(compiled) = compiled {
                        new_options = set_side_options(&mut docs, *side, &compiled);
                    }
                }
                Cause::Reopen(side) => {
                    let k = index_of(*side);
                    let spec: OsString = if k == 1 {
                        specs.right.clone()
                    } else {
                        specs.left.clone()
                    };
                    let opened = svc.text_edit_open(
                        &TextEditRequest {
                            spec,
                            format: if k == 1 {
                                req.formats.right.clone()
                            } else {
                                req.formats.left.clone()
                            }
                            .or_else(|| req.format.clone()),
                            encoding: if k == 1 {
                                encodings.right.clone()
                            } else {
                                encodings.left.clone()
                            },
                            read_only: read_only.get(*side),
                            save_target: (k == 1).then(|| options.save_target.clone()).flatten(),
                            session_read_only: options.session_read_only.get(*side),
                            parent_read_only: false,
                            no_backups: options.no_backups,
                        },
                        control,
                    )?;
                    let mut doc = opened.doc;
                    doc.set_editability(opened.editability);
                    match side {
                        Side::Right => docs.right = doc.clone(),
                        _ => docs.left = doc.clone(),
                    }
                    reopened = Some((*side, doc));
                }
            }
            let mut relived = svc.text_relive_full(docs.as_ref(), &req, control, None)?;
            if let Cause::Format(side, None) = &job_cause
                && new_options.is_none()
            {
                // Back to detection: the editing options of the format the comparison chose for that side (F13); a
                // new characters-per-line limit re-wraps the document, so the comparison is made again on it.
                let chosen = match side {
                    Side::Right => relived
                        .response
                        .right_format
                        .as_ref()
                        .unwrap_or(&relived.response.format),
                    _ => &relived.response.format,
                };
                let registry = svc.registry();
                let compiled = match &chosen.id {
                    Some(id) => registry
                        .compiled(id)
                        .map_err(|e| ApiError::InvalidArgument(e.to_string()))?,
                    None => std::sync::Arc::clone(registry.everything_else()),
                };
                let before = match side {
                    Side::Right => docs.right.options().chars_per_line_limit,
                    _ => docs.left.options().chars_per_line_limit,
                };
                new_options = set_side_options(&mut docs, *side, &compiled);
                if new_options
                    .as_ref()
                    .is_some_and(|(_, o)| o.chars_per_line_limit != before)
                {
                    relived = svc.text_relive_full(docs.as_ref(), &req, control, None)?;
                }
            }
            Ok(Landed {
                relived,
                options: new_options,
                reopened,
            })
        },
    );
    view.relive = Some(Running {
        cause,
        request,
        job,
    });
    view.bump();
    true
}

/// Polls the running relive: when it lands, the new comparison is installed (the documents keep their edits), the
/// carets follow, the queued edits are applied. True when something changed.
pub fn poll(view: &mut TextViewModel, service: &Service) -> bool {
    let Some(running) = &view.relive else {
        return false;
    };
    let Some(result) = running.job.try_result() else {
        return false;
    };
    let Some(running) = view.relive.take() else {
        return false;
    };
    match result {
        Ok(landed) => {
            if let Some(live) = view.view.as_mut() {
                if let Some((side, opts)) = landed.options {
                    let doc = match side {
                        Side::Right => &mut live.docs.right,
                        _ => &mut live.docs.left,
                    };
                    let map = doc.set_options(opts);
                    let pane = &mut view.panes[index_of(side)];
                    pane.caret = map.pos(doc, pane.caret);
                    pane.anchor = map.pos(doc, pane.anchor);
                    pane.lines = None;
                    pane.sticky_x = None;
                }
                if let Some((side, doc)) = landed.reopened {
                    let k = index_of(side);
                    match side {
                        Side::Right => live.docs.right = doc,
                        _ => live.docs.left = doc,
                    }
                    view.binary_answer[k] = None;
                    // The tab's caption and the prompts name the file now shown (F15).
                    let spec = if k == 1 {
                        &running.request.right
                    } else {
                        &running.request.left
                    };
                    view.names[k] = super::display_name(spec);
                }
                live.live = landed.relived.live;
                live.response = landed.relived.response;
            }
            view.request = running.request;
            for k in 0..2 {
                if let Some(doc) = view.doc(super::side_of(k)).cloned() {
                    view.panes[k].clamp(&doc);
                }
            }
            view.apply_write_policy();
            view.installed();
            view.rebuild_keep();
            view.caret_moved();
        }
        Err(ApiError::Cancelled) => {}
        Err(e) => {
            view.request_window(TextRequest::Warning(format!(
                "The comparison could not be redone: {e}"
            )));
        }
    }
    // The panes thaw: their queued inputs are replayed at the next frame (`input::replay`).
    view.dirty = true;
    // A reopen of the other side asked while this one ran starts now, on the request this one left (M2 W11b: a
    // second reopen never cancels the first).
    if view.relive.is_none() && !view.deferred_reopen.is_empty() {
        let side = view.deferred_reopen.remove(0);
        let request = view.take_reopen_request(side);
        start(view, service, request, Cause::Reopen(side));
    }
    true
}

/// A reopen of `side` (D-302, after its prompt): starts at once, or — while a reopen of the other side runs — when
/// that one has landed, so answering one side's prompt never cancels the other side's running reopen.
pub fn reopen(view: &mut TextViewModel, service: &Service, side: Side) {
    if view
        .relive
        .as_ref()
        .is_some_and(|r| matches!(r.cause, Cause::Reopen(s) if s != side))
    {
        if !view.deferred_reopen.contains(&side) {
            view.deferred_reopen.push(side);
        }
        return;
    }
    let request = view.take_reopen_request(side);
    start(view, service, request, Cause::Reopen(side));
}

/// The running relive as a job of the indicator.
pub fn jobs(view: &TextViewModel, id: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
    view.relive
        .iter()
        .map(|r| JobStatus {
            view: id,
            title: match r.cause {
                Cause::Reopen(_) => "reopen".to_owned(),
                _ => r.job.title().to_owned(),
            },
            phase: None,
            fraction: None,
        })
        .collect()
}
