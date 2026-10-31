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
//!   `overwrite = false` through a hard link, which fails atomically when the target exists); dropping the
//!   writer removes the temporary file. A target that is a symbolic link is resolved first: the file the link
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
    pub(super) fn sync_dir(_dir: &Path) {}
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

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hikasa_core::{CancelToken, CaseSensitivity, ErrorReport, FileTime, LocalPath, Spec};

use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{
    AtomicWrite, AttrChange, EntryError, FollowLinks, ListOptions, Listing, NameOnly, ReadSeek,
    SetTimes, Vfs, WriteOptions, op_unsupported,
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
    let (file, tmp) = create_temp(parent, name).map_err(io_error)?;
    let dir = parent.to_path_buf();
    Ok(Prepared::Temp(LocalWriter {
        file: Some(BufWriter::new(file)),
        tmp,
        target,
        dir,
        spec: spec_text,
        overwrite: opts.overwrite,
        keep_permissions: !replaces_other,
        pending: true,
    }))
}

/// Longest target name that is embedded in the temporary file name.
const MAX_EMBEDDED_NAME: usize = 200;

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

/// Creates a new temporary file next to the target: `.<name>.hikasa-tmp`, then `-1`, `-2`, … when taken.
fn create_temp(dir: &Path, name: &OsStr) -> io::Result<(File, PathBuf)> {
    let mut base = OsString::from(".");
    if name.len() <= MAX_EMBEDDED_NAME {
        base.push(name);
        base.push(".");
    }
    base.push("hikasa-tmp");
    for attempt in 0..1000u32 {
        let mut file_name = base.clone();
        if attempt > 0 {
            file_name.push(format!("-{attempt}"));
        }
        let tmp = dir.join(&file_name);
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
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
    /// The temporary file still exists and belongs to this writer.
    pending: bool,
}

impl LocalWriter {
    fn writer(&mut self) -> io::Result<&mut BufWriter<File>> {
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
        {
            // The handle is closed at the end of this block, before the rename (Windows cannot rename an
            // open file).
            let file = writer.into_inner().map_err(|e| self.fail(e.into_error()))?;
            file.sync_all().map_err(|e| self.fail(e))?;
        }
        if self.overwrite {
            if self.keep_permissions
                && let Ok(old) = fs::metadata(&self.target)
            {
                // Keep the permissions of the file being replaced (best effort).
                let _ = fs::set_permissions(&self.tmp, old.permissions());
            }
            fs::rename(&self.tmp, &self.target).map_err(|e| self.fail(e))?;
            self.pending = false;
        } else {
            match fs::hard_link(&self.tmp, &self.target) {
                Ok(()) => {
                    self.pending = false;
                    let _ = fs::remove_file(&self.tmp);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    return Err(VfsError::AlreadyExists {
                        spec: self.spec.clone(),
                    });
                }
                Err(_) => {
                    // No hard links on this file system: check, then rename (not atomic against a racing
                    // creator, which the hard link would have caught).
                    if fs::symlink_metadata(&self.target).is_ok() {
                        return Err(VfsError::AlreadyExists {
                            spec: self.spec.clone(),
                        });
                    }
                    fs::rename(&self.tmp, &self.target).map_err(|e| self.fail(e))?;
                    self.pending = false;
                }
            }
        }
        platform::sync_dir(&self.dir);
        Ok(())
    }
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
    fn commit(mut self: Box<Self>) -> Result<(), VfsError> {
        self.commit_inner()
    }
}

impl Drop for LocalWriter {
    /// Discards the temporary file unless it was renamed onto the target.
    fn drop(&mut self) {
        drop(self.file.take());
        if self.pending {
            let _ = fs::remove_file(&self.tmp);
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
