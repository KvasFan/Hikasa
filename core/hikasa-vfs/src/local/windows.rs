//! Windows metadata mapping (`std::os::windows::fs::MetadataExt::file_attributes`).

use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::Path;

use crate::caps::Capabilities;
use crate::entry::{Attrs, DosAttrs, FileId};

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

/// Read, write, list, seek, mtime, ctime (creation time), DOS attributes, symlinks, local paths.
pub(super) fn capabilities() -> Capabilities {
    Capabilities::READ
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
