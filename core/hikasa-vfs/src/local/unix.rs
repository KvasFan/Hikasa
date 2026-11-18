//! Unix metadata mapping (`std::os::unix::fs::MetadataExt`): mode, uid, gid, device and inode; the Unix parts
//! of the mutating primitives (M2 W3): permission bits, symbolic links, creation times (macOS).

use std::ffi::OsStr;
use std::fs::{self, File, FileTimes, Metadata, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use super::AttrFailure;
use crate::caps::Capabilities;
use crate::entry::{Attrs, FileId, SpecialKind, UnixMeta};
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

/// What a special file is.
pub(super) fn special(meta: &Metadata) -> Option<SpecialKind> {
    let kind = meta.file_type();
    if kind.is_block_device() {
        Some(SpecialKind::Block)
    } else if kind.is_char_device() {
        Some(SpecialKind::Char)
    } else if kind.is_fifo() {
        Some(SpecialKind::Fifo)
    } else if kind.is_socket() {
        Some(SpecialKind::Socket)
    } else {
        None
    }
}

/// Unix modes have no DOS bits: `set_attrs` takes a mode here, so the DOS set is not consulted.
pub(super) const SETTABLE_DOS_ATTRS: crate::entry::DosAttrs = crate::entry::DosAttrs::all();

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

/// A new file, owner-only from the start.
pub(super) fn open_new(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Gives an open file its final mode.
pub(super) fn set_mode(file: &File, mode: Option<u32>) -> io::Result<()> {
    match mode {
        Some(mode) => file.set_permissions(Permissions::from_mode(mode & 0o7777)),
        None => Ok(()),
    }
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

/// [`super::rename_new`]'s call: nothing to wait for here.
pub(super) fn persist_new(from: &Path, to: &Path) -> io::Result<()> {
    super::persist_new(from, to)
}

/// [`super::rename_free`]'s call: the kernel's no-replace rename, through `rustix` (no `unsafe` here): Linux
/// `renameat2` with `RENAME_NOREPLACE`, macOS `renameatx_np` with `RENAME_EXCL`. `EEXIST` is `AlreadyExists`. A file
/// system or kernel without it (`EINVAL`, `ENOSYS`, `ENOTSUP`/`EOPNOTSUPP`), and the other Unix systems, answer
/// `Unsupported`, and [`super::rename_free`] looks again and renames; never a hard link and an unlink, which could
/// leave both names.
pub(super) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    ))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        use rustix::io::Errno;
        match renameat_with(CWD, from, CWD, to, RenameFlags::NOREPLACE) {
            Ok(()) => return Ok(()),
            Err(e)
                if e == Errno::INVAL
                    || e == Errno::NOSYS
                    || e == Errno::NOTSUP
                    || e == Errno::OPNOTSUPP => {}
            Err(e) => return Err(e.into()),
        }
    }
    let _ = (from, to);
    Err(io::ErrorKind::Unsupported.into())
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

/// Adds the creation time to what a handle sets (macOS).
#[cfg(target_os = "macos")]
pub(super) fn with_created(times: FileTimes, created: std::time::SystemTime) -> FileTimes {
    use std::os::macos::fs::FileTimesExt;
    times.set_created(created)
}

/// No creation times on this platform.
#[cfg(not(target_os = "macos"))]
pub(super) fn with_created(times: FileTimes, _created: std::time::SystemTime) -> FileTimes {
    times
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

/// Write access for this process (`access(W_OK)`): `Some(false)` on EACCES, EPERM and EROFS, `None` on any other
/// error (the save decides).
pub(super) fn writable(path: &Path) -> Option<bool> {
    use rustix::fs::{Access, access};
    access_verdict(access(path, Access::WRITE_OK))
}

/// The verdict of an `access(W_OK)` result: a read-only file system (EROFS) is read-only like a missing
/// permission (EACCES, EPERM); any other error cannot tell.
fn access_verdict(result: rustix::io::Result<()>) -> Option<bool> {
    use rustix::io::Errno;
    match result {
        Ok(()) => Some(true),
        Err(e) if e == Errno::ACCESS || e == Errno::PERM || e == Errno::ROFS => Some(false),
        Err(_) => None,
    }
}

// ---- saves (M2 W10b, F2) -----------------------------------------------------------------------------------------

/// The names a file has (`st_nlink`).
pub(super) fn links(meta: &Metadata) -> u64 {
    meta.nlink()
}

/// The file's owner.
pub(super) fn owner(meta: &Metadata) -> Option<u32> {
    Some(meta.uid())
}

/// The user a file this process creates belongs to: the effective user.
pub(super) fn saver() -> Option<u32> {
    Some(rustix::process::geteuid().as_raw())
}

/// Whether this process can make a file in `dir` (`access(W_OK)`); a folder that cannot tell counts as writable, and
/// making the file decides.
pub(super) fn folder_writable(dir: &Path) -> bool {
    writable(dir) != Some(false)
}

/// No DOS attributes on Unix.
pub(super) fn dos_attributes(_meta: &Metadata) -> u32 {
    0
}

/// No DOS attributes on Unix.
pub(super) fn temp_attributes(_options: &mut OpenOptions, _attributes: u32) {}

/// What the new file of a save takes over from the regular file `old` it replaces, on its handle before the rename
/// (M2 W10b, F2): the owner and group where they differ and this process may set them (`fchown`; a group the user is in
/// needs no privilege), then every extended attribute the process may set (`user.*`, `security.*` such as an SELinux
/// label, `trusted.*` for a privileged one, and on Linux the POSIX ACLs, which are the attributes
/// `system.posix_acl_access` and `system.posix_acl_default`). `false` when the new file would lose something that
/// matters: an owner or group it cannot take, or an ACL; the save then writes in place. Other attributes are best effort
/// (a label the policy does not let this process set stays the folder's default one).
pub(super) fn take_identity(new: &File, old: &Path, old_meta: &Metadata) -> bool {
    let Ok(now) = new.metadata() else {
        return false;
    };
    let owner = (now.uid() != old_meta.uid()).then(|| rustix::fs::Uid::from_raw(old_meta.uid()));
    let group = (now.gid() != old_meta.gid()).then(|| rustix::fs::Gid::from_raw(old_meta.gid()));
    if (owner.is_some() || group.is_some()) && rustix::fs::fchown(new, owner, group).is_err() {
        return false;
    }
    copy_xattrs(new, old)
}

/// The prefix of the attributes that hold a POSIX ACL on Linux (`system.posix_acl_access`, `…_default`).
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
const ACL_PREFIX: &[u8] = b"system.posix_acl_";

/// [`take_identity`]'s extended attributes: `false` when an ACL could not be set.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn copy_xattrs(new: &File, old: &Path) -> bool {
    use rustix::fs::{XattrFlags, fsetxattr, getxattr, listxattr};
    let Some(names) = xattr_buffer(|buffer| listxattr(old, buffer)) else {
        // No attributes, or a file system without them: nothing to keep.
        return true;
    };
    let mut kept = true;
    for name in names
        .split(|&byte| byte == 0)
        .filter(|name| !name.is_empty())
    {
        let Ok(text) = std::str::from_utf8(name) else {
            continue;
        };
        let Some(value) = xattr_buffer(|buffer| getxattr(old, text, buffer)) else {
            continue;
        };
        if fsetxattr(new, text, &value, XattrFlags::empty()).is_err()
            && name.starts_with(ACL_PREFIX)
        {
            kept = false;
        }
    }
    kept
}

/// No extended attributes here.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
)))]
fn copy_xattrs(_new: &File, _old: &Path) -> bool {
    true
}

/// The bytes a list or get call of extended attributes gives, growing the buffer while it is too small (`ERANGE`).
/// `None` on any other error.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios"
))]
fn xattr_buffer(call: impl Fn(&mut [u8]) -> rustix::io::Result<usize>) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 1024];
    // A value or a list is at most 64 KiB on Linux; four rounds reach that, and the rest is for other systems.
    for _ in 0..8 {
        match call(&mut buffer) {
            Ok(len) => {
                buffer.truncate(len);
                return Some(buffer);
            }
            Err(e) if e == rustix::io::Errno::RANGE => {
                let grown = buffer.len() * 4;
                buffer.resize(grown, 0);
            }
            Err(_) => return None,
        }
    }
    None
}

#[cfg(test)]
mod writable_tests {
    use rustix::io::Errno;

    use super::access_verdict;

    /// A read-only mount cannot be made in a test, so the errno table is checked directly.
    #[test]
    fn a_read_only_file_system_reads_as_read_only() {
        assert_eq!(access_verdict(Ok(())), Some(true));
        for e in [Errno::ROFS, Errno::ACCESS, Errno::PERM] {
            assert_eq!(access_verdict(Err(e)), Some(false), "{e:?}");
        }
        for e in [Errno::NOENT, Errno::IO, Errno::TXTBSY, Errno::INVAL] {
            assert_eq!(access_verdict(Err(e)), None, "{e:?}");
        }
    }
}
