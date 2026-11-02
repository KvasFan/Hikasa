//! What execution checks before an item of a plan runs (§1.6.2 steps 2 and 3): the fence of the base folders (D-166) and
//! the stamps of what the plan saw (D-163). The model a folder plan was made from may be old, so both refuse here again.

use hikasa_core::Spec;
use serde::{Deserialize, Serialize};

use super::plan::ItemStamp;
use super::{Action, Exec, Expect, PlanItem, lstat_target, spec_key};
use crate::entry::{Entry, EntryKind};
use crate::error::VfsError;
use crate::vfs::{FollowLinks, Vfs};

/// The reason of a stamp that does not hold.
pub(crate) const CHANGED: &str = "changed since the plan was made";
/// The reason of an item the fence refuses.
pub(crate) const OUTSIDE: &str = "is outside the base folders";

/// The folders a folder operation stays in (D-166): a mirror must never delete outside them, whatever the tree holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fence {
    /// The base folders.
    pub bases: Vec<Spec>,
    /// Every target must lie below a base too. `copyto`/`moveto` set it false: their target is elsewhere, by request.
    pub writes: bool,
}

/// The comparison keys of `bases` on one back-end.
fn keys_of(vfs: &dyn Vfs, bases: &[Spec]) -> Vec<String> {
    bases.iter().map(|base| spec_key(vfs, base)).collect()
}

/// True when `spec` lies strictly below one of `keys`.
fn below(vfs: &dyn Vfs, keys: &[String], spec: &Spec) -> bool {
    let hit = |at: &Spec| keys.contains(&spec_key(vfs, at));
    let mut cur = spec.parent();
    while let Some(dir) = cur {
        // `Spec::parent` of an archive's root skips the file it is in, which can be a base too.
        let file = match &dir {
            Spec::ArchiveMember { archive, member } if member.is_empty() => Some(&**archive),
            _ => None,
        };
        if hit(&dir) || file.is_some_and(hit) {
            return true;
        }
        cur = dir.parent();
    }
    false
}

/// True when `spec` lies strictly below `base`: [`Spec::parent`] is walked up (it leaves an archive through its root,
/// §1.5.2) and canonical texts compared, case-folded when the back-end is not `CASE_SENSITIVE`. Never true for `base`
/// itself. The comparison is of texts: what a followed link leads to is the planner's to refuse (§1.6.4).
pub fn within(vfs: &dyn Vfs, base: &Spec, spec: &Spec) -> bool {
    below(vfs, &[spec_key(vfs, base)], spec)
}

/// A [`Fence`] with its keys on the two back-ends of an execution, so an item folds and formats each base once.
pub(crate) struct Keys {
    source: Vec<String>,
    target: Vec<String>,
    writes: bool,
}

impl Keys {
    pub(crate) fn new(src: &dyn Vfs, dst: &dyn Vfs, fence: &Fence) -> Keys {
        Keys {
            source: keys_of(src, &fence.bases),
            target: keys_of(dst, &fence.bases),
            writes: fence.writes,
        }
    }

    /// The error of an item outside the bases (§1.6.4). A failing item leaves everything as it was, so the first spec
    /// that is outside decides.
    pub(crate) fn refuses(
        &self,
        src: &dyn Vfs,
        dst: &dyn Vfs,
        item: &PlanItem,
    ) -> Result<(), VfsError> {
        let outside = |spec: &Spec| VfsError::InvalidTarget {
            spec: spec.canonical(),
            reason: OUTSIDE,
        };
        let removed = matches!(
            item.action,
            Action::Remove | Action::RemoveDir | Action::Trash
        );
        if removed && !below(src, &self.source, &item.target) {
            return Err(outside(&item.target));
        }
        if matches!(
            item.action,
            Action::Move | Action::Rename | Action::Exchange
        ) && let Some(source) = &item.source
            && !below(src, &self.source, source)
        {
            return Err(outside(source));
        }
        if self.writes && !removed && !below(dst, &self.target, &item.target) {
            return Err(outside(&item.target));
        }
        Ok(())
    }
}

/// What the checks of `run_item` already read of an item. The action reuses it, so a stamp costs no system call beyond
/// W3's re-check (§1.6.2).
#[derive(Debug, Default)]
pub(crate) struct Seen {
    /// The source: unfollowed, or through the link with `follow_source`.
    pub(crate) source: Option<Entry>,
    /// The target, unfollowed: `Some(None)` when it is absent.
    pub(crate) target: Option<Option<Entry>>,
}

/// True when `entry` is what `stamp` saw: the same kind; for files and links the same size and time, exactly, where the
/// stamp has them. A folder's time moves with its children, and nothing else is known of a special file.
fn holds(stamp: &ItemStamp, entry: &Entry) -> bool {
    stamp.kind == entry.kind
        && match entry.kind {
            EntryKind::File | EntryKind::Symlink => {
                stamp.size.is_none_or(|size| size == entry.size)
                    && stamp.mtime.is_none_or(|time| entry.mtime == Some(time))
            }
            EntryKind::Dir | EntryKind::Other => true,
        }
}

/// "Changed since the plan was made", naming `spec`.
pub(crate) fn changed(spec: &Spec) -> VfsError {
    VfsError::InvalidTarget {
        spec: spec.canonical(),
        reason: CHANGED,
    }
}

/// Step 3 of `run_item`: the stamps of `item` against one `lstat` per side.
pub(crate) fn check_expect(
    exec: &Exec<'_>,
    item: &PlanItem,
    expect: &Expect,
    seen: &mut Seen,
) -> Result<(), VfsError> {
    if let (Some(stamp), Some(source)) = (&expect.source, &item.source) {
        let follow = if item.follow_source {
            FollowLinks::Yes
        } else {
            FollowLinks::No
        };
        let entry = match exec.src.stat(source, follow) {
            Ok(entry) => entry,
            Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => {
                return Err(changed(source));
            }
            Err(e) => return Err(e),
        };
        if !holds(stamp, &entry) {
            return Err(changed(source));
        }
        seen.source = Some(entry);
    }
    if let Some(stamp) = &expect.target {
        let found = match lstat_target(exec.dst_for(item.action), &item.target) {
            Ok(found) => found,
            Err(VfsError::NotADirectory { .. }) => None,
            Err(e) => return Err(e),
        };
        if !found.as_ref().is_some_and(|entry| holds(stamp, entry)) {
            return Err(changed(&item.target));
        }
        seen.target = Some(found);
    }
    Ok(())
}
