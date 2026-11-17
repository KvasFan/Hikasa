//! Save in the window (M2 W11a, contract §1.11.9, D-300), toolkit-free: `Save`, `SaveAll`, `SaveAs` and `SaveAsLocal`
//! of a Text Compare view's panes. The steps are core's: the format's *when saving* options as one undo step
//! (`apply_save_options`, through [`super::edit`]), `Service::live_save_request`, `text_save_plan` on **clones** of the
//! documents (a job), `save_confirmation`'s questions one at a time through the window's prompts, `text_save_execute`
//! (a job), then `apply_save_report` on the view's documents. While a save runs its documents are frozen: their edits
//! wait in the view's queue and are applied in order after it ([`super::input::replay`]).

use hikasa_core::{JobKind, Reply, Side, Spec};
use hikasa_session::api::{SaveAnswers, SaveDriver, apply_save_report};
use hikasa_session::log::LogKind;
use hikasa_session::{ApiError, Service};
use hikasa_text::EditableDocument;
use hikasa_text::save::{SavePlan, SaveQuestion, SaveReport, SaveRequest};

use super::{TextRequest, TextViewModel, index_of, input};
use crate::job::Job;
use crate::status::{JobStatus, Message};

/// Where a save writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveTarget {
    /// The document's own file (or the view's `--savetarget`).
    InPlace,
    /// Save As to this file.
    As(Spec),
}

/// An answer to a save question (the prompt's buttons).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveReply {
    /// Yes, No, Yes to All, No to All, Cancel.
    Reply(Reply),
    /// *Save as UTF-8* (an `Unmappable` question): the document's encoding becomes UTF-8, then the save plans again.
    Utf8,
}

/// The question the window asks now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAsk {
    /// The question's text.
    pub text: String,
    /// The questions left, this one included.
    pub remaining: u64,
    /// The question is about unmappable characters: *Save as UTF-8* is offered.
    pub unmappable: bool,
}

/// The stage of a save.
enum Stage {
    /// The plan is being made (a job on clones).
    Planning(Job<(SavePlan, Vec<EditableDocument>)>),
    /// The questions are asked.
    Asking {
        driver: SaveDriver,
        plan: SavePlan,
        clones: Vec<EditableDocument>,
    },
    /// The plan is executed.
    Executing(Job<SaveReport>),
    /// Over (the flow is about to end).
    Done,
}

/// A save of one or more panes of the view.
pub struct SaveFlow {
    /// The panes saved, in the plan's order.
    pub sides: Vec<Side>,
    /// Their targets.
    pub targets: Vec<SaveTarget>,
    stage: Stage,
    /// The answer of a close that waits for this save (D-301): the window reads it when the save ends.
    pub outcome: Option<bool>,
    /// The targets of the plan's items (the log lines).
    shown: Vec<String>,
}

/// What a save of the window's prompts left behind (taken by the window after the save ended).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// Every item was written.
    Saved,
    /// Nothing was written (declined, cancelled) or an item failed.
    NotSaved,
}

/// The message of a save started while another runs.
pub const ALREADY: &str = "A save of this view is running.";

/// Starts a save of `sides` (§1.11.9): blocked panes refuse with D-297's message; the format's save options are
/// applied first (one undo step each); then the plan runs as a job. False when nothing started.
pub fn begin(
    view: &mut TextViewModel,
    service: &Service,
    sides: Vec<Side>,
    targets: Vec<SaveTarget>,
) -> bool {
    if view.save.is_some() {
        view.request_window(TextRequest::Message(Message::info(ALREADY)));
        return false;
    }
    if view.view.is_none() || sides.is_empty() {
        return false;
    }
    for side in &sides {
        let Some(doc) = view.doc(*side) else {
            return false;
        };
        if !doc.editability().editable() {
            super::edit::blocked_message(view, *side);
            return false;
        }
    }
    // The format's *when saving* options, as one undo step, before planning (D-272): what is shown is written.
    for side in &sides {
        super::edit::apply_save_options(view, *side);
    }
    plan(view, service, sides, targets)
}

/// The planning job of `sides` (the documents frozen from here on).
fn plan(
    view: &mut TextViewModel,
    service: &Service,
    sides: Vec<Side>,
    targets: Vec<SaveTarget>,
) -> bool {
    let Some(live) = view.view.as_ref() else {
        return false;
    };
    let mut items = Vec::new();
    for (side, target) in sides.iter().zip(&targets) {
        let mut req = match service.live_save_request(live, *side) {
            Ok(req) => req,
            Err(e) => {
                view.request_window(TextRequest::Warning(format!(
                    "The file cannot be saved: {e}"
                )));
                return false;
            }
        };
        if let SaveTarget::As(spec) = target {
            req.target = Some(spec.clone());
        }
        let doc = match side {
            Side::Right => live.docs.right.clone(),
            _ => live.docs.left.clone(),
        };
        items.push((*side, doc, req));
    }
    for side in &sides {
        view.saving[index_of(*side)] = true;
    }
    let job = Job::spawn(
        service,
        JobKind::TextSave,
        "save".into(),
        &view.wake,
        move |svc, control| {
            let refs: Vec<(Option<Side>, &EditableDocument, &SaveRequest)> =
                items.iter().map(|(s, d, r)| (Some(*s), d, r)).collect();
            let plan = svc.text_save_plan(&refs, control)?;
            Ok((plan, items.into_iter().map(|(_, d, _)| d).collect()))
        },
    );
    view.save = Some(SaveFlow {
        sides,
        targets,
        stage: Stage::Planning(job),
        outcome: None,
        shown: Vec::new(),
    });
    view.bump();
    true
}

/// The question asked now (`None`: no save asks).
pub fn asking(view: &TextViewModel) -> Option<SaveAsk> {
    let flow = view.save.as_ref()?;
    let Stage::Asking { driver, .. } = &flow.stage else {
        return None;
    };
    let ask = driver.next()?;
    Some(SaveAsk {
        text: ask.text.clone(),
        remaining: ask.remaining,
        unmappable: ask.kind == SaveQuestion::Unmappable,
    })
}

/// The answer to the question asked now. Cancel ends the save with nothing written; the last answer executes the
/// plan, or ends it when every item was declined.
pub fn answer(view: &mut TextViewModel, service: &Service, reply: SaveReply) {
    let Some(flow) = view.save.as_mut() else {
        return;
    };
    let Stage::Asking { driver, .. } = &mut flow.stage else {
        return;
    };
    match reply {
        SaveReply::Utf8 => {
            let item = driver.next().map(|a| a.item as usize);
            let Some(flow) = view.save.take() else {
                return;
            };
            unfreeze(view, &flow.sides);
            if let Some(side) = item.and_then(|i| flow.sides.get(i).copied()) {
                super::header::save_with_encoding(view, side, "UTF-8");
            }
            plan(view, service, flow.sides, flow.targets);
            return;
        }
        SaveReply::Reply(Reply::Cancel) => {
            driver.reply(Reply::Cancel);
            end(view, Ended::NotSaved, None);
            return;
        }
        SaveReply::Reply(r) => driver.reply(r),
    }
    if driver.next().is_some() {
        view.request_window(TextRequest::SaveAsked);
        return;
    }
    let Some(flow) = view.save.take() else {
        return;
    };
    let SaveFlow {
        sides,
        targets,
        stage,
        outcome,
        ..
    } = flow;
    let Stage::Asking {
        driver,
        plan,
        clones,
    } = stage
    else {
        return;
    };
    // Back in place while the plan executes (or while `end` reads it).
    match driver.finish(plan) {
        Ok(plan) => {
            let shown = shown_targets(&plan);
            let job = execute_job(view, service, plan, clones);
            view.save = Some(SaveFlow {
                sides,
                targets,
                stage: Stage::Executing(job),
                outcome,
                shown,
            });
        }
        Err(_) => {
            view.save = Some(SaveFlow {
                sides,
                targets,
                stage: Stage::Done,
                outcome,
                shown: Vec::new(),
            });
            view.request_window(TextRequest::Message(Message::info("Nothing was saved.")));
            end(view, Ended::NotSaved, None);
        }
    }
}

/// The targets of a plan's items, as the log names them.
fn shown_targets(plan: &SavePlan) -> Vec<String> {
    plan.items.iter().map(|i| i.target.canonical()).collect()
}

/// The execution job of a confirmed plan.
fn execute_job(
    view: &TextViewModel,
    service: &Service,
    plan: SavePlan,
    clones: Vec<EditableDocument>,
) -> Job<SaveReport> {
    Job::spawn(
        service,
        JobKind::TextSave,
        "save".into(),
        &view.wake,
        move |svc, control| {
            let refs: Vec<&EditableDocument> = clones.iter().collect();
            svc.text_save_execute(plan, &refs, control)
        },
    )
}

/// Executes a confirmed plan (the flow keeps its sides).
fn execute(
    view: &mut TextViewModel,
    service: &Service,
    plan: SavePlan,
    clones: Vec<EditableDocument>,
) {
    let shown = shown_targets(&plan);
    let job = execute_job(view, service, plan, clones);
    if let Some(flow) = view.save.as_mut() {
        flow.stage = Stage::Executing(job);
        flow.shown = shown;
    }
}

/// Polls the save's jobs; true when something changed.
pub fn poll(view: &mut TextViewModel, service: &Service) -> bool {
    let Some(flow) = view.save.as_ref() else {
        return false;
    };
    match &flow.stage {
        Stage::Planning(job) => {
            let Some(result) = job.try_result() else {
                return false;
            };
            match result {
                Ok((plan, clones)) => {
                    let driver = service.save_confirmation(&plan, SaveAnswers::default());
                    if driver.next().is_none() {
                        match driver.finish(plan) {
                            Ok(plan) => execute(view, service, plan, clones),
                            Err(_) => end(view, Ended::NotSaved, None),
                        }
                    } else if let Some(flow) = view.save.as_mut() {
                        flow.stage = Stage::Asking {
                            driver,
                            plan,
                            clones,
                        };
                        view.request_window(TextRequest::SaveAsked);
                    }
                }
                Err(ApiError::Cancelled) => end(view, Ended::NotSaved, None),
                Err(ApiError::EditBlocked { .. }) => {
                    let sides = view
                        .save
                        .as_ref()
                        .map(|f| f.sides.clone())
                        .unwrap_or_default();
                    end(view, Ended::NotSaved, None);
                    if let Some(side) = sides.first() {
                        super::edit::blocked_message(view, *side);
                    }
                }
                Err(e) => end(
                    view,
                    Ended::NotSaved,
                    Some(format!("The file could not be saved: {e}")),
                ),
            }
            true
        }
        Stage::Asking { .. } | Stage::Done => false,
        Stage::Executing(job) => {
            let Some(result) = job.try_result() else {
                return false;
            };
            match result {
                Ok(report) => landed(view, &report),
                Err(ApiError::Cancelled) => end(view, Ended::NotSaved, None),
                Err(e) => end(
                    view,
                    Ended::NotSaved,
                    Some(format!("The file could not be saved: {e}")),
                ),
            }
            true
        }
    }
}

/// The report landed: the marks become `SavedEdit` on the view's documents, a Save As moves the location, one notice
/// and a log line per item.
fn landed(view: &mut TextViewModel, report: &SaveReport) {
    let Some(flow) = view.save.as_ref() else {
        return;
    };
    let sides = flow.sides.clone();
    let shown = flow.shown.clone();
    if let Some(live) = view.view.as_mut() {
        let (left, right) = (&mut live.docs.left, &mut live.docs.right);
        let mut docs: Vec<&mut EditableDocument> = Vec::new();
        let (mut l, mut r) = (Some(left), Some(right));
        for side in &sides {
            let doc = match side {
                Side::Right => r.take(),
                _ => l.take(),
            };
            if let Some(doc) = doc {
                docs.push(doc);
            }
        }
        apply_save_report(report, &mut docs);
    }
    let mut lines = Vec::new();
    let mut names = Vec::new();
    for (i, side) in sides.iter().enumerate() {
        let target = shown.get(i).cloned().unwrap_or_default();
        match report.op.results.get(i) {
            Some(r) if r.error.is_none() && report.saved.get(i).is_some_and(Option::is_some) => {
                lines.push((LogKind::Detail, format!("write {target}")));
                names.push(super::name_of(&target));
                // A Save As moved the location: the request, the names and the title follow it.
                if let Some(spec) = view
                    .doc(*side)
                    .and_then(EditableDocument::location)
                    .map(Spec::canonical)
                {
                    let canonical = std::ffi::OsString::from(spec.clone());
                    let k = index_of(*side);
                    if k == 1 {
                        view.request.right = canonical;
                    } else {
                        view.request.left = canonical;
                    }
                    view.names[k] = super::name_of(&spec);
                }
            }
            Some(r) => {
                let why = r
                    .error
                    .as_ref()
                    .map_or_else(|| "not written".to_owned(), |e| e.message.clone());
                lines.push((LogKind::Error, format!("write {target}: {why}")));
            }
            None => lines.push((LogKind::Error, format!("write {target}: not written"))),
        }
    }
    let ok = report.ok() && !report.op.cancelled;
    for (kind, text) in &lines {
        view.note(*kind, text.clone());
    }
    let message = if ok {
        format!("Saved {}", names.join(", "))
    } else {
        "The save did not write every file.".to_owned()
    };
    view.request_window(TextRequest::Saved { text: message, ok });
    end(view, if ok { Ended::Saved } else { Ended::NotSaved }, None);
}

/// The save ended: the documents thaw, their queued inputs are applied, a waiting close learns the result.
fn end(view: &mut TextViewModel, ended: Ended, warning: Option<String>) {
    let Some(flow) = view.save.take() else {
        return;
    };
    unfreeze(view, &flow.sides);
    if let Some(w) = warning {
        view.request_window(TextRequest::Warning(w));
    }
    view.last_save = Some(ended);
    view.rebuild_keep();
}

fn unfreeze(view: &mut TextViewModel, sides: &[Side]) {
    for side in sides {
        view.saving[index_of(*side)] = false;
    }
    for side in sides {
        if view.relive.is_none() {
            input::replay(view, *side);
        }
    }
}

/// Cancels a running save (the view closes or reloads): nothing more is written; a running execution job decides
/// for itself (`JobControl`).
pub fn cancel(view: &TextViewModel) {
    if let Some(flow) = &view.save {
        match &flow.stage {
            Stage::Planning(job) => job.cancel(),
            Stage::Executing(job) => job.cancel(),
            Stage::Asking { .. } | Stage::Done => {}
        }
    }
}

/// Cancels the questions of a save (a closed prompt is a Cancel).
pub fn abandon(view: &mut TextViewModel) {
    if view
        .save
        .as_ref()
        .is_some_and(|f| matches!(f.stage, Stage::Asking { .. }))
    {
        end(view, Ended::NotSaved, None);
    }
}

/// The save's job for the indicator.
pub fn jobs(view: &TextViewModel, id: hikasa_session::ipc::ViewId) -> Vec<JobStatus> {
    let Some(flow) = &view.save else {
        return Vec::new();
    };
    let job = match &flow.stage {
        Stage::Planning(job) => job.title(),
        Stage::Executing(job) => job.title(),
        Stage::Asking { .. } | Stage::Done => return Vec::new(),
    };
    vec![JobStatus {
        view: id,
        title: job.to_owned(),
        phase: None,
        fraction: None,
    }]
}

/// True while a save job runs (planning or writing; not while a question waits for the user).
pub fn working(view: &TextViewModel) -> bool {
    view.save
        .as_ref()
        .is_some_and(|f| matches!(f.stage, Stage::Planning(_) | Stage::Executing(_)))
}

/// True while a save of the view runs (planning, asking or writing).
pub fn running(view: &TextViewModel) -> bool {
    view.save.is_some()
}

/// The result of the last save that ended (taken).
pub fn take_ended(view: &mut TextViewModel) -> Option<Ended> {
    view.last_save.take()
}

/// The sides `Save` writes: the focus pane. A document without location or that cannot be saved in place goes to Save
/// As (`None`: the window asks for a target first).
pub fn save_target_of(view: &TextViewModel, side: Side) -> Option<SaveTarget> {
    let doc = view.doc(side)?;
    let redirected = side == Side::Right && view.live_options.save_target.is_some();
    if redirected || doc.can_save_in_place() && doc.location().is_some() {
        Some(SaveTarget::InPlace)
    } else {
        None
    }
}

/// The modified panes (Save All, the close prompts).
pub fn modified_sides(view: &TextViewModel) -> Vec<Side> {
    [Side::Left, Side::Right]
        .into_iter()
        .filter(|s| view.doc(*s).is_some_and(EditableDocument::is_modified))
        .collect()
}
