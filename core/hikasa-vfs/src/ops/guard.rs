//! What execution checks before an item of a plan runs (§1.6.2 steps 2 and 3): the fence of the base folders (D-166) and
//! the stamps of what the plan saw (D-163). The model a folder plan was made from may be old, so both refuse here again.

use std::collections::BTreeSet;

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
/// The reason of an item below a link that stands in a base: what it names is not what the plan saw.
pub(crate) const LINKED: &str = "is a link below a base folder, where the plan saw none";

/// The folders a folder operation stays in (D-166): a mirror must never delete outside them, whatever the tree holds, so
/// execution looks at the tree (§1.6.4).
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

/// The way from `spec` up to the nearest of `keys`: what lies strictly between them on a disk, nearest `spec` first.
/// `None` when `spec` is not strictly below one of them.
fn way_to_base(vfs: &dyn Vfs, keys: &[String], spec: &Spec) -> Option<Vec<Spec>> {
    let hit = |at: &Spec| keys.contains(&spec_key(vfs, at));
    let mut way = Vec::new();
    let mut cur = spec.parent();
    while let Some(dir) = cur {
        // `Spec::parent` of an archive's root skips the file it is in, which can be a base too.
        let file = match &dir {
            Spec::ArchiveMember { archive, member } if member.is_empty() => Some(&**archive),
            _ => None,
        };
        if hit(&dir) || file.is_some_and(hit) {
            return Some(way);
        }
        way.extend(file.filter(|file| on_disk(file)).cloned());
        cur = dir.parent();
        if on_disk(&dir) {
            way.push(dir);
        }
    }
    None
}

/// False for what lies in an archive: a member cannot be swapped for a link, and no link member is followed.
fn on_disk(spec: &Spec) -> bool {
    !matches!(spec, Spec::ArchiveMember { .. })
}

/// True when `spec` lies strictly below `base`: [`Spec::parent`] is walked up (it leaves an archive through its root,
/// §1.5.2) and canonical texts compared, case-folded when the back-end is not `CASE_SENSITIVE`. Never true for `base`
/// itself. The comparison is of texts: execution looks at the tree too (§1.6.4), and a path with a `..` in it is below
/// nothing the `..` is not below in the same spelling.
pub fn within(vfs: &dyn Vfs, base: &Spec, spec: &Spec) -> bool {
    way_to_base(vfs, &[spec_key(vfs, base)], spec).is_some()
}

/// Which back-end of an execution a spec is on.
#[derive(Clone, Copy)]
enum Side {
    Source,
    Target,
}

/// What one back-end's specs have shown: folders with no link on the way up from them to a base.
#[derive(Default)]
struct Walked {
    keys: Vec<String>,
    clean: BTreeSet<String>,
    /// The folder of the last item, so that the items of one folder walk once: it is most of what a copy of a tree asks.
    last: Option<Spec>,
}

/// A [`Fence`] with its keys on the two back-ends of an execution, so an item folds and formats each base once.
pub(crate) struct Keys {
    source: Walked,
    target: Walked,
    writes: bool,
}

impl Keys {
    pub(crate) fn new(src: &dyn Vfs, dst: &dyn Vfs, fence: &Fence) -> Keys {
        Keys {
            source: Walked {
                keys: keys_of(src, &fence.bases),
                ..Walked::default()
            },
            target: Walked {
                keys: keys_of(dst, &fence.bases),
                ..Walked::default()
            },
            writes: fence.writes,
        }
    }

    /// The folders that were checked no longer say anything: an item may have put a link where one stood.
    pub(crate) fn forget(&mut self) {
        for walked in [&mut self.source, &mut self.target] {
            walked.clean.clear();
            walked.last = None;
        }
    }

    /// Refuses `spec`, unless it lies below a base and the tree agrees: from the base down to `spec`'s folder, with an
    /// `lstat`, no component is a link. A link at or above a base is the user's choice, and `spec` itself is the item,
    /// which nothing follows. `fresh` looks at the tree whatever is known, for what goes or is replaced there. A folder
    /// that is not there has nothing below it, and the item fails in its own way.
    fn holds(
        &mut self,
        vfs: &dyn Vfs,
        side: Side,
        spec: &Spec,
        fresh: bool,
    ) -> Result<(), VfsError> {
        let walked = match side {
            Side::Source => &mut self.source,
            Side::Target => &mut self.target,
        };
        let folder = spec.parent();
        if !fresh && walked.last.is_some() && walked.last == folder {
            return Ok(());
        }
        let Some(way) = way_to_base(vfs, &walked.keys, spec) else {
            return Err(VfsError::InvalidTarget {
                spec: spec.canonical(),
                reason: OUTSIDE,
            });
        };
        for dir in way.iter().rev() {
            let key = dir.canonical();
            if !fresh && walked.clean.contains(&key) {
                continue;
            }
            match vfs.stat(dir, FollowLinks::No) {
                Ok(entry) if entry.kind == EntryKind::Symlink => {
                    return Err(VfsError::InvalidTarget {
                        spec: dir.canonical(),
                        reason: LINKED,
                    });
                }
                Ok(_) => {}
                Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => return Ok(()),
                Err(e) => return Err(e),
            }
            walked.clean.insert(key);
        }
        walked.last = folder;
        Ok(())
    }

    /// The error of an item outside the bases (§1.6.4). A failing item leaves everything as it was, so the first spec
    /// that is outside decides.
    pub(crate) fn refuses(
        &mut self,
        src: &dyn Vfs,
        dst: &dyn Vfs,
        item: &PlanItem,
    ) -> Result<(), VfsError> {
        let removed = matches!(
            item.action,
            Action::Remove | Action::RemoveDir | Action::Trash
        );
        if removed {
            self.holds(src, Side::Source, &item.target, true)?;
        }
        if matches!(
            item.action,
            Action::Move | Action::Rename | Action::Exchange
        ) && let Some(source) = &item.source
        {
            self.holds(src, Side::Source, source, true)?;
        }
        if self.writes && !removed {
            self.holds(dst, Side::Target, &item.target, item.replaces)?;
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
pub(crate) fn holds(stamp: &ItemStamp, entry: &Entry) -> bool {
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
