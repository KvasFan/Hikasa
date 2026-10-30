//! `mkdir` and `touch` planning and execution.

use hikasa_core::Spec;

use super::plan::report;
use super::{Action, Exec, OpKind, Plan, PlanItem, PlanOptions, TouchOptions, check_name, is_blob};
use crate::entry::EntryKind;
use crate::error::{VfsError, VfsOp};
use crate::vfs::{FollowLinks, Vfs, WriteOptions};

/// Plans `mkdir` (a final component `.` or `..` → `InvalidTarget "needs a file name"`, a blob →
/// `OperationUnsupported`): a missing folder → `create_dir`; an existing folder (or a link to one) → `keep_dir` with
/// `parents`, else an item error `already_exists`; an existing non-folder → `already_exists` (`not_a_directory`
/// with `parents`).
pub fn plan_mkdir(vfs: &dyn Vfs, specs: &[Spec], parents: bool) -> Plan {
    let mut items = Vec::new();
    for spec in specs {
        let fail = |items: &mut Vec<PlanItem>, kind: EntryKind, error: &VfsError| {
            items.push(PlanItem::failed(
                Action::CreateDir,
                kind,
                None,
                spec.clone(),
                0,
                report(error),
            ));
        };
        if is_blob(spec) {
            let error = VfsError::OperationUnsupported {
                spec: spec.canonical(),
                op: VfsOp::Mkdir,
            };
            fail(&mut items, EntryKind::Other, &error);
            continue;
        }
        if let Err(e) = check_name(spec) {
            fail(&mut items, EntryKind::Other, &e);
            continue;
        }
        let found = match vfs.stat(spec, FollowLinks::Yes) {
            Ok(entry) => Some(entry),
            // A broken link is an existing item.
            Err(VfsError::NotFound { .. }) => match vfs.stat(spec, FollowLinks::No) {
                Ok(entry) => Some(entry),
                Err(VfsError::NotFound { .. }) => None,
                Err(e) => {
                    fail(&mut items, EntryKind::Other, &e);
                    continue;
                }
            },
            Err(e) => {
                fail(&mut items, EntryKind::Other, &e);
                continue;
            }
        };
        match found {
            None => items.push(PlanItem::new(
                Action::CreateDir,
                EntryKind::Dir,
                None,
                spec.clone(),
                0,
            )),
            Some(entry) if entry.kind == EntryKind::Dir && parents => {
                items.push(PlanItem::new(
                    Action::KeepDir,
                    EntryKind::Dir,
                    None,
                    spec.clone(),
                    0,
                ));
            }
            Some(entry) => {
                let error = if parents {
                    VfsError::NotADirectory {
                        spec: spec.canonical(),
                    }
                } else {
                    VfsError::AlreadyExists {
                        spec: spec.canonical(),
                    }
                };
                fail(&mut items, entry.kind, &error);
            }
        }
    }
    Plan {
        op: OpKind::Mkdir,
        options: PlanOptions::Mkdir { parents },
        items,
        warnings: Vec::new(),
    }
}

/// Plans `touch` (a final component `.` or `..` → `InvalidTarget "needs a file name"`, a blob →
/// `OperationUnsupported`): an existing item (a link itself, never its target) → `touch`; a missing one → `create` with
/// `opts.create`, else an item error `not_found`.
pub fn plan_touch(vfs: &dyn Vfs, specs: &[Spec], opts: &TouchOptions) -> Plan {
    let mut items = Vec::new();
    for spec in specs {
        let refused = if is_blob(spec) {
            Some(VfsError::OperationUnsupported {
                spec: spec.canonical(),
                op: VfsOp::SetTimes,
            })
        } else {
            check_name(spec).err()
        };
        if let Some(error) = refused {
            items.push(PlanItem::failed(
                Action::Touch,
                EntryKind::Other,
                None,
                spec.clone(),
                0,
                report(&error),
            ));
            continue;
        }
        match vfs.stat(spec, FollowLinks::No) {
            Ok(entry) => items.push(PlanItem::new(
                Action::Touch,
                entry.kind,
                None,
                spec.clone(),
                0,
            )),
            Err(VfsError::NotFound { .. }) if opts.create => {
                items.push(PlanItem::new(
                    Action::Create,
                    EntryKind::File,
                    None,
                    spec.clone(),
                    0,
                ));
            }
            Err(e) => items.push(PlanItem::failed(
                Action::Touch,
                EntryKind::Other,
                None,
                spec.clone(),
                0,
                report(&e),
            )),
        }
    }
    Plan {
        op: OpKind::Touch,
        options: PlanOptions::Touch(*opts),
        items,
        warnings: Vec::new(),
    }
}

/// `touch` (`set_times`) and `create` (an empty file through `open_write`, then `set_times`).
pub(crate) fn run_touch(exec: &mut Exec<'_>, item: &PlanItem) -> Result<(), VfsError> {
    let times = match exec.options {
        PlanOptions::Touch(opts) => opts.times,
        _ => return Ok(()),
    };
    if item.action == Action::Create {
        let opts = WriteOptions {
            create_parents: false,
            overwrite: false,
            replace_link: true,
        };
        exec.dst.open_write(&item.target, &opts)?.commit()?;
    }
    if times.modified.is_some() || times.created.is_some() {
        exec.dst.set_times(&item.target, &times)?;
    }
    Ok(())
}
