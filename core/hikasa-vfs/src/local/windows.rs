//! Windows metadata mapping (`std::os::windows::fs::MetadataExt::file_attributes`); the Windows parts of the
//! mutating primitives (M2 W3): the `R` attribute, folder links and junctions, creation times.

use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata, OpenOptions};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use super::AttrFailure;
use crate::caps::Capabilities;
use crate::entry::{Attrs, DosAttrs, FileId};
use crate::vfs::AttrChange;

/// `FILE_FLAG_BACKUP_SEMANTICS`: needed to open a folder.
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// `FILE_FLAG_OPEN_REPARSE_POINT`: open a link itself.
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// `FILE_WRITE_ATTRIBUTES`.
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
/// `FILE_ATTRIBUTE_DIRECTORY`.
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0010;

/// `Attrs::Dos` with the attribute bits that have a letter (D-30).
pub(super) fn attrs(meta: &Metadata) -> Attrs {
    Attrs::Dos {
        attrs: DosAttrs::from_bits_truncate(meta.file_attributes()),
    }
}

/// No file ids on Windows in M1 (loop detection uses `same-file` handles, D-33).
pub(super) fn file_id(_meta: &Metadata) -> Option<FileId> {
    None
}

/// Read, write, list, seek, mtime, ctime (creation time), DOS attributes, symlinks, local paths; set times
/// (modification and creation), mkdir, remove, rename, set attributes (`R` only, D-99), trash, create symlinks
/// (which may still fail without the privilege).
pub(super) fn capabilities() -> Capabilities {
    Capabilities::SET_TIMES
        | Capabilities::SET_CREATED
        | Capabilities::MKDIR
        | Capabilities::REMOVE
        | Capabilities::RENAME
        | Capabilities::SET_ATTRS
        | Capabilities::TRASH
        | Capabilities::CREATE_SYMLINKS
        | Capabilities::READ
        | Capabilities::WRITE
        | Capabilities::LIST
        | Capabilities::SEEK
        | Capabilities::MTIME
        | Capabilities::CTIME
        | Capabilities::DOS_ATTRS
        | Capabilities::SYMLINKS
        | Capabilities::LOCAL_PATH
}

/// Folders cannot be synced on Windows; `MoveFileEx` is already durable enough for M1.
pub(super) fn sync_dir(_dir: &Path) {}

/// A file's ACL comes from its folder: there is no owner-only mode to give it.
pub(super) fn make_private(_file: &std::fs::File) -> Option<std::fs::Permissions> {
    None
}

/// No mode to go by: the reserved name and the lock decide.
pub(super) fn looks_private(_meta: &Metadata) -> bool {
    true
}

/// Sets the creation time through a handle opened for `FILE_WRITE_ATTRIBUTES` (folders with backup semantics).
pub(super) fn set_created(path: &Path, time: std::time::SystemTime) -> Option<io::Result<()>> {
    use std::os::windows::fs::FileTimesExt;
    let times = fs::FileTimes::new().set_created(time);
    Some(
        OpenOptions::new()
            .access_mode(FILE_WRITE_ATTRIBUTES)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .and_then(|file| file.set_times(times)),
    )
}

/// `Dos` with `R` only → `Permissions::set_readonly`; other bits are refused (they need Win32 calls, D-99);
/// `Mode` → the wrong family.
pub(super) fn set_attrs(
    path: &Path,
    meta: &Metadata,
    change: &AttrChange,
) -> Result<(), AttrFailure> {
    match *change {
        AttrChange::Dos { set, clear } => {
            if !(set | clear).difference(DosAttrs::READONLY).is_empty() {
                return Err(AttrFailure::OtherDosBits);
            }
            let mut permissions = meta.permissions();
            if set.contains(DosAttrs::READONLY) {
                permissions.set_readonly(true);
            } else if clear.contains(DosAttrs::READONLY) {
                #[expect(
                    clippy::permissions_set_readonly_false,
                    reason = "this file is Windows only, where the call clears the DOS bit"
                )]
                permissions.set_readonly(false);
            } else {
                return Ok(());
            }
            fs::set_permissions(path, permissions).map_err(AttrFailure::Io)
        }
        AttrChange::Mode(_) => Err(AttrFailure::WrongFamily),
    }
}

/// Removes a link itself: a folder link or junction with `remove_dir`, a file link with `remove_file`.
pub(super) fn remove_link(path: &Path, meta: &Metadata) -> io::Result<()> {
    if meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

/// True for a link whose entry is a folder entry (a folder link or junction).
pub(super) fn is_folder_link(meta: &Metadata) -> bool {
    meta.file_type().is_symlink() && meta.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// `symlink_dir` when `target_is_dir`, else `symlink_file`. The target is stored with `\`: the OS keeps a `/`
/// verbatim, and a link holding one resolves to nothing (`ERROR_INVALID_NAME`).
pub(super) fn symlink(target: &OsStr, path: &Path, target_is_dir: bool) -> io::Result<()> {
    let separators = target
        .encode_wide()
        .map(|unit| if unit == 0x2F { 0x5C } else { unit });
    let target = OsString::from_wide(&separators.collect::<Vec<u16>>());
    if target_is_dir {
        std::os::windows::fs::symlink_dir(target, path)
    } else {
        std::os::windows::fs::symlink_file(target, path)
    }
}
