//! Recursive and cross-back-end write operations over the single-item primitives of [`Vfs`] (ARCH-031,
//! D-92…D-98, D-103; contract M2 §1.3.4–§1.3.5): copy, move, remove (and trash), mkdir and touch.
//!
//! Every operation is first computed as a complete, serializable [`Plan`] (every item of every tree, in execution
//! order) without writing anything; [`dry_run`] reports that plan, [`execute`] runs it with per-item results. The
//! algorithms live here once, so the faults of [`crate::MemoryVfs`] test them for every back-end (T11).
//!
//! - **Copy** (D-95): per file `open_read` → `open_write` to a temporary sibling (`replace_link`) → `commit`;
//!   any read, write or cancel fault drops the writer, so the target is untouched. A copy within one back-end
//!   instance first asks [`Vfs::copy_file_fast`]. Metadata after content: times, then attributes; a created
//!   folder's metadata after its whole subtree; owner and group never. A missing capability skips the step with
//!   one `capability_missing` warning per operation.
//! - **Overwrite** (D-96): [`OverwritePolicy`]; a link at the target counts as a file; a folder is never removed
//!   to make room for a file; folders merge; special files are skipped, never opened.
//! - **Links** (D-94): copied as links ([`LinkPolicy`]); trees are never descended through links; root links
//!   are followed only with [`CopyOptions::follow_roots`].
//! - **Move** (D-97): one `rename` where possible, else copy + remove per item (folders merged, a source folder
//!   removed only when empty); a `rename` failing with `CrossDevice` at execution is expanded in place.
//! - **Remove and trash** (D-98): post-order; a folder whose child failed is kept (`children_failed`); trash is
//!   one call per root and never falls back to a permanent delete.
//! - **Execution** (D-103) re-checks every target: an item planned without a target that finds one fails with
//!   "changed since the plan was made"; cancellation fails the running item (target untouched) and leaves the
//!   later ones `not_run`. Nothing is rolled back across items.
//! - **Archives** (D-148): `execute` is the single place an operation ends, so it opens a batch on each archive it
//!   touches (`roots`) and finishes them once, target roots first. A move into an archive removes its sources only
//!   after the target's rebuild. An aborted or failed rebuild turns the done items of its root into failed ones.

mod copy;
mod plan;
mod remove;
mod rename;
mod roots;
mod touch;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;

use hikasa_core::{Ctl, Phase, Progress, Spec, Unit, Warning, WarningCode, case};

pub use copy::plan_copy;
pub use plan::{
    Action, CopyOptions, ItemResult, ItemStatus, LinkPolicy, MoveOptions, OpKind, OpReport,
    OverwritePolicy, Plan, PlanItem, PlanOptions, Preserve, RemoveOptions, SkipReason, Summary,
    TouchOptions, dry_run,
};
pub use remove::plan_remove;
pub use rename::plan_move;
pub use touch::{plan_mkdir, plan_touch};

use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileId, FileName};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{FollowLinks, Vfs};
use plan::report;
use roots::Roots;

/// Applies the `cp`/`mv` naming rule (D-102), owned by core so the CLI, scripts and the GUI agree (T11):
/// `target` an existing folder (a link to a folder is followed: the user named it) → each source goes to
/// `target/<source file name>`; otherwise one source → `target` is the new name, several sources →
/// `InvalidTarget "is not an existing folder"`. `into_folder` (the argument ended with a path separator) and
/// `target` missing → with `create_parents` the sources go into it (the folder is created at execution), else
/// `NotFound`; `into_folder` and an existing non-folder → `NotADirectory`. A source without a file name (`-`,
/// `clipboard://`) into a folder → `InvalidTarget "needs a file name"`. A blob target (`-`, `clipboard://`) →
/// `OperationUnsupported` (op `write`) before anything is read. One source whose target is the same item spelt
/// differently (a case-only rename on a case-insensitive back-end) → the target is the new name, not a folder to
/// move into.
pub fn resolve_targets(
    dst: &dyn Vfs,
    sources: &[Spec],
    target: &Spec,
    into_folder: bool,
    create_parents: bool,
) -> Result<Vec<(Spec, Spec)>, VfsError> {
    if is_blob(target) {
        return Err(VfsError::OperationUnsupported {
            spec: target.canonical(),
            op: VfsOp::Write,
        });
    }
    if let [one] = sources
        && !into_folder
        && is_respelling(dst, dst, one, target)
    {
        return Ok(vec![(one.clone(), target.clone())]);
    }
    let into = |dir: &Spec| -> Result<Vec<(Spec, Spec)>, VfsError> {
        sources
            .iter()
            .map(|source| {
                let name = source_name(source).ok_or_else(|| VfsError::InvalidTarget {
                    spec: source.canonical(),
                    reason: "needs a file name",
                })?;
                Ok((source.clone(), dst.child(dir, &name)?))
            })
            .collect()
    };
    let several = || VfsError::InvalidTarget {
        spec: target.canonical(),
        reason: "is not an existing folder",
    };
    match dst.stat(target, FollowLinks::Yes) {
        Ok(entry) if entry.kind == EntryKind::Dir => into(target),
        Ok(_) if into_folder => Err(VfsError::NotADirectory {
            spec: target.canonical(),
        }),
        Ok(_) => match sources {
            [one] => Ok(vec![(one.clone(), target.clone())]),
            _ => Err(several()),
        },
        Err(VfsError::NotFound { .. }) if into_folder => {
            if create_parents {
                into(target)
            } else {
                Err(VfsError::NotFound {
                    spec: target.canonical(),
                })
            }
        }
        Err(VfsError::NotFound { .. }) => match sources {
            [one] => Ok(vec![(one.clone(), target.clone())]),
            _ => Err(several()),
        },
        Err(e) => Err(e),
    }
}

/// The file name of a source (`None` for blobs and roots).
fn source_name(source: &Spec) -> Option<FileName> {
    match source {
        Spec::Local(path) => path.file_name().map(FileName::from_os),
        _ => source
            .file_name()
            .and_then(|name| FileName::new(&name).ok()),
    }
}

// ---- shared planning helpers ---------------------------------------------------------------------------------

/// True for the blob specs (`-`, `clipboard://`): they can be read, never written, removed or renamed.
pub(crate) fn is_blob(spec: &Spec) -> bool {
    matches!(spec, Spec::Stdin | Spec::Clipboard)
}

/// The checks of an item an operation writes, removes, renames or trashes (§1.3.12): a final component `.` or
/// `..` → `InvalidTarget "needs a file name"`; a local spec whose lexically normalized path is a file-system root
/// → `InvalidTarget "is a root folder"`; so is the root of an archive, which is removed as the file it is
/// (`rm a.zip`, never `rm a.zip/`, §1.5.2).
pub(crate) fn check_item(spec: &Spec) -> Result<(), VfsError> {
    check_name(spec)?;
    if let Spec::ArchiveMember { member, .. } = spec
        && member.is_empty()
    {
        return Err(VfsError::InvalidTarget {
            spec: spec.canonical(),
            reason: "is a root folder",
        });
    }
    if let Spec::Local(path) = spec
        && (path.file_name().is_none() || lexical_is_root(path.as_path()))
    {
        return Err(VfsError::InvalidTarget {
            spec: spec.canonical(),
            reason: "is a root folder",
        });
    }
    Ok(())
}

/// A final component `.` or `..` → `InvalidTarget "needs a file name"` (`mkdir`, `touch` and [`check_item`]).
pub(crate) fn check_name(spec: &Spec) -> Result<(), VfsError> {
    if spec
        .file_name()
        .is_some_and(|name| name == "." || name == "..")
    {
        return Err(VfsError::InvalidTarget {
            spec: spec.canonical(),
            reason: "needs a file name",
        });
    }
    Ok(())
}

/// True when `path` names a file-system root once `.` and `..` are resolved lexically.
fn lexical_is_root(path: &std::path::Path) -> bool {
    use std::path::Component;
    let mut names = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => names += 1,
            Component::ParentDir => names = names.saturating_sub(1),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    names == 0
}

/// True when `target` is `source` spelt differently: the same back-end instance, canonical texts that differ but
/// fold to the same text, and the same item (the same comparison key on a case-insensitive back-end, or the same
/// `FileId`). Such a move is a case-only rename.
pub(crate) fn is_respelling(src: &dyn Vfs, dst: &dyn Vfs, source: &Spec, target: &Spec) -> bool {
    if !same_instance(src, dst) {
        return false;
    }
    let (a, b) = (source.canonical(), target.canonical());
    if a == b || case::fold(&a) != case::fold(&b) {
        return false;
    }
    if spec_key(src, source) == spec_key(dst, target) {
        return src.stat(source, FollowLinks::No).is_ok();
    }
    match (
        src.stat(source, FollowLinks::No),
        dst.stat(target, FollowLinks::No),
    ) {
        (Ok(s), Ok(t)) => s.file_id.is_some() && s.file_id == t.file_id,
        _ => false,
    }
}

/// Reports `Phase::Scanning` progress while planning (`done` = items planned so far).
pub(crate) fn scanning(ctl: &Ctl<'_>, done: usize, current: &Spec) {
    ctl.progress(Phase::Scanning, || Progress {
        phase: Phase::Scanning,
        done: done as u64,
        total: None,
        unit: Unit::Items,
        current: Some(current.canonical()),
    });
}

/// True when `a` and `b` are the same back-end instance.
pub(crate) fn same_instance(a: &dyn Vfs, b: &dyn Vfs) -> bool {
    std::ptr::addr_eq(a as *const dyn Vfs, b as *const dyn Vfs)
}

/// The comparison key of a spec on a back-end: canonical, case-folded when the back-end is not case-sensitive.
fn spec_key(vfs: &dyn Vfs, spec: &Spec) -> String {
    let insensitive = vfs
        .capabilities(spec)
        .is_ok_and(|caps| !caps.contains(Capabilities::CASE_SENSITIVE));
    let text = spec.canonical();
    if insensitive {
        case::fold(&text).into_owned()
    } else {
        text
    }
}

/// Same-item and nesting checks of a root (§1.3.4 step 2): `InvalidTarget "is the source itself"` or `"is
/// inside the source folder"` (the latter for folder sources only).
pub(crate) fn check_nesting(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    source: &Spec,
    source_entry: &Entry,
    target: &Spec,
) -> Result<(), VfsError> {
    if !same_instance(src, dst) {
        return Ok(());
    }
    let source_key = spec_key(src, source);
    let same_id = || {
        let target_id = dst
            .stat(target, FollowLinks::No)
            .ok()
            .and_then(|e| e.file_id);
        source_entry.file_id.is_some() && target_id == source_entry.file_id
    };
    if spec_key(dst, target) == source_key || same_id() {
        return Err(VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: "is the source itself",
        });
    }
    check_holder(src, dst, source, target)?;
    if source_entry.kind == EntryKind::Dir {
        let mut cur = target.parent();
        while let Some(parent) = cur {
            if spec_key(dst, &parent) == source_key {
                return Err(VfsError::InvalidTarget {
                    spec: target.canonical(),
                    reason: "is inside the source folder",
                });
            }
            cur = parent.parent();
        }
    }
    Ok(())
}

/// The archives around `spec`, nearest first.
fn levels(spec: &Spec) -> impl Iterator<Item = &Spec> {
    std::iter::successors(around(spec), |spec| around(spec))
}

/// The archive that directly holds `spec`.
fn around(spec: &Spec) -> Option<&Spec> {
    match spec {
        Spec::ArchiveMember { archive, .. } => Some(archive),
        _ => None,
    }
}

/// True when `a` on `src` and `b` on `dst` are one file: by name, or by identity, which sees through a link.
fn one_file(src: &dyn Vfs, dst: &dyn Vfs, a: &Spec, b: &Spec) -> bool {
    if spec_key(src, a) == spec_key(dst, b) {
        return true;
    }
    let id = |vfs: &dyn Vfs, spec: &Spec| {
        vfs.stat(spec, FollowLinks::Yes)
            .ok()
            .and_then(|entry| entry.file_id)
    };
    !matches!(a, Spec::ArchiveMember { .. })
        && !matches!(b, Spec::ArchiveMember { .. })
        && id(src, a).is_some_and(|found| id(dst, b) == Some(found))
}

/// True when `a` and `b`, two names of one file, are hard links in one folder: a write to one replaces that name and
/// leaves the other, so one is no archive around the other. Anything less sure stays one file.
fn hard_linked(src: &dyn Vfs, dst: &dyn Vfs, a: &Spec, b: &Spec) -> bool {
    let plain = |vfs: &dyn Vfs, spec: &Spec| {
        vfs.stat(spec, FollowLinks::No)
            .is_ok_and(|entry| entry.kind == EntryKind::File)
    };
    let folder = |vfs: &dyn Vfs, spec: &Spec| {
        let parent = spec.parent()?;
        vfs.stat(&parent, FollowLinks::Yes).ok()?.file_id
    };
    spec_key(src, a) != spec_key(dst, b)
        && plain(src, a)
        && plain(dst, b)
        && folder(src, a).is_some_and(|found| folder(dst, b) == Some(found))
}

/// A target that is an archive around the source, or the file behind one: writing it would replace the archive the
/// source lives in, and take every other member with it → `InvalidTarget` (§1.5.10). `Ok` for two back-ends.
pub(crate) fn check_holder(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    source: &Spec,
    target: &Spec,
) -> Result<(), VfsError> {
    let holds =
        |level: &Spec| one_file(src, dst, level, target) && !hard_linked(src, dst, level, target);
    if same_instance(src, dst) && levels(source).any(holds) {
        return Err(VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: "is the archive that holds the source",
        });
    }
    Ok(())
}

/// A move between two names of one archive, one of them a link: both would rewrite one file, and the second waits for
/// a lock the first holds → `InvalidTarget`, which says so. Hard links are two files to a rewrite, and are let through.
pub(crate) fn check_same_archive(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    source: &Spec,
    target: &Spec,
) -> Result<(), VfsError> {
    let (Some(from), Some(to)) = (source.outermost_file(), target.outermost_file()) else {
        return Ok(());
    };
    let linked = |vfs: &dyn Vfs, spec: &Spec| {
        vfs.stat(spec, FollowLinks::No)
            .is_ok_and(|entry| entry.kind == EntryKind::Symlink)
    };
    if same_instance(src, dst)
        && spec_key(src, from) != spec_key(dst, to)
        && one_file(src, dst, from, to)
        && (linked(src, from) || linked(dst, to))
    {
        return Err(VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: "is in the same archive as the source, under another name: use one name for both",
        });
    }
    Ok(())
}

/// A move of a file into an archive it is, or holds: its source removal would take the copy with it → `InvalidTarget`.
pub(crate) fn check_move_inside(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    source: &Spec,
    target: &Spec,
) -> Result<(), VfsError> {
    if same_instance(src, dst) && levels(target).any(|level| one_file(dst, src, level, source)) {
        return Err(VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: "is inside the source",
        });
    }
    Ok(())
}

/// True when `source` and `target` are members of two archives: a move between them is a copy and a removal, and the
/// older records of what moves travel with it (D-156).
fn between_archives(source: &Spec, target: &Spec) -> bool {
    matches!(source, Spec::ArchiveMember { .. })
        && matches!(target, Spec::ArchiveMember { .. })
        && source.container() != target.container()
}

/// The warnings of `--dry-run` for the older records that share a path with what a plan moves or removes (§1.5.7
/// rule 3): the ones that go with a member, and the ones that travel with it to another archive. A run says the same
/// when its rebuild is in place. A record that turns out not to be able to leave (an encrypted one, a hard link) is
/// only known when the run takes it out.
pub fn note_shadow_drops(src: &dyn Vfs, dst: &dyn Vfs, plan: &mut Plan) {
    let mut gone = 0;
    let mut travel = 0;
    for item in plan.items.iter().filter(|item| item.error.is_none()) {
        match (item.action, item.source.as_ref()) {
            (Action::Remove | Action::RemoveDir | Action::Trash, _) => {
                gone += src.shadows_of(&item.target);
            }
            (Action::Move, Some(at)) if between_archives(at, &item.target) => {
                travel += src.shadows_of(at);
            }
            (Action::Move, Some(at)) => gone += src.shadows_of(at),
            (Action::Rename, Some(at)) if between_archives(at, &item.target) => {
                travel += src.shadows_below(at);
            }
            _ => {}
        }
        if item.replaces {
            gone += dst.shadows_of(&item.target);
        }
    }
    let mut note = |message: String| {
        plan.warnings.push(Warning {
            code: WarningCode::ArchiveDuplicateMember,
            message,
            spec: None,
        });
    };
    if gone > 0 {
        note(format!(
            "{gone} shadowed members would go with the members they share a path with"
        ));
    }
    if travel > 0 {
        note(format!(
            "{travel} shadowed members would move to the other archive with the members they share a path with"
        ));
    }
}

/// The target's state at planning: absent, or its unfollowed entry.
pub(crate) fn lstat_target(dst: &dyn Vfs, target: &Spec) -> Result<Option<Entry>, VfsError> {
    match dst.stat(target, FollowLinks::No) {
        Ok(entry) => Ok(Some(entry)),
        Err(VfsError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// The decision of the overwrite table (D-96) for a non-folder source onto an existing non-folder target.
pub(crate) enum Decision {
    /// Write (replacing the target).
    Replace,
    /// Skip with the reason.
    Skip(SkipReason),
    /// The item fails.
    Fail(VfsError),
}

/// The overwrite table for an existing non-folder target.
pub(crate) fn decide(
    policy: OverwritePolicy,
    source: &Entry,
    target_entry: &Entry,
    target: &Spec,
) -> Decision {
    match policy {
        OverwritePolicy::Fail => Decision::Fail(VfsError::AlreadyExists {
            spec: target.canonical(),
        }),
        OverwritePolicy::Skip => Decision::Skip(SkipReason::Exists),
        OverwritePolicy::Always => Decision::Replace,
        OverwritePolicy::Newer => match (source.mtime, target_entry.mtime) {
            (Some(s), Some(t)) if s > t => Decision::Replace,
            _ => Decision::Skip(SkipReason::NotNewer),
        },
    }
}

/// Adds one `capability_missing` warning per operation when a source has no modification time to preserve.
pub(crate) fn warn_no_mtime(
    warned: &mut BTreeSet<&'static str>,
    warnings: &mut Vec<Warning>,
    at: &Spec,
) {
    if warned.insert("mtime") {
        warnings.push(Warning {
            code: WarningCode::CapabilityMissing,
            message: format!(
                "{}: the source has no `mtime`; its time was not copied",
                at.canonical()
            ),
            spec: Some(at.canonical()),
        });
    }
}

/// Adds one `capability_missing` warning per capability name and operation.
pub(crate) fn warn_missing(
    warned: &mut BTreeSet<&'static str>,
    warnings: &mut Vec<Warning>,
    name: &'static str,
    at: &Spec,
) {
    if warned.insert(name) {
        warnings.push(Warning {
            code: WarningCode::CapabilityMissing,
            message: format!(
                "{}: the target does not support `{name}`; that step was skipped",
                at.canonical()
            ),
            spec: Some(at.canonical()),
        });
    }
}

/// Adds one `archive_symlink_skipped` warning per operation (§1.5.7 rule 5).
pub(crate) fn warn_archive_link(
    warned: &mut BTreeSet<&'static str>,
    warnings: &mut Vec<Warning>,
    at: &Spec,
) {
    if warned.insert("archive_symlink") {
        warnings.push(Warning {
            code: WarningCode::ArchiveSymlinkSkipped,
            message: format!(
                "{}: link members of an archive are not copied out of it",
                at.canonical()
            ),
            spec: Some(at.canonical()),
        });
    }
}

// ---- execution ------------------------------------------------------------------------------------------------

/// The outcome of one item.
pub(crate) enum Outcome {
    /// Done; bytes copied.
    Done(u64),
    /// Skipped.
    Skipped(SkipReason),
    /// Failed.
    Failed(VfsError),
    /// A `rename` that met `CrossDevice`: these items replace it (D-97).
    Expand(Vec<PlanItem>),
    /// A `remove_dir` that waits for a rebuild (D-148): the result comes later.
    Deferred,
}

/// Pending metadata of a created folder, applied after its whole subtree.
pub(crate) struct PendingDir {
    depth: u32,
    result: usize,
    target: Spec,
    source: Entry,
}

/// Execution state shared by the item runners.
pub(crate) struct Exec<'a> {
    pub(crate) src: &'a dyn Vfs,
    pub(crate) dst: &'a dyn Vfs,
    pub(crate) ctl: &'a Ctl<'a>,
    pub(crate) options: PlanOptions,
    pub(crate) warned: BTreeSet<&'static str>,
    pub(crate) warnings: Vec<Warning>,
    pub(crate) total: u64,
    pub(crate) done_bytes: u64,
    pending: Vec<PendingDir>,
    /// Canonical specs of folders below which an item failed or was skipped (`children_failed`).
    blocked: BTreeSet<String>,
    /// The folders of the plan (canonical spec of the source folder of `create_dir`/`keep_dir`, the folder of
    /// `remove_dir`) with the identity seen at planning.
    folders: BTreeMap<String, (Option<FileId>, FollowLinks)>,
    /// Folders found changed since the plan was made: nothing below them is touched.
    changed: BTreeSet<String>,
    /// Depths of the copy/move folder items that failed or were skipped: the items below them (deeper, until the
    /// next item at that depth or above) are `not_run` (§1.3.12).
    cut: Vec<u32>,
    /// A move runs in two phases: its removals wait for the rebuild of the archive it moves into (D-148).
    pub(crate) two_phase: bool,
    /// What the second phase removes: result indexes, in plan order, with the entry copied for a `move`.
    pub(crate) deferred: Vec<(usize, Option<Entry>)>,
}

impl Exec<'_> {
    /// True when a move's source is removed only after a rebuild: its target is in an archive the source is not in.
    pub(crate) fn defers_removal(&self, source: &Spec, target: &Spec) -> bool {
        match target.outermost_file() {
            Some(outer) => {
                !same_instance(self.src, self.dst) || source.outermost_file() != Some(outer)
            }
            None => false,
        }
    }

    /// Reports `Phase::Writing` progress.
    pub(crate) fn progress(&self, extra: u64, current: &Spec) {
        let done = self.done_bytes + extra;
        let total = self.total;
        self.ctl.progress(Phase::Writing, || Progress {
            phase: Phase::Writing,
            done,
            total: Some(total),
            unit: Unit::Bytes,
            current: Some(current.canonical()),
        });
    }

    /// The preserve settings of the operation (moves keep times and attributes).
    pub(crate) fn preserve(&self) -> Preserve {
        match self.options {
            PlanOptions::Copy(opts) => opts.preserve,
            _ => Preserve::default(),
        }
    }

    /// Whether root targets get their missing parents.
    pub(crate) fn create_parents(&self, depth: u32) -> bool {
        depth == 0
            && match self.options {
                PlanOptions::Copy(opts) => opts.create_parents,
                PlanOptions::Move(opts) => opts.create_parents,
                _ => false,
            }
    }

    /// Remembers a created folder whose metadata is applied after its subtree.
    pub(crate) fn defer_dir(&mut self, depth: u32, result: usize, target: Spec, source: Entry) {
        self.pending.push(PendingDir {
            depth,
            result,
            target,
            source,
        });
    }

    /// Marks the folders above `item`'s own spec (up to `depth` levels; a folder item also itself) as having a
    /// failed or skipped child.
    fn block(&mut self, item: &PlanItem) {
        let spec = own_spec(item);
        if matches!(item.action, Action::CreateDir | Action::KeepDir) {
            self.blocked.insert(spec.canonical());
        }
        self.block_ancestors(spec, item.depth);
    }

    /// Marks the folders above `spec` (up to `depth` levels) as having a failed or skipped child.
    fn block_ancestors(&mut self, spec: &Spec, depth: u32) {
        let mut cur = spec.parent();
        for _ in 0..depth {
            let Some(parent) = cur else { break };
            self.blocked.insert(parent.canonical());
            cur = parent.parent();
        }
    }

    /// True when a child of the folder `spec` failed or was skipped.
    pub(crate) fn is_blocked(&self, spec: &Spec) -> bool {
        self.blocked.contains(&spec.canonical())
    }

    /// Applies the metadata of every pending folder at depth ≥ `depth`.
    fn flush_dirs(&mut self, depth: u32, results: &mut [ItemResult]) {
        while self.pending.last().is_some_and(|p| p.depth >= depth) {
            let Some(dir) = self.pending.pop() else { break };
            if let Err(e) = copy::apply_metadata(self, &dir.target, &dir.source)
                && let Some(result) = results.get_mut(dir.result)
            {
                *result = ItemResult::failed(report(&e));
            }
        }
    }
}

/// Executes `plan`: items in order, each re-checked against the target; per-item results (D-103). `src` is
/// read (and its sources removed by moves), `dst` written; remove, mkdir and touch use one back-end passed as
/// both. Below a folder item of a copy or move that failed or was skipped, and below a folder that changed since
/// the plan was made (another kind or another `FileId` than at planning), nothing runs: those items are
/// `not_run`.
pub fn execute(src: &dyn Vfs, dst: &dyn Vfs, plan: Plan, ctl: &Ctl<'_>) -> OpReport {
    let Plan {
        op,
        options,
        items,
        warnings: plan_warnings,
    } = plan;
    let total = items
        .iter()
        .filter(|i| matches!(i.action, Action::Copy | Action::Move) && i.error.is_none())
        .filter_map(|i| i.size)
        .sum();
    let mut exec = Exec {
        src,
        dst,
        ctl,
        options,
        warned: BTreeSet::new(),
        warnings: Vec::new(),
        total,
        done_bytes: 0,
        pending: Vec::new(),
        blocked: BTreeSet::new(),
        folders: BTreeMap::new(),
        changed: BTreeSet::new(),
        cut: Vec::new(),
        two_phase: false,
        deferred: Vec::new(),
    };
    let mut roots = Roots::new(src, dst);
    for item in &items {
        exec.remember_folder(item);
    }
    let tree_op = matches!(op, OpKind::Copy | OpKind::Move);
    let mut queue: VecDeque<PlanItem> = items.into();
    let mut done_items: Vec<PlanItem> = Vec::with_capacity(queue.len());
    let mut results: Vec<ItemResult> = Vec::with_capacity(queue.len());
    let mut item_bytes: Vec<u64> = Vec::with_capacity(queue.len());
    let mut cancelled = false;
    while let Some(item) = queue.pop_front() {
        exec.flush_dirs(item.depth, &mut results);
        while exec.cut.last().is_some_and(|&d| d >= item.depth) {
            exec.cut.pop();
        }
        if cancelled || ctl.check().is_err() {
            cancelled = true;
            roots.note(None);
            done_items.push(item);
            results.push(ItemResult::status(ItemStatus::NotRun));
            item_bytes.push(0);
            continue;
        }
        // Below a failed or skipped folder item, or a folder that changed since the plan was made: not run.
        if !exec.cut.is_empty() || exec.parent_changed(&item) {
            exec.block(&item);
            if tree_op && matches!(item.action, Action::CreateDir | Action::KeepDir) {
                exec.cut.push(item.depth);
            }
            roots.note(None);
            done_items.push(item);
            results.push(ItemResult::status(ItemStatus::NotRun));
            item_bytes.push(0);
            continue;
        }
        let index = results.len();
        let touched = match (&item.error, item.skip) {
            (None, None) => roots.touch(&item),
            _ => Ok(None),
        };
        let outcome = match (&item.error, touched) {
            (Some(error), _) => Err(ItemResult::failed(error.clone())),
            (None, Err(e)) => Ok(Outcome::Failed(e)),
            (None, Ok(owned)) => {
                let outcome = run_item(&mut exec, &item, index);
                if !matches!(outcome, Outcome::Expand(_)) {
                    roots.note(owned);
                }
                Ok(outcome)
            }
        };
        if !matches!(outcome, Ok(Outcome::Expand(_))) && roots.noted() == index {
            roots.note(None);
        }
        let mut bytes_done = 0;
        let result = match outcome {
            Err(result) => result,
            Ok(Outcome::Expand(expansion)) => {
                for expanded in expansion.into_iter().rev() {
                    exec.remember_folder(&expanded);
                    queue.push_front(expanded);
                }
                continue;
            }
            Ok(Outcome::Done(bytes)) => {
                exec.done_bytes += bytes;
                bytes_done = bytes;
                ItemResult::status(ItemStatus::Done)
            }
            Ok(Outcome::Deferred) => {
                done_items.push(item);
                results.push(ItemResult::status(ItemStatus::NotRun));
                item_bytes.push(0);
                continue;
            }
            Ok(Outcome::Skipped(reason)) => ItemResult::skipped(reason),
            Ok(Outcome::Failed(e)) => {
                if e.is_cancelled() {
                    cancelled = true;
                }
                ItemResult::failed(report(&e))
            }
        };
        if result.status != ItemStatus::Done {
            exec.block(&item);
            if tree_op && matches!(item.action, Action::CreateDir | Action::KeepDir) {
                exec.cut.push(item.depth);
            }
        }
        done_items.push(item);
        results.push(result);
        item_bytes.push(bytes_done);
    }
    exec.flush_dirs(0, &mut results);
    roots::finish(
        &mut roots,
        &mut exec,
        &done_items,
        &mut results,
        &mut cancelled,
    );
    let bytes = results
        .iter()
        .zip(&item_bytes)
        .filter(|(result, _)| result.status == ItemStatus::Done)
        .map(|(_, bytes)| bytes)
        .sum();
    let summary = plan::Summary::count(&results, bytes);
    OpReport {
        plan: Plan {
            op,
            options,
            items: done_items,
            warnings: plan_warnings,
        },
        results,
        summary,
        cancelled,
        warnings: exec.warnings,
    }
}

impl Exec<'_> {
    /// The second phase of a move: sources, then `remove_dir`, in plan order (D-148). `allowed` says whether the
    /// rebuild of the target of a result went through.
    fn run_deferred(
        &mut self,
        items: &[PlanItem],
        results: &mut [ItemResult],
        allowed: &dyn Fn(usize, bool) -> bool,
    ) {
        let deferred = std::mem::take(&mut self.deferred);
        let (moves, folders): (Vec<_>, Vec<_>) =
            deferred.into_iter().partition(|(_, entry)| entry.is_some());
        for (index, entry) in moves.into_iter().chain(folders) {
            let (Some(item), Some(result)) = (items.get(index), results.get_mut(index)) else {
                continue;
            };
            if !allowed(index, entry.is_some()) {
                self.block(item);
                continue;
            }
            let outcome = match &entry {
                Some(entry) => rename::remove_moved(self, item, entry),
                None => remove::finish_remove(self, item),
            };
            *result = match outcome {
                Outcome::Skipped(reason) => ItemResult::skipped(reason),
                Outcome::Failed(e) => ItemResult::failed(report(&e)),
                _ => ItemResult::status(ItemStatus::Done),
            };
            if result.status != ItemStatus::Done {
                self.block(item);
            }
        }
    }

    /// Records a folder item of the plan with the identity seen at planning.
    fn remember_folder(&mut self, item: &PlanItem) {
        if item.kind == EntryKind::Dir
            && matches!(
                item.action,
                Action::CreateDir | Action::KeepDir | Action::RemoveDir
            )
            && (item.source.is_some() || item.action == Action::RemoveDir)
        {
            // A root link followed by `--follow-symlinks` is checked through the link (D-94 rule 7).
            let follow = match self.options {
                PlanOptions::Copy(opts) if opts.follow_roots && item.depth == 0 => FollowLinks::Yes,
                _ => FollowLinks::No,
            };
            self.folders
                .insert(own_spec(item).canonical(), (item.seen, follow));
        }
    }

    /// True when the folder that holds `item` (on the side of its own spec) changed since the plan was made
    /// (D-94 rule 3, D-103). Where both identities are known the parent's `FileId` must match (a link swapped in
    /// anywhere above resolves to another folder); otherwise every planned folder above must still be a folder.
    fn parent_changed(&mut self, item: &PlanItem) -> bool {
        if item.depth == 0 {
            return false;
        }
        let mut cur = own_spec(item).parent();
        for _ in 0..item.depth {
            let Some(dir) = cur else { break };
            let key = dir.canonical();
            if self.changed.contains(&key) {
                return true;
            }
            let Some(&(seen, follow)) = self.folders.get(&key) else {
                break;
            };
            match self.src.stat(&dir, follow) {
                Ok(now) if now.kind == EntryKind::Dir => {
                    if let (Some(seen), Some(now)) = (seen, now.file_id) {
                        if seen == now {
                            return false;
                        }
                        self.changed.insert(key);
                        return true;
                    }
                }
                _ => {
                    self.changed.insert(key);
                    return true;
                }
            }
            cur = dir.parent();
        }
        false
    }

    /// Re-checks the folder of a folder item (the source of `create_dir`/`keep_dir` in copies and moves, the
    /// folder of `remove_dir`) against the identity seen at planning: `Err("changed since the plan was made")`
    /// when it is no longer a folder or has another `FileId`.
    pub(crate) fn check_folder(
        &mut self,
        spec: &Spec,
        seen: Option<FileId>,
        follow: FollowLinks,
    ) -> Result<(), VfsError> {
        let changed = match self.src.stat(spec, follow) {
            Ok(now) => {
                now.kind != EntryKind::Dir
                    || matches!((seen, now.file_id), (Some(a), Some(b)) if a != b)
            }
            Err(VfsError::NotFound { .. }) => true,
            Err(e) => return Err(e),
        };
        if changed {
            self.changed.insert(spec.canonical());
            return Err(VfsError::InvalidTarget {
                spec: spec.canonical(),
                reason: "changed since the plan was made",
            });
        }
        Ok(())
    }
}

/// The spec whose folders a failure blocks: the source of moves (their source folders are removed), else the
/// target.
fn own_spec(item: &PlanItem) -> &Spec {
    item.source.as_ref().unwrap_or(&item.target)
}

/// Runs one item (without a planning error).
fn run_item(exec: &mut Exec<'_>, item: &PlanItem, index: usize) -> Outcome {
    if let Some(reason) = item.skip {
        return Outcome::Skipped(reason);
    }
    // `newer` is decided again at execution: a target that is no longer older is skipped.
    if item.replaces && exec.overwrite() == Some(OverwritePolicy::Newer) {
        match still_newer(exec, item) {
            Ok(true) => {}
            Ok(false) => return Outcome::Skipped(SkipReason::NotNewer),
            Err(e) => return Outcome::Failed(e),
        }
    }
    // The execution re-check (D-103): an item planned without a target never replaces one that appeared.
    if item.action.needs_absent_target() && !item.replaces {
        match lstat_target(exec.dst_for(item.action), &item.target) {
            Ok(None) => {}
            // A case-only rename: the target "exists" as the source itself.
            Ok(Some(_))
                if item.action == Action::Rename
                    && item
                        .source
                        .as_ref()
                        .is_some_and(|s| is_respelling(exec.src, exec.dst, s, &item.target)) => {}
            Ok(Some(_)) => {
                return Outcome::Failed(VfsError::InvalidTarget {
                    spec: item.target.canonical(),
                    reason: "changed since the plan was made",
                });
            }
            Err(e) => return Outcome::Failed(e),
        }
    }
    // The source folder of a copy or move is still the folder that was planned (D-94 rule 3).
    if matches!(item.action, Action::CreateDir | Action::KeepDir)
        && let Some(source) = &item.source
        && let Some(&(seen, follow)) = exec.folders.get(&source.canonical())
        && let Err(e) = exec.check_folder(source, seen, follow)
    {
        return Outcome::Failed(e);
    }
    match item.action {
        Action::CreateDir | Action::KeepDir => copy::run_dir(exec, item, index),
        Action::Copy => copy::run_copy(exec, item).map_or_else(Outcome::Failed, Outcome::Done),
        Action::Link => {
            copy::run_link(exec, item).map_or_else(Outcome::Failed, |()| Outcome::Done(0))
        }
        Action::Rename => rename::run_rename(exec, item),
        Action::Move => rename::run_move(exec, item, index),
        Action::Remove | Action::RemoveDir | Action::Trash => remove::run_remove(exec, item, index),
        Action::Touch | Action::Create => {
            touch::run_touch(exec, item).map_or_else(Outcome::Failed, |()| Outcome::Done(0))
        }
        Action::CreateArchive => touch::run_create_archive(exec, item)
            .map_or_else(Outcome::Failed, |()| Outcome::Done(0)),
        Action::Skip => Outcome::Skipped(item.skip.unwrap_or(SkipReason::Exists)),
    }
}

/// True when the source of a replacing item is still strictly newer than its target (or the target is gone).
fn still_newer(exec: &Exec<'_>, item: &PlanItem) -> Result<bool, VfsError> {
    let Some(source) = &item.source else {
        return Ok(true);
    };
    let Some(target) = lstat_target(exec.dst, &item.target)? else {
        return Ok(true);
    };
    let follow = match exec.options {
        PlanOptions::Copy(opts) if opts.follow_roots && item.depth == 0 => FollowLinks::Yes,
        _ => FollowLinks::No,
    };
    let source = exec.src.stat(source, follow)?;
    Ok(matches!((source.mtime, target.mtime), (Some(s), Some(t)) if s > t))
}

impl Exec<'_> {
    /// The overwrite policy of a copy or move.
    fn overwrite(&self) -> Option<OverwritePolicy> {
        match self.options {
            PlanOptions::Copy(opts) => Some(opts.overwrite),
            PlanOptions::Move(opts) => Some(opts.overwrite),
            _ => None,
        }
    }

    /// The back-end an action's target lives on.
    fn dst_for(&self, action: Action) -> &dyn Vfs {
        match action {
            Action::Remove | Action::RemoveDir | Action::Trash => self.src,
            _ => self.dst,
        }
    }
}

/// The text of a link target as an `OsStr` (helper for link copies).
pub(crate) fn link_target(entry: &Entry) -> Option<std::ffi::OsString> {
    entry.link_target_os()
}

/// An empty `OsStr` (a link without a readable target is copied as a link to "").
pub(crate) fn empty_os() -> &'static OsStr {
    OsStr::new("")
}
