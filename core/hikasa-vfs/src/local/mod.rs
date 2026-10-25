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
        C::READ | C::WRITE | C::LIST | C::SEEK | C::MTIME | C::SYMLINKS | C::LOCAL_PATH
    }
    pub(super) fn sync_dir(_dir: &Path) {}
}

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use hikasa_core::{CancelToken, CaseSensitivity, ErrorReport, FileTime, LocalPath, Spec};

use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{
    AtomicWrite, EntryError, FollowLinks, ListOptions, Listing, NameOnly, ReadSeek, Vfs,
    WriteOptions,
};

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
        let file = File::open(path.as_path())
            .map_err(|e| VfsError::from_io(spec.canonical(), VfsOp::Open, e))?;
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
        let spec_text = spec.canonical();
        if path.parent().is_none() || path.file_name().is_none() {
            return Err(VfsError::IsADirectory { spec: spec_text });
        }
        let io_error = |e: io::Error| VfsError::from_io(spec_text.as_str(), VfsOp::Write, e);
        // The file to replace: a symbolic link is resolved, so the file it points to is replaced and the link
        // stays (contract §1.2.7 item 12).
        let target = match fs::symlink_metadata(path.as_path()) {
            Ok(meta) if meta.is_dir() => return Err(VfsError::IsADirectory { spec: spec_text }),
            Ok(_) if !opts.overwrite => return Err(VfsError::AlreadyExists { spec: spec_text }),
            Ok(meta) if meta.file_type().is_symlink() => {
                resolve_links(path.as_path()).map_err(io_error)?
            }
            Ok(_) => path.as_path().to_path_buf(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => path.as_path().to_path_buf(),
            Err(e) => return Err(io_error(e)),
        };
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => return Err(VfsError::IsADirectory { spec: spec_text }),
            // A FIFO, a character or block device, a socket: written to directly, never replaced.
            Ok(meta) if !meta.is_file() => {
                let file = OpenOptions::new()
                    .write(true)
                    .open(&target)
                    .map_err(io_error)?;
                return Ok(Box::new(DirectWriter {
                    file: Some(BufWriter::new(file)),
                    spec: spec_text,
                }));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(e)),
        }
        let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
            return Err(VfsError::IsADirectory { spec: spec_text });
        };
        if opts.create_parents {
            fs::create_dir_all(parent)
                .map_err(|e| VfsError::from_io(text(parent), VfsOp::Write, e))?;
        }
        let (file, tmp) = create_temp(parent, name).map_err(io_error)?;
        let dir = parent.to_path_buf();
        Ok(Box::new(LocalWriter {
            file: Some(BufWriter::new(file)),
            tmp,
            target,
            dir,
            spec: spec_text,
            overwrite: opts.overwrite,
            pending: true,
        }))
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
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
            if let Ok(old) = fs::metadata(&self.target) {
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
