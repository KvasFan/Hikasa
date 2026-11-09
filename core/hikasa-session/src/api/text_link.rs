//! The session link of a child text view (M2 W10c; contract §1.10.21, TC-052, D-288): the child's row in its parent
//! folder comparison, the parent compared again when its view is closed, Next/Previous Difference Files, and *Copy File
//! to Right/Left and Open Next Difference* as a list of steps.
//!
//! ```text
//! parent_row(read, link)                   the row of a ParentLink (left/right paths relative to the bases)
//! text_parent_compare(link, job, observer) the parent session compared again (folder_prepare_in + folder_run; a
//!                                          folder-sync session through folder_sync_prepare), then its row
//! text_copy_file_steps(req, job)           the steps, nothing written: the source's edits saved first (SavePlan), the
//!                                          target's lost, W6's copy of the row (folder_op_plan), the next pair
//! text_copy_file_execute(steps, …)         save → copy → refresh the parent's rows → the next pair; a failed step
//!                                          stops the chain
//! ```
//!
//! Refusals come before anything is planned: a source pane with edits that cannot be saved is
//! [`ApiError::EditBlocked`] (§1.10.7, A10.6), a read-only target side [`ApiError::ReadOnlySide`] (D-184).

use hikasa_core::{ErrorReport, ExitCode, Pair, Side};
use hikasa_folder::ops::{ConfirmMode, FolderOp, OpPlan, ReadOnlySides};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::view::{Dir, NextFileOptions, next_difference_file};
use hikasa_folder::{DisplayOptions, NodeId, NodeKind, ScanObserver, TreeRead};
use hikasa_text::save::{SavePlan, SaveReport, SaveRequest, UnmappablePolicy};
use hikasa_text::{EditBlock, EditableDocument};
use hikasa_vfs::ops::OverwritePolicy;

use super::folder_ops::{FolderOpOutcome, FolderOpRequest, OpRequest, RefreshDto, SelectRequest};
use super::responses::{FolderCompareResponse, PreparedFolderCompare};
use super::sessions::SessionRunRequest;
use super::text_edit::{apply_save_report, save_exit_code};
use super::{ApiError, Service, ensure_editable};
use crate::jobs::{JobControl, run_guarded};
use crate::sessions::ParentLink;

/// The row of a child view in its parent comparison (§1.10.21): the row whose left item is at `link.left` below the
/// left base and whose right item is at `link.right` below the right base. Each side's path is looked up by that side's
/// names (exact, then by the comparison's case when it is not case-sensitive; a file row before a folder of the same
/// name, D-20), so a pair aligned across a case difference is found. When the two paths name different rows the row
/// that has both items wins. `None` when neither path names a row (the parent changed since the child was opened).
/// Pure; O(depth · siblings), a flat comparison O(rows).
pub fn parent_row(read: &TreeRead<'_>, link: &ParentLink) -> Option<NodeId> {
    let left = (!link.left.is_empty())
        .then(|| row_on_side(read, &link.left, Side::Left))
        .flatten();
    let right = (!link.right.is_empty())
        .then(|| row_on_side(read, &link.right, Side::Right))
        .flatten();
    let both = |id: NodeId| {
        read.node(id)
            .is_some_and(|view| view.left.is_some() && view.right.is_some())
    };
    match (left, right) {
        (Some(l), Some(r)) if l == r => Some(l),
        (Some(l), Some(r)) => Some(if !both(l) && both(r) { r } else { l }),
        (l, r) => l.or(r),
    }
}

/// The parent link of the pair at `row` (what a child view opened from it, or the next pair opened in the same view,
/// stores, TC-052): `session` and each side's path below its base. `None` for the root or a row of another model.
pub fn parent_link_of(read: &TreeRead<'_>, row: NodeId, session: &str) -> Option<ParentLink> {
    if row == NodeId::ROOT {
        return None;
    }
    read.node(row)?;
    Some(ParentLink {
        session: session.to_owned(),
        left: side_path(read, row, Side::Left).unwrap_or_default(),
        right: side_path(read, row, Side::Right).unwrap_or_default(),
    })
}

/// The `/`-separated path of `row`'s item on `side` below that side's base (a flat comparison: the item's folder and
/// name). `None` when the row has no item there.
fn side_path(read: &TreeRead<'_>, row: NodeId, side: Side) -> Option<String> {
    let view = read.node(row)?;
    let entry = view.side(side)?;
    if let Some(dir) = entry.dir {
        return Some(if dir.is_empty() {
            entry.name.to_owned()
        } else {
            format!("{dir}/{}", entry.name)
        });
    }
    let mut names = vec![entry.name.to_owned()];
    for ancestor in read.ancestors(row) {
        if ancestor == NodeId::ROOT {
            break;
        }
        names.push(read.node(ancestor)?.side(side)?.name.to_owned());
    }
    names.reverse();
    Some(names.join("/"))
}

/// The row whose item on `side` is at `path`: a walk by that side's names, then (a flat comparison, or a tree that moved)
/// a scan of every row's path on that side.
fn row_on_side(read: &TreeRead<'_>, path: &str, side: Side) -> Option<NodeId> {
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if components.is_empty() {
        return None;
    }
    let insensitive = read.case() == Some(hikasa_core::CaseSensitivity::Insensitive);
    let mut at = NodeId::ROOT;
    let mut found = true;
    for (k, component) in components.iter().enumerate() {
        let last = k + 1 == components.len();
        let named = |id: NodeId, exact: bool| {
            read.node(id).is_some_and(|view| {
                view.side(side).is_some_and(|entry| {
                    entry.dir.is_none()
                        && if exact {
                            entry.name == *component
                        } else {
                            hikasa_core::case::fold(entry.name)
                                == hikasa_core::case::fold(component)
                        }
                })
            })
        };
        // The last component prefers a file row, the others a folder (a file and a folder of one name are two rows).
        let wanted = |id: NodeId| {
            read.node(id)
                .is_some_and(|view| (view.kind == NodeKind::Folder) != last)
        };
        let pick = |exact: bool| {
            let mut matches = read.children(at).filter(|&c| named(c, exact));
            let first = matches.next()?;
            Some(if wanted(first) {
                first
            } else {
                matches.find(|&c| wanted(c)).unwrap_or(first)
            })
        };
        match pick(true).or_else(|| if insensitive { pick(false) } else { None }) {
            Some(next) => at = next,
            None => {
                found = false;
                break;
            }
        }
    }
    if found {
        return Some(at);
    }
    read.preorder()
        .map(|(id, _)| id)
        .find(|&id| side_path(read, id, side).as_deref() == Some(path))
}

/// The parent session compared again (§1.10.21's open default): its comparison, its effective rules and read-only
/// sides, and the child's row in it.
pub struct ParentCompare {
    /// The prepared comparison (the model the copies plan on).
    pub prepared: PreparedFolderCompare,
    /// The finished comparison.
    pub response: FolderCompareResponse,
    /// The parent session's `[specs] read_only_*` (D-184: the child views carry them).
    pub read_only: ReadOnlySides,
    /// The child's row ([`parent_row`]); `None` when the parent no longer has it.
    pub row: Option<NodeId>,
}

impl std::fmt::Debug for ParentCompare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParentCompare")
            .field("read_only", &self.read_only)
            .field("row", &self.row)
            .finish_non_exhaustive()
    }
}

/// Copy File to Right/Left and Open Next Difference (§1.10.21): the parent's comparison, the row, the side copied from,
/// the panes' documents and the parent's read-only sides.
#[derive(Debug, Clone, Copy)]
pub struct CopyFileRequest<'a> {
    /// The parent folder comparison (its view's, or [`Service::text_parent_compare`]'s).
    pub parent: &'a PreparedFolderCompare,
    /// The child's row ([`parent_row`]).
    pub row: NodeId,
    /// The side copied from (`Left` for Copy File to Right).
    pub from: Side,
    /// The panes' documents.
    pub docs: Pair<&'a EditableDocument>,
    /// The parent's read-only sides (D-184).
    pub read_only: ReadOnlySides,
    /// How the next pair is found ([`Service::next_file_options`]). Additive to the contract's fields.
    pub next: NextFileOptions,
    /// The save of the source's edits ([`Service::live_save_request`]); `None`: in place, `[backups]
    /// back_up_before_save`, unmappable characters asked. Additive.
    pub save: Option<&'a SaveRequest>,
}

/// The steps of a copy (§1.10.21), in their order; the confirmation lists them (TC-052). Nothing is written yet.
#[derive(Debug, Clone)]
pub struct CopyFileSteps {
    /// The source pane's unsaved edits, saved first (`None`: nothing to save).
    pub save_source: Option<SavePlan>,
    /// The target pane's edits are lost.
    pub discard_target: bool,
    /// W6's plan of the copy: `folder op copy --to <other> --path <row>`.
    pub copy: OpPlan,
    /// The pair that opens next (as the tree stands before the copy).
    pub next: Option<NodeId>,
}

/// A step of [`Service::text_copy_file_execute`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyFileStep {
    /// Saving the source's edits.
    Save,
    /// The copy.
    Copy,
    /// Refreshing the parent's rows.
    Refresh,
}

/// What [`Service::text_copy_file_execute`] did; `stopped` names the step that failed (the chain ends there).
#[derive(Debug)]
pub struct CopyFileReport {
    /// The save of the source's edits, when it ran.
    pub save: Option<SaveReport>,
    /// The copy, when it ran (or was answered no: then nothing was written).
    pub copy: Option<FolderOpOutcome>,
    /// The refresh of the parent's rows, when it ran.
    pub refresh: Option<RefreshDto>,
    /// The copied row as it stands after the refresh.
    pub row: Option<NodeId>,
    /// The pair to open next (after the refresh).
    pub next: Option<NodeId>,
    /// The step that failed, was declined or asked what no one answered.
    pub stopped: Option<CopyFileStep>,
    /// Why it stopped, when it was not an item of a report.
    pub error: Option<ErrorReport>,
}

impl CopyFileReport {
    /// True when every step ran to its end.
    pub fn ok(&self) -> bool {
        self.stopped.is_none()
    }

    /// 0 when every step ran, else 100 (§1.10.24's rule for a write with failed items).
    pub fn exit_code(&self) -> ExitCode {
        if self.ok() {
            ExitCode::Success
        } else {
            ExitCode::UnknownError
        }
    }
}

/// What a copy runs with besides the steps (§1.10.21).
#[derive(Debug, Clone, Copy)]
pub struct CopyFileExecute<'a> {
    /// The parent folder comparison (refreshed in place).
    pub parent: &'a PreparedFolderCompare,
    /// The row.
    pub row: NodeId,
    /// The side copied from.
    pub from: Side,
    /// The parent's read-only sides.
    pub read_only: ReadOnlySides,
    /// How the next pair is found.
    pub next: NextFileOptions,
    /// The copy's own questions (W6: a read-only or newer target, …): `YesToAll` when the confirmation that listed the
    /// steps answers them (the GUI's OK, `--yes`), `NoToAll` declines every item that asks, `Prompt` stops the chain
    /// at the copy when a question is left.
    pub copy_mode: ConfirmMode,
}

/// The blocks that make a target pane read-only as a side (D-184): the parent's, the session's and the switches'. A
/// location the process cannot write is the copy's question, as in W6.
const TARGET_BLOCKS: [EditBlock; 3] = [
    EditBlock::ParentReadOnly,
    EditBlock::SessionReadOnly,
    EditBlock::CommandLine,
];

impl Service {
    /// The options of Next/Previous Difference Files (OPT-007): `[next_difference]
    /// limit_next_difference_files_to_current_folder` and `wrap_around` read now, with the parent view's display and
    /// ignore-unimportant.
    pub fn next_file_options(
        &self,
        display: DisplayOptions,
        ignore_unimportant: bool,
    ) -> NextFileOptions {
        let options = self.fresh_options();
        NextFileOptions {
            limit_to_folder: options
                .next_difference
                .limit_next_difference_files_to_current_folder,
            wrap: options.next_difference.wrap_around,
            display,
            ignore_unimportant,
        }
    }

    /// The child's row in an open parent comparison ([`parent_row`]); `NotFound` (exit 100) when the parent has no such
    /// row.
    pub fn text_parent_row(
        &self,
        parent: &PreparedFolderCompare,
        link: &ParentLink,
    ) -> Result<NodeId, ApiError> {
        run_guarded(|| {
            parent_row(&parent.model().read(), link).ok_or_else(|| {
                ApiError::NotFound(format!(
                    "the parent comparison has no row `{}` / `{}`",
                    link.left, link.right
                ))
            })
        })
    }

    /// The parent session compared again (§1.10.21's open default, D-288: its view is closed, as in a restored
    /// workspace): the session of `link.session` (a named reference or `_autosaved/<id>`) with its effective rules,
    /// `folder_prepare_in` + `folder_run` (a folder-sync session: `folder_sync_prepare`, the comparison its sync view
    /// shows), then the child's row. Run it in a job (`JobKind::FolderCompare`). A text session as parent is
    /// `InvalidArgument`; a missing session `NotFound`.
    pub fn text_parent_compare(
        &self,
        link: &ParentLink,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<ParentCompare, ApiError> {
        run_guarded(|| {
            let store = self.session_store();
            let (mut session, _) = self.load_parent(&store, &link.session)?;
            self.resolve_parent(&store, &mut session)?;
            let read_only = ReadOnlySides {
                left: session.specs.read_only_left,
                right: session.specs.read_only_right,
            };
            let prepared = match super::sessions::run_request(&session.for_script())? {
                SessionRunRequest::Folder(request) => self.folder_prepare_in(&request, job)?,
                SessionRunRequest::Sync { request, options } => {
                    self.folder_sync_prepare(&request, &options, job)?
                }
                SessionRunRequest::Text(_) => {
                    return Err(ApiError::InvalidArgument(format!(
                        "the parent `{}` is a {} session; a parent is a folder session",
                        link.session, session.kind
                    )));
                }
            };
            let response = self.folder_run(&prepared, job, observer)?;
            let row = parent_row(&prepared.model().read(), link);
            Ok(ParentCompare {
                prepared,
                response,
                read_only,
                row,
            })
        })
    }

    /// The copy of `row` from `from` to the other side as `folder op copy --to <other> --path <row>` plans it.
    fn copy_file_request(
        parent: &PreparedFolderCompare,
        row: NodeId,
        from: Side,
        read_only: ReadOnlySides,
    ) -> FolderOpRequest {
        let mut selection = Selection::default();
        selection.insert(row, SideSet::of(from));
        FolderOpRequest {
            op: OpRequest::Op(FolderOp::Copy {
                to: from.opposite(),
                overwrite: OverwritePolicy::Always,
            }),
            select: SelectRequest {
                selection: Some(selection),
                iu: parent.ignore_unimportant(),
                ..SelectRequest::default()
            },
            read_only,
            overrides: super::folder_ops::OpOverrides::default(),
        }
    }

    /// Plans Copy File to Right/Left and Open Next Difference (§1.10.21, TC-052; run as a job): nothing is written.
    ///
    /// Refusals first: `from` must be `Left` or `Right`; a source pane with edits whose document is blocked is
    /// [`ApiError::EditBlocked`] (A10.6, as `text_save_plan`); a read-only target side (the parent's `read_only`, or the
    /// target pane's `ParentReadOnly`, `SessionReadOnly`, `CommandLine`) is [`ApiError::ReadOnlySide`] (D-184). Then the
    /// steps: the source's save plan when it has edits (`text_save_plan`), whether the target's edits are lost, W6's
    /// plan of the copy (`folder_op_plan`, the same plan as `folder op copy --to <other> --path <row>`), and the next pair
    /// from `row` ([`next_difference_file`], forward).
    pub fn text_copy_file_steps(
        &self,
        req: &CopyFileRequest<'_>,
        job: &JobControl,
    ) -> Result<CopyFileSteps, ApiError> {
        run_guarded(|| {
            let from = req.from;
            if !matches!(from, Side::Left | Side::Right) {
                return Err(ApiError::InvalidArgument(format!(
                    "a file is copied from the left or the right side, not the {from} side"
                )));
            }
            let to = from.opposite();
            let (source, target) = if from == Side::Left {
                (req.docs.left, req.docs.right)
            } else {
                (req.docs.right, req.docs.left)
            };
            if source.is_modified() {
                ensure_editable(Some(from), &source.editability())?;
            }
            let target_read_only = target
                .editability()
                .blocks
                .iter()
                .any(|block| TARGET_BLOCKS.contains(block));
            if req.read_only.get(to) || target_read_only {
                return Err(ApiError::ReadOnlySide { side: to });
            }
            let save_source = if source.is_modified() {
                let default;
                let save = match req.save {
                    Some(save) => save,
                    None => {
                        default = SaveRequest::in_place(
                            self.backup_policy_for_save(false)?,
                            UnmappablePolicy::Ask,
                        );
                        &default
                    }
                };
                let router = self.router();
                Some(self.plan_save_in(&*router, &[(Some(from), source, save)], job)?)
            } else {
                None
            };
            let copy = self.folder_op_plan(
                req.parent.model(),
                &Self::copy_file_request(req.parent, req.row, from, req.read_only),
                job,
            )?;
            let next =
                next_difference_file(&req.parent.model().read(), req.row, Dir::Forward, &req.next);
            Ok(CopyFileSteps {
                save_source,
                discard_target: target.is_modified(),
                copy,
                next,
            })
        })
    }

    /// Runs the steps of [`Service::text_copy_file_steps`] in order (§1.10.21): the source's save (`steps.save_source`,
    /// answered by the caller through [`Service::save_confirmation`]) applied to `source`; the copy — planned again when
    /// the save changed the file, since W6's plan expects the source it saw — with `exec.copy_mode` answering its
    /// questions; the refresh of the parent's rows the copy touched; then the next pair from the copied row. A failed,
    /// declined or unanswered step stops the chain ([`CopyFileReport::stopped`]); the target pane's edits are the
    /// caller's to drop. The call itself fails only when it is refused (as the steps are) or cancelled.
    pub fn text_copy_file_execute(
        &self,
        steps: CopyFileSteps,
        exec: &CopyFileExecute<'_>,
        source: &mut EditableDocument,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<CopyFileReport, ApiError> {
        run_guarded(|| {
            let model = exec.parent.model();
            let link = parent_link_of(&model.read(), exec.row, "")
                .ok_or_else(|| ApiError::NotFound("the copied row is not in the parent".into()))?;
            let mut report = CopyFileReport {
                save: None,
                copy: None,
                refresh: None,
                row: Some(exec.row),
                next: None,
                stopped: None,
                error: None,
            };
            let mut copy = Some(steps.copy);
            if let Some(plan) = steps.save_source {
                ensure_editable(Some(exec.from), &source.editability())?;
                let router = self.router();
                let saved = Self::save_execute_in(&*router, plan, &[&*source], job);
                apply_save_report(&saved, &mut [&mut *source]);
                let ok = save_exit_code(&saved) == ExitCode::Success;
                let wrote = saved.saved.iter().any(Option::is_some);
                report.save = Some(saved);
                if !ok {
                    report.stopped = Some(CopyFileStep::Save);
                    return Ok(report);
                }
                if wrote {
                    // The model's stamp of the source is the scan's: refresh its folder, find the row again, plan again.
                    let folder = model
                        .read()
                        .node(exec.row)
                        .and_then(|view| view.parent)
                        .unwrap_or(NodeId::ROOT);
                    if let Err(e) = self.folder_refresh(model, &[folder], job, observer) {
                        report.stopped = Some(CopyFileStep::Refresh);
                        report.error = Some(e.report());
                        return Ok(report);
                    }
                    report.row = parent_row(&model.read(), &link);
                    copy = None;
                }
            }
            let Some(row) = report.row else {
                report.stopped = Some(CopyFileStep::Copy);
                report.error = Some(
                    ApiError::NotFound(format!(
                        "the parent comparison no longer has the row `{}` / `{}`",
                        link.left, link.right
                    ))
                    .report(),
                );
                return Ok(report);
            };
            let plan = match copy {
                Some(plan) => plan,
                None => self.folder_op_plan(
                    model,
                    &Self::copy_file_request(exec.parent, row, exec.from, exec.read_only),
                    job,
                )?,
            };
            let mut confirmation = self.folder_confirmation(&plan, Some(exec.copy_mode));
            if exec.copy_mode == ConfirmMode::YesToAll {
                while confirmation.next().is_some() {
                    confirmation.reply(hikasa_core::confirm::Reply::YesToAll);
                }
            }
            if let Some(question) = confirmation.next() {
                report.stopped = Some(CopyFileStep::Copy);
                report.error = Some(
                    ApiError::InvalidArgument(format!(
                        "the copy asks a question no one answered: {}",
                        question.text
                    ))
                    .report(),
                );
                return Ok(report);
            }
            let Ok(plan) = confirmation.finish(plan) else {
                report.stopped = Some(CopyFileStep::Copy);
                report.error = Some(ApiError::Cancelled.report());
                return Ok(report);
            };
            let outcome = self.folder_op_execute(plan, job)?;
            // A skipped item (declined, filtered, …) copied nothing: the pair is not the same, the chain stops.
            let copied = outcome.ok()
                && !outcome.report.cancelled
                && outcome
                    .report
                    .results
                    .iter()
                    .all(|result| result.status == hikasa_vfs::ops::ItemStatus::Done);
            let refresh_nodes = outcome.refresh_nodes.clone();
            report.copy = Some(outcome);
            if !copied {
                report.stopped = Some(CopyFileStep::Copy);
                return Ok(report);
            }
            match self.folder_refresh(model, &refresh_nodes, job, observer) {
                Ok(dto) => report.refresh = Some(dto),
                Err(e) => {
                    report.stopped = Some(CopyFileStep::Refresh);
                    report.error = Some(e.report());
                    return Ok(report);
                }
            }
            let read = model.read();
            report.row = parent_row(&read, &link);
            report.next = report
                .row
                .and_then(|row| next_difference_file(&read, row, Dir::Forward, &exec.next));
            Ok(report)
        })
    }
}
