//! Backups (D-167, D-168; §1.6.5): **a backup keeps what a copy or move replaces.** The core owns the name, the session
//! the choice: [`BackupPolicy`] names the backup of an item, [`BackupNames`] keeps the names of one plan apart, and
//! [`write_backup`] is the copy for what cannot be renamed aside (another disk, an archive).

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind as IoKind, Read, Write};
use std::path::Path;

use hikasa_core::{Ctl, Phase, Progress, RelPath, Spec, Unit};

use super::copy::free_sibling;
use super::{lstat_target, spec_key};
use crate::entry::{Attrs, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::stream::StreamOptions;
use crate::vfs::{AttrChange, Durability, FollowLinks, ReadSeek, SetTimes, Vfs, WriteOptions};

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
    /// as the item itself (`abc.bak` with `stem_bak`) is numbered: a backup must not be what it keeps.
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
        if self.style == BackupStyle::Folder {
            let mut at = self.folder.clone().ok_or_else(|| VfsError::InvalidTarget {
                spec: item.canonical(),
                reason: "needs a backup folder",
            })?;
            match rel.filter(|rel| !rel.is_root()) {
                Some(rel) => {
                    for component in rel.components() {
                        at = vfs.child(&at, &FileName::new(component)?)?;
                    }
                    return Ok(at);
                }
                None => return vfs.child(&at, &FileName::from_os(&name)),
            }
        }
        let parent = item.parent().ok_or_else(unnamed)?;
        let wanted = self.style.sibling_name(&name);
        let wanted = if wanted == name {
            numbered(&name, 2)
        } else {
            wanted
        };
        vfs.child(&parent, &FileName::from_os(&wanted))
    }
}

/// The names of the backups of one plan. Names are final at planning, so a dry run shows them and two items can be told
/// apart: one a previous item took gets `-2`, `-3` … before its extension, in plan order.
#[derive(Debug, Default)]
pub struct BackupNames {
    taken: BTreeSet<String>,
}

impl BackupNames {
    /// The backup of `item` in this plan. A folder at that name → `IsADirectory`: on disk one generation is kept, and an
    /// existing file or link is replaced, but a folder is never removed to make room.
    pub fn assign(
        &mut self,
        policy: &BackupPolicy,
        vfs: &dyn Vfs,
        item: &Spec,
        rel: Option<&RelPath>,
    ) -> Result<Spec, VfsError> {
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
    ctl.check()?;
    let entry = vfs.stat(item, FollowLinks::No)?;
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
