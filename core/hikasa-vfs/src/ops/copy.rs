//! Copy planning (shared with the expansion of moves) and the execution of folders, files and links (D-94…D-96).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{ErrorKind as IoKind, Read, Write};

use hikasa_core::{Ctl, ErrorKind, Spec, Warning};

use super::backup::write_backup;
use super::guard::Seen;
use super::plan::{report, report_with};
use super::{
    Action, CopyOptions, Decision, Exec, LinkPolicy, OpKind, Outcome, OverwritePolicy, Plan,
    PlanItem, PlanOptions, Preserve, SkipReason, check_holder, check_item, check_nesting, decide,
    lstat_target, same_instance, scanning, warn_archive_link, warn_missing, warn_no_mtime,
};
use crate::caps::Capabilities;
use crate::entry::{Attrs, DosAttrs, Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::stream::StreamOptions;
use crate::vfs::{
    AtomicWrite, AttrChange, Durability, FollowLinks, HandleMeta, ListOptions, SetTimes, Vfs,
    WriteOptions,
};

/// How a tree is planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// `copy` items; folders need `recursive`.
    Copy,
    /// The expansion of a move: `move` items, and `remove_dir` of each source folder after its contents.
    Move,
}

/// Whether the target of a node is known to be absent (its parent folder is created by the plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetState {
    /// `lstat` it.
    Unknown,
    /// Absent.
    Absent,
}

/// The tree planner of copies and expanded moves.
pub(crate) struct Planner<'a> {
    pub(crate) src: &'a dyn Vfs,
    pub(crate) dst: &'a dyn Vfs,
    pub(crate) mode: Mode,
    pub(crate) overwrite: OverwritePolicy,
    pub(crate) links: LinkPolicy,
    pub(crate) recursive: bool,
    pub(crate) ctl: &'a Ctl<'a>,
    pub(crate) items: Vec<PlanItem>,
    pub(crate) warnings: Vec<Warning>,
    pub(crate) warned: BTreeSet<&'static str>,
}

impl Planner<'_> {
    /// The action of a file in this mode.
    fn file_action(&self) -> Action {
        match self.mode {
            Mode::Copy => Action::Copy,
            Mode::Move => Action::Move,
        }
    }

    /// Plans `source` (described by `entry`) onto `target`; `Err` only for cancellation.
    pub(crate) fn node(
        &mut self,
        source: &Spec,
        entry: &Entry,
        target: &Spec,
        state: TargetState,
        depth: u32,
    ) -> Result<(), VfsError> {
        self.ctl.check()?;
        scanning(self.ctl, self.items.len(), source);
        let existing = match state {
            TargetState::Absent => None,
            TargetState::Unknown => match lstat_target(self.dst, target) {
                Ok(existing) => existing,
                Err(e) if e.is_cancelled() => return Err(e),
                Err(e) => {
                    self.fail(self.file_action(), entry.kind, source, target, depth, &e);
                    return Ok(());
                }
            },
        };
        match entry.kind {
            EntryKind::Other => self.items.push(PlanItem::skipped(
                EntryKind::Other,
                Some(source.clone()),
                target.clone(),
                depth,
                SkipReason::SpecialFile,
            )),
            EntryKind::Symlink => {
                // A link from a hostile archive must not appear on disk, whatever the policy says (D-142);
                // between archives it stays a link.
                let out_of_archive = matches!(source, Spec::ArchiveMember { .. })
                    && !matches!(target, Spec::ArchiveMember { .. });
                if out_of_archive {
                    warn_archive_link(&mut self.warned, &mut self.warnings, source);
                }
                if self.links == LinkPolicy::Skip || out_of_archive {
                    self.items.push(PlanItem::skipped(
                        EntryKind::Symlink,
                        Some(source.clone()),
                        target.clone(),
                        depth,
                        SkipReason::Symlink,
                    ));
                } else if !self
                    .dst
                    .capabilities(target)
                    .is_ok_and(|caps| caps.contains(Capabilities::CREATE_SYMLINKS))
                {
                    warn_missing(
                        &mut self.warned,
                        &mut self.warnings,
                        "create_symlinks",
                        target,
                    );
                    self.items.push(PlanItem::skipped(
                        EntryKind::Symlink,
                        Some(source.clone()),
                        target.clone(),
                        depth,
                        SkipReason::SymlinksUnsupported,
                    ));
                } else {
                    let action = match self.mode {
                        Mode::Copy => Action::Link,
                        Mode::Move => Action::Move,
                    };
                    self.non_folder(action, source, entry, target, existing.as_ref(), depth);
                }
            }
            EntryKind::File => {
                let action = self.file_action();
                self.non_folder(action, source, entry, target, existing.as_ref(), depth);
            }
            EntryKind::Dir => self.folder(source, entry, target, existing.as_ref(), depth)?,
        }
        Ok(())
    }

    /// Pushes a failed item.
    fn fail(
        &mut self,
        action: Action,
        kind: EntryKind,
        source: &Spec,
        target: &Spec,
        depth: u32,
        error: &VfsError,
    ) {
        self.items.push(PlanItem::failed(
            action,
            kind,
            Some(source.clone()),
            target.clone(),
            depth,
            report(error),
        ));
    }

    /// The overwrite table for a file or link (D-96).
    fn non_folder(
        &mut self,
        action: Action,
        source: &Spec,
        entry: &Entry,
        target: &Spec,
        existing: Option<&Entry>,
        depth: u32,
    ) {
        let mut item = PlanItem::planned(
            action,
            entry.kind,
            Some(source.clone()),
            target.clone(),
            depth,
        );
        if entry.kind == EntryKind::File {
            item.size = Some(entry.size);
        }
        let holder = match existing {
            Some(_) => check_holder(self.src, self.dst, source, target),
            None => Ok(()),
        };
        match existing {
            None => self.items.push(item),
            Some(found) if found.kind == EntryKind::Dir => {
                let error = VfsError::IsADirectory {
                    spec: target.canonical(),
                };
                self.fail(action, entry.kind, source, target, depth, &error);
            }
            // Below a root too: a member of `a.zip` copied out next to it can have the archive's own name.
            Some(_) if holder.is_err() => {
                if let Err(error) = holder {
                    self.fail(action, entry.kind, source, target, depth, &error);
                }
            }
            Some(found) => match decide(self.overwrite, entry, found, target) {
                Decision::Replace => {
                    item.replaces = true;
                    self.items.push(item);
                }
                Decision::Skip(reason) => self.items.push(PlanItem::skipped(
                    entry.kind,
                    Some(source.clone()),
                    target.clone(),
                    depth,
                    reason,
                )),
                Decision::Fail(error) => {
                    self.fail(action, entry.kind, source, target, depth, &error)
                }
            },
        }
    }

    /// A folder: created, kept (merge) or refused; then its children in name order.
    fn folder(
        &mut self,
        source: &Spec,
        entry: &Entry,
        target: &Spec,
        existing: Option<&Entry>,
        depth: u32,
    ) -> Result<(), VfsError> {
        if self.mode == Mode::Copy && !self.recursive {
            self.items.push(PlanItem::failed(
                Action::CreateDir,
                EntryKind::Dir,
                Some(source.clone()),
                target.clone(),
                depth,
                report_with(
                    ErrorKind::IsADirectory,
                    source,
                    format!("{}: is a folder (use --recursive)", source.canonical()),
                ),
            ));
            return Ok(());
        }
        let (action, replaces, children) = match existing {
            None => (Action::CreateDir, false, TargetState::Absent),
            Some(found) if found.kind == EntryKind::Dir => {
                (Action::KeepDir, false, TargetState::Unknown)
            }
            Some(found) => match decide(self.overwrite, entry, found, target) {
                Decision::Replace => (Action::CreateDir, true, TargetState::Absent),
                Decision::Skip(reason) => {
                    self.items.push(PlanItem::skipped(
                        EntryKind::Dir,
                        Some(source.clone()),
                        target.clone(),
                        depth,
                        reason,
                    ));
                    return Ok(());
                }
                Decision::Fail(error) => {
                    self.fail(
                        Action::CreateDir,
                        EntryKind::Dir,
                        source,
                        target,
                        depth,
                        &error,
                    );
                    return Ok(());
                }
            },
        };
        let options = ListOptions {
            follow: FollowLinks::No,
            prefilter: None,
        };
        let mut listing = match self.src.list(source, &options, self.ctl.cancel) {
            Ok(listing) => listing,
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                self.fail(action, EntryKind::Dir, source, target, depth, &e);
                return Ok(());
            }
        };
        let mut item = PlanItem::planned(
            action,
            EntryKind::Dir,
            Some(source.clone()),
            target.clone(),
            depth,
        );
        item.replaces = replaces;
        item.seen = entry.file_id;
        self.items.push(item);
        listing.sort_by_name();
        let file_action = self.file_action();
        for failed in &listing.errors {
            let (child_source, child_target) = match self.children(source, target, &failed.name) {
                Ok(pair) => pair,
                Err(e) => {
                    self.fail(file_action, EntryKind::Other, source, target, depth + 1, &e);
                    continue;
                }
            };
            self.items.push(PlanItem::failed(
                file_action,
                EntryKind::Other,
                Some(child_source),
                child_target,
                depth + 1,
                failed.error.clone(),
            ));
        }
        for child in &listing.entries {
            match self.children(source, target, &child.name) {
                Ok((child_source, child_target)) => {
                    self.node(&child_source, child, &child_target, children, depth + 1)?;
                }
                Err(e) => self.fail(file_action, child.kind, source, target, depth + 1, &e),
            }
        }
        if self.mode == Mode::Move {
            let mut remove = PlanItem::planned(
                Action::RemoveDir,
                EntryKind::Dir,
                None,
                source.clone(),
                depth,
            );
            remove.seen = entry.file_id;
            self.items.push(remove);
        }
        Ok(())
    }

    /// The source and target specs of child `name`.
    fn children(
        &self,
        source: &Spec,
        target: &Spec,
        name: &crate::entry::FileName,
    ) -> Result<(Spec, Spec), VfsError> {
        Ok((self.src.child(source, name)?, self.dst.child(target, name)?))
    }
}

/// Plans a copy of each `(source, target)` pair (targets from [`super::resolve_targets`]). Planning never writes;
/// every problem of an item is an item with `error` set (a target whose final component is `.` or `..`:
/// `InvalidTarget "needs a file name"`). Reports `Phase::Scanning` progress and checks cancellation; `Err` only
/// for cancellation.
pub fn plan_copy(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    pairs: &[(Spec, Spec)],
    opts: &CopyOptions,
    ctl: &Ctl<'_>,
) -> Result<Plan, VfsError> {
    let mut planner = Planner {
        src,
        dst,
        mode: Mode::Copy,
        overwrite: opts.overwrite,
        links: opts.links,
        recursive: opts.recursive,
        ctl,
        items: Vec::new(),
        warnings: Vec::new(),
        warned: BTreeSet::new(),
    };
    let follow = if opts.follow_roots {
        FollowLinks::Yes
    } else {
        FollowLinks::No
    };
    for (source, target) in pairs {
        ctl.check()?;
        if let Err(e) = check_item(target) {
            planner.fail(Action::Copy, EntryKind::Other, source, target, 0, &e);
            continue;
        }
        let entry = match src.stat(source, follow) {
            Ok(entry) => entry,
            Err(e) if e.is_cancelled() => return Err(e),
            Err(e) => {
                planner.fail(Action::Copy, EntryKind::Other, source, target, 0, &e);
                continue;
            }
        };
        if let Err(e) = check_nesting(src, dst, source, &entry, target) {
            planner.fail(Action::Copy, entry.kind, source, target, 0, &e);
            continue;
        }
        planner.node(source, &entry, target, TargetState::Unknown, 0)?;
    }
    Ok(Plan {
        op: OpKind::Copy,
        options: PlanOptions::Copy(*opts),
        items: planner.items,
        warnings: planner.warnings,
    })
}

// ---- execution ------------------------------------------------------------------------------------------------

/// "The source changed since the plan was made".
pub(crate) fn source_changed(source: &Spec) -> VfsError {
    VfsError::InvalidTarget {
        spec: source.canonical(),
        reason: "changed since the plan was made",
    }
}

/// `create_dir` / `keep_dir` (copies, moves and `mkdir`).
pub(crate) fn run_dir(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    index: usize,
    seen: &mut Seen,
) -> Outcome {
    let mkdir_parents = match exec.options {
        PlanOptions::Mkdir { parents } => Some(parents),
        _ => None,
    };
    if item.action == Action::KeepDir {
        if exec.moving()
            && let Some(source) = &item.source
            && exec.defers_removal(source, &item.target)
        {
            exec.two_phase = true;
        }
        let follow = if mkdir_parents.is_some() {
            FollowLinks::Yes
        } else {
            FollowLinks::No
        };
        return match exec.dst.stat(&item.target, follow) {
            Ok(entry) if entry.kind == EntryKind::Dir => Outcome::Done(0),
            Ok(_) => Outcome::Failed(VfsError::InvalidTarget {
                spec: item.target.canonical(),
                reason: "changed since the plan was made",
            }),
            Err(e) => Outcome::Failed(e),
        };
    }
    let parents = mkdir_parents.unwrap_or_else(|| exec.create_parents(item.depth));
    // An empty source folder of a move into an archive waits for the rebuild too (D-148).
    if exec.moving()
        && let Some(source) = &item.source
        && exec.defers_removal(source, &item.target)
    {
        exec.two_phase = true;
    }
    let replaced = if item.replaces {
        match known_target(exec.dst, &item.target, seen) {
            Ok(Some(found)) if found.kind == EntryKind::Dir => {
                return Outcome::Failed(VfsError::InvalidTarget {
                    spec: item.target.canonical(),
                    reason: "changed since the plan was made",
                });
            }
            Ok(Some(_)) => Some(()),
            Ok(None) => None,
            Err(e) => return Outcome::Failed(e),
        }
    } else {
        None
    };
    exec.note_new_folder(&item.target, parents);
    let created = match replaced {
        // The file or link being replaced is first moved aside, so a failing `create_dir` puts it back.
        Some(()) => replace_with(exec, item, |dst| {
            dst.create_dir(&item.target, parents).map(drop)
        }),
        None => exec.dst.create_dir(&item.target, parents).map(drop),
    };
    if let Err(e) = created {
        return Outcome::Failed(e);
    }
    if let Some(source) = &item.source {
        let preserve = exec.preserve();
        if preserve.times || preserve.attrs {
            let found = match seen.source.take() {
                Some(entry) => Ok(entry),
                None => exec.src.stat(source, exec.source_follow(item)),
            };
            match found {
                Ok(mut entry) => {
                    narrow_archive_mode(exec, source, &item.target, &mut entry);
                    exec.defer_dir(item.depth, index, item.target.clone(), entry);
                }
                Err(e) => return Outcome::Failed(e),
            }
        }
    }
    Outcome::Done(0)
}

/// The target as the checks of `run_item` saw it, else an `lstat` now.
pub(crate) fn known_target(
    dst: &dyn Vfs,
    target: &Spec,
    seen: &mut Seen,
) -> Result<Option<Entry>, VfsError> {
    match seen.target.take() {
        Some(found) => Ok(found),
        None => lstat_target(dst, target),
    }
}

/// `copy`: one file (D-95), by the path its case takes (§1.6.3).
pub(crate) fn run_copy(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    seen: &mut Seen,
) -> Result<u64, VfsError> {
    let source = item
        .source
        .as_ref()
        .ok_or_else(|| source_changed(&item.target))?;
    let mut entry = source_file(exec, item, source, seen)?;
    let preserve = exec.preserve();
    if !item.replaces
        && preserve.attrs
        && same_instance(exec.src, exec.dst)
        && !exec.create_parents(item.depth)
    {
        let meta = handle_meta(exec, &item.target, &entry, preserve);
        if let Some(bytes) = exec
            .dst
            .copy_new_file(source, &item.target, &meta, exec.ctl.cancel)?
        {
            exec.progress(bytes, &item.target);
            // Set on the handle but for `R`, which a rename could not carry on Windows.
            if entry
                .attrs
                .dos()
                .is_some_and(|attrs| attrs.contains(DosAttrs::READONLY))
            {
                apply_attrs(exec, exec.dst, &item.target, &entry)?;
            }
            return Ok(bytes);
        }
    }
    let durability = if item.replaces {
        Durability::FileOnly
    } else {
        Durability::None
    };
    let bytes = copy_content(exec, item, source, durability, seen)?;
    narrow_archive_mode(exec, source, &item.target, &mut entry);
    apply_metadata(exec, &item.target, &entry)?;
    Ok(bytes)
}

/// The source entry of a file copy: seen by the stamp check, or read now. A link the comparison followed is read through
/// it, and so is a root link (only roots can be links otherwise, rule 7); anything but a file means the source changed.
pub(crate) fn source_file(
    exec: &Exec<'_>,
    item: &PlanItem,
    source: &Spec,
    seen: &mut Seen,
) -> Result<Entry, VfsError> {
    let mut entry = match seen.source.take() {
        Some(entry) => entry,
        None => exec.src.stat(source, exec.source_follow(item))?,
    };
    if entry.kind == EntryKind::Symlink && item.depth == 0 {
        entry = exec.src.stat(source, FollowLinks::Yes)?;
    }
    if entry.kind != EntryKind::File {
        return Err(source_changed(source));
    }
    Ok(entry)
}

/// What [`Vfs::copy_new_file`] sets on the handle, with the warnings of the metadata step.
fn handle_meta(
    exec: &mut Exec<'_>,
    target: &Spec,
    source: &Entry,
    preserve: Preserve,
) -> HandleMeta {
    let caps = exec
        .dst
        .capabilities(target)
        .unwrap_or_else(|_| Capabilities::empty());
    if preserve.times && source.mtime.is_none() {
        warn_no_mtime(&mut exec.warned, &mut exec.warnings, target);
    }
    let mut meta = HandleMeta::default();
    if preserve.times && source.mtime.is_some() {
        if caps.contains(Capabilities::SET_TIMES) {
            meta.modified = source.mtime;
            if preserve.created {
                if caps.contains(Capabilities::SET_CREATED) {
                    meta.created = source.ctime;
                } else {
                    warn_missing(&mut exec.warned, &mut exec.warnings, "set_created", target);
                }
            }
        } else {
            warn_missing(&mut exec.warned, &mut exec.warnings, "set_times", target);
        }
    }
    if let Attrs::Unix { meta: unix } = source.attrs
        && caps.contains(Capabilities::UNIX_MODE)
    {
        meta.mode = Some(unix.mode & 0o7777);
    }
    meta
}

/// True when the item at `spec` carries a DOS `R`.
fn read_only(vfs: &dyn Vfs, spec: &Spec) -> Result<bool, VfsError> {
    Ok(lstat_target(vfs, spec)?
        .and_then(|found| found.attrs.dos())
        .is_some_and(|attrs| attrs.contains(DosAttrs::READONLY)))
}

/// Clears a DOS `R` at `spec`, if there is one: `true` when it was there, so a failure can give it back.
pub(crate) fn clear_read_only_of(vfs: &dyn Vfs, spec: &Spec) -> Result<bool, VfsError> {
    if !read_only(vfs, spec)? {
        return Ok(false);
    }
    let change = AttrChange::Dos {
        set: DosAttrs::empty(),
        clear: DosAttrs::READONLY,
    };
    vfs.set_attrs(spec, &change).map(|()| true)
}

/// Gives a DOS `R` back (best effort: the item is failing already).
pub(crate) fn restore_read_only(vfs: &dyn Vfs, spec: &Spec) {
    let change = AttrChange::Dos {
        set: DosAttrs::READONLY,
        clear: DosAttrs::empty(),
    };
    let _ = vfs.set_attrs(spec, &change);
}

/// Copies an open reader into `writer`, reporting progress at `target` (§1.3.4 step 4). Nothing is committed here.
fn pump(
    exec: &Exec<'_>,
    from: (&dyn Vfs, &Spec),
    target: &Spec,
    writer: &mut Box<dyn AtomicWrite>,
) -> Result<u64, VfsError> {
    let (vfs, source) = from;
    let mut reader = vfs.open_read_with(source, exec.ctl.cancel)?;
    let mut buffer = vec![0u8; StreamOptions::default().effective_buffer_size()];
    let mut copied: u64 = 0;
    loop {
        exec.ctl.check()?;
        let n = match reader.read(&mut buffer) {
            Ok(0) => return Ok(copied),
            Ok(n) => n,
            Err(e) if e.kind() == IoKind::Interrupted => continue,
            Err(e) => return Err(VfsError::from_io(source.canonical(), VfsOp::Read, e)),
        };
        writer
            .write_all(buffer.get(..n).unwrap_or_default())
            .map_err(|e| VfsError::from_io(target.canonical(), VfsOp::Write, e))?;
        copied += n as u64;
        exec.progress(copied, target);
    }
}

/// The hidden copy of an `exchange`: a new file, on the fast path where one back-end has both ends (§1.6.3).
pub(crate) fn copy_file_on(
    exec: &mut Exec<'_>,
    from: (&dyn Vfs, &Spec),
    to: (&dyn Vfs, &Spec),
) -> Result<u64, VfsError> {
    let (from_vfs, source) = from;
    let (to_vfs, target) = to;
    let opts = WriteOptions {
        replace_link: true,
        durability: Durability::FileOnly,
        ..WriteOptions::default()
    };
    if same_instance(from_vfs, to_vfs)
        && let Some(bytes) = to_vfs.copy_file_fast(source, target, &opts, exec.ctl.cancel)?
    {
        exec.progress(bytes, target);
        return Ok(bytes);
    }
    let mut writer = to_vfs.open_write(target, &opts)?;
    let copied = pump(exec, from, target, &mut writer)?;
    exec.ctl.check()?;
    writer.commit()?;
    Ok(copied)
}

/// Copies the content of `source` onto the target of `item` atomically: the fast path within one back-end instance, else
/// the stream (§1.3.4 steps 4–5). An item with a backup always streams, so the replaced file can go aside just before
/// the commit (§1.6.5).
pub(crate) fn copy_content(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    source: &Spec,
    durability: Durability,
    seen: &mut Seen,
) -> Result<u64, VfsError> {
    let target = &item.target;
    let opts = WriteOptions {
        create_parents: exec.create_parents(item.depth),
        overwrite: item.replaces,
        replace_link: true,
        durability,
    };
    let clear = item.clear_read_only && item.replaces && read_only(exec.dst, target)?;
    if item.backup.is_none()
        && !clear
        && same_instance(exec.src, exec.dst)
        && let Some(bytes) = exec
            .dst
            .copy_file_fast(source, target, &opts, exec.ctl.cancel)?
    {
        exec.progress(bytes, target);
        return Ok(bytes);
    }
    let replaced = match &item.backup {
        Some(_) => known_target(exec.dst, target, seen)?,
        None => None,
    };
    let mut writer = exec.dst.open_write(target, &opts)?;
    let copied = pump(exec, (exec.src, source), target, &mut writer)?;
    exec.ctl.check()?;
    match &item.backup {
        Some(backup) => {
            with_backup(exec, target, backup, item.clear_read_only, || {
                writer.commit()
            })?;
            // The writer keeps the mode of a regular file it replaces, and this one has gone aside. What a link or a
            // special file has for a mode is no mode for a file: the new one has the default, as without a backup.
            if let Some(Entry {
                kind: EntryKind::File,
                attrs: Attrs::Unix { meta },
                ..
            }) = replaced
                && !exec.preserve().attrs
            {
                exec.dst
                    .set_attrs(target, &AttrChange::Mode(meta.mode & 0o7777))?;
            }
        }
        None if clear => {
            clear_read_only_of(exec.dst, target)?;
            writer
                .commit()
                .inspect_err(|_| restore_read_only(exec.dst, target))?;
        }
        None => writer.commit()?,
    }
    Ok(copied)
}

/// What replaces the target, with a backup of the old one (§1.6.5): the old item is renamed to its backup just before
/// `commit`, and back when it fails. The target is missing for one rename only. On another disk the backup is a copy, made
/// while the target is intact, and then the target is still there for `commit` to replace, so `clear` takes its `R` first
/// and a failure gives it back. **A failed backup fails the item and the target stays.**
pub(crate) fn with_backup(
    exec: &Exec<'_>,
    target: &Spec,
    backup: &Spec,
    clear: bool,
    commit: impl FnOnce() -> Result<(), VfsError>,
) -> Result<(), VfsError> {
    let dst = exec.dst;
    if let Some(parent) = backup.parent() {
        dst.create_dir(&parent, true)?;
    }
    match dst.rename(target, backup, true) {
        Ok(()) => commit().map_err(|e| match dst.rename(backup, target, true) {
            Ok(()) => e,
            Err(back) => kept_as(target, backup, &e, &back),
        }),
        Err(VfsError::CrossDevice { .. } | VfsError::OperationUnsupported { .. }) => {
            write_backup(dst, target, backup, exec.ctl)?;
            let cleared = clear && clear_read_only_of(dst, target)?;
            commit().inspect_err(|_| {
                if cleared {
                    restore_read_only(dst, target);
                }
            })
        }
        Err(e) => Err(e),
    }
}

/// A replacement failed and the old item could not go back: the message names where it is.
fn kept_as(target: &Spec, kept: &Spec, error: &VfsError, back: &VfsError) -> VfsError {
    VfsError::from_io(
        target.canonical(),
        VfsOp::Rename,
        std::io::Error::other(format!(
            "{error}; the replaced item was kept as {} ({back})",
            kept.canonical()
        )),
    )
}

/// `link`: a new link with the source link's target text (D-94 rule 6).
pub(crate) fn run_link(
    exec: &mut Exec<'_>,
    item: &PlanItem,
    seen: &mut Seen,
) -> Result<(), VfsError> {
    let source = item
        .source
        .as_ref()
        .ok_or_else(|| source_changed(&item.target))?;
    create_link_copy(exec, source, item, seen)
}

/// Creates at the target of `item` a link with the target text of the link `source` (replacing a non-folder target when
/// the item `replaces`).
pub(crate) fn create_link_copy(
    exec: &mut Exec<'_>,
    source: &Spec,
    item: &PlanItem,
    seen: &mut Seen,
) -> Result<(), VfsError> {
    let target = &item.target;
    let entry = match seen.source.take() {
        Some(entry) => entry,
        None => exec.src.stat(source, FollowLinks::No)?,
    };
    if entry.kind != EntryKind::Symlink {
        return Err(source_changed(source));
    }
    let text = super::link_target(&entry).unwrap_or_default();
    let target_is_dir = exec
        .src
        .stat(source, FollowLinks::Yes)
        .is_ok_and(|e| e.kind == EntryKind::Dir);
    if exec.create_parents(item.depth)
        && let Some(parent) = target.parent()
    {
        exec.dst.create_dir(&parent, true)?;
    }
    let text = if text.is_empty() {
        super::empty_os().to_os_string()
    } else {
        text
    };
    let replaced = if item.replaces {
        match known_target(exec.dst, target, seen)? {
            Some(found) if found.kind == EntryKind::Dir => {
                return Err(VfsError::InvalidTarget {
                    spec: target.canonical(),
                    reason: "changed since the plan was made",
                });
            }
            Some(_) => true,
            None => false,
        }
    } else {
        false
    };
    if replaced {
        // The replaced item is moved aside first, so a failing `create_symlink` puts it back.
        replace_with(exec, item, |dst| {
            dst.create_symlink(target, &text, target_is_dir)
        })
    } else {
        exec.dst.create_symlink(target, &text, target_is_dir)
    }
}

/// A free hidden name next to `target`: `name`, then `name-1`, `name-2`, … It does not grow with `target`'s, or a name
/// near the limit of the file system could not have one.
pub(crate) fn free_sibling(dst: &dyn Vfs, target: &Spec, name: &str) -> Result<Spec, VfsError> {
    let Some(parent) = target.parent() else {
        return Err(VfsError::InvalidTarget {
            spec: target.canonical(),
            reason: "needs a file name",
        });
    };
    for attempt in 0..1000u32 {
        let mut text = OsString::from(name);
        if attempt > 0 {
            text.push(format!("-{attempt}"));
        }
        let sibling = dst.child(&parent, &FileName::from_os(&text))?;
        if lstat_target(dst, &sibling)?.is_none() {
            return Ok(sibling);
        }
    }
    Err(VfsError::AlreadyExists {
        spec: target.canonical(),
    })
}

/// Replaces the non-folder item at the target of `item` by what `create` makes there: the item is renamed to a temporary
/// sibling, `create` runs, and the sibling is removed, or becomes the backup of the item; when `create` fails, or the
/// backup does, the sibling is renamed back, so the target is as before (D-95: a failing item leaves its target
/// untouched). Its `R` goes only when the sibling does, so a failure has none to give back, and a backup keeps it.
/// Without `RENAME` on the back-end the item is removed first (no way to keep it aside).
fn replace_with(
    exec: &Exec<'_>,
    item: &PlanItem,
    create: impl FnOnce(&dyn Vfs) -> Result<(), VfsError>,
) -> Result<(), VfsError> {
    let (dst, target) = (exec.dst, &item.target);
    let can_rename = dst
        .capabilities(target)
        .is_ok_and(|caps| caps.contains(Capabilities::RENAME));
    let aside = if can_rename {
        Some(free_sibling(dst, target, ".hikasa-old")?)
    } else {
        None
    };
    let Some(aside) = aside else {
        if let Some(backup) = &item.backup {
            write_backup(dst, target, backup, exec.ctl)?;
        }
        let cleared = item.clear_read_only && clear_read_only_of(dst, target)?;
        dst.remove(target).inspect_err(|_| {
            if cleared {
                restore_read_only(dst, target);
            }
        })?;
        return create(dst);
    };
    dst.rename(target, &aside, false)?;
    let back = |e: VfsError| match dst.rename(&aside, target, false) {
        Ok(()) => e,
        Err(back) => kept_as(target, &aside, &e, &back),
    };
    if let Err(e) = create(dst) {
        return Err(back(e));
    }
    match &item.backup {
        Some(backup) => keep_aside(exec, &aside, backup).map_err(|e| {
            // What `create` made goes, for what was there.
            let _ = dst.remove(target);
            back(e)
        }),
        None => {
            // The new item is in place; the old one is discarded (best effort: a leftover sibling is no loss).
            if item.clear_read_only {
                let _ = clear_read_only_of(dst, &aside);
            }
            let _ = dst.remove(&aside);
            Ok(())
        }
    }
}

/// The hidden sibling of [`replace_with`] becomes the backup; on another disk it is copied there and removed.
fn keep_aside(exec: &Exec<'_>, aside: &Spec, backup: &Spec) -> Result<(), VfsError> {
    let dst = exec.dst;
    if let Some(parent) = backup.parent() {
        dst.create_dir(&parent, true)?;
    }
    match dst.rename(aside, backup, true) {
        Err(VfsError::CrossDevice { .. } | VfsError::OperationUnsupported { .. }) => {
            write_backup(dst, aside, backup, exec.ctl)?;
            let _ = dst.remove(aside);
            Ok(())
        }
        done => done,
    }
}

/// §1.5.7 rule 18: the mode of a member stands for what an archive asks, and an archive is hostile. Outside an archive
/// it keeps `rwx` bits only, and only those the platform gave the new `target`: a file created with the default mode
/// has the umask (and any default ACL) in it, so the umask needs no call and no environment (T1). Such a file was
/// created without `x`, so a reader's `r` stands for its `x`. A target that shows no mode counts as umask 022.
pub(crate) fn narrow_archive_mode(
    exec: &Exec<'_>,
    source: &Spec,
    target: &Spec,
    entry: &mut Entry,
) {
    narrow_mode_on(exec.dst, source, target, entry);
}

/// [`narrow_archive_mode`] of a `target` on any back-end.
fn narrow_mode_on(dst: &dyn Vfs, source: &Spec, target: &Spec, entry: &mut Entry) {
    if !matches!(source, Spec::ArchiveMember { .. }) || matches!(target, Spec::ArchiveMember { .. })
    {
        return;
    }
    if let Attrs::Unix { meta } = &mut entry.attrs {
        let created = dst
            .stat(target, FollowLinks::No)
            .ok()
            .and_then(|entry| entry.attrs.unix())
            .map_or(0o755, |created| created.mode);
        meta.mode &= 0o777 & (created | ((created & 0o444) >> 2));
    }
}

/// Applies the metadata of the operation's [`Preserve`] to `target`.
pub(crate) fn apply_metadata(
    exec: &mut Exec<'_>,
    target: &Spec,
    source: &Entry,
) -> Result<(), VfsError> {
    let preserve = exec.preserve();
    apply_preserved(exec, exec.dst, target, source, preserve)
}

/// The metadata of one hidden copy of an `exchange`, on its own side.
pub(crate) fn apply_metadata_on(
    exec: &mut Exec<'_>,
    from: (&dyn Vfs, &Spec),
    vfs: &dyn Vfs,
    target: &Spec,
    source: &Entry,
) -> Result<(), VfsError> {
    let mut entry = source.clone();
    narrow_mode_on(vfs, from.1, target, &mut entry);
    let preserve = exec.preserve();
    apply_preserved(exec, vfs, target, &entry, preserve)
}

/// Applies the preserved metadata of `source` to `target`: times, then attributes (D-95). A missing capability
/// skips the step with one `capability_missing` warning per operation; a failing call is the item's error.
pub(crate) fn apply_preserved(
    exec: &mut Exec<'_>,
    dst: &dyn Vfs,
    target: &Spec,
    source: &Entry,
    preserve: Preserve,
) -> Result<(), VfsError> {
    let caps = dst
        .capabilities(target)
        .unwrap_or_else(|_| Capabilities::empty());
    if preserve.times && source.mtime.is_none() {
        warn_no_mtime(&mut exec.warned, &mut exec.warnings, target);
    }
    if preserve.times
        && let Some(mtime) = source.mtime
    {
        if caps.contains(Capabilities::SET_TIMES) {
            let created = if preserve.created {
                if caps.contains(Capabilities::SET_CREATED) {
                    source.ctime
                } else {
                    warn_missing(&mut exec.warned, &mut exec.warnings, "set_created", target);
                    None
                }
            } else {
                None
            };
            dst.set_times(
                target,
                &SetTimes {
                    modified: Some(mtime),
                    created,
                },
            )?;
        } else {
            warn_missing(&mut exec.warned, &mut exec.warnings, "set_times", target);
        }
    }
    if preserve.attrs {
        apply_attrs(exec, dst, target, source)?;
    }
    Ok(())
}

/// The attribute step of the metadata (within one family only; between families nothing is mapped).
fn apply_attrs(
    exec: &mut Exec<'_>,
    dst: &dyn Vfs,
    target: &Spec,
    source: &Entry,
) -> Result<(), VfsError> {
    let caps = dst
        .capabilities(target)
        .unwrap_or_else(|_| Capabilities::empty());
    // DOS: the `R` bit only (D-99).
    let change = match source.attrs {
        Attrs::Unix { meta } if caps.contains(Capabilities::UNIX_MODE) => {
            Some(AttrChange::Mode(meta.mode & 0o7777))
        }
        Attrs::Dos { attrs } if caps.contains(Capabilities::DOS_ATTRS) => {
            let set = attrs & DosAttrs::READONLY;
            Some(AttrChange::Dos {
                set,
                clear: DosAttrs::READONLY.difference(set),
            })
        }
        Attrs::Unix { .. } | Attrs::Dos { .. } | Attrs::None => None,
    };
    if let Some(change) = change {
        if caps.contains(Capabilities::SET_ATTRS) {
            dst.set_attrs(target, &change)?;
        } else {
            warn_missing(&mut exec.warned, &mut exec.warnings, "set_attrs", target);
        }
    }
    Ok(())
}
