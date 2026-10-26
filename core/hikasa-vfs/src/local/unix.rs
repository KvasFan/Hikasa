//! Unix metadata mapping (`std::os::unix::fs::MetadataExt`): mode, uid, gid, device and inode.

use std::fs::{File, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::caps::Capabilities;
use crate::entry::{Attrs, FileId, UnixMeta};

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
/// symlinks, local paths.
pub(super) fn capabilities() -> Capabilities {
    Capabilities::READ
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

/// Makes a rename in `dir` durable (best effort: `fsync` of the folder).
pub(super) fn sync_dir(dir: &Path) {
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
}
