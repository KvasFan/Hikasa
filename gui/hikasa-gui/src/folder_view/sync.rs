//! The Folder Sync view and the Synchronize preview (M2 W9b, FS-001, FS-002, FS-004, FS-005, FC-078; D-246…D-248;
//! contract §1.9.14, §1.9.15). The sync half of a folder view: a Folder Sync view is W9a's tree (columns, selection,
//! glyphs, Find, Legend, the log) with this state beside it, so every tree path serves both view types; its centre
//! column shows the planned action of each row from `SyncPlan::rows`. Toolkit-free (check 7); a child module of
//! `folder_view` (it reads the view's fields).
//!
//! The plan follows the comparison: after it ends, after each refresh and after each change of the preset, the matrix,
//! the orphans reading, an option or an override, one plan job (`folder_sync_plan` + `folder_sync_preview`) runs and
//! the older one is cancelled; a change of `within_archives` runs the comparison again (its rules change).

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use hikasa_core::{ExitCode, JobKind};
use hikasa_folder::ops::{ConfirmMode, Confirmation};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::sync::{LeaveReason, SyncAction, SyncCounts, SyncOptions, SyncPlan, SyncPreset};
use hikasa_folder::{DisplayOptions, FolderModel, NodeId};
use hikasa_session::api::{
    FolderCompareRequest, FolderSyncOutcome, FolderSyncRequest, OpOverrides, SelectRequest,
    SyncPreview,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::log::LogKind;
use hikasa_session::{ApiError, Service};

use super::{FolderEvent, FolderViewModel, Mark, Task};
use crate::commands::{CommandId, Ctx};
use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::job::Job;
use crate::notify::NoticeKind;
use crate::op_dialogs::PlanConfirmDialog;
use crate::op_preview::PreviewList;
use crate::status::LogNote;

/// The help topic of the Synchronize preview.
pub const SYNC_PREVIEW_TOPIC: &str = "dialog-sync-preview";

type Wake = Arc<dyn Fn() + Send + Sync>;

/// What a row of the sync view does (the centre column, FS-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowAction {
    /// The action.
    pub action: SyncAction,
    /// An override chose it (bold, "overridden").
    pub overridden: bool,
    /// A conflict (FS-004).
    pub conflict: bool,
    /// Why it is left.
    pub reason: Option<LeaveReason>,
}

/// The plan of the view.
enum PlanState {
    /// Nothing planned (the comparison runs, or failed).
    None,
    /// A plan job for this model generation (a newer one cancels it, its answer is dropped).
    Planning(Job<(SyncPlan, SyncPreview)>, u64),
    /// The plan and its preview.
    Ready(Box<SyncPlan>, Box<SyncPreview>),
    /// The plan was refused (`EmptySource`, an incomplete comparison with deletes, an invalid matrix): the core
    /// message, Run disabled.
    Failed(String),
    /// The plan went to its questions and its run (Run Sync).
    Taken,
}

/// The sync half of a Folder Sync view (§1.9.15).
pub struct SyncViewModel {
    /// The session's `[rules.sync]`, edited by the header and the Sync tab.
    pub options: SyncOptions,
    /// `--allow-empty-source` of the request; never set by the window.
    pub allow_empty_source: bool,
    plan: PlanState,
    generation: u64,
    /// The (model, generation) the plan was asked for: a change of either plans again.
    planned_for: Option<(usize, u64)>,
    /// An option or override changed: plan again at the next poll.
    due: bool,
    /// The exit code of the last Run (D-248).
    pub last_run: Option<ExitCode>,
    /// `SyncPlan::rows` by node (O(1) per rendered row).
    index: HashMap<NodeId, u32>,
    /// How many plan jobs started (tests).
    pub plans: u64,
}

impl SyncViewModel {
    fn new(options: SyncOptions, allow_empty_source: bool) -> SyncViewModel {
        SyncViewModel {
            options,
            allow_empty_source,
            plan: PlanState::None,
            generation: 0,
            planned_for: None,
            due: false,
            last_run: None,
            index: HashMap::new(),
            plans: 0,
        }
    }

    /// The plan, when ready.
    pub fn plan(&self) -> Option<&SyncPlan> {
        match &self.plan {
            PlanState::Ready(plan, _) => Some(plan),
            _ => None,
        }
    }

    /// The preview of the plan, when ready.
    pub fn preview(&self) -> Option<&SyncPreview> {
        match &self.plan {
            PlanState::Ready(_, preview) => Some(preview),
            _ => None,
        }
    }

    /// "Planning…" while the plan job runs.
    pub fn planning(&self) -> bool {
        matches!(self.plan, PlanState::Planning(..)) || self.due
    }

    /// The core message of a refused plan.
    pub fn error(&self) -> Option<&str> {
        match &self.plan {
            PlanState::Failed(message) => Some(message),
            _ => None,
        }
    }

    /// True when Run Sync has something to do.
    pub fn plan_ready(&self) -> bool {
        self.preview().is_some_and(SyncPreview::acts)
    }
}

/// The preset of a Synchronize command (FC-078).
pub fn preset_of(id: CommandId) -> Option<SyncPreset> {
    Some(match id {
        CommandId::UpdateRight => SyncPreset::UpdateRight,
        CommandId::UpdateLeft => SyncPreset::UpdateLeft,
        CommandId::UpdateBoth => SyncPreset::UpdateBoth,
        CommandId::MirrorRight => SyncPreset::MirrorRight,
        CommandId::MirrorLeft => SyncPreset::MirrorLeft,
        _ => return None,
    })
}

/// The override action of a Folder Sync command (FS-004).
pub fn override_of(id: CommandId) -> Option<SyncAction> {
    Some(match id {
        CommandId::SyncLeave => SyncAction::Leave,
        CommandId::SyncCopyLeft => SyncAction::CopyLeft,
        CommandId::SyncCopyRight => SyncAction::CopyRight,
        CommandId::SyncDeleteLeft => SyncAction::DeleteLeft,
        CommandId::SyncDeleteRight => SyncAction::DeleteRight,
        _ => return None,
    })
}

/// The tooltip of an action (the Legend's and the centre column's text).
pub fn action_text(action: SyncAction) -> &'static str {
    match action {
        SyncAction::CopyRight => "Copy left to right",
        SyncAction::CopyLeft => "Copy right to left",
        SyncAction::DeleteLeft => "Delete on the left",
        SyncAction::DeleteRight => "Delete on the right",
        SyncAction::Leave => "Leave alone",
    }
}

/// A folder's summary (FS-001): "contains files that will be copied to the right (3), deleted on the left (1)".
pub fn summary_text(counts: &SyncCounts) -> Option<String> {
    let parts: Vec<String> = [
        (counts.copy_right, "copied to the right"),
        (counts.copy_left, "copied to the left"),
        (counts.delete_left, "deleted on the left"),
        (counts.delete_right, "deleted on the right"),
    ]
    .into_iter()
    .filter(|(n, _)| *n > 0)
    .map(|(n, what)| format!("{what} ({n})"))
    .collect();
    let mut text =
        (!parts.is_empty()).then(|| format!("contains files that will be {}", parts.join(", ")));
    if counts.conflicts > 0 {
        let conflicts = format!("{} conflicts", counts.conflicts);
        text = Some(match text {
            Some(t) => format!("{t}; {conflicts}"),
            None => format!("contains {conflicts}"),
        });
    }
    text
}

impl FolderViewModel {
    /// A Folder Sync view (D-247): the comparison is `folder_sync_prepare` (the effective rules of
    /// `sync_folder_rules`), then `folder_run`; the plan follows once it ended.
    #[allow(clippy::too_many_arguments)] // the request's parts
    pub fn new_sync(
        service: &Service,
        request: FolderCompareRequest,
        options: SyncOptions,
        allow_empty_source: bool,
        display: DisplayOptions,
        titles: [Option<String>; 2],
        wake: Wake,
    ) -> FolderViewModel {
        FolderViewModel::create(
            service,
            request,
            display,
            false,
            titles,
            wake,
            Some(Box::new(SyncViewModel::new(options, allow_empty_source))),
        )
    }

    /// True for a Folder Sync view.
    pub fn is_sync(&self) -> bool {
        self.sync.is_some()
    }

    /// The sync half of a Folder Sync view.
    pub fn sync(&self) -> Option<&SyncViewModel> {
        self.sync.as_deref()
    }

    /// The sync options the comparison is prepared with (`folder_sync_prepare`).
    pub(super) fn sync_options(&self) -> Option<&SyncOptions> {
        self.sync.as_ref().map(|s| &s.options)
    }

    /// Plans when the comparison ended or changed, or an option changed; takes a finished plan job. True when the
    /// view changed.
    pub(super) fn sync_poll(&mut self, service: &Service) -> bool {
        let Some(sync) = self.sync.as_deref_mut() else {
            return false;
        };
        let mut changed = false;
        if let PlanState::Planning(job, generation) = &sync.plan
            && let Some(result) = job.try_result()
        {
            let generation = *generation;
            if generation == sync.generation {
                changed = true;
                sync.index.clear();
                sync.plan = match result {
                    Ok((plan, preview)) => {
                        for (i, row) in plan.rows.iter().enumerate() {
                            sync.index.insert(row.node, i as u32);
                        }
                        // An override the plan did not take goes (FS-004): its warning is the notice.
                        let mut dropped = Vec::new();
                        for path in &plan.ignored_overrides {
                            if sync.options.overrides.remove(path).is_some() {
                                dropped.push(path.clone());
                            }
                        }
                        for warning in plan.warnings.iter().filter(|w| {
                            w.code == hikasa_core::WarningCode::SwitchIgnored
                                && dropped.iter().any(|p| w.message.contains(p.as_str()))
                        }) {
                            self.outbox.push(FolderEvent::Notice {
                                kind: NoticeKind::Warning,
                                text: warning.message.clone(),
                            });
                        }
                        if !dropped.is_empty() {
                            self.dirty_options = true;
                        }
                        PlanState::Ready(Box::new(plan), Box::new(preview))
                    }
                    Err(ApiError::Cancelled) => PlanState::None,
                    Err(e) => PlanState::Failed(e.to_string()),
                };
            }
        }
        if self.scanning() || self.task.is_some() {
            return changed;
        }
        let Some(prepared) = self.prepared() else {
            return changed;
        };
        let model = Arc::clone(prepared.model());
        let key = (Arc::as_ptr(&model) as usize, model.generation());
        let Some(sync) = self.sync.as_deref_mut() else {
            return changed;
        };
        if !(sync.due || sync.planned_for != Some(key)) {
            return changed;
        }
        sync.due = false;
        sync.planned_for = Some(key);
        if let PlanState::Planning(job, _) = &sync.plan {
            job.cancel();
        }
        sync.generation += 1;
        sync.plans += 1;
        let request = FolderSyncRequest {
            options: sync.options.clone(),
            select: None,
            visible: None,
            include_hidden: None,
            iu: self.display.ignore_unimportant,
            read_only: self.write.read_only,
            allow_empty_source: sync.allow_empty_source,
            overrides: self.write.overrides(),
        };
        let job = Job::spawn(
            service,
            JobKind::FolderSync,
            "plan the sync".into(),
            &self.wake,
            move |svc, job| {
                let plan = svc.folder_sync_plan(&model, &request, job)?;
                let preview = svc.folder_sync_preview(&plan);
                Ok((plan, preview))
            },
        );
        sync.plan = PlanState::Planning(job, sync.generation);
        true
    }

    /// True while a Folder Sync view's plan is being made or is due (the window polls on until it is shown).
    pub fn sync_pending(&self) -> bool {
        self.sync.as_deref().is_some_and(|s| {
            s.planning()
                || (s.planned_for.is_none()
                    && !matches!(s.plan, PlanState::Failed(_))
                    && self.prepared().is_some()
                    && !self.scanning())
        })
    }

    /// The planned action of `node` (`None`: no plan, or no row there).
    pub fn sync_action(&self, node: NodeId) -> Option<RowAction> {
        let sync = self.sync.as_deref()?;
        let plan = sync.plan()?;
        let row = plan.rows.get(*sync.index.get(&node)? as usize)?;
        Some(RowAction {
            action: row.action,
            overridden: row.overridden,
            conflict: row.conflict,
            reason: row.reason,
        })
    }

    /// The summary of a folder row's subtree (its tooltip, FS-001).
    pub fn sync_folder_summary(&self, node: NodeId) -> Option<String> {
        let plan = self.sync.as_deref()?.plan()?;
        summary_text(&plan.folder(node))
    }

    /// The counts of the plan (Info adds them, D-242).
    pub fn sync_counts(&self) -> Option<SyncCounts> {
        self.sync.as_deref()?.plan().map(|p| p.counts)
    }

    /// Changes the sync options (the header's preset, the Sync tab): the plan is made again; a change of *Sync within
    /// archives* runs the comparison again (its rules change). True when it changed anything.
    pub fn edit_sync(&mut self, service: &Service, change: impl FnOnce(&mut SyncOptions)) -> bool {
        let Some(sync) = self.sync.as_deref_mut() else {
            return false;
        };
        let before = sync.options.clone();
        change(&mut sync.options);
        if sync.options == before {
            return false;
        }
        self.dirty_options = true;
        let rerun = sync.options.within_archives != before.within_archives;
        sync.due = true;
        if rerun {
            self.refresh(service);
        }
        true
    }

    /// The preset of the header's drop-down (Custom keeps the matrix).
    pub fn set_sync_preset(&mut self, service: &Service, preset: SyncPreset) -> bool {
        self.edit_sync(service, |o| {
            if preset == SyncPreset::Custom && o.preset != SyncPreset::Custom {
                o.matrix = o.effective_matrix();
            }
            o.preset = preset;
        })
    }

    /// FS-004: `action` becomes the override of each selected row, by its path (a folder's covers its subtree); the plan
    /// is made again.
    pub fn override_selected(&mut self, service: &Service, action: SyncAction) -> bool {
        let Some((items, _)) = self.operable() else {
            return false;
        };
        let Some(prepared) = self.prepared() else {
            return false;
        };
        let paths: Vec<String> = {
            let read = prepared.model().read();
            items
                .iter()
                .filter(|(node, _)| *node != NodeId::ROOT)
                .map(|(node, _)| read.path(node).as_str().to_owned())
                .collect()
        };
        self.edit_sync(service, |o| {
            for path in paths {
                o.overrides.insert(path, action);
            }
        })
    }

    /// `ConflictNext`, `ConflictPrev`: the next (previous) shown row whose plan row is a conflict.
    pub fn conflict_step(&mut self, forward: bool) -> bool {
        let len = self.rows.len();
        if len == 0 {
            return false;
        }
        // Without a current row the walk starts at either end.
        let from = self.selection.current.and(self.selected_index);
        let order: Vec<usize> = if forward {
            let start = from.map_or(0, |i| i + 1);
            (start..len).collect()
        } else {
            let end = from.unwrap_or(len);
            (0..end).rev().collect()
        };
        for index in order {
            let node = self.rows[index].node;
            if self.sync_action(node).is_some_and(|a| a.conflict) {
                self.select(index);
                self.scroll_to = Some(index as u32);
                return true;
            }
        }
        false
    }

    /// The context of a Folder Sync view: a plan ready, a conflict shown.
    pub(super) fn sync_ctx_into(&self, ctx: &mut Ctx) {
        let Some(sync) = self.sync.as_deref() else {
            return;
        };
        ctx.plan_ready = self.task.is_none() && !self.busy() && sync.plan_ready();
        ctx.has_conflict = sync.plan().is_some_and(|p| p.counts.conflicts > 0);
    }

    /// Run Sync (FS-005): the plan's questions in the mode of `sync_confirmations` (the operation question with the
    /// counts), then `folder_sync_execute`; the outcome goes to the log and one notice, the rows are refreshed and the
    /// plan follows.
    pub fn run_sync(&mut self, service: &Service, view: ViewId, caption: &str) -> bool {
        if self.task.is_some() || self.busy() {
            return false;
        }
        let Some(mark) = self.prepared().map(Mark::of) else {
            return false;
        };
        let Some(sync) = self.sync.as_deref_mut() else {
            return false;
        };
        if !sync.plan_ready() {
            return false;
        }
        let PlanState::Ready(plan, preview) = std::mem::replace(&mut sync.plan, PlanState::Taken)
        else {
            return false;
        };
        let confirmation = service.folder_confirmation(&plan.op, None);
        let list = PreviewList::sync(*plan, *preview, Arc::clone(&mark.model));
        self.sync_confirm(service, view, caption, list, confirmation, mark);
        true
    }

    /// A sync plan and its questions: asked in a dialog (the plan inside it), or run at once.
    fn sync_confirm(
        &mut self,
        service: &Service,
        view: ViewId,
        caption: &str,
        list: PreviewList,
        confirmation: Confirmation,
        mark: Mark,
    ) {
        self.cancel_crc();
        if confirmation.next().is_none() {
            self.confirmed_sync(service, caption, confirmation, list, mark);
            return;
        }
        let subject = list.counts().last().cloned().unwrap_or_default();
        let dialog = PlanConfirmDialog::new(view, caption, subject, list, confirmation, None);
        self.task = Some(Task::Asking {
            caption: caption.to_owned(),
            mark,
            again: None,
        });
        self.outbox.push(FolderEvent::Dialog(Box::new(dialog)));
    }

    /// The questions of a sync were answered: the plan runs (`folder_sync_execute`), or nothing does.
    pub(super) fn confirmed_sync(
        &mut self,
        service: &Service,
        caption: &str,
        confirmation: Confirmation,
        list: PreviewList,
        mark: Mark,
    ) {
        let Some(mut plan) = list.into_sync() else {
            return;
        };
        if !mark.holds(self.prepared()) {
            self.refuse_changed(caption);
            return;
        }
        let stand_in = empty_op(&plan.op);
        let op = std::mem::replace(&mut plan.op, stand_in);
        match confirmation.finish(op) {
            Ok(op) => plan.op = op,
            Err(_) => {
                self.notes.push(LogNote {
                    kind: LogKind::Summary,
                    text: format!("{caption}: cancelled, nothing was written."),
                });
                self.sync_replan();
                return;
            }
        }
        #[cfg(test)]
        {
            self.executed = Some(serde_json::to_string(&plan.op).expect("a plan serializes"));
        }
        let crc = self.crc_yield();
        let refresh = plan.op.refresh.clone();
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = Job::spawn(
            service,
            JobKind::FolderSync,
            caption.to_owned(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                super::wait_at(gate.as_ref());
                crc.wait();
                svc.folder_sync_execute(plan, job)
            },
        );
        self.task = Some(Task::Syncing {
            caption: caption.to_owned(),
            mark,
            refresh,
            began: Instant::now(),
            job,
        });
    }

    /// The plan is made again at the next poll (after a cancel, a refusal).
    pub(super) fn sync_replan(&mut self) {
        if let Some(sync) = self.sync.as_deref_mut() {
            if matches!(sync.plan, PlanState::Taken) {
                sync.plan = PlanState::None;
            }
            sync.due = true;
        }
    }

    /// A sync ended: its exit code is the view's last Run (D-248).
    pub(super) fn sync_ran(&mut self, outcome: &FolderSyncOutcome) {
        if let Some(sync) = self.sync.as_deref_mut() {
            sync.last_run = Some(outcome.exit_code());
        }
        self.sync_replan();
    }

    /// The waited outcome of a Folder Sync view (D-248).
    pub(super) fn sync_outcome(&self) -> Option<hikasa_session::api::ViewOutcome> {
        let sync = self.sync.as_deref()?;
        let complete = !self.scanning()
            && self.prepared().is_some_and(|p| {
                p.view_verdict(self.display.ignore_unimportant, false)
                    != hikasa_core::CompareVerdict::Incomplete
            });
        Some(hikasa_session::api::ViewOutcome::Sync {
            last_run: sync.last_run,
            complete,
        })
    }

    /// Synchronize ▸ (FC-078) in a Folder Compare view: the preview dialog of the preset on the model as it stands.
    pub fn start_sync_dialog(
        &mut self,
        view: ViewId,
        command: CommandId,
        caption: &str,
        defaults: &SyncOptions,
        mode: ConfirmMode,
    ) -> Option<Box<dyn DialogModel>> {
        let preset = preset_of(command)?;
        if self.task.is_some() || self.busy() || self.flat() {
            return None;
        }
        let mark = Mark::of(self.prepared()?);
        let mut selection = self.selection.items.clone();
        selection.remove(NodeId::ROOT, SideSet::all());
        let options = SyncOptions {
            preset,
            orphans: defaults.orphans,
            create_empty: defaults.create_empty,
            within_archives: defaults.within_archives,
            recycle: defaults.recycle,
            recycle_fallback: defaults.recycle_fallback,
            ..SyncOptions::default()
        };
        let dialog = SyncDialog {
            view,
            caption: caption.to_owned(),
            model: Arc::clone(&mark.model),
            generation: mark.generation,
            just_selection: !selection.is_empty(),
            selection,
            options,
            mode,
            include_hidden: false,
            display: self.display,
            read_only: self.write.read_only,
            overrides: self.write.overrides(),
            plan: DialogPlan::Idle,
            dirty: true,
            job_generation: 0,
            started: None,
            wake: Arc::clone(&self.wake),
            plans: 0,
        };
        let items = dialog.selection.clone();
        self.cancel_crc();
        self.task = Some(Task::Dialog {
            caption: caption.to_owned(),
            mark,
            items,
        });
        Some(Box::new(dialog))
    }

    /// The Synchronize preview closed: Run hands its plan and mode; the questions follow (a sync always asks its
    /// operation question in Prompt), then the run.
    pub fn sync_dialog_closed(
        &mut self,
        service: &Service,
        view: ViewId,
        started: Option<(PreviewList, ConfirmMode)>,
    ) {
        let Some(Task::Dialog { caption, mark, .. }) = self.task.take() else {
            return;
        };
        let Some((list, mode)) = started else {
            return;
        };
        if !mark.holds(self.prepared()) {
            self.refuse_changed(&caption);
            return;
        }
        let Some(plan) = list.op_plan() else {
            return;
        };
        let confirmation = service.folder_confirmation(plan, Some(mode));
        self.sync_confirm(service, view, &caption, list, confirmation, mark);
    }
}

/// A stand-in while a sync's batch is answered.
fn empty_op(op: &hikasa_folder::ops::OpPlan) -> hikasa_folder::ops::OpPlan {
    hikasa_folder::ops::OpPlan {
        verb: op.verb,
        bases: op.bases.clone(),
        plan: hikasa_vfs::ops::Plan {
            op: op.plan.op,
            options: op.plan.options,
            items: Vec::new(),
            warnings: Vec::new(),
        },
        meta: Vec::new(),
        exec: hikasa_vfs::ops::ExecOptions::default(),
        warnings: Vec::new(),
        refresh: Vec::new(),
    }
}

// ---- the Synchronize preview (FC-078, D-246) --------------------------------------------------------------------

enum DialogPlan {
    Idle,
    Planning(Job<(SyncPlan, SyncPreview)>, u64),
    Ready(Box<PreviewList>),
    Failed(String),
}

/// The *Synchronize* preview (`dialog-sync-preview`): *Just selection*, the recycle bin, the confirmation mode,
/// *Include hidden items*; each change plans again on the view's model as it stands (no re-scan).
pub struct SyncDialog {
    /// The view.
    pub view: ViewId,
    caption: String,
    model: Arc<FolderModel>,
    generation: u64,
    /// *Just selection* (preset on with a selection).
    pub just_selection: bool,
    selection: Selection,
    /// The preset's options with the `folder-sync` defaults' orphans, empty folders, archives and bin.
    pub options: SyncOptions,
    /// Prompt, Yes to All, No to All (`sync_confirmations`).
    pub mode: ConfirmMode,
    /// *Include hidden items* (off: a sync reads only its own flag).
    pub include_hidden: bool,
    display: DisplayOptions,
    read_only: hikasa_folder::ops::ReadOnlySides,
    overrides: OpOverrides,
    plan: DialogPlan,
    dirty: bool,
    job_generation: u64,
    started: Option<(PreviewList, ConfirmMode)>,
    wake: Wake,
    /// How many plan jobs started (tests).
    pub plans: u64,
}

impl SyncDialog {
    /// True when *Just selection* can be ticked (something is selected).
    pub fn has_selection(&self) -> bool {
        !self.selection.is_empty()
    }

    /// Changes the options (re-plans).
    pub fn edit(&mut self, change: impl FnOnce(&mut SyncDialog)) {
        let before = (
            self.just_selection,
            self.options.clone(),
            self.mode,
            self.include_hidden,
        );
        change(self);
        if !self.has_selection() {
            self.just_selection = false;
        }
        if (
            self.just_selection,
            self.options.clone(),
            self.mode,
            self.include_hidden,
        ) != before
        {
            self.dirty = true;
        }
    }

    /// The preview, once planned.
    pub fn list(&self) -> Option<&PreviewList> {
        match &self.plan {
            DialogPlan::Ready(list) => Some(list),
            _ => None,
        }
    }

    /// The core message of a refused plan (`EmptySource`, an incomplete comparison with deletes).
    pub fn message(&self) -> Option<&str> {
        match &self.plan {
            DialogPlan::Failed(message) => Some(message),
            _ => None,
        }
    }

    /// "Planning…".
    pub fn planning(&self) -> bool {
        self.dirty || matches!(self.plan, DialogPlan::Planning(..) | DialogPlan::Idle)
    }

    /// The plan and mode after Run (taken by the view).
    pub fn take_started(&mut self) -> Option<(PreviewList, ConfirmMode)> {
        self.started.take()
    }

    fn request(&self) -> FolderSyncRequest {
        FolderSyncRequest {
            options: self.options.clone(),
            select: self.just_selection.then(|| SelectRequest {
                selection: Some(self.selection.clone()),
                display: Some(self.display),
                expand: hikasa_session::api::ExpandWords::None,
                iu: self.display.ignore_unimportant,
                ..SelectRequest::default()
            }),
            visible: None,
            include_hidden: Some(self.include_hidden),
            iu: self.display.ignore_unimportant,
            read_only: self.read_only,
            allow_empty_source: false,
            overrides: self.overrides,
        }
    }

    fn run_enabled(&self) -> bool {
        !self.dirty && self.list().is_some_and(PreviewList::acts)
    }
}

impl DialogModel for SyncDialog {
    fn title(&self) -> String {
        format!("Synchronize: {}", self.caption.trim_end_matches("..."))
    }

    fn help_topic(&self) -> &'static str {
        SYNC_PREVIEW_TOPIC
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((660.0, 420.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "Run".to_owned(),
                role: ButtonRole::Ok,
                enabled: self.run_enabled(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => {
                if !self.run_enabled() || self.model.generation() != self.generation {
                    return DialogStep::Stay;
                }
                match std::mem::replace(&mut self.plan, DialogPlan::Idle) {
                    DialogPlan::Ready(list) => {
                        self.started = Some((*list, self.mode));
                        DialogStep::Close
                    }
                    other => {
                        self.plan = other;
                        DialogStep::Stay
                    }
                }
            }
            ButtonRole::Cancel => {
                if let DialogPlan::Planning(job, _) = &self.plan {
                    job.cancel();
                }
                DialogStep::Close
            }
            _ => DialogStep::Stay,
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if self.dirty {
            self.dirty = false;
            if let DialogPlan::Planning(job, _) = &self.plan {
                job.cancel();
            }
            self.job_generation += 1;
            self.plans += 1;
            let (model, request) = (Arc::clone(&self.model), self.request());
            let job = Job::spawn(
                svc,
                JobKind::FolderSync,
                "plan the sync".into(),
                &self.wake,
                move |svc, job| {
                    let plan = svc.folder_sync_plan(&model, &request, job)?;
                    let preview = svc.folder_sync_preview(&plan);
                    Ok((plan, preview))
                },
            );
            self.plan = DialogPlan::Planning(job, self.job_generation);
            return true;
        }
        let DialogPlan::Planning(job, generation) = &self.plan else {
            return false;
        };
        let Some(result) = job.try_result() else {
            return false;
        };
        if *generation != self.job_generation {
            return false;
        }
        self.plan = match result {
            Ok((plan, preview)) => DialogPlan::Ready(Box::new(PreviewList::sync(
                plan,
                preview,
                Arc::clone(&self.model),
            ))),
            Err(ApiError::Cancelled) => DialogPlan::Idle,
            Err(e) => DialogPlan::Failed(e.to_string()),
        };
        true
    }

    fn busy(&self) -> bool {
        matches!(self.plan, DialogPlan::Planning(..))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
