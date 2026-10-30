//! The recycle bin of `LocalVfs` (FC-073 primitive, D-98): the **only** file that calls the `trash` crate
//! (architecture check 2).
//!
//! `trash::delete` canonicalizes the parent folder only, so on the same file system a symbolic link is trashed
//! as a link and its target stays. Across file systems the crate copies the item into the trash and removes the
//! original, and its copy of a link copies what the link points to (or fails half-way for a link to a folder):
//! so on the freedesktop platforms a **link** whose file system differs from the one of the home trash folder is
//! refused (`Unavailable`), never trashed as a copy and never deleted (§1.3.12). It never falls back to a
//! permanent delete: any failure is an error and the item stays where it was. The crate reads the home and data
//! folders from the environment and the clock (the freedesktop `DeletionDate`) inside the call; the link check
//! reads the same two variables here. Both are side effects of the OS operation, not inputs of any result,
//! accepted by D-98 and confined to this file (T1).

use std::fs::Metadata;
use std::io;
use std::path::Path;

use crate::error::{VfsError, VfsOp};

/// Moves `path` (absolute, existing, `meta` its own `lstat`) to the platform's recycle bin; `spec` names it in
/// errors.
pub(super) fn move_to_trash(path: &Path, meta: &Metadata, spec: &str) -> Result<(), VfsError> {
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    ))]
    if meta.file_type().is_symlink()
        && let Some(trash) = freedesktop::home_trash()
        && freedesktop::link_crosses_device(meta, &trash)
    {
        return Err(VfsError::Unavailable {
            spec: spec.to_owned(),
            reason:
                "a symbolic link on another file system than the recycle bin cannot be moved there \
                     as a link; nothing was changed"
                    .to_owned(),
        });
    }
    #[cfg(not(all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )))]
    let _ = meta;
    trash::delete(path).map_err(|e| {
        let kind = match &e {
            trash::Error::CouldNotAccess { .. } => io::ErrorKind::NotFound,
            trash::Error::TargetedRoot => io::ErrorKind::InvalidInput,
            _ => io::ErrorKind::Other,
        };
        VfsError::from_io(
            spec,
            VfsOp::Trash,
            io::Error::new(kind, format!("cannot move to the recycle bin: {e}")),
        )
    })
}

/// The freedesktop home trash and the device check of links.
#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
pub(super) mod freedesktop {
    use std::fs::{self, Metadata};
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    /// The home trash folder as the `trash` crate finds it: `$XDG_DATA_HOME/Trash`, else
    /// `$HOME/.local/share/Trash` (empty values ignored); `None` when neither is set (the crate then fails).
    pub(in crate::local) fn home_trash() -> Option<PathBuf> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        if let Some(data) = var("XDG_DATA_HOME") {
            return Some(PathBuf::from(data).join("Trash"));
        }
        var("HOME").map(|home| PathBuf::from(home).join(".local/share/Trash"))
    }

    /// True when the link described by `link_meta` lives on another device than `trash` (or the nearest
    /// existing folder above it, followed as the crate canonicalizes it). An unknown device counts as different.
    pub(in crate::local) fn link_crosses_device(link_meta: &Metadata, trash: &Path) -> bool {
        let mut cur = Some(trash);
        while let Some(path) = cur {
            if let Ok(meta) = fs::metadata(path) {
                return meta.dev() != link_meta.dev();
            }
            cur = path.parent();
        }
        true
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::freedesktop::link_crosses_device;

    #[test]
    fn a_link_and_a_trash_on_different_devices_cross() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("l");
        std::os::unix::fs::symlink("nowhere", &link).unwrap();
        let meta = std::fs::symlink_metadata(&link).unwrap();
        // The same file system (a trash folder that does not exist yet: its nearest existing parent counts).
        assert!(!link_crosses_device(&meta, &dir.path().join("data/Trash")));
        // `/proc` is always another file system than a tempdir.
        assert!(link_crosses_device(
            &meta,
            std::path::Path::new("/proc/hikasa-none/Trash")
        ));
    }
}
