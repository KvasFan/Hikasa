//! The preview list of an operation dialog (M2 W9b, D-244, D-246; contract §1.9.12): a plan and the preview core made
//! of it, shown as they come. The list is virtualized: the drawing asks for the lines of the rows in view, and each
//! line is made by core from the plan (`OpPreview::line`, `SyncPreview::line`), equal to the `--dry-run` line of the
//! same request. This model holds no toolkit type (check 7) and computes nothing of the plan itself.

use std::sync::Arc;

use hikasa_folder::FolderModel;
use hikasa_folder::ops::OpPlan;
use hikasa_folder::sync::SyncPlan;
use hikasa_session::api::{OpPreview, PreviewItem, SyncLine, SyncPreview};
use hikasa_vfs::ops::ItemStatus;

/// How a line of the list is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// An item that acts, a row with an action.
    Normal,
    /// An item left as it is (the muted colour).
    Muted,
    /// An item that cannot run, a row the comparison could not tell (the error colour).
    Error,
    /// An item that asks a question, a conflict (highlighted).
    Conflict,
    /// The summary and the counts (bold).
    Summary,
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewLine {
    /// The text: the `--dry-run` line.
    pub text: String,
    /// Its colour.
    pub tone: Tone,
}

/// What a dialog previews.
enum Planned {
    /// A folder operation.
    Op {
        plan: Box<OpPlan>,
        preview: OpPreview,
    },
    /// A sync: the comparison's model gives the rows their paths.
    Sync {
        plan: Box<SyncPlan>,
        preview: SyncPreview,
        model: Arc<FolderModel>,
    },
}

/// A plan with its preview: what a dialog shows and what its Start hands on unchanged.
pub struct PreviewList {
    planned: Planned,
}

impl std::fmt::Debug for PreviewList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewList")
            .field("len", &self.len())
            .finish()
    }
}

fn tone_of(item: PreviewItem, conflicts: bool) -> Tone {
    match (item.status, item.reason) {
        (ItemStatus::Failed, _) => Tone::Error,
        (_, Some(_)) => Tone::Muted,
        _ if conflicts => Tone::Conflict,
        _ => Tone::Normal,
    }
}

impl PreviewList {
    /// The list of a folder operation's plan.
    pub fn op(plan: OpPlan, preview: OpPreview) -> PreviewList {
        PreviewList {
            planned: Planned::Op {
                plan: Box::new(plan),
                preview,
            },
        }
    }

    /// The list of a sync plan made on `model`.
    pub fn sync(plan: SyncPlan, preview: SyncPreview, model: Arc<FolderModel>) -> PreviewList {
        PreviewList {
            planned: Planned::Sync {
                plan: Box::new(plan),
                preview,
                model,
            },
        }
    }

    /// The number of lines.
    pub fn len(&self) -> usize {
        match &self.planned {
            Planned::Op { preview, .. } => preview.len() + 1,
            Planned::Sync { preview, .. } => preview.len(),
        }
    }

    /// Never empty: the summary line is always there.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Lines `range` (the rows in view; the whole list in tests).
    pub fn lines(&self, range: std::ops::Range<usize>) -> Vec<PreviewLine> {
        let end = range.end.min(self.len());
        let start = range.start.min(end);
        match &self.planned {
            Planned::Op { plan, preview } => (start..end)
                .map(|i| PreviewLine {
                    text: preview.line(plan, i),
                    tone: match preview.item(i) {
                        Some(item) => tone_of(item, !preview.conflicts(plan, i).is_empty()),
                        None => Tone::Summary,
                    },
                })
                .collect(),
            Planned::Sync {
                plan,
                preview,
                model,
            } => {
                let read = model.read();
                (start..end)
                    .map(|i| PreviewLine {
                        text: preview.line(plan, &read, i),
                        tone: match preview.kind(plan, i) {
                            Some(SyncLine::Row {
                                unresolved: true, ..
                            }) => Tone::Error,
                            Some(SyncLine::Row { conflict: true, .. }) => Tone::Conflict,
                            Some(SyncLine::Row { .. }) => Tone::Normal,
                            Some(SyncLine::Emptied) => Tone::Conflict,
                            Some(SyncLine::Item(item)) => tone_of(item, false),
                            Some(SyncLine::Summary | SyncLine::Counts) | None => Tone::Summary,
                        },
                    })
                    .collect()
            }
        }
    }

    /// Every line's text (tests, *Copy*).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn texts(&self) -> Vec<String> {
        self.lines(0..self.len())
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    /// The counts' lines: per action items and bytes, the skipped and the failed ones; a sync adds its row counts.
    pub fn counts(&self) -> Vec<String> {
        match &self.planned {
            Planned::Op { preview, .. } => preview.counts().iter().map(|c| c.text()).collect(),
            Planned::Sync { preview, .. } => {
                let mut lines: Vec<String> =
                    preview.op().counts().iter().map(|c| c.text()).collect();
                lines.push(preview.counts_line().trim_start_matches("# ").to_owned());
                lines
            }
        }
    }

    /// The plan's warnings, as the dry run reports them.
    pub fn warnings(&self) -> Vec<String> {
        let warnings = match &self.planned {
            Planned::Op { preview, .. } => preview.warnings(),
            Planned::Sync { preview, .. } => preview.warnings(),
        };
        warnings.iter().map(|w| w.message.clone()).collect()
    }

    /// The archives' backups: `<file> → <backup>`.
    pub fn backups(&self) -> Vec<String> {
        let backups = match &self.planned {
            Planned::Op { preview, .. } => preview.backups(),
            Planned::Sync { preview, .. } => preview.op().backups(),
        };
        backups
            .iter()
            .map(|(file, backup)| format!("{} → {}", file.canonical(), backup.canonical()))
            .collect()
    }

    /// True when Start has something to do (an item acts).
    pub fn acts(&self) -> bool {
        match &self.planned {
            Planned::Op { preview, .. } => preview.acts(),
            Planned::Sync { preview, .. } => preview.acts(),
        }
    }

    /// The operation's plan.
    pub fn op_plan(&self) -> Option<&OpPlan> {
        match &self.planned {
            Planned::Op { plan, .. } => Some(plan),
            Planned::Sync { plan, .. } => Some(&plan.op),
        }
    }

    /// The sync plan.
    pub fn sync_plan(&self) -> Option<&SyncPlan> {
        match &self.planned {
            Planned::Sync { plan, .. } => Some(plan),
            Planned::Op { .. } => None,
        }
    }

    /// The operation's plan, handed on unchanged (Start).
    pub fn into_op(self) -> Option<OpPlan> {
        match self.planned {
            Planned::Op { plan, .. } => Some(*plan),
            Planned::Sync { .. } => None,
        }
    }

    /// The sync plan, handed on unchanged (Run).
    pub fn into_sync(self) -> Option<SyncPlan> {
        match self.planned {
            Planned::Sync { plan, .. } => Some(*plan),
            Planned::Op { .. } => None,
        }
    }
}
