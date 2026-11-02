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
//!
//! **Read-only.** `clear_read_only` takes `R` off the sides on disk just before their renames, and every failure gives it
//! back, on both sides. The side that waits for a rebuild clears it just before that rename, so a failed rebuild has
//! nothing to give back. A hidden copy can hold `R`, from the side it copies: it is cleared before the copy is removed.

use hikasa_core::Spec;

use super::copy::{
    apply_metadata_on, clear_read_only_of, copy_file_on, free_sibling, restore_read_only,
    source_changed,
};
use super::guard::Seen;
use super::{Exec, Later, Outcome, PlanItem, lstat_target};
use crate::entry::{Entry, EntryKind};
use crate::error::VfsError;
use crate::vfs::{FollowLinks, Vfs};

/// The name of the hidden copies of an exchange.
const HIDDEN: &str = ".hikasa-xchg";
/// What a failed last step says.
const KEPT: &str = "changed during the move; the copy was kept";

/// Removes a hidden copy. One that took `R` from its side loses it first: Windows would keep the file.
pub(crate) fn drop_copy(vfs: &dyn Vfs, copy: &Spec) {
    let _ = clear_read_only_of(vfs, copy);
    let _ = vfs.remove(copy);
}

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
        drop_copy(to_vfs, copy);
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
    let t1 = free_sibling(dst, target, HIDDEN)?;
    let mut bytes = stage(exec, (src, source), &left, (dst, &t1))?;
    let drop_t1 = |e: VfsError| {
        drop_copy(dst, &t1);
        e
    };
    exec.ctl.check().map_err(VfsError::from).map_err(drop_t1)?;
    let t2 = free_sibling(src, source, HIDDEN).map_err(drop_t1)?;
    bytes += stage(exec, (dst, target), &right, (src, &t2)).map_err(drop_t1)?;
    let drop_both = |e: VfsError| {
        drop_copy(src, &t2);
        drop_t1(e)
    };
    exec.ctl
        .check()
        .map_err(VfsError::from)
        .map_err(drop_both)?;
    let source_member = matches!(source, Spec::ArchiveMember { .. });
    let target_member = matches!(target, Spec::ArchiveMember { .. });
    // A rename in an overlay meets no `R`, and a plain side against a member clears it with its rename, after the rebuild.
    let clear = item.clear_read_only && !source_member && !target_member;
    let cleared_target = clear && clear_read_only_of(dst, target).map_err(drop_both)?;
    let give_back = |vfs: &dyn Vfs, spec: &Spec, cleared: bool| {
        if cleared {
            restore_read_only(vfs, spec);
        }
    };
    let cleared_source = clear
        && clear_read_only_of(src, source).map_err(|e| {
            give_back(dst, target, cleared_target);
            drop_both(e)
        })?;
    let undo = |e: VfsError| {
        give_back(dst, target, cleared_target);
        give_back(src, source, cleared_source);
        drop_both(e)
    };
    // Each side's old content lives on in the other's hidden copy, whose file is synced: its name goes first (§1.6.3).
    if let Some(folder) = t2.parent() {
        src.sync_folder(&folder);
    }
    if source_member && !target_member {
        src.rename(&t2, source, true).map_err(undo)?;
        exec.deferred.push((
            index,
            Later::Rename {
                on_source: false,
                from: t1,
                to: target.clone(),
                clear: item.clear_read_only,
            },
        ));
        return Ok(Outcome::Done(bytes));
    }
    dst.rename(&t1, target, true).map_err(undo)?;
    if target_member && !source_member {
        exec.deferred.push((
            index,
            Later::Rename {
                on_source: true,
                from: t2,
                to: source.clone(),
                clear: item.clear_read_only,
            },
        ));
        return Ok(Outcome::Done(bytes));
    }
    if let Some(folder) = target.parent() {
        dst.sync_folder(&folder);
    }
    src.rename(&t2, source, true).map_err(|_| {
        give_back(src, source, cleared_source);
        kept(&t2)
    })?;
    Ok(Outcome::Done(bytes))
}

/// What a failed last step says: `copy` holds the other side's old content.
fn kept(copy: &Spec) -> VfsError {
    VfsError::InvalidTarget {
        spec: copy.canonical(),
        reason: KEPT,
    }
}

/// The last rename of an exchange with an archive, after its rebuild went through. It fails like step 4.
pub(crate) fn finish_rename(
    vfs: &dyn Vfs,
    from: &Spec,
    to: &Spec,
    clear: bool,
) -> Result<(), VfsError> {
    let cleared = clear && clear_read_only_of(vfs, to).map_err(|_| kept(from))?;
    vfs.rename(from, to, true).map_err(|_| {
        if cleared {
            restore_read_only(vfs, to);
        }
        kept(from)
    })
}
