//! Backups (D-167, D-168; §1.6.5): **a backup keeps what a copy or move replaces.** The core owns the name, the session
//! the choice: [`BackupPolicy`] names the backup of an item, [`BackupNames`] keeps the names of one plan apart, and
//! [`write_backup`] is the copy for what cannot be renamed aside (another disk, an archive).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind as IoKind, Read, Write};
use std::path::Path;

use hikasa_core::{Ctl, Phase, Progress, RelPath, Spec, Unit};

use super::copy::free_sibling;
use super::{Action, PlanItem, hard_linked, lstat_target, one_file, same_instance, spec_key};
use crate::entry::{Attrs, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::stream::StreamOptions;
use crate::vfs::{AttrChange, Durability, FollowLinks, ReadSeek, SetTimes, Vfs, WriteOptions};

/// The reason of a backup that would be its own item.
const THE_ITEM: &str = "is the item it backs up";
/// The reason of a backup that would take a path the plan names.
const IN_THE_PLAN: &str = "is a path of another item of the plan";

/// How a backup is named (the table of §1.6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupStyle {
    /// `abc.txt~`.
    NameTilde,
    /// `abc~.txt`.
    StemTilde,
    /// `abc.txt.bak`.
    NameBak,
    /// `~abc.txt`.
    TildeName,
    /// `abc.bak`.
    StemBak,
    /// `abc.bak.txt`.
    StemBakExt,
    /// `<folder>/<rel>/abc.txt`.
    Folder,
}

/// Where the backups of an operation go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupPolicy {
    /// The form of the name.
    pub style: BackupStyle,
    /// The folder of [`BackupStyle::Folder`].
    pub folder: Option<Spec>,
}

/// The final component of `spec` as an OS string.
fn name_of(spec: &Spec) -> Option<OsString> {
    match spec {
        Spec::Local(path) => path.file_name().map(OsStr::to_os_string),
        other => other.file_name().map(OsString::from),
    }
}

/// The extension of a name as D-07 has it, a `.` that is neither first nor last, the last one counting; and the rest. On
/// the OS string, so a name that is not UTF-8 keeps its bytes.
fn split_name(name: &OsStr) -> (OsString, Option<OsString>) {
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) if !extension.is_empty() => {
            (stem.to_os_string(), Some(extension.to_os_string()))
        }
        _ => (name.to_os_string(), None),
    }
}

/// Joins texts into one OS string.
fn join(parts: &[&OsStr]) -> OsString {
    let mut out = OsString::new();
    for part in parts {
        out.push(part);
    }
    out
}

/// `name` with `-<number>` before its extension.
fn numbered(name: &OsStr, number: u32) -> OsString {
    let (stem, extension) = split_name(name);
    let suffix = OsString::from(format!("-{number}"));
    match extension {
        Some(extension) => join(&[&stem, &suffix, OsStr::new("."), &extension]),
        None => join(&[&stem, &suffix]),
    }
}

impl BackupStyle {
    /// The name of the backup of `name`, for every style that keeps it next to the file.
    fn sibling_name(self, name: &OsStr) -> OsString {
        let (stem, extension) = split_name(name);
        let tail = |middle: &str| match &extension {
            Some(extension) => join(&[&stem, OsStr::new(middle), OsStr::new("."), extension]),
            None => join(&[&stem, OsStr::new(middle)]),
        };
        match self {
            BackupStyle::NameTilde => join(&[name, OsStr::new("~")]),
            BackupStyle::StemTilde => tail("~"),
            BackupStyle::NameBak => join(&[name, OsStr::new(".bak")]),
            BackupStyle::TildeName => join(&[OsStr::new("~"), name]),
            BackupStyle::StemBak => join(&[&stem, OsStr::new(".bak")]),
            BackupStyle::StemBakExt => tail(".bak"),
            BackupStyle::Folder => name.to_os_string(),
        }
    }
}

impl BackupPolicy {
    /// The backup of `item`: a sibling, or `<folder>/<rel>`; `rel` is the path below the base (`None`: the file name, as
    /// a save of W10 uses it). `Folder` without a folder → `InvalidTarget "needs a backup folder"`. A name that comes out
    /// as the item itself, in any style (`abc.bak` with `stem_bak`, the item's own folder with `folder`), is numbered: a
    /// backup must not be what it keeps.
    pub fn target(
        &self,
        vfs: &dyn Vfs,
        item: &Spec,
        rel: Option<&RelPath>,
    ) -> Result<Spec, VfsError> {
        let unnamed = || VfsError::InvalidTarget {
            spec: item.canonical(),
            reason: "needs a file name",
        };
        let name = name_of(item).ok_or_else(unnamed)?;
        let wanted = if self.style == BackupStyle::Folder {
            let mut at = self.folder.clone().ok_or_else(|| VfsError::InvalidTarget {
                spec: item.canonical(),
                reason: "needs a backup folder",
            })?;
            match rel.filter(|rel| !rel.is_root()) {
                Some(rel) => {
                    for component in rel.components() {
                        at = vfs.child(&at, &FileName::new(component)?)?;
                    }
                    at
                }
                None => vfs.child(&at, &FileName::from_os(&name))?,
            }
        } else {
            let parent = item.parent().ok_or_else(unnamed)?;
            vfs.child(&parent, &FileName::from_os(&self.style.sibling_name(&name)))?
        };
        if spec_key(vfs, &wanted) != spec_key(vfs, item) {
            return Ok(wanted);
        }
        let parent = wanted.parent().ok_or_else(unnamed)?;
        vfs.child(&parent, &FileName::from_os(&numbered(&name, 2)))
    }
}

/// The names of the backups of one plan. Names are final at planning, so a dry run shows them and two items can be told
/// apart: one that another path of the plan has gets `-2`, `-3` … before its extension. A planner [`reserve`]s
/// every path the plan writes or removes first, or `x.bak` of a backup can be the target of the next row.
///
/// [`reserve`]: BackupNames::reserve
#[derive(Debug, Default)]
pub struct BackupNames {
    taken: BTreeSet<String>,
}

impl BackupNames {
    /// Keeps a path the plan writes, removes or reads from every backup of the plan.
    pub fn reserve(&mut self, vfs: &dyn Vfs, spec: &Spec) {
        self.taken.insert(spec_key(vfs, spec));
    }

    /// The backup of `item` in this plan. A folder at that name → `IsADirectory`: on disk one generation is kept, and an
    /// existing file or link is replaced, but a folder is never removed to make room.
    pub fn assign(
        &mut self,
        policy: &BackupPolicy,
        vfs: &dyn Vfs,
        item: &Spec,
        rel: Option<&RelPath>,
    ) -> Result<Spec, VfsError> {
        self.reserve(vfs, item);
        let first = policy.target(vfs, item, rel)?;
        let mut name = first.clone();
        let mut number = 1;
        while self.taken.contains(&spec_key(vfs, &name)) {
            number += 1;
            let (Some(parent), Some(text)) = (first.parent(), name_of(&first)) else {
                return Err(VfsError::InvalidTarget {
                    spec: first.canonical(),
                    reason: "needs a file name",
                });
            };
            name = vfs.child(&parent, &FileName::from_os(&numbered(&text, number)))?;
        }
        if lstat_target(vfs, &name)?.is_some_and(|found| found.kind == EntryKind::Dir) {
            return Err(VfsError::IsADirectory {
                spec: name.canonical(),
            });
        }
        self.taken.insert(spec_key(vfs, &name));
        Ok(name)
    }
}

/// What a backup of the plan would do to the plan itself, with the index of the item it fails: a backup that is its own
/// item, or the target or source of any item of the plan, or another item's backup, or an archive's backup that is the
/// archive or such a path. Each of these loses a file, so it is refused for a hand-built plan too, before any item runs.
pub(crate) fn clashes(
    src: &dyn Vfs,
    dst: &dyn Vfs,
    items: &[PlanItem],
    archive_backups: &BTreeMap<String, Spec>,
) -> Vec<(usize, VfsError)> {
    if archive_backups.is_empty() && items.iter().all(|item| item.backup.is_none()) {
        return Vec::new();
    }
    let one = same_instance(src, dst);
    let mut named = BTreeSet::new();
    for item in items {
        let removed = matches!(
            item.action,
            Action::Remove | Action::RemoveDir | Action::Trash
        );
        if one || !removed {
            named.insert(spec_key(dst, &item.target));
        }
        if let Some(source) = item.source.as_ref().filter(|_| one) {
            named.insert(spec_key(dst, source));
        }
    }
    let refused = |backup: &Spec, reason: &'static str| VfsError::InvalidTarget {
        spec: backup.canonical(),
        reason,
    };
    let mut found = Vec::new();
    let mut backups = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        if item.error.is_some() || item.skip.is_some() {
            continue;
        }
        if let Some(backup) = &item.backup {
            let key = spec_key(dst, backup);
            if key == spec_key(dst, &item.target) {
                found.push((index, refused(backup, THE_ITEM)));
            } else if named.contains(&key) || !backups.insert(key) {
                found.push((index, refused(backup, IN_THE_PLAN)));
            }
        }
    }
    let mut owners = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let outers = [item.target.outermost_file(), item.source.as_ref().and_then(Spec::outermost_file)];
        for outer in outers.into_iter().flatten() {
            let Some(backup) = archive_backups.get(&outer.canonical()) else {
                continue;
            };
            if !owners.insert(outer.canonical()) {
                continue;
            }
            let key = spec_key(dst, backup);
            if key == spec_key(dst, outer) {
                found.push((index, refused(backup, THE_ITEM)));
            } else if named.contains(&key) || !backups.insert(key) {
                found.push((index, refused(backup, IN_THE_PLAN)));
            }
        }
    }
    found
}

/// Copies an open source into `writer`, with progress in bytes of `size` and a cancellation check per block.
fn stream(
    reader: &mut Box<dyn ReadSeek>,
    writer: &mut dyn Write,
    item: &Spec,
    backup: &Spec,
    size: u64,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    let mut buffer = vec![0u8; StreamOptions::default().effective_buffer_size()];
    let mut done = 0;
    loop {
        ctl.check()?;
        let n = match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == IoKind::Interrupted => continue,
            Err(e) => return Err(VfsError::from_io(item.canonical(), VfsOp::Read, e)),
        };
        writer
            .write_all(buffer.get(..n).unwrap_or_default())
            .map_err(|e| VfsError::from_io(backup.canonical(), VfsOp::Write, e))?;
        done += n as u64;
        report(ctl, done, size, backup);
    }
}

/// `Phase::Writing` progress of a backup.
fn report(ctl: &Ctl<'_>, done: u64, size: u64, backup: &Spec) {
    ctl.progress(Phase::Writing, || Progress {
        phase: Phase::Writing,
        done,
        total: Some(size),
        unit: Unit::Bytes,
        current: Some(backup.canonical()),
    });
}

/// A copy of `item` at `backup`, parents created, replacing a backup that is there (atomic, `Durability::FileOnly`), with
/// `item`'s time and mode; a link stays a link. Its size is reported in `Phase::Writing`.
pub fn write_backup(
    vfs: &dyn Vfs,
    item: &Spec,
    backup: &Spec,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    copy_aside(vfs, item, FollowLinks::No, backup, ctl)
}

/// The backup of an archive (D-168): the bytes of the file a rebuild rewrites, through a link that leads to it, which a
/// link at the backup would not keep. One that is that file itself keeps nothing → `InvalidTarget`.
pub(crate) fn write_archive_backup(
    vfs: &dyn Vfs,
    outer: &Spec,
    backup: &Spec,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    if one_file(vfs, vfs, outer, backup) && !hard_linked(vfs, vfs, outer, backup) {
        return Err(VfsError::InvalidTarget {
            spec: backup.canonical(),
            reason: THE_ITEM,
        });
    }
    copy_aside(vfs, outer, FollowLinks::Yes, backup, ctl)
}

/// [`write_backup`], reading `item` by `follow`.
fn copy_aside(
    vfs: &dyn Vfs,
    item: &Spec,
    follow: FollowLinks,
    backup: &Spec,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    ctl.check()?;
    let entry = vfs.stat(item, follow)?;
    if let Some(parent) = backup.parent() {
        vfs.create_dir(&parent, true)?;
    }
    match entry.kind {
        EntryKind::File => {}
        EntryKind::Symlink => {
            let text = entry.link_target_os().unwrap_or_default();
            let is_dir = vfs
                .stat(item, FollowLinks::Yes)
                .is_ok_and(|e| e.kind == EntryKind::Dir);
            let temporary = free_sibling(vfs, backup, ".hikasa-tmp")?;
            vfs.create_symlink(&temporary, &text, is_dir)?;
            return vfs.rename(&temporary, backup, true).inspect_err(|_| {
                let _ = vfs.remove(&temporary);
            });
        }
        EntryKind::Dir => {
            return Err(VfsError::IsADirectory {
                spec: item.canonical(),
            });
        }
        EntryKind::Other => {
            return Err(VfsError::Unsupported {
                spec: item.canonical(),
                what: "backing up a special file",
            });
        }
    }
    let options = WriteOptions {
        create_parents: false,
        overwrite: true,
        replace_link: true,
        durability: Durability::FileOnly,
    };
    match vfs.copy_file_fast(item, backup, &options, ctl.cancel)? {
        Some(_) => report(ctl, entry.size, entry.size, backup),
        None => {
            let mut writer = vfs.open_write(backup, &options)?;
            let mut reader = vfs.open_read_with(item, ctl.cancel)?;
            stream(&mut reader, &mut writer, item, backup, entry.size, ctl)?;
            ctl.check()?;
            writer.commit()?;
        }
    }
    // What a rename would have kept: on a back-end without these, the content is what a backup is for.
    if entry.mtime.is_some() {
        let times = SetTimes {
            modified: entry.mtime,
            created: None,
        };
        vfs.set_times(backup, &times)?;
    }
    if let Attrs::Unix { meta } = entry.attrs {
        vfs.set_attrs(backup, &AttrChange::Mode(meta.mode & 0o7777))?;
    }
    Ok(())
}
