//! The plan of a VFS write operation and its per-item results (T12, D-103; contract M2 §1.3.4–§1.3.5).

use std::collections::BTreeMap;

use hikasa_core::{ErrorKind, ErrorReport, FileTime, Spec, Warning};
use serde::{Deserialize, Serialize};

use crate::entry::{EntryKind, FileId};
use crate::error::VfsError;
use crate::vfs::SetTimes;

use super::guard::Fence;

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

/// Options of a batch: one plan of many actions, executed once (D-162).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BatchOptions {
    /// Metadata of copies; moves keep [`Preserve::default`], as in W3.
    pub preserve: Preserve,
    /// How links are copied.
    pub links: LinkPolicy,
    /// `move`/`moveto`: the folder items of the plan are sources of a move (two-phase, D-148).
    pub moves: bool,
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
    /// A folder operation or a sync: any mix of actions (D-162).
    Batch,
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
            OpKind::Batch => "batch",
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
    /// Create an empty archive (M2 W5b, D-150).
    CreateArchive,
    /// Swap two files or links (FC-071, D-165): `source` is the left one, `target` the right one.
    Exchange,
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
            Action::CreateArchive => "create_archive",
            Action::Exchange => "exchange",
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
                | Action::CreateArchive
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
    /// Answered no.
    Declined,
    /// The row is hidden by a filter.
    Filtered,
    /// The row failed in the comparison: a failure is not a difference (D-19).
    CompareError,
    /// The row lies below a folder the comparison followed (FC-013): the path leads outside the base.
    FollowedLink,
    /// The side is read-only.
    ReadOnlySide,
    /// The back-end cannot do this here.
    Capability,
    /// Another item of the plan has the same target.
    DuplicateTarget,
    /// The name does not match the mask.
    NoMatch,
    /// The new name is the old one.
    SameName,
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
            SkipReason::Declined => "declined",
            SkipReason::Filtered => "filtered",
            SkipReason::CompareError => "compare_error",
            SkipReason::FollowedLink => "followed_link",
            SkipReason::ReadOnlySide => "read_only_side",
            SkipReason::Capability => "capability",
            SkipReason::DuplicateTarget => "duplicate_target",
            SkipReason::NoMatch => "no_match",
            SkipReason::SameName => "same_name",
        }
    }
}

/// What a plan saw of an item: execution compares it, so a plan that waited on a confirmation stays honest (D-163).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStamp {
    /// The kind.
    pub kind: EntryKind,
    /// The size (`None`: not compared; files and links only).
    pub size: Option<u64>,
    /// The modification time (`None`: not compared; files and links only).
    pub mtime: Option<FileTime>,
}

/// The stamps of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Expect {
    /// The source as the plan saw it.
    pub source: Option<ItemStamp>,
    /// The target as the plan saw it; `None` asks nothing here, and the absent-target check of D-103 stays.
    pub target: Option<ItemStamp>,
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
    /// What the plan saw (D-163).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<Expect>,
    /// Where the replaced target goes first (§1.6.5). `None` for members: their archive is backed up whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<Spec>,
    /// The times of a `touch` item of a batch; other plans use [`TouchOptions`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<SetTimes>,
    /// The source is a link the comparison followed (FC-013): stat and read through it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follow_source: bool,
    /// Answered yes to `read_only`: a DOS `R` on the target is cleared just before it is replaced or removed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_read_only: bool,
    /// A `trash` item that may become a permanent delete when the recycle bin fails (§1.6.12).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub trash_fallback: bool,
}

impl PlanItem {
    /// An item with no stamps, no backup and no flags: what a planner starts from.
    pub fn planned(
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
            expect: None,
            backup: None,
            times: None,
            follow_source: false,
            clear_read_only: false,
            trash_fallback: false,
        }
    }

    /// The bytes this item copies: an `exchange` copies both sides, and its target's size is in its stamp (§1.6.2).
    pub(crate) fn bytes(&self) -> u64 {
        if self.error.is_some() {
            return 0;
        }
        match self.action {
            Action::Copy | Action::Move => self.size.unwrap_or(0),
            Action::Exchange => {
                let target = self.expect.and_then(|expect| expect.target);
                self.size.unwrap_or(0) + target.and_then(|stamp| stamp.size).unwrap_or(0)
            }
            _ => 0,
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
            ..PlanItem::planned(Action::Skip, kind, source, target, depth)
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
            ..PlanItem::planned(action, kind, source, target, depth)
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
    /// A folder operation or a sync.
    Batch(BatchOptions),
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
    /// The bytes the plan copies (sizes of `copy` and `move` items without an error, both sizes of an `exchange`).
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(PlanItem::bytes).sum()
    }
}

/// What [`crate::ops::execute_with`] adds to [`crate::ops::execute`]. `Default` is `execute`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOptions {
    /// Refuses what lies outside the base folders (§1.6.4).
    pub fence: Option<Fence>,
    /// Canonical outermost archive file → its backup (§1.6.5). Filled by the planner, so a dry run shows them.
    pub archive_backups: BTreeMap<String, Spec>,
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
    /// What took the place of an item at execution, in order: its index in this report and how many items stand there
    /// (a `rename` that met `CrossDevice`, a `trash` that fell back). A planner's parallel lists follow by it.
    pub expanded: Vec<(usize, usize)>,
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
        expanded: Vec::new(),
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
