//! The sync engine (FC-078, FS-001, FS-002, FS-004, FS-005; D-181, D-182): a matrix says, per situation of a row, what a
//! sync does; five presets fill it, `--custom` writes it, overrides change it per path. [`plan_sync`] turns a comparison
//! into rows with their decisions and one batch plan made by the builder of the folder operations (§1.6.10), so a sync
//! inherits every check a copy and a delete make: stamps, conflicts, the fence, backups, Disable editing.
//!
//! Safety comes first, because a mirror deletes by design: a row that failed or was not scanned is never written or
//! deleted, and below it nothing is an orphan; a side hidden by a filter is left; a comparison that did not finish
//! deletes nothing; a source with nothing on it deletes nothing unless the caller says it may (`EmptySource`).

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use hikasa_core::{Ctl, RelPath, Warning};
use hikasa_vfs::Vfs;
use serde::{Deserialize, Serialize};

use crate::display::DisplayOptions;
use crate::error::FolderError;
use crate::model::{NodeId, TreeRead};
use crate::ops::{OpPlan, OpSettings, ReadOnlySides};
use crate::rules::{ArchiveHandling, FolderRules};
use crate::select::Selection;

/// A word in kebab or snake case, as the serde names are: `copy-right` and `copy_right` alike.
fn normalized(word: &str) -> String {
    word.trim().to_ascii_lowercase().replace('-', "_")
}

/// A sync preset (FS-002). The default is Update Right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPreset {
    /// Update Left: what is newer or new on the right goes left.
    UpdateLeft,
    /// Update Right: what is newer or new on the left goes right.
    #[default]
    UpdateRight,
    /// Update Both.
    UpdateBoth,
    /// Mirror to Left: the left becomes the right.
    MirrorLeft,
    /// Mirror to Right: the right becomes the left.
    MirrorRight,
    /// The matrix of [`SyncOptions::matrix`].
    Custom,
}

impl SyncPreset {
    /// Every preset, in declaration order.
    pub const ALL: [SyncPreset; 6] = [
        SyncPreset::UpdateLeft,
        SyncPreset::UpdateRight,
        SyncPreset::UpdateBoth,
        SyncPreset::MirrorLeft,
        SyncPreset::MirrorRight,
        SyncPreset::Custom,
    ];

    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SyncPreset::UpdateLeft => "update_left",
            SyncPreset::UpdateRight => "update_right",
            SyncPreset::UpdateBoth => "update_both",
            SyncPreset::MirrorLeft => "mirror_left",
            SyncPreset::MirrorRight => "mirror_right",
            SyncPreset::Custom => "custom",
        }
    }

    /// A preset by its name, kebab or snake case.
    pub fn parse(word: &str) -> Option<SyncPreset> {
        let word = normalized(word);
        SyncPreset::ALL.into_iter().find(|p| p.as_str() == word)
    }
}

/// What a sync does with a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    /// Nothing.
    #[default]
    Leave,
    /// The left item goes right.
    CopyRight,
    /// The right item goes left.
    CopyLeft,
    /// The left item is deleted.
    DeleteLeft,
    /// The right item is deleted.
    DeleteRight,
}

impl SyncAction {
    /// Every action, in declaration order.
    pub const ALL: [SyncAction; 5] = [
        SyncAction::Leave,
        SyncAction::CopyRight,
        SyncAction::CopyLeft,
        SyncAction::DeleteLeft,
        SyncAction::DeleteRight,
    ];

    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SyncAction::Leave => "leave",
            SyncAction::CopyRight => "copy_right",
            SyncAction::CopyLeft => "copy_left",
            SyncAction::DeleteLeft => "delete_left",
            SyncAction::DeleteRight => "delete_right",
        }
    }

    /// An action by its name, kebab or snake case.
    pub fn parse(word: &str) -> Option<SyncAction> {
        let word = normalized(word);
        SyncAction::ALL.into_iter().find(|a| a.as_str() == word)
    }

    /// The side read (`None` for `leave` and the deletes).
    pub const fn reads(self) -> Option<hikasa_core::Side> {
        match self {
            SyncAction::CopyRight => Some(hikasa_core::Side::Left),
            SyncAction::CopyLeft => Some(hikasa_core::Side::Right),
            SyncAction::Leave | SyncAction::DeleteLeft | SyncAction::DeleteRight => None,
        }
    }

    /// The side written or deleted on (`None` for `leave`).
    pub const fn writes(self) -> Option<hikasa_core::Side> {
        match self {
            SyncAction::CopyRight | SyncAction::DeleteRight => Some(hikasa_core::Side::Right),
            SyncAction::CopyLeft | SyncAction::DeleteLeft => Some(hikasa_core::Side::Left),
            SyncAction::Leave => None,
        }
    }

    /// True for the two deletes.
    pub const fn deletes(self) -> bool {
        matches!(self, SyncAction::DeleteLeft | SyncAction::DeleteRight)
    }
}

impl fmt::Display for SyncAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The situation of a row (FS-002's eight rows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Situation {
    /// Both sides, the left newer.
    NewerLeft,
    /// Both sides, the right newer.
    NewerRight,
    /// Left only, read as new there.
    AddedLeft,
    /// Right only, read as new there.
    AddedRight,
    /// Right only, read as deleted on the left.
    DeletedLeft,
    /// Left only, read as deleted on the right.
    DeletedRight,
    /// Both sides, different, no time difference.
    Different,
    /// Both sides, the same.
    Same,
}

impl Situation {
    /// Every situation, in declaration order.
    pub const ALL: [Situation; 8] = [
        Situation::NewerLeft,
        Situation::NewerRight,
        Situation::AddedLeft,
        Situation::AddedRight,
        Situation::DeletedLeft,
        Situation::DeletedRight,
        Situation::Different,
        Situation::Same,
    ];

    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Situation::NewerLeft => "newer_left",
            Situation::NewerRight => "newer_right",
            Situation::AddedLeft => "added_left",
            Situation::AddedRight => "added_right",
            Situation::DeletedLeft => "deleted_left",
            Situation::DeletedRight => "deleted_right",
            Situation::Different => "different",
            Situation::Same => "same",
        }
    }

    /// A situation by its name, kebab or snake case.
    pub fn parse(word: &str) -> Option<Situation> {
        let word = normalized(word);
        Situation::ALL.into_iter().find(|s| s.as_str() == word)
    }

    /// The one side the item is on: the left for `added_left`, `deleted_right`; the right for `added_right`,
    /// `deleted_left`; `None` for the situations of both sides.
    pub const fn only_on(self) -> Option<hikasa_core::Side> {
        match self {
            Situation::AddedLeft | Situation::DeletedRight => Some(hikasa_core::Side::Left),
            Situation::AddedRight | Situation::DeletedLeft => Some(hikasa_core::Side::Right),
            Situation::NewerLeft
            | Situation::NewerRight
            | Situation::Different
            | Situation::Same => None,
        }
    }

    /// True when `action` names a side the item is on (§1.6.14 "valid cells").
    pub fn allows(self, action: SyncAction) -> bool {
        match self.only_on() {
            None => true,
            Some(side) => {
                action == SyncAction::Leave
                    || action.reads() == Some(side)
                    || (action.deletes() && action.writes() == Some(side))
            }
        }
    }
}

impl fmt::Display for Situation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The sync matrix (FS-002): an action per situation. `Default` is `leave` everywhere, the base `[rules.sync.matrix]`
/// is sparse against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncMatrix {
    /// Both sides, the left newer.
    pub newer_left: SyncAction,
    /// Both sides, the right newer.
    pub newer_right: SyncAction,
    /// Left only, new.
    pub added_left: SyncAction,
    /// Right only, new.
    pub added_right: SyncAction,
    /// Right only, deleted on the left.
    pub deleted_left: SyncAction,
    /// Left only, deleted on the right.
    pub deleted_right: SyncAction,
    /// Both sides, different, no time difference.
    pub different: SyncAction,
    /// Both sides, the same.
    pub same: SyncAction,
}

impl SyncMatrix {
    /// The matrix of a preset (§1.6.14's table, normative); `None` for `Custom`.
    pub fn of(preset: SyncPreset) -> Option<SyncMatrix> {
        use SyncAction::{
            CopyLeft as L, CopyRight as R, DeleteLeft as DL, DeleteRight as DR, Leave as N,
        };
        let cells = |c: [SyncAction; 8]| SyncMatrix {
            newer_left: c[0],
            newer_right: c[1],
            added_left: c[2],
            added_right: c[3],
            deleted_left: c[4],
            deleted_right: c[5],
            different: c[6],
            same: c[7],
        };
        Some(match preset {
            SyncPreset::UpdateRight => cells([R, N, R, N, N, N, N, N]),
            SyncPreset::UpdateLeft => cells([N, L, N, L, N, N, N, N]),
            SyncPreset::UpdateBoth => cells([R, L, R, L, N, N, N, N]),
            SyncPreset::MirrorRight => cells([R, R, R, DR, DR, R, R, N]),
            SyncPreset::MirrorLeft => cells([L, L, DL, L, L, DL, L, N]),
            SyncPreset::Custom => return None,
        })
    }

    /// The action of `situation`.
    pub fn get(&self, situation: Situation) -> SyncAction {
        match situation {
            Situation::NewerLeft => self.newer_left,
            Situation::NewerRight => self.newer_right,
            Situation::AddedLeft => self.added_left,
            Situation::AddedRight => self.added_right,
            Situation::DeletedLeft => self.deleted_left,
            Situation::DeletedRight => self.deleted_right,
            Situation::Different => self.different,
            Situation::Same => self.same,
        }
    }

    /// Sets the action of `situation`.
    pub fn set(&mut self, situation: Situation, action: SyncAction) {
        let cell = match situation {
            Situation::NewerLeft => &mut self.newer_left,
            Situation::NewerRight => &mut self.newer_right,
            Situation::AddedLeft => &mut self.added_left,
            Situation::AddedRight => &mut self.added_right,
            Situation::DeletedLeft => &mut self.deleted_left,
            Situation::DeletedRight => &mut self.deleted_right,
            Situation::Different => &mut self.different,
            Situation::Same => &mut self.same,
        };
        *cell = action;
    }

    /// `invalid_input` for a cell that names a side its item is not on: the left-only situations take `copy_right`,
    /// `delete_left`, `leave`; the right-only ones `copy_left`, `delete_right`, `leave`; the others all five.
    pub fn validate(&self) -> Result<(), FolderError> {
        for situation in Situation::ALL {
            let action = self.get(situation);
            if !situation.allows(action) {
                return Err(FolderError::InvalidRules(format!(
                    "sync matrix: `{situation}={action}` names a side a {situation} item is not on"
                )));
            }
        }
        Ok(())
    }

    /// `--custom <matrix>`: `<situation>=<action>` items separated by commas, kebab or snake case, on `base`; a preset
    /// name may come first as the base (`mirror-right,same=leave`). An unknown word, a repeated situation, an empty item
    /// and an invalid cell are `invalid_input`.
    pub fn parse(text: &str, base: SyncMatrix) -> Result<SyncMatrix, FolderError> {
        let bad = |message: String| FolderError::InvalidRules(format!("--custom: {message}"));
        let mut matrix = base;
        let mut seen = Vec::new();
        for (index, item) in text.split(',').enumerate() {
            let item = item.trim();
            if item.is_empty() {
                return Err(bad(format!("an empty item in {text:?}")));
            }
            let Some((situation, action)) = item.split_once('=') else {
                let preset = SyncPreset::parse(item)
                    .and_then(|preset| SyncMatrix::of(preset).map(|m| (preset, m)));
                match (index, preset) {
                    (0, Some((_, preset))) => matrix = preset,
                    (0, None) => {
                        return Err(bad(format!(
                            "unknown word {item:?} (a preset: update-left, update-right, update-both, mirror-left, \
                             mirror-right; or <situation>=<action>)"
                        )));
                    }
                    _ => {
                        return Err(bad(format!(
                            "{item:?}: a preset may only come first, and items are <situation>=<action>"
                        )));
                    }
                }
                continue;
            };
            let situation = Situation::parse(situation).ok_or_else(|| {
                bad(format!(
                    "unknown situation {:?} (newer-left, newer-right, added-left, added-right, deleted-left, \
                     deleted-right, different, same)",
                    situation.trim()
                ))
            })?;
            let action = SyncAction::parse(action).ok_or_else(|| {
                bad(format!(
                    "unknown action {:?} (leave, copy-right, copy-left, delete-left, delete-right)",
                    action.trim()
                ))
            })?;
            if seen.contains(&situation) {
                return Err(bad(format!("{situation} is given twice")));
            }
            seen.push(situation);
            matrix.set(situation, action);
        }
        matrix.validate()?;
        Ok(matrix)
    }
}

/// How an item on one side only is read (D-181): without Sync Memory (FS-003) it can be new there or gone from the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanReading {
    /// A left-only item is `added_left`, a right-only one `added_right`.
    #[default]
    Added,
    /// A left-only item is `deleted_right`, a right-only one `deleted_left`.
    Deleted,
}

impl OrphanReading {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            OrphanReading::Added => "added",
            OrphanReading::Deleted => "deleted",
        }
    }

    /// `added`, `deleted`.
    pub fn parse(word: &str) -> Option<OrphanReading> {
        match normalized(word).as_str() {
            "added" => Some(OrphanReading::Added),
            "deleted" => Some(OrphanReading::Deleted),
            _ => None,
        }
    }
}

/// The options of a sync (`[rules.sync]` of a folder-sync session, §1.6.15).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncOptions {
    /// The preset.
    pub preset: SyncPreset,
    /// The matrix of `custom`; other presets have their own.
    pub matrix: SyncMatrix,
    /// How an item on one side only is read.
    pub orphans: OrphanReading,
    /// *Sync empty folders* (FS-005).
    pub create_empty: bool,
    /// *Sync within archives* (FS-005): archives are folders.
    pub within_archives: bool,
    /// Deletes go to the recycle bin where there is one.
    pub recycle: bool,
    /// A delete the bin refuses becomes permanent.
    pub recycle_fallback: bool,
    /// FS-004: an action per path below the bases, `/`-separated; a folder covers its subtree, the longest path wins.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<String, SyncAction>,
}

impl SyncOptions {
    /// The matrix in effect: the preset's, or [`SyncOptions::matrix`] for `custom`.
    pub fn effective_matrix(&self) -> SyncMatrix {
        SyncMatrix::of(self.preset).unwrap_or(self.matrix)
    }

    /// `invalid_input` for an invalid cell of the matrix in effect, and for an override whose path names no row below the
    /// bases (empty, `.`, `..`, the base itself).
    pub fn validate(&self) -> Result<(), FolderError> {
        self.effective_matrix().validate()?;
        for path in self.overrides.keys() {
            override_path(path)?;
        }
        Ok(())
    }
}

/// The path of an override, checked: below the bases, `/`-separated, no `.` or `..`. A trailing `/` is dropped. The base
/// is no override: an empty path would change the whole tree by one forgotten word.
pub(crate) fn override_path(text: &str) -> Result<RelPath, FolderError> {
    let trimmed = text.trim_end_matches('/');
    match RelPath::parse(trimmed) {
        Ok(path) if !trimmed.is_empty() => Ok(path),
        _ => Err(FolderError::InvalidRules(format!(
            "override {text:?}: a path below the bases, `/`-separated, without `.` or `..`"
        ))),
    }
}

/// The rules of a folder-sync session: `FolderRules` and `[rules.sync]` (§1.6.15). The schema version is that of
/// `FolderRules`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncRules {
    /// The comparison's rules.
    #[serde(flatten)]
    pub folder: FolderRules,
    /// The sync's options.
    pub sync: SyncOptions,
}

/// What a sync plan is asked (§1.6.14).
#[derive(Debug, Clone)]
pub struct SyncRequest<'a> {
    /// The options.
    pub options: &'a SyncOptions,
    /// FC-078 *Just selection*: only these rows (`resolve`).
    pub just_selection: Option<&'a Selection>,
    /// Script `sync visible`: only the rows this display shows.
    pub visible: Option<DisplayOptions>,
    /// A side hidden by a file filter is synced too.
    pub include_hidden: bool,
    /// *Ignore unimportant differences*.
    pub iu: bool,
    /// The sides that may not be written: their rows are left.
    pub read_only: ReadOnlySides,
    /// A sync from an empty side may delete (`--allow-empty-source`).
    pub allow_empty_source: bool,
    /// Metadata, links, backups.
    pub settings: OpSettings,
}

/// Why a row is left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaveReason {
    /// The sides are the same.
    Same,
    /// The matrix, an override or an option says so.
    Matrix,
    /// A side is hidden by a filter.
    Filtered,
    /// The comparison failed here, or did not reach it.
    CompareError,
    /// The side written lies below a link the comparison followed.
    FollowedLink,
    /// The side written is read-only (Disable editing).
    ReadOnlySide,
    /// The side written cannot do it.
    Capability,
    /// Outside *Just selection* or the visible rows.
    OutOfScope,
}

impl LeaveReason {
    /// The serde name.
    pub const fn as_str(self) -> &'static str {
        match self {
            LeaveReason::Same => "same",
            LeaveReason::Matrix => "matrix",
            LeaveReason::Filtered => "filtered",
            LeaveReason::CompareError => "compare_error",
            LeaveReason::FollowedLink => "followed_link",
            LeaveReason::ReadOnlySide => "read_only_side",
            LeaveReason::Capability => "capability",
            LeaveReason::OutOfScope => "out_of_scope",
        }
    }
}

/// One row of a sync plan: 12 bytes (§1.6.22: at most 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncRow {
    /// The row.
    pub node: NodeId,
    /// Its situation; `None` for a pair of folders, and for a row the comparison could not tell.
    pub situation: Option<Situation>,
    /// What the sync does.
    pub action: SyncAction,
    /// Why it is left.
    pub reason: Option<LeaveReason>,
    /// The action is an override's.
    pub overridden: bool,
    /// `different`, left by the matrix: nothing says which side wins (FS-004's conflicts).
    pub conflict: bool,
}

/// What a sync does, counted by row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncCounts {
    /// Rows copied to the right.
    pub copy_right: u64,
    /// Rows copied to the left.
    pub copy_left: u64,
    /// Rows deleted on the left.
    pub delete_left: u64,
    /// Rows deleted on the right.
    pub delete_right: u64,
    /// Rows left (a pair of folders, which has no situation of its own, is not counted).
    pub leave: u64,
    /// Conflicts.
    pub conflicts: u64,
    /// Rows the comparison could not tell (`compare_error`).
    pub unresolved: u64,
    /// Bytes copied to the right.
    pub bytes_right: u64,
    /// Bytes copied to the left.
    pub bytes_left: u64,
}

impl SyncCounts {
    /// Adds `other`.
    pub fn add(&mut self, other: &SyncCounts) {
        self.copy_right += other.copy_right;
        self.copy_left += other.copy_left;
        self.delete_left += other.delete_left;
        self.delete_right += other.delete_right;
        self.leave += other.leave;
        self.conflicts += other.conflicts;
        self.unresolved += other.unresolved;
        self.bytes_right += other.bytes_right;
        self.bytes_left += other.bytes_left;
    }

    /// The counts of one row of `bytes` (its file's size, when it copies).
    pub fn of_row(row: &SyncRow, bytes: u64) -> SyncCounts {
        let mut counts = SyncCounts::default();
        match row.action {
            SyncAction::CopyRight => {
                counts.copy_right = 1;
                counts.bytes_right = bytes;
            }
            SyncAction::CopyLeft => {
                counts.copy_left = 1;
                counts.bytes_left = bytes;
            }
            SyncAction::DeleteLeft => counts.delete_left = 1,
            SyncAction::DeleteRight => counts.delete_right = 1,
            SyncAction::Leave => {
                counts.leave = u64::from(row.situation.is_some() || row.reason.is_some());
            }
        }
        counts.conflicts = u64::from(row.conflict);
        counts.unresolved = u64::from(row.reason == Some(LeaveReason::CompareError));
        counts
    }
}

/// A sync, planned: its rows, the batch plan that runs it, the counts. Plain data: a dry run shows it, execution takes
/// [`SyncPlan::op`].
#[derive(Debug, Clone)]
pub struct SyncPlan {
    /// One per row of the comparison, in tree order, a folder after its children.
    pub rows: Vec<SyncRow>,
    /// The batch: deletes (post-order), copies right, copies left (D-182).
    pub op: OpPlan,
    /// The counts of `rows`.
    pub counts: SyncCounts,
    /// What the rows found: `read_only_side`, `switch_ignored`, `sync_incomplete`.
    pub warnings: Vec<Warning>,
    /// The preset (additive: what the plan was made of, for its report).
    pub preset: SyncPreset,
    /// The matrix in effect (additive).
    pub matrix: SyncMatrix,
    /// How orphans were read (additive).
    pub orphans: OrphanReading,
    /// Each folder's counts, its children's added up.
    pub(crate) folders: HashMap<NodeId, SyncCounts>,
}

impl SyncPlan {
    /// FS-001's folder summaries: the counts of the rows below `node`, O(1). The base's are [`SyncPlan::counts`].
    pub fn folder(&self, node: NodeId) -> SyncCounts {
        self.folders.get(&node).copied().unwrap_or_default()
    }
}

/// The rules a sync compares by (§1.6.14), so a plan does not depend on a view: every folder scanned, orphans too;
/// archives are folders with *Sync within archives*, else files to copy. A flat comparison has no folders to sync:
/// `invalid_input`. The options are checked too, before anything is scanned.
pub fn sync_folder_rules(
    rules: &FolderRules,
    options: &SyncOptions,
) -> Result<FolderRules, FolderError> {
    options.validate()?;
    if rules.handling.flatten {
        return Err(FolderError::InvalidRules(
            "a sync needs the folder structure: it cannot run on a flat comparison".to_owned(),
        ));
    }
    let mut rules = rules.clone();
    rules.handling.auto_scan_subfolders = true;
    rules.handling.scan_orphan_folders = true;
    rules.handling.archives = if options.within_archives {
        ArchiveHandling::Always
    } else {
        ArchiveHandling::AsFiles
    };
    Ok(rules)
}

/// Plans a sync from the model: **nothing is written** (T12). Rows in tree order with their situations and actions
/// (§1.6.14's algorithm), and one batch plan from the builder of `folder op`. `Err` for what makes the whole sync wrong:
/// a flat comparison, an invalid matrix or override, an empty source (`EmptySource`), deletes on a comparison that did
/// not finish, a stop.
pub fn plan_sync(
    read: &TreeRead<'_>,
    vfs: &dyn Vfs,
    request: &SyncRequest<'_>,
    ctl: &Ctl<'_>,
) -> Result<SyncPlan, FolderError> {
    crate::ops::sync::plan(read, vfs, request, ctl)
}
