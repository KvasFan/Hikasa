//! [`VfsRouter`]: dispatches specs to back-ends; [`Blob`]: in-memory content injected by adapters.
//!
//! Router dispatch (normative, §1.2.4):
//!
//! | Spec | `stat`/`open_read` | `list` | `open_write` | `capabilities` |
//! |---|---|---|---|---|
//! | `Local` | `LocalVfs` | `LocalVfs` | `LocalVfs` (atomic temp + rename) | platform set |
//! | `Stdin` | blob (name `-`, kind File, mtime from blob), else `Unsupported{what:"stdin was not provided"}` | `NotADirectory` | `Unsupported{what:"writing to standard input"}` | READ \| SEEK \| MTIME |
//! | `Clipboard` | blob (name `clipboard://`), else `Unsupported{what:"clipboard access"}` | `NotADirectory` | `Unsupported{what:"writing to the clipboard"}` | READ \| SEEK \| MTIME |
//! | `ArchiveMember`, `Snapshot`, `Remote`, `Profile`, `Http` | `Unsupported{what:"<scheme> specs"}` | same | same | `Unsupported` |
//!
//! `<scheme>` is [`Spec::scheme_name`]: `archive`, `snapshot`, `ftp`, …, `svn+ssh`, `mtp`, `profile`, `http`,
//! `https`. `child` of a blob spec is `NotADirectory`; of an unsupported spec, `Unsupported`.
//!
//! Mutating primitives (M2 W3, contract M2 §1.3.1):
//!
//! | Spec | `create_dir`, `remove`, `set_times`, `set_attrs`, `create_symlink`, `move_to_trash` | `rename(from, to)` | `copy_file_fast` |
//! |---|---|---|---|
//! | `Local` | `LocalVfs` | `LocalVfs` when both are `Local` | `LocalVfs` when both are `Local` |
//! | `Stdin`, `Clipboard` | `OperationUnsupported` | `OperationUnsupported` | `Ok(None)` |
//! | archive, snapshot, remote, profile, HTTP | `Unsupported{what:"<scheme> specs"}` | same | `Ok(None)` |
//! | mixed kinds | — | `OperationUnsupported` | `Ok(None)` |
//!
//! Archives (M2 W5, contract §1.5.3): with [`VfsRouter::with_archives`] an `ArchiveMember` goes to the
//! [`ArchiveVfs`] for every read method, `child`, `capabilities`, the primitives, `prepare` and `open_read_with`;
//! `rename` needs both specs inside one archive (two archives → `CrossDevice`, mixed kinds →
//! `OperationUnsupported`); `copy_file_fast` is `Ok(None)`, `local_path` `None`. Without archives it stays the M1
//! row above. `archive_format` and `take_warnings` ask the `ArchiveVfs`.

use std::ffi::OsStr;
use std::fmt;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::spec::RemoteScheme;
use hikasa_core::{CancelToken, Ctl, FileTime, Spec, Warning};

use crate::archive::{ArchiveFormat, ArchiveVfs};
use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};
use crate::local::LocalVfs;
use crate::vfs::{
    AtomicWrite, AttrChange, Finish, FollowLinks, ListOptions, Listing, ReadSeek, SetTimes, Vfs,
    WriteLock, WriteOptions, op_unsupported,
};

/// In-memory content injected by an adapter (stdin bytes read once; clipboard text).
#[derive(Clone)]
pub struct Blob {
    /// Bytes.
    pub bytes: Arc<[u8]>,
    /// Timestamp to report (the adapter passes "now" at read time; VFS table: timestamps = now).
    pub mtime: FileTime,
    /// Name hint for format detection (`--stdin-name`, D-15); `None` = no name.
    pub name: Option<String>,
}

impl fmt::Debug for Blob {
    /// Shows the length, not the bytes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Blob")
            .field("len", &self.bytes.len())
            .field("mtime", &self.mtime)
            .field("name", &self.name)
            .finish()
    }
}

/// Why a blob could not be captured (M2 W3, D-100): `stat` and `open_read` of its spec return this error, so a
/// failed clipboard read reaches every command with its own kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobFailure {
    /// No source (no clipboard): `VfsError::Unavailable` (kind `unsupported`).
    Unavailable(String),
    /// The source holds nothing: `NotFound`.
    Empty,
    /// Reading failed: `Io` (op `read`).
    Failed(String),
    /// The content is larger than `limit` bytes: `TooLarge`.
    TooLarge(u64),
}

impl BlobFailure {
    /// The error for `spec`.
    pub fn error(&self, spec: &Spec) -> VfsError {
        let spec = spec.canonical();
        match self {
            BlobFailure::Unavailable(reason) => VfsError::Unavailable {
                spec,
                reason: reason.clone(),
            },
            BlobFailure::Empty => VfsError::NotFound { spec },
            BlobFailure::Failed(reason) => VfsError::from_io(
                spec,
                crate::error::VfsOp::Read,
                std::io::Error::other(reason.clone()),
            ),
            BlobFailure::TooLarge(limit) => VfsError::TooLarge {
                spec,
                limit: *limit,
            },
        }
    }
}

/// Dispatches specs to back-ends (table in the module documentation). This is the `Vfs` the session layer
/// uses.
#[derive(Debug, Clone)]
pub struct VfsRouter {
    local: LocalVfs,
    stdin: Option<Blob>,
    clipboard: Option<Blob>,
    clipboard_failure: Option<BlobFailure>,
    archives: Option<Arc<ArchiveVfs>>,
}

/// Capabilities of a blob spec.
const BLOB_CAPS: Capabilities = Capabilities::READ
    .union(Capabilities::SEEK)
    .union(Capabilities::MTIME);

/// What an unsupported spec kind is called in `VfsError::Unsupported` (`"<scheme> specs"`).
fn unsupported_what(spec: &Spec) -> &'static str {
    match spec {
        Spec::ArchiveMember { .. } => "archive specs",
        Spec::Snapshot { .. } => "snapshot specs",
        Spec::Remote(url) => match url.scheme {
            RemoteScheme::Ftp => "ftp specs",
            RemoteScheme::Ftps => "ftps specs",
            RemoteScheme::Sftp => "sftp specs",
            RemoteScheme::Dav => "dav specs",
            RemoteScheme::Davs => "davs specs",
            RemoteScheme::S3 => "s3 specs",
            RemoteScheme::Svn => "svn specs",
            RemoteScheme::SvnHttp => "svn+http specs",
            RemoteScheme::SvnHttps => "svn+https specs",
            RemoteScheme::SvnSsh => "svn+ssh specs",
            RemoteScheme::Mtp => "mtp specs",
        },
        Spec::Profile { .. } => "profile specs",
        Spec::Http(url) => {
            if url.secure {
                "https specs"
            } else {
                "http specs"
            }
        }
        Spec::Local(_) => "local specs",
        Spec::Clipboard => "clipboard access",
        Spec::Stdin => "stdin was not provided",
        // Spec kinds added after M1.
        _ => "these specs",
    }
}

fn unsupported(spec: &Spec) -> VfsError {
    VfsError::Unsupported {
        spec: spec.canonical(),
        what: unsupported_what(spec),
    }
}

/// How a spec is served.
enum Route<'a> {
    Local,
    Blob(&'a Blob),
    /// `Stdin` or `Clipboard` without a blob.
    MissingBlob,
    /// A capture that failed.
    FailedBlob(&'a BlobFailure),
    /// An archive spec, archives set.
    Archive(&'a ArchiveVfs),
    Unsupported,
}

impl VfsRouter {
    /// Router over a local back-end.
    pub fn new(local: LocalVfs) -> VfsRouter {
        VfsRouter {
            local,
            stdin: None,
            clipboard: None,
            clipboard_failure: None,
            archives: None,
        }
    }

    /// Serves archive specs through `archives` (M2 W5, §1.5.3).
    pub fn with_archives(self, archives: Arc<ArchiveVfs>) -> VfsRouter {
        VfsRouter {
            archives: Some(archives),
            ..self
        }
    }

    /// The archive back-end, when set.
    pub fn archives(&self) -> Option<&Arc<ArchiveVfs>> {
        self.archives.as_ref()
    }

    /// Makes `Spec::Stdin` readable.
    pub fn with_stdin(self, blob: Blob) -> VfsRouter {
        VfsRouter {
            stdin: Some(blob),
            ..self
        }
    }

    /// Makes `Spec::Clipboard` readable.
    pub fn with_clipboard(self, blob: Blob) -> VfsRouter {
        VfsRouter {
            clipboard: Some(blob),
            clipboard_failure: None,
            ..self
        }
    }

    /// Makes `stat` and `open_read` of `Spec::Clipboard` fail with `failure` (a failed capture, M2 W3).
    pub fn with_clipboard_failure(self, failure: BlobFailure) -> VfsRouter {
        VfsRouter {
            clipboard: None,
            clipboard_failure: Some(failure),
            ..self
        }
    }

    /// The name hint of the blob behind `Spec::Stdin` / `Spec::Clipboard` (D-15); `None` otherwise.
    pub fn name_hint(&self, spec: &Spec) -> Option<&str> {
        match spec {
            Spec::Stdin => self.stdin.as_ref()?.name.as_deref(),
            Spec::Clipboard => self.clipboard.as_ref()?.name.as_deref(),
            _ => None,
        }
    }

    /// The local back-end.
    pub fn local(&self) -> &LocalVfs {
        &self.local
    }

    fn route(&self, spec: &Spec) -> Route<'_> {
        match spec {
            Spec::Local(_) => Route::Local,
            Spec::Stdin => self.stdin.as_ref().map_or(Route::MissingBlob, Route::Blob),
            Spec::Clipboard => match (&self.clipboard, &self.clipboard_failure) {
                (Some(blob), _) => Route::Blob(blob),
                (None, Some(failure)) => Route::FailedBlob(failure),
                (None, None) => Route::MissingBlob,
            },
            Spec::ArchiveMember { .. } => self
                .archives
                .as_deref()
                .map_or(Route::Unsupported, Route::Archive),
            _ => Route::Unsupported,
        }
    }
}

/// The entry of a blob: named after the spec (`-`, `clipboard://`), kind File, the blob's length and mtime.
fn blob_entry(spec: &Spec, blob: &Blob) -> Entry {
    let name = spec.canonical();
    let mut entry = Entry::new(FileName::from_os(OsStr::new(&name)), EntryKind::File);
    entry.size = blob.bytes.len() as u64;
    entry.mtime = Some(blob.mtime);
    entry
}

/// `Unsupported` for writing to a blob spec.
fn write_unsupported(spec: &Spec) -> VfsError {
    let what = match spec {
        Spec::Stdin => "writing to standard input",
        Spec::Clipboard => "writing to the clipboard",
        _ => unsupported_what(spec),
    };
    VfsError::Unsupported {
        spec: spec.canonical(),
        what,
    }
}

impl Vfs for VfsRouter {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        match self.route(at) {
            Route::Local => self.local.capabilities(at),
            Route::Blob(_) | Route::MissingBlob | Route::FailedBlob(_) => Ok(BLOB_CAPS),
            Route::Archive(archive) => archive.capabilities(at),
            Route::Unsupported => Err(unsupported(at)),
        }
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        match self.route(spec) {
            Route::Local => self.local.stat(spec, follow),
            Route::Blob(blob) => Ok(blob_entry(spec, blob)),
            Route::FailedBlob(failure) => Err(failure.error(spec)),
            Route::Archive(archive) => archive.stat(spec, follow),
            Route::MissingBlob | Route::Unsupported => Err(unsupported(spec)),
        }
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        match self.route(dir) {
            Route::Local => self.local.list(dir, opts, cancel),
            Route::Blob(_) | Route::MissingBlob | Route::FailedBlob(_) => {
                Err(VfsError::NotADirectory {
                    spec: dir.canonical(),
                })
            }
            Route::Archive(archive) => archive.list(dir, opts, cancel),
            Route::Unsupported => Err(unsupported(dir)),
        }
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        match self.route(spec) {
            Route::Local => self.local.open_read(spec),
            Route::Blob(blob) => Ok(Box::new(Cursor::new(Arc::clone(&blob.bytes)))),
            Route::FailedBlob(failure) => Err(failure.error(spec)),
            Route::Archive(archive) => archive.open_read(spec),
            Route::MissingBlob | Route::Unsupported => Err(unsupported(spec)),
        }
    }

    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        match self.route(spec) {
            Route::Local => self.local.open_write(spec, opts),
            Route::Archive(archive) => archive.open_write(spec, opts),
            Route::Blob(_) | Route::MissingBlob | Route::FailedBlob(_) | Route::Unsupported => {
                Err(write_unsupported(spec))
            }
        }
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        match self.route(dir) {
            Route::Local => self.local.child(dir, name),
            Route::Blob(_) | Route::MissingBlob | Route::FailedBlob(_) => {
                Err(VfsError::NotADirectory {
                    spec: dir.canonical(),
                })
            }
            Route::Archive(archive) => archive.child(dir, name),
            Route::Unsupported => Err(unsupported(dir)),
        }
    }

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        match self.route(spec) {
            Route::Local => self.local.local_path(spec),
            Route::Blob(_)
            | Route::MissingBlob
            | Route::FailedBlob(_)
            | Route::Archive(_)
            | Route::Unsupported => None,
        }
    }

    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.mutate(spec, VfsOp::Mkdir, |l: &dyn Vfs| {
            l.create_dir(spec, parents)
        })
    }

    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.mutate(spec, VfsOp::Remove, |l: &dyn Vfs| l.remove(spec))
    }

    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        match (self.route(from), self.route(to)) {
            (Route::Local, Route::Local) => self.local.rename(from, to, replace),
            (Route::Archive(archive), Route::Archive(_)) => {
                if from.container() == to.container() {
                    archive.rename(from, to, replace)
                } else {
                    Err(VfsError::CrossDevice {
                        from: from.canonical(),
                        to: to.canonical(),
                    })
                }
            }
            (Route::Unsupported, _) => Err(unsupported(from)),
            (_, Route::Unsupported) => Err(unsupported(to)),
            _ => Err(op_unsupported(from, VfsOp::Rename)),
        }
    }

    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        self.mutate(spec, VfsOp::SetTimes, |l: &dyn Vfs| {
            l.set_times(spec, times)
        })
    }

    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        self.mutate(spec, VfsOp::SetAttrs, |l: &dyn Vfs| {
            l.set_attrs(spec, change)
        })
    }

    fn create_symlink(
        &self,
        spec: &Spec,
        target: &OsStr,
        target_is_dir: bool,
    ) -> Result<(), VfsError> {
        self.mutate(spec, VfsOp::Symlink, |l: &dyn Vfs| {
            l.create_symlink(spec, target, target_is_dir)
        })
    }

    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        self.mutate(spec, VfsOp::Trash, |l: &dyn Vfs| l.move_to_trash(spec))
    }

    fn copy_file_fast(
        &self,
        from: &Spec,
        to: &Spec,
        opts: &WriteOptions,
        cancel: &CancelToken,
    ) -> Result<Option<u64>, VfsError> {
        match (self.route(from), self.route(to)) {
            (Route::Local, Route::Local) => self.local.copy_file_fast(from, to, opts, cancel),
            _ => Ok(None),
        }
    }

    fn prepare(&self, spec: &Spec, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        match self.route(spec) {
            Route::Archive(archive) => archive.prepare(spec, ctl),
            _ => Ok(()),
        }
    }

    fn archive_entries(&self, spec: &Spec, ctl: &Ctl<'_>) -> Result<Option<u64>, VfsError> {
        match self.route(spec) {
            Route::Archive(archive) => archive.archive_entries(spec, ctl),
            _ => Ok(None),
        }
    }

    fn open_read_with(
        &self,
        spec: &Spec,
        cancel: &CancelToken,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        match self.route(spec) {
            Route::Archive(archive) => archive.open_read_with(spec, cancel),
            _ => self.open_read(spec),
        }
    }

    fn archive_format(&self, name: &str) -> Option<ArchiveFormat> {
        self.archives.as_ref()?.archive_format(name)
    }

    fn take_warnings(&self) -> Vec<Warning> {
        self.archives
            .as_ref()
            .map_or_else(Vec::new, |archive| archive.take_warnings())
    }

    fn lock_writer(&self, spec: &Spec) -> Result<Option<Box<dyn WriteLock>>, VfsError> {
        match spec {
            Spec::Local(_) => self.local.lock_writer(spec),
            _ => Ok(None),
        }
    }

    fn shadows_of(&self, spec: &Spec) -> u64 {
        self.archives.as_ref().map_or(0, |vfs| vfs.shadows_of(spec))
    }

    fn shadows_below(&self, spec: &Spec) -> u64 {
        self.archives
            .as_ref()
            .map_or(0, |vfs| vfs.shadows_below(spec))
    }

    fn carry_older(&self, source: &Spec, target: &Spec, ctl: &Ctl<'_>) -> Result<u64, VfsError> {
        self.archives
            .as_ref()
            .map_or(Ok(0), |vfs| vfs.carry_older(source, target, ctl))
    }

    fn begin(&self, at: &Spec) -> Result<(), VfsError> {
        self.archives.as_ref().map_or(Ok(()), |a| a.begin(at))
    }

    fn finish(&self, at: &Spec, how: Finish) -> Result<(), VfsError> {
        self.archives.as_ref().map_or(Ok(()), |a| a.finish(at, how))
    }

    fn finish_with(&self, at: &Spec, how: Finish, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        self.archives
            .as_ref()
            .map_or(Ok(()), |a| a.finish_with(at, how, ctl))
    }

    /// `ArchiveVfs` makes the file, on the same `LocalVfs`, or the member. Without archives no plan asks for it.
    fn create_archive(&self, spec: &Spec) -> Result<(), VfsError> {
        match (&self.archives, self.route(spec)) {
            (Some(archives), Route::Local | Route::Archive(_)) => archives.create_archive(spec),
            _ => Err(op_unsupported(spec, VfsOp::Mkdir)),
        }
    }
}

impl VfsRouter {
    /// Dispatches a single-spec mutating primitive (table in the module documentation).
    fn mutate<T>(
        &self,
        spec: &Spec,
        op: VfsOp,
        call: impl FnOnce(&dyn Vfs) -> Result<T, VfsError>,
    ) -> Result<T, VfsError> {
        match self.route(spec) {
            Route::Local => call(&self.local),
            Route::Archive(archive) => call(archive),
            Route::Blob(_) | Route::MissingBlob | Route::FailedBlob(_) => {
                Err(op_unsupported(spec, op))
            }
            Route::Unsupported => Err(unsupported(spec)),
        }
    }
}
