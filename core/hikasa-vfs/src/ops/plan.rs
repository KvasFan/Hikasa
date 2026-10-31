//! The plan of a VFS write operation and its per-item results (T12, D-103; contract M2 §1.3.4–§1.3.5).

use hikasa_core::{ErrorKind, ErrorReport, Spec, Warning};
use serde::{Deserialize, Serialize};

use crate::entry::{EntryKind, FileId};
use crate::error::VfsError;
use crate::vfs::SetTimes;

/// What happens when a copy or move meets an existing target (D-96).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverwritePolicy {
    /// The item fails `already_exists` (default).
    #[default]
    Fail,
    /// The item is skipped (`exists`).
    Skip,
    /// The target is replaced.
    Always,
    /// The target is replaced when the source `mtime` is strictly newer, else skipped (`not_newer`; an unknown
    /// `mtime` on either side counts as not newer).
    Newer,
}

impl OverwritePolicy {
    /// Every policy, in declaration order.
    pub const ALL: [OverwritePolicy; 4] = [
        OverwritePolicy::Fail,
        OverwritePolicy::Skip,
        OverwritePolicy::Always,
        OverwritePolicy::Newer,
    ];

    /// The serde name (`"fail"`, `"skip"`, `"always"`, `"newer"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            OverwritePolicy::Fail => "fail",
            OverwritePolicy::Skip => "skip",
            OverwritePolicy::Always => "always",
            OverwritePolicy::Newer => "newer",
        }
    }

    /// The policy named `word` (`None` for an unknown word; exact, lower case).
    pub fn parse(word: &str) -> Option<OverwritePolicy> {
        OverwritePolicy::ALL
            .into_iter()
            .find(|p| p.as_str() == word)
    }
}

/// How symbolic links met inside a copied tree (and root links without `follow_roots`) are copied (D-94).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkPolicy {
    /// A new link with the same target text (default).
    #[default]
    Copy,
    /// The link is skipped (reason `symlink`).
    Skip,
}

impl LinkPolicy {
    /// Every policy, in declaration order.
    pub const ALL: [LinkPolicy; 2] = [LinkPolicy::Copy, LinkPolicy::Skip];

    /// The serde name (`"copy"`, `"skip"`).
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkPolicy::Copy => "copy",
            LinkPolicy::Skip => "skip",
        }
    }

    /// The policy named `word` (`None` for an unknown word).
    pub fn parse(word: &str) -> Option<LinkPolicy> {
        LinkPolicy::ALL.into_iter().find(|p| p.as_str() == word)
    }
}

/// Which metadata a copy applies after the content (D-95). Owner and group are never copied (D-99).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preserve {
    /// The modification time.
    pub times: bool,
    /// Unix mode or the DOS `R` bit (within one family; nothing is mapped between families).
    pub attrs: bool,
    /// The creation time (where the target has `SET_CREATED`).
    pub created: bool,
}

impl Default for Preserve {
    /// Times and attributes, not the creation time.
    fn default() -> Preserve {
        Preserve {
            times: true,
            attrs: true,
            created: false,
        }
    }
}

impl Preserve {
    /// Nothing preserved (`--no-preserve`).
    pub const NONE: Preserve = Preserve {
        times: false,
        attrs: false,
        created: false,
    };
}

/// Options of a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CopyOptions {
    /// Copy folders with their contents (without it a folder source is an item error `is_a_directory`).
    pub recursive: bool,
    /// What an existing target means.
    pub overwrite: OverwritePolicy,
    /// How links are copied.
    pub links: LinkPolicy,
    /// Metadata applied after the content.
    pub preserve: Preserve,
    /// A root argument that is a link is replaced by what it points to (D-94 rule 7; `--follow-symlinks`).
    pub follow_roots: bool,
    /// Missing parent folders of the root targets are created.
    pub create_parents: bool,
}

/// Options of a move (D-97).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MoveOptions {
    /// What an existing target means.
    pub overwrite: OverwritePolicy,
    /// Missing parent folders of the root targets are created.
    pub create_parents: bool,
}

/// Options of a remove (D-98).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoveOptions {
    /// Non-empty folders are removed with their contents (post-order).
    pub recursive: bool,
    /// Each root goes to the recycle bin as one item (never a permanent delete).
    pub trash: bool,
}

/// Options of a touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TouchOptions {
    /// The times to set.
    pub times: SetTimes,
    /// A missing item is created as an empty file (else it is an item error `not_found`).
    pub create: bool,
}

/// The operation of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    /// `cp`.
    Copy,
    /// `mv`.
    Move,
    /// `rm`.
    Remove,
    /// `mkdir`.
    Mkdir,
    /// `touch`.
    Touch,
}

impl OpKind {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            OpKind::Copy => "copy",
            OpKind::Move => "move",
            OpKind::Remove => "remove",
            OpKind::Mkdir => "mkdir",
            OpKind::Touch => "touch",
        }
    }
}

/// What one plan item does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Create the target folder.
    CreateDir,
    /// The target folder exists and is kept (merge, or `mkdir -p` of an existing folder).
    KeepDir,
    /// Copy a file.
    Copy,
    /// Create a link with the source link's target text.
    Link,
    /// One `rename` of the whole root (same back-end, D-97).
    Rename,
    /// Copy a file or link, then remove the source.
    Move,
    /// Remove a file, link or special file.
    Remove,
    /// Remove a folder (after its contents).
    RemoveDir,
    /// Move the whole root to the recycle bin.
    Trash,
    /// Set the times of an existing item.
    Touch,
    /// Create an empty file, then set its times.
    Create,
    /// Nothing is done (see [`PlanItem::skip`]).
    Skip,
}

impl Action {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Action::CreateDir => "create_dir",
            Action::KeepDir => "keep_dir",
            Action::Copy => "copy",
            Action::Link => "link",
            Action::Rename => "rename",
            Action::Move => "move",
            Action::Remove => "remove",
            Action::RemoveDir => "remove_dir",
            Action::Trash => "trash",
            Action::Touch => "touch",
            Action::Create => "create",
            Action::Skip => "skip",
        }
    }

    /// True for the actions planned without a target (D-103: execution fails them when one appeared).
    pub const fn needs_absent_target(self) -> bool {
        matches!(
            self,
            Action::CreateDir
                | Action::Copy
                | Action::Link
                | Action::Rename
                | Action::Move
                | Action::Create
        )
    }
}

/// Why an item is skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The target exists (`--overwrite skip`).
    Exists,
    /// The source is not strictly newer (`--overwrite newer`).
    NotNewer,
    /// A FIFO, socket or device (never opened).
    SpecialFile,
    /// A link with `LinkPolicy::Skip`.
    Symlink,
    /// A link and a target back-end without `CREATE_SYMLINKS`.
    SymlinksUnsupported,
    /// A folder whose child failed or was skipped is not removed.
    ChildrenFailed,
}

impl SkipReason {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SkipReason::Exists => "exists",
            SkipReason::NotNewer => "not_newer",
            SkipReason::SpecialFile => "special_file",
            SkipReason::Symlink => "symlink",
            SkipReason::SymlinksUnsupported => "symlinks_unsupported",
            SkipReason::ChildrenFailed => "children_failed",
        }
    }
}

/// One item of a plan, in execution order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    /// What is done.
    pub action: Action,
    /// The kind of the item (the source's for copies and moves, else the target's; `other` when unknown).
    pub kind: EntryKind,
    /// The source (copies and moves).
    pub source: Option<Spec>,
    /// The item written, created, removed or touched.
    pub target: Spec,
    /// Bytes (files).
    pub size: Option<u64>,
    /// The target exists and is replaced.
    pub replaces: bool,
    /// Why the item is skipped (`action == Skip`).
    pub skip: Option<SkipReason>,
    /// Planning failed for this item: it fails at execution without a call.
    pub error: Option<ErrorReport>,
    /// Level below its root (0 = the root).
    pub depth: u32,
    /// The identity of a folder item seen at planning (the source folder of `create_dir`/`keep_dir`, the folder
    /// of `remove_dir`): execution re-checks it before the folder and its contents are touched (D-103). Not
    /// serialized (a plan read back executes without this check).
    #[serde(skip)]
    pub(crate) seen: Option<FileId>,
}

impl PlanItem {
    /// An item.
    pub(crate) fn new(
        action: Action,
        kind: EntryKind,
        source: Option<Spec>,
        target: Spec,
        depth: u32,
    ) -> PlanItem {
        PlanItem {
            action,
            kind,
            source,
            target,
            size: None,
            replaces: false,
            skip: None,
            error: None,
            depth,
            seen: None,
        }
    }

    /// A skipped item.
    pub(crate) fn skipped(
        kind: EntryKind,
        source: Option<Spec>,
        target: Spec,
        depth: u32,
        reason: SkipReason,
    ) -> PlanItem {
        PlanItem {
            skip: Some(reason),
            ..PlanItem::new(Action::Skip, kind, source, target, depth)
        }
    }

    /// An item whose planning failed.
    pub(crate) fn failed(
        action: Action,
        kind: EntryKind,
        source: Option<Spec>,
        target: Spec,
        depth: u32,
        error: ErrorReport,
    ) -> PlanItem {
        PlanItem {
            error: Some(error),
            ..PlanItem::new(action, kind, source, target, depth)
        }
    }
}

/// The options a plan was made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanOptions {
    /// `cp`.
    Copy(CopyOptions),
    /// `mv`.
    Move(MoveOptions),
    /// `rm`.
    Remove(RemoveOptions),
    /// `mkdir`.
    Mkdir {
        /// `--parents`.
        parents: bool,
    },
    /// `touch`.
    Touch(TouchOptions),
}

/// A complete, serializable plan: every item of every tree, in execution order (T12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The operation.
    pub op: OpKind,
    /// Its options.
    pub options: PlanOptions,
    /// The items.
    pub items: Vec<PlanItem>,
    /// Warnings found while planning (`capability_missing`).
    pub warnings: Vec<Warning>,
}

impl Plan {
    /// The bytes the plan copies (sizes of `copy` and `move` items without an error).
    pub fn bytes(&self) -> u64 {
        self.items
            .iter()
            .filter(|i| matches!(i.action, Action::Copy | Action::Move) && i.error.is_none())
            .filter_map(|i| i.size)
            .sum()
    }
}

/// The status of one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    /// Dry run: would be done.
    Planned,
    /// Done.
    Done,
    /// Skipped (see the reason).
    Skipped,
    /// Failed (see the error).
    Failed,
    /// Not run: the operation was cancelled, or the folder item above it failed, was skipped or changed since
    /// the plan was made (nothing below such a folder is touched).
    NotRun,
}

impl ItemStatus {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Planned => "planned",
            ItemStatus::Done => "done",
            ItemStatus::Skipped => "skipped",
            ItemStatus::Failed => "failed",
            ItemStatus::NotRun => "not_run",
        }
    }
}

/// The result of one plan item (same index as the item).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemResult {
    /// The status.
    pub status: ItemStatus,
    /// The skip reason.
    pub skip: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
}

impl ItemResult {
    /// A result with a status only.
    pub(crate) fn status(status: ItemStatus) -> ItemResult {
        ItemResult {
            status,
            skip: None,
            error: None,
        }
    }

    /// A failed result.
    pub(crate) fn failed(error: ErrorReport) -> ItemResult {
        ItemResult {
            status: ItemStatus::Failed,
            skip: None,
            error: Some(error),
        }
    }

    /// A skipped result.
    pub(crate) fn skipped(reason: SkipReason) -> ItemResult {
        ItemResult {
            status: ItemStatus::Skipped,
            skip: Some(reason),
            error: None,
        }
    }
}

/// Counts of a report (ARCH-101).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Summary {
    /// Items in the plan.
    pub planned: u64,
    /// Items done.
    pub done: u64,
    /// Items skipped.
    pub skipped: u64,
    /// Items failed.
    pub failed: u64,
    /// Bytes copied by done items.
    pub bytes: u64,
}

impl Summary {
    /// Counts `results`; `bytes` as given.
    pub(crate) fn count(results: &[ItemResult], bytes: u64) -> Summary {
        let count =
            |status: ItemStatus| results.iter().filter(|r| r.status == status).count() as u64;
        Summary {
            planned: results.len() as u64,
            done: count(ItemStatus::Done),
            skipped: count(ItemStatus::Skipped),
            failed: count(ItemStatus::Failed),
            bytes,
        }
    }
}

/// A plan with its per-item results: the outcome of [`crate::ops::execute`] or [`crate::ops::dry_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpReport {
    /// The plan as executed (a `rename` expanded at execution shows its expansion).
    pub plan: Plan,
    /// One result per plan item.
    pub results: Vec<ItemResult>,
    /// The counts.
    pub summary: Summary,
    /// The operation was cancelled (later items are `not_run`).
    pub cancelled: bool,
    /// Warnings of the execution (`capability_missing`); the plan's own are in `plan.warnings`.
    pub warnings: Vec<Warning>,
}

impl OpReport {
    /// No item failed and the operation was not cancelled.
    pub fn ok(&self) -> bool {
        self.summary.failed == 0 && !self.cancelled
    }
}

/// Every result `Planned` (or `Failed` for items with an error); nothing is written.
pub fn dry_run(plan: Plan) -> OpReport {
    let results: Vec<ItemResult> = plan
        .items
        .iter()
        .map(|item| match &item.error {
            Some(error) => ItemResult::failed(error.clone()),
            None => ItemResult {
                status: ItemStatus::Planned,
                skip: item.skip,
                error: None,
            },
        })
        .collect();
    let summary = Summary::count(&results, 0);
    OpReport {
        plan,
        results,
        summary,
        cancelled: false,
        warnings: Vec::new(),
    }
}

/// The report of a VFS error: kind, message, the spec it names.
pub(crate) fn report(error: &VfsError) -> ErrorReport {
    let mut report = ErrorReport::capture(error, None);
    report.spec = error.spec().map(str::to_owned);
    report
}

/// A report with a kind and a message of its own (a hint such as "use --recursive").
pub(crate) fn report_with(kind: ErrorKind, spec: &Spec, message: String) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: Some(spec.canonical()),
        causes: Vec::new(),
    }
}
