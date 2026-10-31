//! Remove and trash planning and execution (D-98, D-94 rule 3).

use hikasa_core::{Ctl, Spec};

use super::plan::report;
use super::{
    Action, Exec, OpKind, Outcome, Plan, PlanItem, PlanOptions, RemoveOptions, SkipReason,
    check_item, is_blob, scanning,
};
use crate::entry::{Entry, EntryKind};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{FollowLinks, ListOptions, Vfs};

/// The listing options of removals: links are never followed.
const NO_FOLLOW: ListOptions<'static> = ListOptions {
    follow: FollowLinks::No,
    prefilter: None,
};

/// Plans the removal of `specs`. Files, links and special files → `remove`; a folder → `remove_dir` when empty,
/// its whole tree post-order with `recursive` (links removed as links, never descended), else an item error
/// `not_empty`; with `trash` each root is one `trash` item (a non-empty folder still needs `recursive`). A spec
/// whose final component is `.` or `..` → `InvalidTarget "needs a file name"`; a file-system root (also after
/// resolving `..` lexically) → `InvalidTarget "is a root folder"`; a blob → `OperationUnsupported`. Reports
/// `Phase::Scanning` progress and checks cancellation; `Err` only for cancellation.
pub fn plan_remove(
    vfs: &dyn Vfs,
    specs: &[Spec],
    opts: &RemoveOptions,
    ctl: &Ctl<'_>,
) -> Result<Plan, VfsError> {
    let mut items = Vec::new();
    for spec in specs {
        ctl.check()?;
        scanning(ctl, items.len(), spec);
        let fail =
            |items: &mut Vec<PlanItem>, action: Action, kind: EntryKind, error: &VfsError| {
                items.push(PlanItem::failed(
                    action,
                    kind,
                    None,
                    spec.clone(),
                    0,
                    report(error),
                ));
            };
        let first = if opts.trash {
            Action::Trash
        } else {
            Action::Remove
        };
        if is_blob(spec) {
            let op = if opts.trash {
                VfsOp::Trash
            } else {
                VfsOp::Remove
            };
            let error = VfsError::OperationUnsupported {
                spec: spec.canonical(),
                op,
            };
            fail(&mut items, first, EntryKind::Other, &error);
            continue;
        }
        let entry = match vfs.stat(spec, FollowLinks::No) {
            Ok(entry) => entry,
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                let e = check_item(spec).err().unwrap_or(e);
                fail(&mut items, first, EntryKind::Other, &e);
                continue;
            }
        };
        let action = match (opts.trash, entry.kind) {
            (true, _) => Action::Trash,
            (false, EntryKind::Dir) => Action::RemoveDir,
            (false, _) => Action::Remove,
        };
        if let Err(e) = check_item(spec) {
            fail(&mut items, action, entry.kind, &e);
            continue;
        }
        if entry.kind != EntryKind::Dir {
            items.push(PlanItem::new(action, entry.kind, None, spec.clone(), 0));
            continue;
        }
        if opts.recursive && !opts.trash {
            tree(vfs, spec, &entry, 0, ctl, &mut items)?;
            continue;
        }
        match vfs.list(spec, &NO_FOLLOW, ctl.cancel) {
            Ok(listing) if !listing.is_empty() && !opts.recursive => {
                let error = VfsError::NotEmpty {
                    spec: spec.canonical(),
                };
                fail(&mut items, action, EntryKind::Dir, &error);
            }
            Ok(_) => {
                let mut item = PlanItem::new(action, EntryKind::Dir, None, spec.clone(), 0);
                item.seen = entry.file_id;
                items.push(item);
            }
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => fail(&mut items, action, EntryKind::Dir, &e),
        }
    }
    Ok(Plan {
        op: OpKind::Remove,
        options: PlanOptions::Remove(*opts),
        items,
        warnings: Vec::new(),
    })
}

/// Plans the post-order removal of the folder `dir` (seen as `seen` in its parent's listing). Before listing it
/// re-`lstat`s the folder and requires a folder with the `FileId` seen (narrows the race with a process that
/// swaps a folder for a link; D-94 rule 3); execution checks that identity again.
fn tree(
    vfs: &dyn Vfs,
    dir: &Spec,
    seen: &Entry,
    depth: u32,
    ctl: &Ctl<'_>,
    items: &mut Vec<PlanItem>,
) -> Result<(), VfsError> {
    ctl.check()?;
    scanning(ctl, items.len(), dir);
    let fail = |items: &mut Vec<PlanItem>, error: &VfsError| {
        items.push(PlanItem::failed(
            Action::RemoveDir,
            EntryKind::Dir,
            None,
            dir.clone(),
            depth,
            report(error),
        ));
    };
    let now = match vfs.stat(dir, FollowLinks::No) {
        Ok(now)
            if now.kind == EntryKind::Dir
                && (now.file_id.is_none()
                    || seen.file_id.is_none()
                    || now.file_id == seen.file_id) =>
        {
            now
        }
        Ok(_) => {
            let error = VfsError::InvalidTarget {
                spec: dir.canonical(),
                reason: "changed since the plan was made",
            };
            fail(items, &error);
            return Ok(());
        }
        Err(e) => {
            fail(items, &e);
            return Ok(());
        }
    };
    let mut listing = match vfs.list(dir, &NO_FOLLOW, ctl.cancel) {
        Ok(listing) => listing,
        Err(e) if e.is_cancelled() => return Err(e),
        Err(e) => {
            fail(items, &e);
            return Ok(());
        }
    };
    listing.sort_by_name();
    for failed in &listing.errors {
        let child = vfs.child(dir, &failed.name).unwrap_or_else(|_| dir.clone());
        items.push(PlanItem::failed(
            Action::Remove,
            EntryKind::Other,
            None,
            child,
            depth + 1,
            failed.error.clone(),
        ));
    }
    for entry in &listing.entries {
        ctl.check()?;
        let child = match vfs.child(dir, &entry.name) {
            Ok(child) => child,
            Err(e) => {
                items.push(PlanItem::failed(
                    Action::Remove,
                    entry.kind,
                    None,
                    dir.clone(),
                    depth + 1,
                    report(&e),
                ));
                continue;
            }
        };
        if entry.kind == EntryKind::Dir {
            tree(vfs, &child, entry, depth + 1, ctl, items)?;
        } else {
            scanning(ctl, items.len(), &child);
            items.push(PlanItem::new(
                Action::Remove,
                entry.kind,
                None,
                child,
                depth + 1,
            ));
        }
    }
    let mut item = PlanItem::new(Action::RemoveDir, EntryKind::Dir, None, dir.clone(), depth);
    item.seen = now.file_id.or(seen.file_id);
    items.push(item);
    Ok(())
}

/// `remove`, `remove_dir` (skipped `children_failed` when a child failed or was skipped) and `trash`.
pub(crate) fn run_remove(exec: &mut Exec<'_>, item: &PlanItem) -> Outcome {
    if item.action == Action::RemoveDir {
        // The folder is still the one planned (a link swapped in is never removed in its place).
        if let Err(e) = exec.check_folder(&item.target, item.seen, FollowLinks::No) {
            return Outcome::Failed(e);
        }
        if exec.is_blocked(&item.target) {
            return Outcome::Skipped(SkipReason::ChildrenFailed);
        }
    }
    let result = match item.action {
        Action::Trash => exec.src.move_to_trash(&item.target),
        _ => exec.src.remove(&item.target),
    };
    match result {
        Ok(()) => Outcome::Done(0),
        Err(e) => Outcome::Failed(e),
    }
}
