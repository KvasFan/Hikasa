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

use std::ffi::OsStr;
use std::fmt;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::spec::RemoteScheme;
use hikasa_core::{CancelToken, FileTime, Spec};

use crate::caps::Capabilities;
use crate::entry::{Entry, EntryKind, FileName};
use crate::error::VfsError;
use crate::local::LocalVfs;
use crate::vfs::{AtomicWrite, FollowLinks, ListOptions, Listing, ReadSeek, Vfs, WriteOptions};

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

/// Dispatches specs to back-ends (table in the module documentation). This is the `Vfs` the session layer
/// uses.
#[derive(Debug, Clone)]
pub struct VfsRouter {
    local: LocalVfs,
    stdin: Option<Blob>,
    clipboard: Option<Blob>,
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
    Unsupported,
}

impl VfsRouter {
    /// Router over a local back-end.
    pub fn new(local: LocalVfs) -> VfsRouter {
        VfsRouter {
            local,
            stdin: None,
            clipboard: None,
        }
    }

    /// Makes `Spec::Stdin` readable.
    pub fn with_stdin(self, blob: Blob) -> VfsRouter {
        VfsRouter {
            stdin: Some(blob),
            ..self
        }
    }

    /// Makes `Spec::Clipboard` readable (no clipboard crate in M1: adapters may leave this unset).
    pub fn with_clipboard(self, blob: Blob) -> VfsRouter {
        VfsRouter {
            clipboard: Some(blob),
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
            Spec::Clipboard => self
                .clipboard
                .as_ref()
                .map_or(Route::MissingBlob, Route::Blob),
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
            Route::Blob(_) | Route::MissingBlob => Ok(BLOB_CAPS),
            Route::Unsupported => Err(unsupported(at)),
        }
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        match self.route(spec) {
            Route::Local => self.local.stat(spec, follow),
            Route::Blob(blob) => Ok(blob_entry(spec, blob)),
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
            Route::Blob(_) | Route::MissingBlob => Err(VfsError::NotADirectory {
                spec: dir.canonical(),
            }),
            Route::Unsupported => Err(unsupported(dir)),
        }
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        match self.route(spec) {
            Route::Local => self.local.open_read(spec),
            Route::Blob(blob) => Ok(Box::new(Cursor::new(Arc::clone(&blob.bytes)))),
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
            Route::Blob(_) | Route::MissingBlob | Route::Unsupported => {
                Err(write_unsupported(spec))
            }
        }
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        match self.route(dir) {
            Route::Local => self.local.child(dir, name),
            Route::Blob(_) | Route::MissingBlob => Err(VfsError::NotADirectory {
                spec: dir.canonical(),
            }),
            Route::Unsupported => Err(unsupported(dir)),
        }
    }

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        match self.route(spec) {
            Route::Local => self.local.local_path(spec),
            Route::Blob(_) | Route::MissingBlob | Route::Unsupported => None,
        }
    }
}
