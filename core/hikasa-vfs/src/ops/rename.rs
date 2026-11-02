//! Move planning and execution (D-97): one `rename` where possible, else copy + remove per item.

use std::collections::BTreeSet;

use hikasa_core::{Ctl, Spec, Warning, WarningCode};

use super::copy::{
    Mode, Planner, TargetState, apply_preserved, clear_read_only_of, copy_content,
    create_link_copy, narrow_archive_mode, restore_read_only, source_changed, source_file,
    with_backup,
};
use super::guard::Seen;
use super::plan::report;
use super::{
    Action, Decision, Exec, Later, LinkPolicy, MoveOptions, OpKind, Outcome, OverwritePolicy, Plan,
    PlanItem, PlanOptions, Preserve, check_item, check_move_inside, check_nesting,
    check_same_archive, decide, is_blob, is_respelling, lstat_target, same_instance, scanning,
};
use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{Durability, FollowLinks, Vfs};

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
            let mut rename = PlanItem::planned(
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
        let mut rename = PlanItem::planned(
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

/// `rename`: one call; `CrossDevice` expands the root in place (D-97). A replaced target goes to its backup first, as in
/// a copy (§1.6.5), and comes back when the rename fails.
pub(crate) fn run_rename(exec: &mut Exec<'_>, item: &PlanItem) -> Outcome {
    let Some(source) = item.source.as_ref() else {
        return Outcome::Failed(source_changed(&item.target));
    };
    if let Err(e) = parents_of(exec, item) {
        return Outcome::Failed(e);
    }
    let renamed = match item.backup.as_ref().filter(|_| item.replaces) {
        Some(backup) => with_backup(exec, &item.target, backup, item.clear_read_only, || {
            exec.src.rename(source, &item.target, true)
        }),
        None => clearing_read_only(exec, item, || {
            exec.src.rename(source, &item.target, item.replaces)
        }),
    };
    match renamed {
        Ok(()) => Outcome::Done(0),
        Err(VfsError::CrossDevice { .. }) => expand(exec, item, source),
        Err(e) => Outcome::Failed(e),
    }
}

/// The missing parents of a root target.
fn parents_of(exec: &Exec<'_>, item: &PlanItem) -> Result<(), VfsError> {
    if exec.create_parents(item.depth)
        && let Some(parent) = item.target.parent()
    {
        exec.dst.create_dir(&parent, true)?;
    }
    Ok(())
}

/// Runs `rename` with the `R` of a replaced target off, and gives it back when it fails.
fn clearing_read_only(
    exec: &Exec<'_>,
    item: &PlanItem,
    rename: impl FnOnce() -> Result<(), VfsError>,
) -> Result<(), VfsError> {
    let cleared =
        item.clear_read_only && item.replaces && clear_read_only_of(exec.dst, &item.target)?;
    rename().inspect_err(|_| {
        if cleared {
            restore_read_only(exec.dst, &item.target);
        }
    })
}

/// The expansion of a `rename` that met `CrossDevice`: a replacing item keeps its decision, its backup and its `R`.
fn expand(exec: &mut Exec<'_>, item: &PlanItem, source: &Spec) -> Outcome {
    let opts = match exec.options {
        PlanOptions::Move(opts) => opts,
        // A batch decided at planning, and the item says what it decided.
        _ if item.replaces => MoveOptions {
            overwrite: OverwritePolicy::Always,
            ..MoveOptions::default()
        },
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
    let mut items = planner.items;
    for expanded in items.iter_mut().filter(|expanded| {
        expanded.replaces && expanded.target == item.target && expanded.error.is_none()
    }) {
        expanded.backup.clone_from(&item.backup);
        expanded.clear_read_only = item.clear_read_only;
    }
    Outcome::Expand(items)
}

/// True when the source of a move is what was copied: the same kind, size, time and identity (D-163).
fn still_the_source(exec: &Exec<'_>, item: &PlanItem, source: &Spec, copied: &Entry) -> bool {
    let follow = exec.source_follow(item);
    exec.src.stat(source, follow).is_ok_and(|now| {
        now.kind == copied.kind
            && now.size == copied.size
            && now.mtime == copied.mtime
            && now.file_id == copied.file_id
    })
}

/// The failure of a move whose source changed: it stays, and so does the copy.
fn left_in_place(source: &Spec) -> VfsError {
    VfsError::InvalidTarget {
        spec: source.canonical(),
        reason: "changed during the move; the copy was kept",
    }
}

/// `move`: copy the file or link, then remove the source, once the copy's folders are on disk and the source is still what
/// was copied: one that an edit reached during the copy stays, and the item fails "changed during the move; the copy was
/// kept" (D-163). When the removal fails the copy is removed again unless it replaced a target (D-97), or the source is no
/// longer the item copied (gone, or another item in its place): then the item fails the same way. A move into an archive
/// stops after the copy: the source goes only after the rebuild, in [`remove_moved`] (D-148).
pub(crate) fn run_move(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    index: usize,
    seen: &mut Seen,
) -> Outcome {
    let Some(source) = item.source.as_ref() else {
        return Outcome::Failed(source_changed(&item.target));
    };
    let entry = match seen.source.take() {
        Some(entry) => entry,
        None => match exec.src.stat(source, FollowLinks::No) {
            Ok(entry) => entry,
            Err(e) => return Outcome::Failed(e),
        },
    };
    if let Some(parent) = item.target.parent().filter(|_| exec.create_parents(item.depth)) {
        exec.note_new_folder(&parent, true);
    }
    let copied = match entry.kind {
        EntryKind::File => {
            // A root link of a W3 move never gets here as a file; the entry is what is copied.
            seen.source = Some(entry.clone());
            source_file(exec, item, source, seen)
                .and_then(|_| copy_content(exec, item, source, Durability::Full, seen))
                .and_then(|bytes| {
                    let mut entry = entry.clone();
                    narrow_archive_mode(exec, source, &item.target, &mut entry);
                    let preserve = Preserve::default();
                    apply_preserved(exec, exec.dst, &item.target, &entry, preserve).map(|()| bytes)
                })
        }
        EntryKind::Symlink => {
            seen.source = Some(entry.clone());
            let linked = create_link_copy(exec, source, item, seen).map(|()| 0);
            // A link has no writer to sync its name (§1.6.3).
            if let (Ok(_), Some(parent)) = (&linked, item.target.parent()) {
                exec.dst.sync_folder(&parent);
            }
            linked
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
        exec.deferred.push((index, Later::Moved(entry)));
        return Outcome::Done(bytes);
    }
    exec.sync_new_folders(&item.target);
    if !still_the_source(exec, item, source, &entry) {
        return Outcome::Failed(left_in_place(source));
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
                return Outcome::Failed(left_in_place(source));
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
    if !still_the_source(exec, item, source, copied) {
        return Outcome::Failed(left_in_place(source));
    }
    match exec.src.remove(source) {
        Ok(()) => Outcome::Done(0),
        Err(e) => Outcome::Failed(e),
    }
}
