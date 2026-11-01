//! Unix metadata mapping (`std::os::unix::fs::MetadataExt`): mode, uid, gid, device and inode; the Unix parts
//! of the mutating primitives (M2 W3): permission bits, symbolic links, creation times (macOS).

use std::ffi::OsStr;
use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use super::AttrFailure;
use crate::caps::Capabilities;
use crate::entry::{Attrs, FileId, UnixMeta};
use crate::vfs::AttrChange;

/// `Attrs::Unix` with the permission bits (`st_mode & 0o7777`), uid and gid.
pub(super) fn attrs(meta: &Metadata) -> Attrs {
    Attrs::Unix {
        meta: UnixMeta {
            mode: meta.mode() & 0o7777,
            uid: meta.uid(),
            gid: meta.gid(),
        },
    }
}

/// Device and inode.
pub(super) fn file_id(meta: &Metadata) -> Option<FileId> {
    Some(FileId {
        dev: meta.dev(),
        ino: meta.ino(),
    })
}

/// Read, write, list, seek, mtime, ctime (birth time where the file system records one), Unix mode and owner,
/// symlinks, local paths; set times, mkdir, remove, rename, set attributes (mode), trash, create symlinks; on
/// macOS also set the creation time.
pub(super) fn capabilities() -> Capabilities {
    let created = if cfg!(target_os = "macos") {
        Capabilities::SET_CREATED
    } else {
        Capabilities::empty()
    };
    created
        | Capabilities::SET_TIMES
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
        | Capabilities::UNIX_MODE
        | Capabilities::UNIX_OWNER
        | Capabilities::SYMLINKS
        | Capabilities::LOCAL_PATH
}

/// Makes `file`, still empty, readable by its owner alone. The result is the mode it had.
pub(super) fn make_private(file: &File) -> Option<Permissions> {
    let made = file.metadata().ok()?.permissions();
    file.set_permissions(Permissions::from_mode(0o600)).ok()?;
    Some(made)
}

/// True when a file with these metadata can be a temporary file of this program: owner-only, as `make_private` left
/// it. A file of the user that happens to have the name is not touched for it.
pub(super) fn looks_private(meta: &Metadata) -> bool {
    meta.mode() & 0o777 == 0o600
}

/// `fs::remove_file`: only Windows has others in the way.
pub(super) fn remove_file(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

/// `fs::rename`: only Windows has others in the way.
pub(super) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// The handle stays open through a rename: nothing here keeps an open file from being renamed.
pub(super) fn hold_through_rename(file: File) -> Option<File> {
    Some(file)
}

/// Makes a rename in `dir` durable (best effort: `fsync` of the folder).
pub(super) fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}

/// Sets the creation time (macOS: `File::set_times` with `FileTimesExt::set_created`); `None` where the platform
/// cannot (the caller checked `SET_CREATED` first).
#[cfg(target_os = "macos")]
pub(super) fn set_created(path: &Path, time: std::time::SystemTime) -> Option<io::Result<()>> {
    use std::os::macos::fs::FileTimesExt;
    let times = fs::FileTimes::new().set_created(time);
    Some(File::open(path).and_then(|file| file.set_times(times)))
}

/// No creation times on this platform.
#[cfg(not(target_os = "macos"))]
pub(super) fn set_created(_path: &Path, _time: std::time::SystemTime) -> Option<io::Result<()>> {
    None
}

/// `Mode` → `fs::set_permissions` (the caller refused links); `Dos` → the wrong family.
pub(super) fn set_attrs(
    path: &Path,
    _meta: &Metadata,
    change: &AttrChange,
) -> Result<(), AttrFailure> {
    match *change {
        AttrChange::Mode(mode) => fs::set_permissions(path, Permissions::from_mode(mode & 0o7777))
            .map_err(AttrFailure::Io),
        AttrChange::Dos { .. } => Err(AttrFailure::WrongFamily),
    }
}

/// Removes a symbolic link (never its target).
pub(super) fn remove_link(path: &Path, _meta: &Metadata) -> io::Result<()> {
    fs::remove_file(path)
}

/// Unix renames onto a link like onto any file.
pub(super) fn is_folder_link(_meta: &Metadata) -> bool {
    false
}

/// `symlink(target, path)`; `target_is_dir` is ignored on Unix.
pub(super) fn symlink(target: &OsStr, path: &Path, _target_is_dir: bool) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}
