//! `exchange` (FC-071, D-165; §1.6.2): swaps two files or links, and neither side can be lost at any step, across file
//! systems and into archives, because both copies are made first.
//!
//! | Step | Call | On failure |
//! |---|---|---|
//! | 1 | copy `source` to `T1`, a free hidden sibling of `target` | `T1` removed; both sides untouched |
//! | 2 | copy `target` to `T2`, a free hidden sibling of `source` | `T1`, `T2` removed; both sides untouched |
//! | 3 | `rename(T1, target, replace = true)` | `T1`, `T2` removed; both sides untouched |
//! | 4 | `rename(T2, source, replace = true)` | `T2` is left in place, and the item fails naming it: the other side's old content is that file |
//!
//! A plain file against a member: the rename on disk comes last and waits for the rebuild, as a move's removal does
//! (D-148), so a failed rebuild leaves the plain file; what was staged for it is removed.

use hikasa_core::Spec;

use super::copy::{
    apply_metadata_on, clear_read_only_of, copy_file_on, free_sibling, source_changed,
};
use super::guard::Seen;
use super::{Exec, Later, Outcome, PlanItem, lstat_target};
use crate::entry::{Entry, EntryKind};
use crate::error::VfsError;
use crate::vfs::{FollowLinks, Vfs};

/// The name of the hidden siblings of an exchange.
const SUFFIX: &str = ".hikasa-xchg";
/// What a failed last step says.
const KEPT: &str = "changed during the move; the copy was kept";

/// Makes `copy` of `from` (described by `entry`), on the other side of the exchange: a file with its metadata, a link as
/// a link. On failure nothing of it stays.
fn stage(
    exec: &mut Exec<'_>,
    (from_vfs, from): (&dyn Vfs, &Spec),
    entry: &Entry,
    (to_vfs, copy): (&dyn Vfs, &Spec),
) -> Result<u64, VfsError> {
    let made = match entry.kind {
        EntryKind::File => copy_file_on(exec, (from_vfs, from), (to_vfs, copy)).and_then(|bytes| {
            apply_metadata_on(exec, (from_vfs, from), to_vfs, copy, entry).map(|()| bytes)
        }),
        EntryKind::Symlink => {
            let text = entry.link_target_os().unwrap_or_default();
            let is_dir = from_vfs
                .stat(from, FollowLinks::Yes)
                .is_ok_and(|e| e.kind == EntryKind::Dir);
            to_vfs.create_symlink(copy, &text, is_dir).map(|()| 0)
        }
        EntryKind::Dir | EntryKind::Other => Err(source_changed(from)),
    };
    if made.is_err() {
        let _ = to_vfs.remove(copy);
    }
    made
}

/// Runs an `exchange` item.
pub(crate) fn run_exchange(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    index: usize,
    seen: &mut Seen,
) -> Outcome {
    match exchange(exec, item, index, seen) {
        Ok(outcome) => outcome,
        Err(e) => Outcome::Failed(e),
    }
}

fn exchange(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    index: usize,
    seen: &mut Seen,
) -> Result<Outcome, VfsError> {
    let (src, dst) = (exec.src, exec.dst);
    let target = &item.target;
    let source = item.source.as_ref().ok_or_else(|| source_changed(target))?;
    // The builder gives such an item an error of its own: when the second rebuild failed the first would have lost its
    // old member already.
    if let (Some(left), Some(right)) = (source.outermost_file(), target.outermost_file())
        && left != right
    {
        return Err(VfsError::Unsupported {
            spec: target.canonical(),
            what: "exchanging between two archives",
        });
    }
    let left = match seen.source.take() {
        Some(entry) => entry,
        None => src.stat(source, FollowLinks::No)?,
    };
    let right = match seen.target.take() {
        Some(entry) => entry,
        None => lstat_target(dst, target)?,
    }
    .ok_or_else(|| source_changed(target))?;
    exec.ctl.check()?;
    let t1 = free_sibling(dst, target, SUFFIX)?;
    let mut bytes = stage(exec, (src, source), &left, (dst, &t1))?;
    let drop_t1 = |e: VfsError| {
        let _ = dst.remove(&t1);
        e
    };
    exec.ctl.check().map_err(VfsError::from).map_err(drop_t1)?;
    let t2 = free_sibling(src, source, SUFFIX).map_err(drop_t1)?;
    bytes += stage(exec, (dst, target), &right, (src, &t2)).map_err(drop_t1)?;
    let drop_both = |e: VfsError| {
        let _ = src.remove(&t2);
        drop_t1(e)
    };
    exec.ctl
        .check()
        .map_err(VfsError::from)
        .map_err(drop_both)?;
    if item.clear_read_only {
        clear_read_only_of(dst, target).map_err(drop_both)?;
        clear_read_only_of(src, source).map_err(drop_both)?;
    }
    let source_member = matches!(source, Spec::ArchiveMember { .. });
    let target_member = matches!(target, Spec::ArchiveMember { .. });
    if source_member && !target_member {
        src.rename(&t2, source, true).map_err(drop_both)?;
        exec.deferred.push((
            index,
            Later::Rename {
                on_source: false,
                from: t1,
                to: target.clone(),
            },
        ));
        return Ok(Outcome::Done(bytes));
    }
    dst.rename(&t1, target, true).map_err(drop_both)?;
    if target_member && !source_member {
        exec.deferred.push((
            index,
            Later::Rename {
                on_source: true,
                from: t2,
                to: source.clone(),
            },
        ));
        return Ok(Outcome::Done(bytes));
    }
    src.rename(&t2, source, true)
        .map_err(|_| VfsError::InvalidTarget {
            spec: t2.canonical(),
            reason: KEPT,
        })?;
    Ok(Outcome::Done(bytes))
}

/// The last rename of an exchange with an archive, after its rebuild went through. It fails like step 4.
pub(crate) fn finish_rename(vfs: &dyn Vfs, from: &Spec, to: &Spec) -> Result<(), VfsError> {
    vfs.rename(from, to, true)
        .map_err(|_| VfsError::InvalidTarget {
            spec: from.canonical(),
            reason: KEPT,
        })
}
