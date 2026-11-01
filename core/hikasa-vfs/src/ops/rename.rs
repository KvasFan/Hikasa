//! Move planning and execution (D-97): one `rename` where possible, else copy + remove per item.

use std::collections::BTreeSet;

use hikasa_core::{Ctl, Spec, Warning, WarningCode};

use super::copy::{
    Mode, Planner, TargetState, apply_metadata, copy_content, create_link_copy,
    narrow_archive_mode, source_changed,
};
use super::plan::report;
use super::{
    Action, Decision, Exec, LinkPolicy, MoveOptions, OpKind, Outcome, Plan, PlanItem, PlanOptions,
    check_item, check_move_inside, check_nesting, check_same_archive, decide, is_blob,
    is_respelling, lstat_target, same_instance, scanning,
};
use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{FollowLinks, Vfs};

/// A planner for the expansion of a move.
fn expander<'a>(
    src: &'a dyn Vfs,
    dst: &'a dyn Vfs,
    opts: &MoveOptions,
    ctl: &'a Ctl<'a>,
) -> Planner<'a> {
    Planner {
        src,
        dst,
        mode: Mode::Move,
        overwrite: opts.overwrite,
        links: LinkPolicy::Copy,
        recursive: true,
        ctl,
        items: Vec::new(),
        warnings: Vec::new(),
        warned: BTreeSet::new(),
    }
}

/// True when both devices are known and differ: the source's, and that of the target's existing parent.
fn devices_differ(dst: &dyn Vfs, entry: &Entry, target: &Spec) -> bool {
    let Some(source) = entry.file_id else {
        return false;
    };
    let Some(parent) = target.parent() else {
        return false;
    };
    dst.stat(&parent, FollowLinks::Yes)
        .ok()
        .and_then(|e| e.file_id)
        .is_some_and(|id| id.dev != source.dev)
}

/// Plans a move of each `(source, target)` pair. Per root: the same back-end instance, the same scheme,
/// `RENAME` on both sides, devices not known to differ and a target that is absent (or replaceable by the
/// policy) → one `rename` item; otherwise the root is expanded: `create_dir`/`keep_dir` for folders, `move` (a
/// copy, then the source removed) for files and links, then `remove_dir` of each source folder after its contents. A
/// file-system root as a source is `InvalidTarget "is a root folder"`; a source or target whose final component
/// is `.` or `..` is `InvalidTarget "needs a file name"`; a blob source is `OperationUnsupported`. A target that is
/// the source spelt differently (a case-only rename on a case-insensitive back-end) is one `rename` to the new
/// spelling. Reports `Phase::Scanning` progress and checks cancellation; `Err` only for cancellation.
pub fn plan_move(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    pairs: &[(Spec, Spec)],
    opts: &MoveOptions,
    ctl: &Ctl<'_>,
) -> Result<Plan, VfsError> {
    let mut planner = expander(src, dst, opts, ctl);
    for (source, target) in pairs {
        ctl.check()?;
        scanning(ctl, planner.items.len(), source);
        let fail = |planner: &mut Planner<'_>, kind: EntryKind, error: &VfsError| {
            planner.items.push(PlanItem::failed(
                Action::Move,
                kind,
                Some(source.clone()),
                target.clone(),
                0,
                report(error),
            ));
        };
        if is_blob(source) {
            let error = VfsError::OperationUnsupported {
                spec: source.canonical(),
                op: VfsOp::Rename,
            };
            fail(&mut planner, EntryKind::File, &error);
            continue;
        }
        let entry = match src.stat(source, FollowLinks::No) {
            Ok(entry) => entry,
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                let e = check_item(source).err().unwrap_or(e);
                fail(&mut planner, EntryKind::Other, &e);
                continue;
            }
        };
        if let Err(e) = check_item(source).and_then(|()| check_item(target)) {
            fail(&mut planner, entry.kind, &e);
            continue;
        }
        // A case-only rename: one `rename` to the new spelling (the target "exists" only as the source itself).
        if is_respelling(src, dst, source, target) {
            let mut rename = PlanItem::new(
                Action::Rename,
                entry.kind,
                Some(source.clone()),
                target.clone(),
                0,
            );
            if entry.kind == EntryKind::File {
                rename.size = Some(entry.size);
            }
            planner.items.push(rename);
            continue;
        }
        let inside = check_nesting(src, dst, source, &entry, target)
            .and_then(|()| check_move_inside(src, dst, source, target))
            .and_then(|()| check_same_archive(src, dst, source, target));
        if let Err(e) = inside {
            fail(&mut planner, entry.kind, &e);
            continue;
        }
        let existing = match lstat_target(dst, target) {
            Ok(existing) => existing,
            Err(e) => {
                fail(&mut planner, entry.kind, &e);
                continue;
            }
        };
        let can_rename = same_instance(src, dst)
            && source.scheme_name() == target.scheme_name()
            && src
                .capabilities(source)
                .is_ok_and(|c| c.contains(Capabilities::RENAME))
            && dst
                .capabilities(target)
                .is_ok_and(|c| c.contains(Capabilities::RENAME))
            && !devices_differ(dst, &entry, target);
        let mut rename = PlanItem::new(
            Action::Rename,
            entry.kind,
            Some(source.clone()),
            target.clone(),
            0,
        );
        if entry.kind == EntryKind::File {
            rename.size = Some(entry.size);
        }
        match (can_rename, existing) {
            (true, None) => planner.items.push(rename),
            (true, Some(found)) if entry.kind != EntryKind::Dir && found.kind != EntryKind::Dir => {
                match decide(opts.overwrite, &entry, &found, target) {
                    Decision::Replace => {
                        rename.replaces = true;
                        planner.items.push(rename);
                    }
                    Decision::Skip(reason) => planner.items.push(PlanItem::skipped(
                        entry.kind,
                        Some(source.clone()),
                        target.clone(),
                        0,
                        reason,
                    )),
                    Decision::Fail(error) => fail(&mut planner, entry.kind, &error),
                }
            }
            _ => planner.node(source, &entry, target, TargetState::Unknown, 0)?,
        }
    }
    Ok(Plan {
        op: OpKind::Move,
        options: PlanOptions::Move(*opts),
        items: planner.items,
        warnings: planner.warnings,
    })
}

/// `rename`: one call; `CrossDevice` expands the root in place (D-97).
pub(crate) fn run_rename(exec: &mut Exec<'_>, item: &PlanItem) -> Outcome {
    let Some(source) = item.source.as_ref() else {
        return Outcome::Failed(source_changed(&item.target));
    };
    if exec.create_parents(item.depth)
        && let Some(parent) = item.target.parent()
        && let Err(e) = exec.dst.create_dir(&parent, true)
    {
        return Outcome::Failed(e);
    }
    match exec.src.rename(source, &item.target, item.replaces) {
        Ok(()) => Outcome::Done(0),
        Err(VfsError::CrossDevice { .. }) => {
            let opts = match exec.options {
                PlanOptions::Move(opts) => opts,
                _ => MoveOptions::default(),
            };
            let entry = match exec.src.stat(source, FollowLinks::No) {
                Ok(entry) => entry,
                Err(e) => return Outcome::Failed(e),
            };
            let mut planner = expander(exec.src, exec.dst, &opts, exec.ctl);
            if let Err(e) = planner.node(
                source,
                &entry,
                &item.target,
                TargetState::Unknown,
                item.depth,
            ) {
                return Outcome::Failed(e);
            }
            exec.warnings.extend(planner.warnings);
            Outcome::Expand(planner.items)
        }
        Err(e) => Outcome::Failed(e),
    }
}

/// `move`: copy the file or link, then remove the source; when the removal fails the copy is removed again
/// unless it replaced a target (D-97), or the source is no longer the item copied (gone, or another item in its
/// place): then the copy is kept and the item fails "changed during the move; the copy was kept". A move into an
/// archive stops after the copy: the source goes only after the rebuild, in [`remove_moved`] (D-148).
pub(crate) fn run_move(exec: &mut Exec<'_>, item: &PlanItem, index: usize) -> Outcome {
    let Some(source) = item.source.as_ref() else {
        return Outcome::Failed(source_changed(&item.target));
    };
    let entry = match exec.src.stat(source, FollowLinks::No) {
        Ok(entry) => entry,
        Err(e) => return Outcome::Failed(e),
    };
    let copied = match entry.kind {
        EntryKind::File => copy_content(exec, source, &item.target, item.replaces, item.depth)
            .and_then(|bytes| {
                let mut entry = entry.clone();
                narrow_archive_mode(exec, source, &item.target, &mut entry);
                apply_metadata(exec, &item.target, &entry).map(|()| bytes)
            }),
        EntryKind::Symlink => {
            create_link_copy(exec, source, &item.target, item.replaces, item.depth).map(|()| 0)
        }
        EntryKind::Dir | EntryKind::Other => Err(source_changed(source)),
    };
    let bytes = match copied {
        Ok(bytes) => bytes,
        Err(e) => return Outcome::Failed(e),
    };
    if let Err(e) = carry_shadows(exec, source, &item.target) {
        if !item.replaces {
            let _ = exec.dst.remove(&item.target);
        }
        return Outcome::Failed(e);
    }
    if exec.defers_removal(source, &item.target) {
        exec.two_phase = true;
        exec.deferred.push((index, Some(entry)));
        return Outcome::Done(bytes);
    }
    match exec.src.remove(source) {
        Ok(()) => Outcome::Done(bytes),
        Err(e) => {
            // Roll back only while the source is still the item that was copied: when it is gone (or another
            // item took its place) the copy is the only one left and is kept.
            let still_there = exec
                .src
                .stat(source, FollowLinks::No)
                .is_ok_and(|now| now.kind == entry.kind && now.file_id == entry.file_id);
            if !still_there {
                return Outcome::Failed(VfsError::InvalidTarget {
                    spec: source.canonical(),
                    reason: "changed during the move; the copy was kept",
                });
            }
            if !item.replaces {
                let _ = exec.dst.remove(&item.target);
            }
            Outcome::Failed(e)
        }
    }
}

/// The older records of a moved member (D-156): they go along to another archive, and what cannot (a plain file holds
/// one version) is said, because it ends with the removal of the source.
fn carry_shadows(exec: &mut Exec<'_>, source: &Spec, target: &Spec) -> Result<(), VfsError> {
    let shadows = exec.src.shadows_of(source);
    if shadows == 0 {
        return Ok(());
    }
    let left = if same_instance(exec.src, exec.dst) {
        exec.dst.carry_older(source, target, exec.ctl)?
    } else {
        shadows
    };
    if left > 0 {
        exec.warnings.push(Warning {
            code: WarningCode::ArchiveDuplicateMember,
            message: format!(
                "{left} shadowed members of {} cannot go to {} and end with this move",
                source.canonical(),
                target.canonical()
            ),
            spec: Some(source.canonical()),
        });
    }
    Ok(())
}

/// The second phase of a move into an archive (D-148): the target's rebuild succeeded, so the source goes. It must
/// still be what was copied: anything else is kept, with the copy, and the item fails. A failed removal fails its item
/// and the copy stays: taking it back would need another rebuild.
pub(crate) fn remove_moved(exec: &mut Exec<'_>, item: &PlanItem, copied: &Entry) -> Outcome {
    let Some(source) = item.source.as_ref() else {
        return Outcome::Failed(source_changed(&item.target));
    };
    let same = exec.src.stat(source, FollowLinks::No).is_ok_and(|now| {
        now.kind == copied.kind
            && now.size == copied.size
            && now.mtime == copied.mtime
            && now.file_id == copied.file_id
    });
    if !same {
        return Outcome::Failed(VfsError::InvalidTarget {
            spec: source.canonical(),
            reason: "changed during the move; the copy was kept",
        });
    }
    match exec.src.remove(source) {
        Ok(()) => Outcome::Done(0),
        Err(e) => Outcome::Failed(e),
    }
}
