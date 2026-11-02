//! [`LocalVfs`]: the local file system back-end (`Spec::Local` only).
//!
//! - **Listings** honour the prefilter (D-45): the kind comes from the directory record (`DirEntry::file_type`,
//!   which reads `d_type` on Unix and the find data on Windows). When the record does not state the kind, std
//!   falls back to an `lstat` of its own and the prefilter receives the kind found (an accepted deviation from
//!   "unknown kinds are never passed to the prefilter", contract §1.2.7: std has no API that reads `d_type`
//!   without that fallback, and the verdicts are the same); an entry whose kind still cannot be determined is
//!   stat'ed fully and never passed to the prefilter. Symlinks are always stat'ed.
//! - **Following links.** With [`FollowLinks::Yes`] a symlink is described by its target (`followed = true`,
//!   `link_target` = the link text). A broken link (or a link loop) is reported unfollowed in
//!   `Listing::entries` **and** with an [`EntryError`] in `Listing::errors`; [`Vfs::stat`] of a broken link
//!   with `FollowLinks::Yes` fails.
//! - **Link targets** come from `read_link`: `link_target` is their UTF-8 text (lossy when needed, with the
//!   exact target in `link_target_raw`), and an unfollowed link's `size` is the byte length of the exact
//!   target on every platform.
//! - **Atomic writes** (SES-053, INV-VFS-3) go to a temporary file in the target's folder, created with
//!   `create_new` (`.<name>.hikasa-tmp[-N]`); `commit` flushes, syncs and renames it onto the target (with
//!   `overwrite = false` a rename that never replaces, see [`rename_new`]); dropping the
//!   writer removes the temporary file. Its writer keeps it locked and owner-only until the rename, when it gets the
//!   mode it will have (`lock`). A target that is a symbolic link is resolved first: the file the link
//!   points to is replaced (the temporary file is created next to it) and the link stays. A target that is not
//!   a regular file (a FIFO, a character or block device such as `/dev/null`) is never replaced: it is written
//!   to directly, which is not atomic (contract §1.2.7 item 12).

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as platform;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

mod lock;
mod trash;

#[cfg(not(any(unix, windows)))]
mod platform {
    //! Fallback for targets without Unix or Windows metadata.
    use crate::entry::{Attrs, FileId};
    use std::fs::Metadata;
    use std::path::Path;

    pub(super) fn attrs(_meta: &Metadata) -> Attrs {
        Attrs::None
    }
    pub(super) fn file_id(_meta: &Metadata) -> Option<FileId> {
        None
    }
    pub(super) fn capabilities() -> crate::caps::Capabilities {
        use crate::caps::Capabilities as C;
        C::READ
            | C::WRITE
            | C::LIST
            | C::SEEK
            | C::MTIME
            | C::SYMLINKS
            | C::LOCAL_PATH
            | C::SET_TIMES
            | C::MKDIR
            | C::REMOVE
            | C::RENAME
    }
    pub(super) fn special(_meta: &Metadata) -> Option<crate::entry::SpecialKind> {
        None
    }
    pub(super) fn sync_dir(_dir: &Path) {}
    pub(super) fn open_new(path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
    }
    pub(super) fn set_mode(_file: &std::fs::File, _mode: Option<u32>) -> std::io::Result<()> {
        Ok(())
    }
    pub(super) fn with_created(
        times: std::fs::FileTimes,
        _created: std::time::SystemTime,
    ) -> std::fs::FileTimes {
        times
    }
    pub(super) fn remove_file(path: &Path) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
    pub(super) fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
    pub(super) fn persist_new(from: &Path, to: &Path) -> std::io::Result<()> {
        super::persist_new(from, to)
    }
    pub(super) fn hold_through_rename(file: std::fs::File) -> Option<std::fs::File> {
        Some(file)
    }
    pub(super) fn make_private(_file: &std::fs::File) -> Option<std::fs::Permissions> {
        None
    }
    pub(super) fn looks_private(_meta: &Metadata) -> bool {
        true
    }
    pub(super) fn set_created(
        _path: &Path,
        _time: std::time::SystemTime,
    ) -> Option<std::io::Result<()>> {
        None
    }
    pub(super) fn set_attrs(
        _path: &Path,
        _meta: &Metadata,
        _change: &crate::vfs::AttrChange,
    ) -> Result<(), super::AttrFailure> {
        Err(super::AttrFailure::WrongFamily)
    }
    pub(super) fn remove_link(path: &Path, _meta: &Metadata) -> std::io::Result<()> {
        std::fs::remove_file(path)
    }
    pub(super) fn is_folder_link(_meta: &Metadata) -> bool {
        false
    }
    pub(super) fn symlink(
        _target: &std::ffi::OsStr,
        _path: &Path,
        _target_is_dir: bool,
    ) -> std::io::Result<()> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

use std::ffi::OsStr;
use std::fs::{self, File, FileTimes, Metadata, OpenOptions, TryLockError};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hikasa_core::{CancelToken, CaseSensitivity, ErrorReport, FileTime, LocalPath, Spec};

use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{
    AtomicWrite, AttrChange, BUSY, Durability, EntryError, FollowLinks, HandleMeta, ListOptions,
    Listing, NameOnly, ReadSeek, SetTimes, Vfs, WriteLock, WriteOptions, op_unsupported,
};

/// Files up to this size are copied by `copy_file_fast` with `std::fs::copy` (not cancellable); larger ones are
/// streamed so that cancellation stays within 100 ms (D-95, G4).
pub const FAST_COPY_MAX: u64 = 64 * 1024 * 1024;

/// Why a platform `set_attrs` refused a change.
#[derive(Debug)]
pub(crate) enum AttrFailure {
    /// `Mode` on a DOS back-end or `Dos` on a Unix one.
    WrongFamily,
    /// DOS bits other than `R` (D-99).
    #[cfg_attr(not(windows), allow(dead_code))]
    OtherDosBits,
    /// The OS call failed.
    Io(io::Error),
}

/// Local file system back-end.
#[derive(Debug, Clone, Default)]
pub struct LocalVfs {
    opts: LocalOptions,
}

/// Local back-end options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalOptions {
    /// Overrides the platform default case sensitivity (`CaseSensitivity::HOST_DEFAULT`).
    pub case: Option<CaseSensitivity>,
}

impl LocalVfs {
    /// New local back-end.
    pub fn new(opts: LocalOptions) -> LocalVfs {
        LocalVfs { opts }
    }

    /// The options.
    pub fn options(&self) -> LocalOptions {
        self.opts
    }
}

/// The local path of `spec`, or `Unsupported("non-local spec")`.
fn local(spec: &Spec) -> Result<&LocalPath, VfsError> {
    match spec {
        Spec::Local(path) => Ok(path),
        _ => Err(VfsError::Unsupported {
            spec: spec.canonical(),
            what: "non-local spec",
        }),
    }
}

/// Canonical (possibly lossy) text of a local path.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The entry name of `path`: its last component, or the whole path for a root.
fn name_of(path: &LocalPath) -> FileName {
    FileName::from_os(path.file_name().unwrap_or(path.as_path().as_os_str()))
}

fn kind_of(meta: &Metadata) -> EntryKind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn kind_of_type(ft: fs::FileType) -> EntryKind {
    if ft.is_symlink() {
        EntryKind::Symlink
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// The entry described by `meta` (of the target when `followed`). `link` is the symlink's target as
/// `read_link` returned it.
fn entry_from(name: FileName, meta: &Metadata, link: Option<&Path>, followed: bool) -> Entry {
    let kind = kind_of(meta);
    let size = match kind {
        EntryKind::Dir => 0,
        // The byte length of the exact target on every platform (procfs reports 0 in `lstat`, Windows 0);
        // `lstat`'s size only when the target could not be read.
        EntryKind::Symlink => link.map_or_else(
            || meta.len(),
            |target| target.as_os_str().as_encoded_bytes().len() as u64,
        ),
        EntryKind::File | EntryKind::Other => meta.len(),
    };
    let mut entry = Entry {
        name,
        kind,
        size,
        mtime: meta.modified().ok().map(FileTime::from_system_time),
        ctime: meta.created().ok().map(FileTime::from_system_time),
        attrs: platform::attrs(meta),
        link_target: None,
        link_target_raw: None,
        followed,
        crc32: None,
        file_id: platform::file_id(meta),
        special: if kind == EntryKind::Other {
            platform::special(meta)
        } else {
            None
        },
    };
    if let Some(target) = link {
        entry.set_link_target(target.as_os_str());
    }
    entry
}

/// Describes the entry at `path` whose `lstat` result is `lmeta`. A symlink is followed when `follow` says
/// so; a link whose target cannot be stat'ed is described unfollowed and its failure returned as well.
fn describe(
    path: &Path,
    name: FileName,
    lmeta: &Metadata,
    follow: FollowLinks,
) -> (Entry, Option<io::Error>) {
    if !lmeta.file_type().is_symlink() {
        return (entry_from(name, lmeta, None, false), None);
    }
    let target = fs::read_link(path).ok();
    match follow {
        FollowLinks::No => (entry_from(name, lmeta, target.as_deref(), false), None),
        FollowLinks::Yes => match fs::metadata(path) {
            Ok(meta) => (entry_from(name, &meta, target.as_deref(), true), None),
            Err(e) => (entry_from(name, lmeta, target.as_deref(), false), Some(e)),
        },
    }
}

/// The `EntryError` of child `name` of `dir`.
fn entry_error(dir: &LocalPath, name: &FileName, op: VfsOp, e: io::Error) -> EntryError {
    let child = dir.join_name(&name.to_os_string()).ok().map(Spec::Local);
    let spec_text = child
        .as_ref()
        .map_or_else(|| name.as_str().to_owned(), Spec::canonical);
    let error = VfsError::from_io(spec_text, op, e);
    EntryError {
        name: name.clone(),
        error: ErrorReport::capture(&error, child.as_ref()),
    }
}

impl Vfs for LocalVfs {
    /// The platform set: read, write, list, seek, mtime, ctime (birth time where the platform has one), symlinks,
    /// local paths; Unix mode and owner on Unix, DOS attributes on Windows; `CASE_SENSITIVE` per
    /// `LocalOptions::case` or the host default. Other specs: `Unsupported("non-local spec")`.
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        local(at)?;
        let mut caps = platform::capabilities();
        let case = self.opts.case.unwrap_or(CaseSensitivity::HOST_DEFAULT);
        caps.set(
            Capabilities::CASE_SENSITIVE,
            case == CaseSensitivity::Sensitive,
        );
        Ok(caps)
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        let path = local(spec)?;
        let lmeta = fs::symlink_metadata(path.as_path())
            .map_err(|e| VfsError::from_io(spec.canonical(), VfsOp::Stat, e))?;
        match describe(path.as_path(), name_of(path), &lmeta, follow) {
            (entry, None) => Ok(entry),
            (_, Some(e)) => Err(VfsError::from_io(spec.canonical(), VfsOp::Stat, e)),
        }
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        cancel.check()?;
        let path = local(dir)?;
        let read_dir = fs::read_dir(path.as_path())
            .map_err(|e| VfsError::from_io(dir.canonical(), VfsOp::List, e))?;
        let mut listing = Listing::default();
        for (index, item) in read_dir.enumerate() {
            if index % 1024 == 1023 {
                cancel.check()?;
            }
            let item = item.map_err(|e| VfsError::from_io(dir.canonical(), VfsOp::List, e))?;
            let name = FileName::from_os(&item.file_name());
            // The kind from the directory record (std stats only when the record has no type).
            let kind = item.file_type().ok().map(kind_of_type);
            let lmeta = match kind {
                Some(kind @ (EntryKind::Dir | EntryKind::File | EntryKind::Other)) => {
                    if let Some(prefilter) = opts.prefilter
                        && !prefilter(name.as_str(), kind)
                    {
                        listing.name_only.push(NameOnly { name, kind });
                        continue;
                    }
                    item.metadata()
                }
                Some(EntryKind::Symlink) | None => fs::symlink_metadata(item.path()),
            };
            match lmeta {
                Ok(lmeta) => {
                    let (entry, failure) = describe(&item.path(), name, &lmeta, opts.follow);
                    if let Some(e) = failure {
                        listing
                            .errors
                            .push(entry_error(path, &entry.name, VfsOp::Stat, e));
                    }
                    listing.entries.push(entry);
                }
                Err(e) => listing
                    .errors
                    .push(entry_error(path, &name, VfsOp::Stat, e)),
            }
        }
        Ok(listing)
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let path = local(spec)?;
        let file = File::open(path.as_path()).map_err(|e| {
            // Windows cannot open a folder as a file and reports "access denied".
            if fs::metadata(path.as_path()).is_ok_and(|m| m.is_dir()) {
                VfsError::IsADirectory {
                    spec: spec.canonical(),
                }
            } else {
                VfsError::from_io(spec.canonical(), VfsOp::Open, e)
            }
        })?;
        let meta = file
            .metadata()
            .map_err(|e| VfsError::from_io(spec.canonical(), VfsOp::Open, e))?;
        if meta.is_dir() {
            return Err(VfsError::IsADirectory {
                spec: spec.canonical(),
            });
        }
        Ok(Box::new(file))
    }

    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        let path = local(spec)?;
        match prepare_write(path, &spec.canonical(), opts)? {
            Prepared::Direct(writer) => Ok(Box::new(writer)),
            Prepared::Temp(writer) => Ok(Box::new(writer)),
        }
    }

    /// `.<name>.hikasa-lock` next to the file a link names, for as long as the lock is held (`lock`).
    fn lock_writer(&self, spec: &Spec) -> Result<Option<Box<dyn WriteLock>>, VfsError> {
        let path = local(spec)?;
        let Ok(target) = resolve_links(path.as_path()) else {
            return Ok(None);
        };
        match lock::take(&target) {
            lock::Taken::Held(held) => Ok(Some(Box::new(held))),
            lock::Taken::Busy(lock) => Err(VfsError::Unavailable {
                spec: spec.canonical(),
                reason: format!("{BUSY} (its lock is {})", lock.display()),
            }),
            lock::Taken::Impossible => Ok(None),
        }
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        // Rule 17 of §1.5.7: a member name that is no file name here is an item error of a copy out of an archive.
        #[cfg(windows)]
        if crate::archive::name::windows_name_problem(name.as_str()).is_some() {
            return Err(VfsError::InvalidName {
                name: name.as_str().to_owned(),
            });
        }
        let path = local(dir)?;
        path.join_name(&name.to_os_string())
            .map(Spec::Local)
            .map_err(|_| VfsError::InvalidName {
                name: name.as_str().to_owned(),
            })
    }

    /// The path when `spec` is a local regular file (symlinks followed).
    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        let Spec::Local(path) = spec else {
            return None;
        };
        fs::metadata(path.as_path())
            .ok()
            .filter(Metadata::is_file)
            .map(|_| path.as_path().to_path_buf())
    }

    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        let path = local(spec)?;
        let fail = |e: io::Error| {
            VfsError::from_io(
                spec.canonical(),
                VfsOp::Mkdir,
                below_a_file(path.as_path(), e),
            )
        };
        if !parents {
            return fs::create_dir(path.as_path()).map(|()| true).map_err(fail);
        }
        match fs::metadata(path.as_path()) {
            Ok(meta) if meta.is_dir() => return Ok(false),
            Ok(_) => {
                return Err(VfsError::NotADirectory {
                    spec: spec.canonical(),
                });
            }
            Err(_) => {}
        }
        fs::create_dir_all(path.as_path()).map_err(|e| {
            // An ancestor that is a file: `create_dir_all` reports "already exists" or "not a folder"
            // depending on the platform; both mean a non-folder ancestor.
            if e.kind() == io::ErrorKind::AlreadyExists {
                VfsError::NotADirectory {
                    spec: spec.canonical(),
                }
            } else {
                fail(e)
            }
        })?;
        Ok(true)
    }

    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        let path = local(spec)?;
        operable(path, spec)?;
        let fail = |e: io::Error| {
            VfsError::from_io(
                spec.canonical(),
                VfsOp::Remove,
                below_a_file(path.as_path(), e),
            )
        };
        let meta = fs::symlink_metadata(path.as_path()).map_err(fail)?;
        if meta.file_type().is_symlink() {
            platform::remove_link(path.as_path(), &meta).map_err(fail)
        } else if meta.is_dir() {
            fs::remove_dir(path.as_path()).map_err(|e| {
                // Some platforms report a non-empty folder with an `Other` kind ("directory not empty").
                if e.kind() == io::ErrorKind::DirectoryNotEmpty
                    || fs::read_dir(path.as_path()).is_ok_and(|mut d| d.next().is_some())
                {
                    VfsError::NotEmpty {
                        spec: spec.canonical(),
                    }
                } else {
                    fail(e)
                }
            })
        } else {
            fs::remove_file(path.as_path()).map_err(fail)
        }
    }

    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        let (from_path, to_path) = (local(from)?, local(to)?);
        operable(from_path, from)?;
        operable(to_path, to)?;
        let fail = |e: io::Error| match e.kind() {
            io::ErrorKind::CrossesDevices => VfsError::CrossDevice {
                from: from.canonical(),
                to: to.canonical(),
            },
            _ => VfsError::from_io(from.canonical(), VfsOp::Rename, e),
        };
        let from_meta = fs::symlink_metadata(from_path.as_path())
            .map_err(|e| fail(below_a_file(from_path.as_path(), e)))?;
        let mut cleared = Cleared::Nothing;
        match fs::symlink_metadata(to_path.as_path()) {
            Ok(to_meta) => {
                if !same_item(from_path.as_path(), &from_meta, to_path.as_path(), &to_meta) {
                    if !replace {
                        return Err(VfsError::AlreadyExists {
                            spec: to.canonical(),
                        });
                    }
                    match (from_meta.is_dir(), to_meta.is_dir()) {
                        (true, false) => {
                            return Err(VfsError::NotADirectory {
                                spec: to.canonical(),
                            });
                        }
                        (false, true) => {
                            return Err(VfsError::IsADirectory {
                                spec: to.canonical(),
                            });
                        }
                        (true, true) => {
                            if fs::read_dir(to_path.as_path()).is_ok_and(|mut d| d.next().is_some())
                            {
                                return Err(VfsError::NotEmpty {
                                    spec: to.canonical(),
                                });
                            }
                            // Windows renames onto no folder, not even an empty one.
                            if cfg!(windows) {
                                fs::remove_dir(to_path.as_path()).map_err(|e| {
                                    VfsError::from_io(to.canonical(), VfsOp::Rename, e)
                                })?;
                                cleared = Cleared::Folder;
                            }
                        }
                        (false, false) => {
                            // Nor onto a link to a folder, which is a folder entry there.
                            if platform::is_folder_link(&to_meta) {
                                let fail = |e| VfsError::from_io(to.canonical(), VfsOp::Rename, e);
                                let target = fs::read_link(to_path.as_path()).map_err(fail)?;
                                platform::remove_link(to_path.as_path(), &to_meta).map_err(fail)?;
                                cleared = Cleared::FolderLink(target);
                            }
                        }
                    }
                }
            }
            // Checked here: a rename below a file fails with `ERROR_INVALID_PARAMETER` on Windows.
            Err(e) => {
                let e = below_a_file(to_path.as_path(), e);
                if e.kind() != io::ErrorKind::NotFound {
                    return Err(VfsError::from_io(to.canonical(), VfsOp::Rename, e));
                }
            }
        }
        fs::rename(from_path.as_path(), to_path.as_path()).map_err(|e| {
            // Best effort: what this call removed comes back.
            let _ = match &cleared {
                Cleared::Nothing => Ok(()),
                Cleared::Folder => fs::create_dir(to_path.as_path()),
                Cleared::FolderLink(target) => {
                    platform::symlink(target.as_os_str(), to_path.as_path(), true)
                }
            };
            match e.kind() {
                io::ErrorKind::DirectoryNotEmpty => VfsError::NotEmpty {
                    spec: to.canonical(),
                },
                _ => fail(e),
            }
        })
    }

    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        let path = local(spec)?;
        let fail = |e: io::Error| {
            VfsError::from_io(
                spec.canonical(),
                VfsOp::SetTimes,
                below_a_file(path.as_path(), e),
            )
        };
        let meta = fs::symlink_metadata(path.as_path()).map_err(fail)?;
        let is_link = meta.file_type().is_symlink();
        let created = match times.created {
            Some(created) => {
                if is_link || !platform::capabilities().contains(Capabilities::SET_CREATED) {
                    return Err(op_unsupported(spec, VfsOp::SetTimes));
                }
                Some(system_time(created).ok_or_else(|| fail(out_of_range()))?)
            }
            None => None,
        };
        if let Some(modified) = times.modified {
            let mtime = filetime_of(modified);
            let atime = filetime::FileTime::from_last_access_time(&meta);
            filetime::set_symlink_file_times(path.as_path(), atime, mtime).map_err(fail)?;
        }
        if let Some(created) = created
            && let Some(result) = platform::set_created(path.as_path(), created)
        {
            result.map_err(fail)?;
        }
        Ok(())
    }

    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        let path = local(spec)?;
        change.validate()?;
        let meta = fs::symlink_metadata(path.as_path()).map_err(|e| {
            VfsError::from_io(
                spec.canonical(),
                VfsOp::SetAttrs,
                below_a_file(path.as_path(), e),
            )
        })?;
        if meta.file_type().is_symlink() {
            return Err(op_unsupported(spec, VfsOp::SetAttrs));
        }
        platform::set_attrs(path.as_path(), &meta, change).map_err(|failure| match failure {
            AttrFailure::WrongFamily => op_unsupported(spec, VfsOp::SetAttrs),
            AttrFailure::OtherDosBits => VfsError::Unsupported {
                spec: spec.canonical(),
                what: "changing DOS attributes other than R",
            },
            AttrFailure::Io(e) => VfsError::from_io(spec.canonical(), VfsOp::SetAttrs, e),
        })
    }

    fn create_symlink(
        &self,
        spec: &Spec,
        target: &OsStr,
        target_is_dir: bool,
    ) -> Result<(), VfsError> {
        let path = local(spec)?;
        platform::symlink(target, path.as_path(), target_is_dir).map_err(|e| {
            VfsError::from_io(
                spec.canonical(),
                VfsOp::Symlink,
                below_a_file(path.as_path(), e),
            )
        })
    }

    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        let path = local(spec)?;
        operable(path, spec)?;
        let meta = fs::symlink_metadata(path.as_path())
            .map_err(|e| VfsError::from_io(spec.canonical(), VfsOp::Trash, e))?;
        self::trash::move_to_trash(path.as_path(), &meta, &spec.canonical())
    }

    fn sync_folder(&self, dir: &Spec) {
        if let Ok(path) = local(dir) {
            sync_names(path.as_path());
        }
    }

    /// Local → local files up to [`FAST_COPY_MAX`]: `std::io::copy` from the source into the temporary sibling
    /// `open_write` creates (`copy_file_range` on Linux), then the same commit protocol (D-95). Larger files, and targets `open_write`
    /// would write to directly, return `Ok(None)` (the caller streams).
    fn copy_file_fast(
        &self,
        from: &Spec,
        to: &Spec,
        opts: &WriteOptions,
        cancel: &CancelToken,
    ) -> Result<Option<u64>, VfsError> {
        let (from_path, to_path) = (local(from)?, local(to)?);
        cancel.check()?;
        let meta = fs::metadata(from_path.as_path())
            .map_err(|e| VfsError::from_io(from.canonical(), VfsOp::Open, e))?;
        if !meta.is_file() || meta.len() > FAST_COPY_MAX {
            return Ok(None);
        }
        let mut writer = match prepare_write(to_path, &to.canonical(), opts)? {
            Prepared::Direct(_) => return Ok(None),
            Prepared::Temp(writer) => writer,
        };
        // Stream into the handle `prepare_write` opened (never reopened by name: a read-only source must not
        // make the temporary file unwritable); on Linux `io::copy` between files uses `copy_file_range`. The mode
        // is applied later by the metadata step (D-95 step 6).
        let mut source = File::open(from_path.as_path())
            .map_err(|e| VfsError::from_io(from.canonical(), VfsOp::Open, e))?;
        let buffered = writer
            .file
            .take()
            .ok_or_else(|| writer.fail(io::Error::other("the writer was already committed")))?;
        let mut file = buffered
            .into_inner()
            .map_err(|e| VfsError::from_io(to.canonical(), VfsOp::Write, e.into_error()))?;
        let copied = io::copy(&mut source, &mut file).map_err(|e| {
            if fs::metadata(from_path.as_path()).is_err() {
                VfsError::from_io(from.canonical(), VfsOp::Read, e)
            } else {
                VfsError::from_io(to.canonical(), VfsOp::Write, e)
            }
        })?;
        writer.file = Some(BufWriter::new(file));
        cancel.check()?;
        writer.commit_inner()?;
        Ok(Some(copied))
    }

    /// Local → local files up to [`FAST_COPY_MAX`] (larger ones `Ok(None)`, for G4), with a mode: a file created without
    /// one needs the default mode, which only the sequence of D-158 gives without reading the umask. The temporary
    /// sibling is opened `create_new`, owner-only, takes the content, its final mode and time on the handle, and is
    /// closed and renamed to `to`, so the file never shows under its name with a wrong time or mode. The rename never
    /// replaces: a file another program made at `to` meanwhile stays, and the copy is `AlreadyExists` ([`rename_new`]).
    /// Windows' `R` comes after the rename, from the usual step. The temporary file is not locked as a writer's: nothing
    /// stands at `to`, so no rebuild of `to` looks for it, and this path has no time for it (D-164).
    fn copy_new_file(
        &self,
        from: &Spec,
        to: &Spec,
        meta: &HandleMeta,
        cancel: &CancelToken,
    ) -> Result<Option<u64>, VfsError> {
        let (from_path, to_path) = (local(from)?, local(to)?);
        cancel.check()?;
        if cfg!(unix) && meta.mode.is_none() {
            return Ok(None);
        }
        let modified = match meta.modified {
            Some(time) => match system_time(time) {
                Some(time) => Some(time),
                None => return Ok(None),
            },
            None => None,
        };
        let created = match meta.created {
            Some(time) => match system_time(time) {
                Some(time) => Some(time),
                None => return Ok(None),
            },
            None => None,
        };
        let size = fs::metadata(from_path.as_path())
            .map_err(|e| VfsError::from_io(from.canonical(), VfsOp::Open, e))?;
        if !size.is_file() || size.len() > FAST_COPY_MAX {
            return Ok(None);
        }
        let (Some(dir), Some(name)) = (to_path.as_path().parent(), to_path.as_path().file_name())
        else {
            return Err(VfsError::IsADirectory {
                spec: to.canonical(),
            });
        };
        let written = |e: io::Error| VfsError::from_io(to.canonical(), VfsOp::Write, e);
        let mut source = File::open(from_path.as_path())
            .map_err(|e| VfsError::from_io(from.canonical(), VfsOp::Open, e))?;
        let (mut file, tmp) =
            create_new_temp(dir, name).map_err(|e| written(below_a_file(to_path.as_path(), e)))?;
        let temporary = NewTemp {
            path: tmp.as_path(),
        };
        let copied = io::copy(&mut source, &mut file).map_err(|e| {
            if fs::metadata(from_path.as_path()).is_err() {
                VfsError::from_io(from.canonical(), VfsOp::Read, e)
            } else {
                written(e)
            }
        })?;
        platform::set_mode(&file, meta.mode).map_err(written)?;
        if modified.is_some() || created.is_some() {
            let mut times = FileTimes::new();
            if let Some(time) = modified {
                times = times.set_modified(time);
            }
            if let Some(time) = created {
                times = platform::with_created(times, time);
            }
            file.set_times(times).map_err(written)?;
        }
        drop(file);
        #[cfg(test)]
        tests::before_rename(&tmp);
        cancel.check()?;
        rename_new(&tmp, to_path.as_path()).map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists => VfsError::AlreadyExists {
                spec: to.canonical(),
            },
            _ => VfsError::from_io(to.canonical(), VfsOp::Commit, e),
        })?;
        std::mem::forget(temporary);
        Ok(Some(copied))
    }
}

/// `tmp` takes the name `target`, which must be free, atomically: a program that creates `target` first keeps its file and
/// this is `AlreadyExists`. [`tempfile`] holds the one safe spelling of each OS's call (Linux `renameat2` with
/// `RENAME_NOREPLACE`, macOS `renamex_np` with `RENAME_EXCL`, Windows `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`),
/// and a hard link where the file system has no such rename. Where it has hard links neither, what is left is to look
/// and rename, which a racing creator can get in between.
fn rename_new(tmp: &Path, target: &Path) -> io::Result<()> {
    match platform::persist_new(tmp, target) {
        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => {
            if fs::symlink_metadata(target).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            platform::rename(tmp, target)
        }
        done => done,
    }
}

/// One try of [`rename_new`].
fn persist_new(tmp: &Path, target: &Path) -> io::Result<()> {
    tempfile::TempPath::try_from_path(tmp)?
        .persist_noclobber(target)
        .map_err(|e| {
            // The file stays, and whoever made it removes it.
            std::mem::forget(e.path);
            e.error
        })
}

/// The temporary file of [`LocalVfs::copy_new_file`]: it goes unless it was renamed.
struct NewTemp<'a> {
    path: &'a Path,
}

impl Drop for NewTemp<'_> {
    fn drop(&mut self) {
        let _ = platform::remove_file(self.path);
    }
}

/// A new temporary file next to the target, by the names of [`create_temp`], owner-only from the start, unlocked.
fn create_new_temp(dir: &Path, name: &OsStr) -> io::Result<(File, PathBuf)> {
    let base = lock::temp_base(name);
    for attempt in 0..lock::TEMP_NAMES {
        let tmp = dir.join(lock::temp_name(&base, attempt));
        match platform::open_new(&tmp) {
            Ok(file) => return Ok((file, tmp)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

/// What `rename` removed from the target path first, because Windows renames onto no folder entry.
enum Cleared {
    Nothing,
    Folder,
    FolderLink(PathBuf),
}

/// The error of a failed call on `path`. Windows reports a path below a file as "path not found" where Unix says
/// `ENOTDIR`, and the mutating primitives name it `NotADirectory` on every platform: there the nearest existing
/// ancestor decides.
fn below_a_file(path: &Path, error: io::Error) -> io::Error {
    let below_file = cfg!(windows)
        && error.kind() == io::ErrorKind::NotFound
        && path
            .ancestors()
            .skip(1)
            .find_map(|a| fs::metadata(a).ok())
            .is_some_and(|meta| !meta.is_dir());
    if below_file {
        io::ErrorKind::NotADirectory.into()
    } else {
        error
    }
}

/// The checks of an item `remove`, `rename` and `move_to_trash` act on: a final component `.` or `..` →
/// `InvalidTarget "needs a file name"`; a root, also once its parent is resolved (`fs::canonicalize` of the parent
/// joined with the name) → `InvalidTarget "is a root folder"`.
fn operable(path: &LocalPath, spec: &Spec) -> Result<(), VfsError> {
    let invalid = |reason: &'static str| VfsError::InvalidTarget {
        spec: spec.canonical(),
        reason,
    };
    match path.file_name() {
        Some(name) if name == "." || name == ".." => return Err(invalid("needs a file name")),
        Some(_) => {}
        None => return Err(invalid("is a root folder")),
    }
    let (Some(parent), Some(name)) = (path.as_path().parent(), path.as_path().file_name()) else {
        return Err(invalid("is a root folder"));
    };
    if let Ok(canonical) = fs::canonicalize(parent)
        && canonical.join(name).parent().is_none()
    {
        return Err(invalid("is a root folder"));
    }
    Ok(())
}

/// How a write reaches its target.
enum Prepared {
    /// A FIFO or a device written to directly (never with `replace_link`).
    Direct(DirectWriter),
    /// A temporary sibling renamed onto the target by `commit`.
    Temp(LocalWriter),
}

/// The target resolution of `open_write` (and of `copy_file_fast`): a folder is `IsADirectory`, an existing item
/// without `overwrite` is `AlreadyExists`; without `replace_link` a symbolic link is resolved (the file it points
/// to is replaced) and a target that is not a regular file is written to directly (contract M1 §1.2.7 item 12);
/// with `replace_link` (M2 W3, D-94) the item at `path` itself — a link, a FIFO, a device — is replaced by the new
/// file.
fn prepare_write(
    path: &LocalPath,
    spec_text: &str,
    opts: &WriteOptions,
) -> Result<Prepared, VfsError> {
    let spec_text = spec_text.to_owned();
    if path.parent().is_none() || path.file_name().is_none() {
        return Err(VfsError::IsADirectory { spec: spec_text });
    }
    let io_error = |e: io::Error| VfsError::from_io(spec_text.as_str(), VfsOp::Write, e);
    let mut replaces_other = false;
    // The file to replace: without `replace_link` a symbolic link is resolved, so the file it points to is
    // replaced and the link stays (contract §1.2.7 item 12).
    let target = match fs::symlink_metadata(path.as_path()) {
        Ok(meta) if meta.is_dir() => return Err(VfsError::IsADirectory { spec: spec_text }),
        Ok(_) if !opts.overwrite => return Err(VfsError::AlreadyExists { spec: spec_text }),
        Ok(meta) if opts.replace_link => {
            replaces_other = !meta.is_file();
            path.as_path().to_path_buf()
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            resolve_links(path.as_path()).map_err(io_error)?
        }
        Ok(_) => path.as_path().to_path_buf(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => path.as_path().to_path_buf(),
        Err(e) => return Err(io_error(e)),
    };
    if !opts.replace_link {
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => return Err(VfsError::IsADirectory { spec: spec_text }),
            // A FIFO, a character or block device, a socket: written to directly, never replaced.
            Ok(meta) if !meta.is_file() => {
                let file = OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .map_err(io_error)?;
                return Ok(Prepared::Direct(DirectWriter {
                    file: Some(BufWriter::new(file)),
                    spec: spec_text,
                }));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
    }
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(VfsError::IsADirectory { spec: spec_text });
    };
    if opts.create_parents {
        fs::create_dir_all(parent).map_err(|e| VfsError::from_io(text(parent), VfsOp::Write, e))?;
    }
    let (file, tmp, made) = create_temp(parent, name).map_err(io_error)?;
    let dir = parent.to_path_buf();
    Ok(Prepared::Temp(LocalWriter {
        file: Some(BufWriter::new(file)),
        tmp,
        target,
        dir,
        spec: spec_text,
        overwrite: opts.overwrite,
        keep_permissions: !replaces_other,
        made,
        durability: opts.durability,
        synced: false,
        pending: true,
    }))
}

/// Most symbolic links followed when resolving a write target (the Linux limit).
const MAX_LINK_HOPS: usize = 40;

/// The path a chain of symbolic links ends at: the first path of the chain that is not a symbolic link (it need
/// not exist; a dangling link names the file to create). A relative link target is taken relative to the
/// link's folder; no component is normalised, so the OS resolves `..` as it would through the link.
fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_HOPS {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(dir) if target.is_relative() => dir.join(target),
                    _ => target,
                };
            }
            Ok(_) => return Ok(current),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
        }
    }
    // A loop (or a longer chain than the OS follows): the OS's own error for the path.
    Err(fs::metadata(path)
        .err()
        .unwrap_or_else(|| io::Error::other("too many levels of symbolic links")))
}

/// Creates a new temporary file next to the target: `.<name>.hikasa-tmp`, then `-1`, `-2`, … when taken. Locked by its
/// writer, so a later rebuild can tell it from what a dead one left, and private until it is renamed: the bytes of a
/// private file must not lie around under a wider mode. The result's second path is the file; the mode it was made
/// with comes third, for a target that gives it no other.
fn create_temp(dir: &Path, name: &OsStr) -> io::Result<(File, PathBuf, Option<fs::Permissions>)> {
    let base = lock::temp_base(name);
    for attempt in 0..lock::TEMP_NAMES {
        let tmp = dir.join(lock::temp_name(&base, attempt));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => {
                // Before anything else: where nothing shows that a file is new (Windows), a rebuild can look at it now.
                // It locks first, and the file goes, or it locked and went: either way the next name is free of that.
                if matches!(file.try_lock(), Err(TryLockError::WouldBlock))
                    || !lock::still_named(&file, &tmp)
                {
                    continue;
                }
                let made = platform::make_private(&file);
                return Ok((file, tmp, made));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

/// The atomic writer of `LocalVfs`.
#[derive(Debug)]
struct LocalWriter {
    /// `None` once committed (or after a failed commit).
    file: Option<BufWriter<File>>,
    tmp: PathBuf,
    target: PathBuf,
    dir: PathBuf,
    spec: String,
    overwrite: bool,
    /// Copy the permissions of a replaced regular file onto the new one (not when a link or a special file is
    /// replaced, M2 W3).
    keep_permissions: bool,
    /// The mode the file was created with, before it was made private (Unix).
    made: Option<fs::Permissions>,
    /// What `commit` syncs (D-164).
    durability: Durability,
    /// Everything written so far reached the disk.
    synced: bool,
    /// The temporary file still exists and belongs to this writer.
    pending: bool,
}

impl LocalWriter {
    fn writer(&mut self) -> io::Result<&mut BufWriter<File>> {
        self.synced = false;
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("the writer was already committed"))
    }

    fn fail(&self, e: io::Error) -> VfsError {
        VfsError::from_io(self.spec.as_str(), VfsOp::Commit, e)
    }

    fn commit_inner(&mut self) -> Result<(), VfsError> {
        let writer = self
            .file
            .take()
            .ok_or_else(|| self.fail(io::Error::other("the writer was already committed")))?;
        let file = writer.into_inner().map_err(|e| self.fail(e.into_error()))?;
        if !self.synced && self.durability != Durability::None {
            sync_file(&file).map_err(|e| self.fail(e))?;
        }
        // Holds the lock of the temporary file until the rename is done, so no rebuild takes it for a dead writer's.
        // Windows closes it here, which leaves that moment open (`lock`).
        let _held = platform::hold_through_rename(file);
        // The permissions of the file being replaced, or the ones a new file gets (best effort).
        let old = self.overwrite && self.keep_permissions;
        let old = old.then(|| fs::metadata(&self.target).ok()).flatten();
        if let Some(mode) = old
            .map(|old| old.permissions())
            .or_else(|| self.made.take())
        {
            let _ = fs::set_permissions(&self.tmp, mode);
        }
        let renamed = if self.overwrite {
            platform::rename(&self.tmp, &self.target)
        } else {
            rename_new(&self.tmp, &self.target)
        };
        renamed.map_err(|e| match e.kind() {
            io::ErrorKind::AlreadyExists if !self.overwrite => VfsError::AlreadyExists {
                spec: self.spec.clone(),
            },
            _ => self.fail(e),
        })?;
        self.pending = false;
        if self.durability == Durability::Full {
            sync_names(&self.dir);
        }
        Ok(())
    }
}

/// `sync_all` of a file, counted in tests.
fn sync_file(file: &File) -> io::Result<()> {
    #[cfg(test)]
    tests::count_sync();
    file.sync_all()
}

/// The sync of a folder after a rename, counted in tests where there is one.
fn sync_names(dir: &Path) {
    #[cfg(test)]
    if cfg!(unix) {
        tests::count_sync();
    }
    platform::sync_dir(dir);
}

impl Write for LocalWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer()?.flush()
    }
}

impl Seek for LocalWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        self.writer()?.seek(to)
    }
}

impl AtomicWrite for LocalWriter {
    fn sync(&mut self) -> Result<(), VfsError> {
        let synced = self.writer().and_then(|writer| {
            writer.flush()?;
            writer.get_ref().sync_all()
        });
        synced.map_err(|e| self.fail(e))?;
        self.synced = true;
        Ok(())
    }

    fn commit(mut self: Box<Self>) -> Result<(), VfsError> {
        self.commit_inner()
    }
}

impl Drop for LocalWriter {
    /// Discards the temporary file unless it was renamed onto the target.
    fn drop(&mut self) {
        drop(self.file.take());
        if self.pending {
            let _ = platform::remove_file(&self.tmp);
        }
    }
}

/// The writer of a target that is not a regular file (a FIFO, a character or block device): it writes straight
/// into the target, which is never replaced. Not atomic: what was written before a failure or a drop has reached
/// the target (contract §1.2.7 item 12).
#[derive(Debug)]
struct DirectWriter {
    /// `None` once committed.
    file: Option<BufWriter<File>>,
    spec: String,
}

impl DirectWriter {
    fn writer(&mut self) -> io::Result<&mut BufWriter<File>> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("the writer was already committed"))
    }
}

impl Write for DirectWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer()?.flush()
    }
}

impl Seek for DirectWriter {
    /// A pipe or a device has no position to go back to.
    fn seek(&mut self, _to: SeekFrom) -> io::Result<u64> {
        Err(io::ErrorKind::Unsupported.into())
    }
}

impl AtomicWrite for DirectWriter {
    /// Flushes the buffered bytes into the target (no sync: pipes and devices do not support it).
    fn commit(mut self: Box<Self>) -> Result<(), VfsError> {
        let spec = self.spec.clone();
        let fail = |e: io::Error| VfsError::from_io(spec.as_str(), VfsOp::Commit, e);
        let writer = self
            .file
            .take()
            .ok_or_else(|| fail(io::Error::other("the writer was already committed")))?;
        writer.into_inner().map_err(|e| fail(e.into_error()))?;
        Ok(())
    }
}

/// The `filetime` value of a [`FileTime`].
fn filetime_of(t: FileTime) -> filetime::FileTime {
    filetime::FileTime::from_unix_time(t.unix_seconds(), t.subsec_nanos())
}

/// The `SystemTime` of a [`FileTime`] (`None` when the platform cannot represent it).
fn system_time(t: FileTime) -> Option<std::time::SystemTime> {
    let nanos = std::time::Duration::from_nanos(u64::from(t.subsec_nanos()));
    let secs = t.unix_seconds();
    if secs >= 0 {
        std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(secs.unsigned_abs()))?
            .checked_add(nanos)
    } else {
        std::time::UNIX_EPOCH
            .checked_sub(std::time::Duration::from_secs(secs.unsigned_abs()))?
            .checked_add(nanos)
    }
}

/// "The time is out of range" as an I/O error.
fn out_of_range() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "the time is out of the platform's range",
    )
}

/// True when `from` and `to` are the same item (a case-only rename on a case-insensitive file system): the same
/// device and inode where the platform reports them, else the same file through `same-file` (links are compared
/// as links: two different links are never the same item).
fn same_item(from: &Path, from_meta: &Metadata, to: &Path, to_meta: &Metadata) -> bool {
    if let (Some(a), Some(b)) = (platform::file_id(from_meta), platform::file_id(to_meta)) {
        return a == b;
    }
    if from_meta.file_type().is_symlink() || to_meta.file_type().is_symlink() {
        // Without file ids a link is the same item only when the paths differ by case alone.
        return from.as_os_str().to_string_lossy().to_lowercase()
            == to.as_os_str().to_string_lossy().to_lowercase()
            && from.parent() == to.parent();
    }
    same_file::is_same_file(from, to).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    //! A6.3 (D-164): what a copy syncs, and the path of a new file, seen from inside.

    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::{Duration, UNIX_EPOCH};

    use hikasa_core::{Ctl, PathStyle, SpecContext};

    use super::*;
    use crate::ops::{self, CopyOptions, MoveOptions, OverwritePolicy, Preserve};

    /// What the stop before the rename does.
    type Stop = Option<Box<dyn Fn(&Path) + Send>>;

    /// One test at a time: what they count is one number.
    static SERIAL: Mutex<()> = Mutex::new(());
    static SYNCS: AtomicU32 = AtomicU32::new(0);
    static STOP: Mutex<Stop> = Mutex::new(None);

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_stop(stop: Stop) {
        *STOP.lock().unwrap_or_else(PoisonError::into_inner) = stop;
    }

    pub(super) fn count_sync() {
        SYNCS.fetch_add(1, Ordering::Relaxed);
    }

    /// The stop just before the rename of `copy_new_file`.
    pub(super) fn before_rename(tmp: &Path) {
        if let Some(stop) = &*STOP.lock().unwrap_or_else(PoisonError::into_inner) {
            stop(tmp);
        }
    }

    /// `(tempdir, canonical path)`; macOS' temp folder is a link.
    fn scratch() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = fs::canonicalize(dir.path()).unwrap();
        (dir, path)
    }

    fn spec(path: &Path) -> Spec {
        Spec::parse(
            path.to_str().unwrap(),
            &SpecContext::absolute_only(PathStyle::HOST),
        )
        .unwrap()
    }

    /// The times and attributes of a source; the mode where there is one.
    fn meta_of(path: &Path, seconds: i64) -> HandleMeta {
        let mode = fs::metadata(path)
            .ok()
            .and_then(|meta| platform::attrs(&meta).unix().map(|unix| unix.mode));
        HandleMeta {
            modified: FileTime::from_unix(seconds, 0),
            created: None,
            mode: if cfg!(unix) {
                mode.or(Some(0o644))
            } else {
                None
            },
        }
    }

    fn syncs_of(run: impl FnOnce()) -> u32 {
        SYNCS.store(0, Ordering::Relaxed);
        run();
        SYNCS.load(Ordering::Relaxed)
    }

    #[test]
    fn a_new_target_syncs_nothing_a_replaced_one_its_file_a_move_both() {
        let _one = serial();
        let (_dir, root) = scratch();
        fs::write(root.join("a"), b"one").unwrap();
        fs::write(root.join("b"), b"two").unwrap();
        let vfs = LocalVfs::default();
        let cp = |to: &str, overwrite: OverwritePolicy| {
            let options = CopyOptions {
                overwrite,
                ..CopyOptions::default()
            };
            let pair = [(spec(&root.join("a")), spec(&root.join(to)))];
            let plan = ops::plan_copy(&vfs, &vfs, &pair, &options, &Ctl::detached()).unwrap();
            assert!(ops::execute(&vfs, &vfs, plan, &Ctl::detached()).ok());
        };
        assert_eq!(syncs_of(|| cp("new", OverwritePolicy::Fail)), 0);
        assert_eq!(syncs_of(|| cp("b", OverwritePolicy::Always)), 1);
        // No preserved attributes: the new-file path steps aside, and the durability stays.
        let plain = CopyOptions {
            preserve: Preserve::NONE,
            ..CopyOptions::default()
        };
        let pair = [(spec(&root.join("a")), spec(&root.join("plain")))];
        let plan = ops::plan_copy(&vfs, &vfs, &pair, &plain, &Ctl::detached()).unwrap();
        assert_eq!(
            syncs_of(|| assert!(ops::execute(&vfs, &vfs, plan, &Ctl::detached()).ok())),
            0
        );
        // A move that has to copy: `rename` would sync nothing, so the expansion is what is asked for.
        fs::create_dir(root.join("sub")).unwrap();
        let moved = crate::ops::PlanItem::planned(
            crate::ops::Action::Move,
            EntryKind::File,
            Some(spec(&root.join("new"))),
            spec(&root.join("sub").join("moved")),
            0,
        );
        let plan = ops::Plan {
            op: ops::OpKind::Move,
            options: ops::PlanOptions::Move(MoveOptions::default()),
            items: vec![moved],
            warnings: Vec::new(),
        };
        let expected = if cfg!(unix) { 2 } else { 1 };
        assert_eq!(
            syncs_of(|| assert!(ops::execute(&vfs, &vfs, plan, &Ctl::detached()).ok())),
            expected
        );
    }

    #[test]
    fn the_temporary_sibling_is_final_before_it_gets_its_name() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to) = (root.join("from"), root.join("sub").join("to"));
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(&from, b"content").unwrap();
        let meta = HandleMeta {
            mode: cfg!(unix).then_some(0o640),
            ..meta_of(&from, 1_000_000)
        };
        let checked = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&checked);
        let sub = root.join("sub");
        set_stop(Some(Box::new(move |tmp: &Path| {
            let names: Vec<_> = fs::read_dir(&sub)
                .unwrap()
                .map(|item| item.unwrap().path())
                .collect();
            assert_eq!(names, [tmp.to_path_buf()], "the temporary sibling alone");
            assert_eq!(fs::read(tmp).unwrap(), b"content");
            let made = fs::metadata(tmp).unwrap();
            assert_eq!(
                made.modified().unwrap(),
                UNIX_EPOCH + Duration::from_secs(1_000_000)
            );
            #[cfg(unix)]
            assert_eq!(platform::attrs(&made).unix().unwrap().mode, 0o640);
            seen.store(true, Ordering::Relaxed);
        })));
        let vfs = LocalVfs::default();
        let bytes = vfs
            .copy_new_file(&spec(&from), &spec(&to), &meta, &CancelToken::new())
            .unwrap();
        set_stop(None);
        assert_eq!(bytes, Some(7));
        assert!(checked.load(Ordering::Relaxed), "the stop ran");
        assert_eq!(fs::read(&to).unwrap(), b"content");
    }

    #[test]
    fn a_cancel_before_the_rename_removes_the_temporary_file() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to) = (root.join("from"), root.join("sub").join("to"));
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(&from, b"content").unwrap();
        let cancel = CancelToken::new();
        let token = cancel.clone();
        set_stop(Some(Box::new(move |_: &Path| token.cancel())));
        let vfs = LocalVfs::default();
        let result = vfs.copy_new_file(&spec(&from), &spec(&to), &meta_of(&from, 5), &cancel);
        set_stop(None);
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        assert_eq!(fs::read_dir(root.join("sub")).unwrap().count(), 0);
    }

    #[test]
    fn a_file_over_the_limit_has_no_such_path() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to) = (root.join("from"), root.join("to"));
        let file = File::create(&from).unwrap();
        file.set_len(FAST_COPY_MAX + 1).unwrap();
        let vfs = LocalVfs::default();
        let result = vfs
            .copy_new_file(
                &spec(&from),
                &spec(&to),
                &meta_of(&from, 5),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(result, None);
        assert!(!to.exists());
    }

    /// What `execute` syncs for `items` of a move plan.
    fn syncs_of_move(items: Vec<crate::ops::PlanItem>, options: MoveOptions) -> u32 {
        let plan = ops::Plan {
            op: ops::OpKind::Move,
            options: ops::PlanOptions::Move(options),
            items,
            warnings: Vec::new(),
        };
        let vfs = LocalVfs::default();
        syncs_of(|| {
            let report = ops::execute(&vfs, &vfs, plan, &Ctl::detached());
            assert!(report.ok(), "{:?}", report.results);
        })
    }

    fn moved(from: &Path, to: &Path, kind: EntryKind, depth: u32) -> crate::ops::PlanItem {
        let action = crate::ops::Action::Move;
        crate::ops::PlanItem::planned(action, kind, Some(spec(from)), spec(to), depth)
    }

    /// §1.6.3: "no source is removed before its copy is on disk". A link has no file to sync, only its name.
    #[cfg(unix)]
    #[test]
    fn a_moved_link_is_on_disk_before_its_source_goes() {
        let _one = serial();
        let (_dir, root) = scratch();
        std::os::unix::fs::symlink("somewhere", root.join("link")).unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        let item = moved(
            &root.join("link"),
            &root.join("sub").join("link"),
            EntryKind::Symlink,
            0,
        );
        assert_eq!(syncs_of_move(vec![item], MoveOptions::default()), 1);
    }

    /// The name of a new folder is in its parent, which a sync of the file and of that folder leaves out.
    #[test]
    fn a_move_into_a_new_folder_syncs_the_name_of_the_folder() {
        let _one = serial();
        let (_dir, root) = scratch();
        fs::write(root.join("file"), b"content").unwrap();
        let folder = root.join("new");
        let create = crate::ops::PlanItem::planned(
            crate::ops::Action::CreateDir,
            EntryKind::Dir,
            None,
            spec(&folder),
            0,
        );
        let item = moved(&root.join("file"), &folder.join("file"), EntryKind::File, 1);
        let expected = if cfg!(unix) { 3 } else { 1 };
        assert_eq!(
            syncs_of_move(vec![create, item], MoveOptions::default()),
            expected
        );
    }

    /// `--parents`: every folder a move creates on its way.
    #[test]
    fn a_move_with_parents_syncs_the_name_of_each() {
        let _one = serial();
        let (_dir, root) = scratch();
        fs::write(root.join("file"), b"content").unwrap();
        let to = root.join("a").join("b").join("file");
        let item = moved(&root.join("file"), &to, EntryKind::File, 0);
        let options = MoveOptions {
            create_parents: true,
            ..MoveOptions::default()
        };
        // The file, its folder; then the names of `a` and `b`.
        let expected = if cfg!(unix) { 4 } else { 1 };
        assert_eq!(syncs_of_move(vec![item], options), expected);
        assert!(to.is_file());
        assert!(!root.join("file").exists());
    }

    /// `exchange`: across two disks neither rename may reach the disk without the copy that keeps what it replaces.
    #[test]
    fn an_exchange_syncs_each_copy_and_its_name() {
        let _one = serial();
        let (_dir, root) = scratch();
        fs::create_dir(root.join("l")).unwrap();
        fs::create_dir(root.join("r")).unwrap();
        fs::write(root.join("l").join("f"), b"left").unwrap();
        fs::write(root.join("r").join("f"), b"right").unwrap();
        let action = crate::ops::Action::Exchange;
        let (left, right) = (spec(&root.join("l").join("f")), spec(&root.join("r").join("f")));
        let item = crate::ops::PlanItem::planned(action, EntryKind::File, Some(left), right, 0);
        let plan = ops::Plan {
            op: ops::OpKind::Batch,
            options: ops::PlanOptions::Batch(ops::BatchOptions::default()),
            items: vec![item],
            warnings: Vec::new(),
        };
        let vfs = LocalVfs::default();
        let expected = if cfg!(unix) { 4 } else { 2 };
        let syncs = syncs_of(|| assert!(ops::execute(&vfs, &vfs, plan, &Ctl::detached()).ok()));
        assert_eq!(syncs, expected);
    }

    /// A program that creates the target while the copy runs keeps its file: the rename never replaces.
    #[test]
    fn a_file_created_during_the_copy_is_not_replaced() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to) = (root.join("from"), root.join("to"));
        fs::write(&from, b"copied").unwrap();
        let racer = to.clone();
        set_stop(Some(Box::new(move |_: &Path| {
            fs::write(&racer, b"another program's file").unwrap();
        })));
        let vfs = LocalVfs::default();
        let result = vfs.copy_new_file(
            &spec(&from),
            &spec(&to),
            &meta_of(&from, 5),
            &CancelToken::new(),
        );
        set_stop(None);
        assert!(
            matches!(result, Err(VfsError::AlreadyExists { .. })),
            "{result:?}"
        );
        assert_eq!(fs::read(&to).unwrap(), b"another program's file");
        let left = fs::read_dir(&root).unwrap().count();
        assert_eq!(left, 2, "no temporary file stays");
    }

    /// The same for the writer of a new file.
    #[test]
    fn a_writer_of_a_new_file_does_not_replace_a_racing_creator() {
        let (_dir, root) = scratch();
        let to = root.join("to");
        let vfs = LocalVfs::default();
        let options = WriteOptions {
            durability: Durability::None,
            ..WriteOptions::default()
        };
        let mut writer = vfs.open_write(&spec(&to), &options).unwrap();
        writer.write_all(b"mine").unwrap();
        fs::write(&to, b"another program's file").unwrap();
        let result = writer.commit();
        assert!(
            matches!(result, Err(VfsError::AlreadyExists { .. })),
            "{result:?}"
        );
        assert_eq!(fs::read(&to).unwrap(), b"another program's file");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    /// `preserve.attrs` off: the result needs the default mode, so `ops` takes the old path.
    #[cfg(unix)]
    #[test]
    fn no_mode_no_such_path() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to) = (root.join("from"), root.join("to"));
        fs::write(&from, b"content").unwrap();
        let vfs = LocalVfs::default();
        let meta = HandleMeta::default();
        let result = vfs
            .copy_new_file(&spec(&from), &spec(&to), &meta, &CancelToken::new())
            .unwrap();
        assert_eq!(result, None);
    }

    /// The result of the path equals a plain copy that keeps times.
    #[test]
    fn the_result_equals_a_plain_copy() {
        let _one = serial();
        let (_dir, root) = scratch();
        let (from, to, plain) = (root.join("from"), root.join("to"), root.join("plain"));
        fs::write(&from, vec![7u8; 100_000]).unwrap();
        let time = UNIX_EPOCH + Duration::from_secs(2_000_000);
        File::options()
            .write(true)
            .open(&from)
            .unwrap()
            .set_modified(time)
            .unwrap();
        fs::copy(&from, &plain).unwrap();
        File::options()
            .write(true)
            .open(&plain)
            .unwrap()
            .set_modified(time)
            .unwrap();
        let vfs = LocalVfs::default();
        let source = vfs.stat(&spec(&from), FollowLinks::No).unwrap();
        let meta = HandleMeta {
            modified: source.mtime,
            created: None,
            mode: source.attrs.unix().map(|unix| unix.mode),
        };
        vfs.copy_new_file(&spec(&from), &spec(&to), &meta, &CancelToken::new())
            .unwrap();
        assert_eq!(fs::read(&to).unwrap(), fs::read(&plain).unwrap());
        let (made, wanted) = (fs::metadata(&to).unwrap(), fs::metadata(&plain).unwrap());
        assert_eq!(made.modified().unwrap(), wanted.modified().unwrap());
        assert_eq!(made.permissions(), wanted.permissions());
    }
}
