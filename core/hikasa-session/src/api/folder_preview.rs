//! The previews of folder operations and syncs (M2 W9b, D-244, D-246; contract §1.9.12, §1.9.14): what an operation
//! dialog shows before Start, made once from the plan the Start executes. A preview **borrows** its plan: it keeps per
//! item only the dry run's status and reason (2 bytes), and makes each line on demand from the plan, so a
//! 100,000-item plan is never copied (G15 preview: ≤ 16 bytes an item). Every line equals the line `hikasa folder op …
//! --dry-run` (or `folder sync … --dry-run`) prints for the same request: the lines are built by the same functions
//! (`item_line`, the sync marker, the summary and counts texts), T11.

use hikasa_core::{Spec, Warning};
use hikasa_folder::TreeRead;
use hikasa_folder::ops::{Conflicts, OpPlan, OpVerb};
use hikasa_folder::sync::{LeaveReason, SyncAction, SyncCounts, SyncPlan};
use hikasa_report::escape_controls;
use hikasa_vfs::ops::{self, Action, ItemStatus, SkipReason, Summary};

use super::Service;
use super::folder_ops::{
    EmptiedDto, archive_backups, emptied_text, item_dto_of, item_line, marker_of, summary_text,
    sync_counts_text,
};

/// What one item of a preview is in the dry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewItem {
    /// `planned`, or `failed` for an item whose planning failed.
    pub status: ItemStatus,
    /// Why the item is left as it is (`None`: it acts).
    pub reason: Option<SkipReason>,
}

/// What a line of the counts stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountKind {
    /// Items that act, by the action's word (`copy`, `remove`, `trash`, …), in the order the plan first names them.
    Action(&'static str),
    /// Items left as they are (a skip reason, or a `skip` item).
    Skipped,
    /// Items that cannot run (their planning failed).
    Failed,
}

/// One line of a preview's counts: per action the items and their bytes, then the skipped and the failed ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewCount {
    /// What it counts.
    pub kind: CountKind,
    /// Items.
    pub items: u64,
    /// The items' sizes (what a copy writes, what a removal frees).
    pub bytes: u64,
}

impl PreviewCount {
    /// The line the dialogs show: `copy: 3 items, 120 bytes`, `left as they are: 2 items`, `cannot run: 1 items`.
    pub fn text(&self) -> String {
        match self.kind {
            CountKind::Action(word) if self.bytes > 0 => {
                format!("{word}: {} items, {} bytes", self.items, self.bytes)
            }
            CountKind::Action(word) => format!("{word}: {} items", self.items),
            CountKind::Skipped => format!("left as they are: {} items", self.items),
            CountKind::Failed => format!("cannot run: {} items", self.items),
        }
    }
}

/// The preview of an [`OpPlan`] (D-244): the dry run's status and reason per item, the summary, the counts, the warnings
/// and the archives' backups. Lines are made from the plan the preview was made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpPreview {
    verb: OpVerb,
    items: Vec<PreviewItem>,
    summary: Summary,
    counts: Vec<PreviewCount>,
    warnings: Vec<Warning>,
    backups: Vec<(Spec, Spec)>,
}

impl OpPreview {
    /// The number of items (lines before the summary).
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True for a plan without items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The verb.
    pub fn verb(&self) -> OpVerb {
        self.verb
    }

    /// Item `index` in the dry run.
    pub fn item(&self, index: usize) -> Option<PreviewItem> {
        self.items.get(index).copied()
    }

    /// True when some item acts (neither left nor failed): Start has something to do.
    pub fn acts(&self) -> bool {
        self.counts
            .iter()
            .any(|c| matches!(c.kind, CountKind::Action(_)) && c.items > 0)
    }

    /// Per action the items and bytes, then the skipped and the failed ones (W6d's lines, in core).
    pub fn counts(&self) -> Vec<PreviewCount> {
        self.counts.clone()
    }

    /// The counts as the dry run's summary has them.
    pub fn summary(&self) -> Summary {
        self.summary
    }

    /// The last line of `folder op … --dry-run`: `would copy 4 items`.
    pub fn summary_line(&self) -> String {
        summary_text(self.verb, true, &self.summary, None)
    }

    /// Line `index` of `folder op … --dry-run`'s human output (`plan` is the plan the preview was made of); the
    /// summary line past the items; `""` past the end.
    pub fn line(&self, plan: &OpPlan, index: usize) -> String {
        if index == self.items.len() {
            return self.summary_line();
        }
        let (Some(item), Some(meta), Some(at)) = (
            plan.plan.items.get(index),
            plan.meta.get(index),
            self.items.get(index),
        ) else {
            return String::new();
        };
        let dto = item_dto_of(item, meta, at.status, at.reason, item.error.clone());
        item_line(&dto, true)
    }

    /// What item `index` needs asked.
    pub fn conflicts(&self, plan: &OpPlan, index: usize) -> Conflicts {
        plan.meta
            .get(index)
            .map_or_else(Conflicts::empty, |meta| meta.conflicts)
    }

    /// The warnings the dry run reports (the plan's, then the batch's, then the shadowed members').
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Each outermost archive file with its backup.
    pub fn backups(&self) -> &[(Spec, Spec)] {
        &self.backups
    }
}

/// What a line of a sync preview is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncLine {
    /// A row with its action: a conflict, a row the comparison could not tell (`unresolved`), an override.
    Row {
        /// FS-004's conflict.
        conflict: bool,
        /// `compare_error`: the row is left unresolved.
        unresolved: bool,
        /// An override chose the action.
        overridden: bool,
    },
    /// A folder emptied entirely.
    Emptied,
    /// An item of the batch.
    Item(PreviewItem),
    /// The summary line.
    Summary,
    /// The counts line.
    Counts,
}

/// The preview of a [`SyncPlan`] (D-246): one line per row that is not `same` (`<marker> <action> <path>`), the
/// folders emptied entirely, the items, the summary and the counts, as `folder sync … --dry-run` prints them. Its row
/// lines read the paths of the comparison the plan was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPreview {
    rows: Vec<u32>,
    op: OpPreview,
    counts: SyncCounts,
    warnings: Vec<Warning>,
    emptied: usize,
}

impl SyncPreview {
    /// The number of lines.
    pub fn len(&self) -> usize {
        self.rows.len() + self.emptied + self.op.len() + 2
    }

    /// Never empty: the summary and the counts are always there.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The rows shown, as indices of `SyncPlan::rows`.
    pub fn rows(&self) -> &[u32] {
        &self.rows
    }

    /// The preview of the batch.
    pub fn op(&self) -> &OpPreview {
        &self.op
    }

    /// The counts of the rows.
    pub fn counts(&self) -> SyncCounts {
        self.counts
    }

    /// The counts line: `3 copies right, … 0 unresolved`.
    pub fn counts_line(&self) -> String {
        format!("# {}", sync_counts_text(&self.counts))
    }

    /// True when Run has something to do.
    pub fn acts(&self) -> bool {
        self.op.acts()
    }

    /// The warnings of the dry run (the sync's first).
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// What line `index` is.
    pub fn kind(&self, plan: &SyncPlan, index: usize) -> Option<SyncLine> {
        let rows = self.rows.len();
        let emptied = rows + self.emptied;
        let items = emptied + self.op.len();
        Some(if index < rows {
            let row = plan.rows.get(self.rows[index] as usize)?;
            SyncLine::Row {
                conflict: row.conflict,
                unresolved: row.reason == Some(LeaveReason::CompareError),
                overridden: row.overridden,
            }
        } else if index < emptied {
            SyncLine::Emptied
        } else if index < items {
            SyncLine::Item(self.op.item(index - emptied)?)
        } else if index == items {
            SyncLine::Summary
        } else if index == items + 1 {
            SyncLine::Counts
        } else {
            return None;
        })
    }

    /// Line `index` of `folder sync … --dry-run`'s human output; `read` is the comparison `plan` was made from.
    pub fn line(&self, plan: &SyncPlan, read: &TreeRead<'_>, index: usize) -> String {
        let rows = self.rows.len();
        let emptied = rows + self.emptied;
        let items = emptied + self.op.len();
        if index < rows {
            let Some(row) = plan.rows.get(self.rows[index] as usize) else {
                return String::new();
            };
            let mut line = format!(
                "{} {} {}",
                marker_of(row.action, row.conflict),
                row.action.as_str(),
                escape_controls(read.path(row.node).as_str())
            );
            if let Some(reason) = row.reason {
                line.push_str(&format!(" ({})", reason.as_str()));
            }
            line
        } else if index < emptied {
            let Some(&(node, empty)) = plan.emptied.get(index - rows) else {
                return String::new();
            };
            let folder = EmptiedDto {
                path: read.path(node).as_str().to_owned(),
                empty: empty.as_str(),
                emptied: empty.opposite().as_str(),
            };
            format!("# {}", emptied_text(&folder))
        } else if index <= items {
            // The items, then the summary (`OpPreview::line` past its items).
            self.op.line(&plan.op, index - emptied)
        } else if index == items + 1 {
            self.counts_line()
        } else {
            String::new()
        }
    }
}

/// True for a sync row the human output shows: one with an action, or left for a reason other than `same`.
fn row_shown(action: SyncAction, reason: Option<LeaveReason>) -> bool {
    (action != SyncAction::Leave || !matches!(reason, None | Some(LeaveReason::Same)))
        && reason != Some(LeaveReason::Same)
}

impl Service {
    /// The preview of `plan` (D-244): what `folder_op_dry_run(plan)` would report, without taking or copying the plan.
    pub fn folder_op_preview(&self, plan: &OpPlan) -> OpPreview {
        let mut counts: Vec<PreviewCount> = Vec::new();
        let (mut skipped, mut failed) = (0u64, 0u64);
        let mut items = Vec::with_capacity(plan.plan.items.len());
        for item in &plan.plan.items {
            let status = if item.error.is_some() {
                failed += 1;
                ItemStatus::Failed
            } else {
                if item.skip.is_some() || item.action == Action::Skip {
                    skipped += 1;
                } else {
                    let word = item.action.as_str();
                    let bytes = item_bytes(item);
                    match counts
                        .iter_mut()
                        .find(|c| c.kind == CountKind::Action(word))
                    {
                        Some(count) => {
                            count.items += 1;
                            count.bytes += bytes;
                        }
                        None => counts.push(PreviewCount {
                            kind: CountKind::Action(word),
                            items: 1,
                            bytes,
                        }),
                    }
                }
                ItemStatus::Planned
            };
            items.push(PreviewItem {
                status,
                reason: if item.error.is_some() {
                    None
                } else {
                    item.skip
                },
            });
        }
        for (kind, n) in [(CountKind::Skipped, skipped), (CountKind::Failed, failed)] {
            if n > 0 {
                counts.push(PreviewCount {
                    kind,
                    items: n,
                    bytes: 0,
                });
            }
        }
        let summary = Summary {
            planned: items.len() as u64,
            done: 0,
            skipped: 0,
            failed,
            bytes: 0,
        };
        let router = self.router();
        let mut warnings = plan.warnings.clone();
        warnings.extend(plan.plan.warnings.iter().cloned());
        warnings.extend(ops::shadow_drop_warnings(&*router, &*router, &plan.plan));
        OpPreview {
            verb: plan.verb,
            items,
            summary,
            counts,
            warnings,
            backups: archive_backups(&plan.plan.items, &plan.exec.archive_backups),
        }
    }

    /// The preview of a sync plan (D-246): what `folder_sync_dry_run(plan)` would report.
    pub fn folder_sync_preview(&self, plan: &SyncPlan) -> SyncPreview {
        let rows = plan
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row_shown(row.action, row.reason))
            .map(|(index, _)| index as u32)
            .collect();
        let op = self.folder_op_preview(&plan.op);
        let mut warnings = plan.warnings.clone();
        warnings.extend(op.warnings.iter().cloned());
        SyncPreview {
            rows,
            op,
            counts: plan.counts,
            warnings,
            emptied: plan.emptied.len(),
        }
    }
}

/// The bytes an item copies, as the counts of W6d had them: its size (copies, moves, an exchange's source).
fn item_bytes(item: &hikasa_vfs::ops::PlanItem) -> u64 {
    item.size.unwrap_or(0)
}
