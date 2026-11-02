//! Folder operations (FC-070…FC-074, FC-076, FC-077; D-177, D-178): **one builder** turns a selection of a comparison into
//! one [`hikasa_vfs::ops::Plan`] of kind `Batch`, and the questions it needs. `hikasa_folder::ops` plans;
//! `hikasa_vfs::ops` executes. An [`OpPlan`] is plain data, with no borrow of the model: a dry run prints it, a dialog holds
//! it across frames, and execution takes it.

mod build;
mod confirm;
mod conflict;
mod copyto;
pub mod rename;
mod renames;
mod unlisted;

use std::collections::BTreeSet;

use hikasa_core::{Ctl, FileTime, Pair, Side, Spec, Warning};
use hikasa_vfs::Vfs;
use hikasa_vfs::ops::{BackupPolicy, ExecOptions, LinkPolicy, OverwritePolicy, Plan, Preserve};
use serde::{Deserialize, Serialize};

pub use confirm::{
    ConfirmMode, ConfirmPolicy, Confirmation, Declined, Question, QuestionKind, Reply,
};
pub use conflict::Conflicts;
pub use rename::{RegexRename, base_mask, dos_rename};

use crate::error::FolderError;
use crate::model::{NodeId, TreeRead};
use crate::select::{Include, Selection, SideSet};

/// How `copyto` and `moveto` lay out their sources below the target (FC-072).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathStructure {
    /// The minimum common structure: the longest folder prefix common to all sources of a side goes.
    #[default]
    Relative,
    /// The whole path below the base.
    Base,
    /// Flat.
    None,
}

/// The times of a `touch`. "Now" is resolved by the Service (T1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchFrom {
    /// This time.
    At(FileTime),
    /// The other side's time, on the row's side `to`.
    OtherSide {
        /// The side touched.
        to: Side,
    },
}

/// What a delete does with the recycle bin (§1.6.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Recycle {
    /// *Use recycle bin if possible*.
    pub use_bin: bool,
    /// *If the recycle bin fails, delete permanently*.
    pub fallback: bool,
}

/// How `rename` makes a name (§1.6.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameMask {
    /// One new name: needs one row.
    Name(String),
    /// A DOS mask.
    Dos(String),
    /// A regular expression over the whole name, and what replaces it.
    Regex {
        /// The expression.
        old: String,
        /// The replacement.
        new: String,
    },
}

/// A folder operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderOp {
    /// Copy to the side `to`.
    Copy {
        /// The side written.
        to: Side,
        /// What an existing target means: `Always` names the replacement.
        overwrite: OverwritePolicy,
    },
    /// Move to the side `to`.
    Move {
        /// The side written.
        to: Side,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
    /// Swap both sides.
    Exchange,
    /// Delete on `sides`.
    Delete {
        /// The sides.
        sides: SideSet,
        /// The bin.
        recycle: Recycle,
    },
    /// Rename on `sides`.
    Rename {
        /// The sides.
        sides: SideSet,
        /// The new names.
        mask: RenameMask,
    },
    /// Set times on `sides`.
    Touch {
        /// The sides.
        sides: SideSet,
        /// The time.
        from: TouchFrom,
    },
    /// Make a folder (or an archive) below `parent`.
    Mkdir {
        /// The sides.
        sides: SideSet,
        /// The row it goes into.
        parent: NodeId,
        /// The name.
        name: String,
        /// A trailing separator was given: a folder, whatever the name looks like.
        folder: bool,
    },
    /// Copy to a target outside the comparison.
    CopyTo {
        /// The sides copied from.
        sides: SideSet,
        /// The target.
        target: Spec,
        /// A trailing separator was given: a folder, whatever the name looks like.
        folder: bool,
        /// The layout.
        structure: PathStructure,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
    /// Move to a target outside the comparison.
    MoveTo {
        /// The sides moved from.
        sides: SideSet,
        /// The target.
        target: Spec,
        /// A trailing separator was given: a folder, whatever the name looks like.
        folder: bool,
        /// The layout.
        structure: PathStructure,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
}

/// The sides that may not be written (FC-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadOnlySides {
    /// The left side.
    pub left: bool,
    /// The right side.
    pub right: bool,
}

impl ReadOnlySides {
    /// True when `side` may not be written.
    pub const fn get(&self, side: Side) -> bool {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
            Side::Center | Side::Output => false,
        }
    }
}

/// What an operation works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpScope {
    /// The selection.
    pub selection: Selection,
    /// What a selected folder contributes.
    pub include: Include,
    /// *Ignore unimportant differences*.
    pub iu: bool,
    /// The read-only sides; the Service refuses a write to one before the builder runs.
    pub read_only: ReadOnlySides,
}

/// The settings of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpSettings {
    /// Metadata of copies.
    pub preserve: Preserve,
    /// How links are copied.
    pub links: LinkPolicy,
    /// Where replaced targets go first; `None`: no backups.
    pub backup: Option<BackupPolicy>,
}

/// The verb of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpVerb {
    /// `copy`.
    Copy,
    /// `move`.
    Move,
    /// `exchange`.
    Exchange,
    /// `delete`.
    Delete,
    /// `rename`.
    Rename,
    /// `touch`.
    Touch,
    /// `mkdir`.
    Mkdir,
    /// `copyto`.
    CopyTo,
    /// `moveto`.
    MoveTo,
    /// A sync.
    Sync,
}

impl OpVerb {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            OpVerb::Copy => "copy",
            OpVerb::Move => "move",
            OpVerb::Exchange => "exchange",
            OpVerb::Delete => "delete",
            OpVerb::Rename => "rename",
            OpVerb::Touch => "touch",
            OpVerb::Mkdir => "mkdir",
            OpVerb::CopyTo => "copy_to",
            OpVerb::MoveTo => "move_to",
            OpVerb::Sync => "sync",
        }
    }
}

/// What the plan knows of an item beyond what `hikasa_vfs` needs: same length and order as `Plan::items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemMeta {
    /// The side of the target; `None` for a target outside the bases.
    pub side: Option<Side>,
    /// The row (`None`: the plan made this item from the disk, rule 6).
    pub node: Option<NodeId>,
    /// Its path below the bases.
    pub path: String,
    /// What it needs asked.
    pub conflicts: Conflicts,
    /// What was answered to it.
    pub answer: Option<bool>,
}

/// A folder operation, planned: plain data, with no borrow of the model. A dry run prints this value, and execution takes
/// this value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpPlan {
    /// The verb.
    pub verb: OpVerb,
    /// The bases.
    pub bases: Pair<Spec>,
    /// The plan, of kind `Batch`.
    pub plan: Plan,
    /// One per item of the plan.
    pub meta: Vec<ItemMeta>,
    /// The fence and the archives' backups.
    pub exec: ExecOptions,
    /// What planning found.
    pub warnings: Vec<Warning>,
    /// The folders a refresh takes in once the plan ran (additive: the ids of the model it was made from).
    pub refresh: Vec<NodeId>,
}

/// The folders a refresh takes in after a plan ran: the parent of each row an item names, each once, parents first.
pub fn refresh_nodes(read: &TreeRead<'_>, meta: &[ItemMeta]) -> Vec<NodeId> {
    let mut seen = BTreeSet::new();
    for node in meta.iter().filter_map(|meta| meta.node) {
        seen.insert(
            read.node(node)
                .and_then(|view| view.parent)
                .unwrap_or(NodeId::ROOT),
        );
    }
    let mut folders: Vec<(usize, NodeId)> = seen
        .into_iter()
        .map(|folder| (read.ancestors(folder).len(), folder))
        .collect();
    folders.sort();
    folders.into_iter().map(|(_, folder)| folder).collect()
}

/// Plans `op` on the selection of `scope`, from the model: **nothing is written**. Directory I/O only where the model has
/// no contents (rule 6), checks of what a plan replaces, and the target of `copyto`/`moveto`, which is no row. A row that
/// cannot be planned is an item with an error or a skip, so a plan shows it; `Err` is for what makes a whole plan
/// senseless (nothing selected, a bad mask, a stop).
pub fn plan_op(
    read: &TreeRead<'_>,
    vfs: &dyn Vfs,
    scope: &OpScope,
    op: &FolderOp,
    settings: &OpSettings,
    ctl: &Ctl<'_>,
) -> Result<OpPlan, FolderError> {
    build::plan(read, vfs, scope, op, settings, ctl)
}
