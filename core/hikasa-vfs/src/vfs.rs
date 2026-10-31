//! The [`Vfs`] trait (VFS-001) and its companions: [`ReadSeek`], [`AtomicWrite`], [`FollowLinks`],
//! [`ListOptions`], [`Prefilter`], [`Listing`], [`NameOnly`], [`EntryError`], [`WriteOptions`], and the
//! arguments of the mutating primitives of M2 W3 ([`SetTimes`], [`AttrChange`]; contract M2 §1.3.1).
//!
//! The mutating primitives act on **one** item each and never follow the final symbolic link (D-94); every
//! one has a default body that returns [`VfsError::OperationUnsupported`] and a capability bit. Recursion,
//! copies across back-ends, moves, plans and per-item results live once in [`crate::ops`] (D-92).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use hikasa_core::{CancelToken, Ctl, ErrorReport, FileTime, Spec, Warning};
use serde::{Deserialize, Serialize};

use crate::archive::ArchiveFormat;
use crate::caps::Capabilities;
use crate::entry::{DosAttrs, Entry, EntryKind, FileName};
use crate::error::{VfsError, VfsOp};

/// Whether symlinks are followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowLinks {
    /// Report the link itself (lstat).
    #[default]
    No,
    /// Report the target (stat); broken links are reported unfollowed with an `EntryError`.
    Yes,
}

/// Decides, from the name and the kind found in the directory record, whether an entry is stat'ed
/// (`true`) or reported name-only (`false`, D-45). Called before any per-entry system call.
pub type Prefilter<'a> = &'a (dyn Fn(&str, EntryKind) -> bool + Sync);

/// Listing options.
#[derive(Clone, Copy, Default)]
pub struct ListOptions<'a> {
    /// Symlink handling.
    pub follow: FollowLinks,
    /// Optional prefilter. Back-ends call it only when the directory record states the kind (`d_type`,
    /// Windows find data); entries of unknown kind and symlinks are always stat'ed and never passed to it.
    /// `LocalVfs` reads the kind through `std::fs::DirEntry::file_type`, which `lstat`s an entry whose record
    /// states no kind; that entry is then passed to the prefilter with the kind found (results are the same,
    /// only the saved `stat` is lost on such file systems).
    pub prefilter: Option<Prefilter<'a>>,
}

impl fmt::Debug for ListOptions<'_> {
    /// Prints `follow` and whether a prefilter is set.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListOptions")
            .field("follow", &self.follow)
            .field("prefilter", &self.prefilter.is_some())
            .finish()
    }
}

/// An entry reported without metadata because the prefilter declined it (D-45).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameOnly {
    /// The entry name.
    pub name: FileName,
    /// Kind from the directory record (`Dir`, `File` or `Other`; never `Symlink`).
    pub kind: EntryKind,
}

/// Result of `list`. Order of `entries` and `name_only` is UNSPECIFIED (consumers must sort; INV-VFS-1).
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Entries that could be described.
    pub entries: Vec<Entry>,
    /// Entries the prefilter declined (not stat'ed).
    pub name_only: Vec<NameOnly>,
    /// Names whose metadata could not be read (the listing itself succeeded). **Not disjoint from `entries`:**
    /// with `FollowLinks::Yes` a symlink whose target cannot be stat'ed (broken link, loop) is in `entries`
    /// (unfollowed) and has an error here; a consumer attaches the error to that entry instead of creating a
    /// second item for the name.
    pub errors: Vec<EntryError>,
}

/// Total order of names: the UTF-8 (possibly lossy) bytes, then the exact OS name (two lossy names can share
/// their UTF-8 text).
fn name_order(a: &FileName, b: &FileName) -> Ordering {
    a.as_str().cmp(b.as_str()).then_with(|| {
        a.to_os_string()
            .as_encoded_bytes()
            .cmp(b.to_os_string().as_encoded_bytes())
    })
}

impl Listing {
    /// Sorts `entries`, `name_only` and `errors` by name bytes (the order of `hikasa vfs ls`, §4.3.4). The sort
    /// is total, so the result does not depend on the order in which the back-end listed the folder.
    pub fn sort_by_name(&mut self) {
        self.entries.sort_by(|a, b| name_order(&a.name, &b.name));
        self.name_only.sort_by(|a, b| name_order(&a.name, &b.name));
        self.errors.sort_by(|a, b| {
            name_order(&a.name, &b.name).then_with(|| a.error.message.cmp(&b.error.message))
        });
    }

    /// Number of distinct names in the listing: described and name-only entries, plus the failed names that are
    /// not also described. A broken or looping followed symlink is in `entries` **and** in `errors` (the lists
    /// are not disjoint) but is one name.
    pub fn len(&self) -> usize {
        let described: HashSet<&FileName> = if self.errors.is_empty() {
            HashSet::new()
        } else {
            self.entries.iter().map(|e| &e.name).collect()
        };
        let failed_only: HashSet<&FileName> = self
            .errors
            .iter()
            .map(|e| &e.name)
            .filter(|name| !described.contains(name))
            .collect();
        self.entries.len() + self.name_only.len() + failed_only.len()
    }

    /// True when the listing holds nothing.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.name_only.is_empty() && self.errors.is_empty()
    }
}

/// A per-entry failure inside a successful listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryError {
    /// The entry name.
    pub name: FileName,
    /// The failure.
    pub error: ErrorReport,
}

/// Write options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Create missing parent folders.
    pub create_parents: bool,
    /// Replace an existing file (otherwise `AlreadyExists`).
    pub overwrite: bool,
    /// M2 W3 (D-94; breaks struct literals: use `..Default::default()`). `false` (default, M1): a target that
    /// is a symbolic link is resolved and the file it points to is replaced (save semantics, `docs/11` §1 "Local
    /// writes"). `true`: the link itself is replaced by the new file and the file it pointed to is untouched
    /// (copy semantics).
    pub replace_link: bool,
}

/// What [`Vfs::set_times`] changes; `None` keeps the current value. The access time is never changed (D-99).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SetTimes {
    /// The new modification time.
    pub modified: Option<FileTime>,
    /// The new creation time (needs `SET_CREATED`).
    pub created: Option<FileTime>,
}

/// What [`Vfs::set_attrs`] changes. Owner and group are never changed (D-99).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrChange {
    /// Unix permission bits: `mode & 0o7777` is applied; other bits of the argument are an
    /// [`VfsError::InvalidAttributes`].
    Mode(u32),
    /// DOS attributes: `set` are turned on, `clear` turned off, the others kept. `set ∩ clear ≠ ∅` →
    /// [`VfsError::InvalidAttributes`].
    Dos {
        /// Attributes to turn on.
        set: DosAttrs,
        /// Attributes to turn off.
        clear: DosAttrs,
    },
}

impl AttrChange {
    /// Checks the argument itself: `Mode` with bits above `0o7777`, or `Dos` whose `set` and `clear` overlap,
    /// is [`VfsError::InvalidAttributes`].
    pub fn validate(&self) -> Result<(), VfsError> {
        match *self {
            AttrChange::Mode(mode) if mode & !0o7777 != 0 => Err(VfsError::InvalidAttributes {
                input: format!("{mode:o}"),
            }),
            AttrChange::Dos { set, clear } if set.intersects(clear) => {
                Err(VfsError::InvalidAttributes {
                    input: format!("+{} -{}", set.letters(), clear.letters()),
                })
            }
            AttrChange::Mode(_) | AttrChange::Dos { .. } => Ok(()),
        }
    }
}

/// `OperationUnsupported` for `op` at `spec` (the default body of every mutating primitive).
pub(crate) fn op_unsupported(spec: &Spec, op: VfsOp) -> VfsError {
    VfsError::OperationUnsupported {
        spec: spec.canonical(),
        op,
    }
}

/// `Read + Seek + Send`.
pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// An atomic writer: nothing is visible at the target until `commit`; dropping without `commit` discards the
/// temporary file (SES-053, INV-VFS-3). Exception (contract §1.2.7 item 12): a local target that is not a regular
/// file (a FIFO, a character or block device) is written to directly and is never replaced, so such a write is
/// not atomic.
pub trait AtomicWrite: Write + Send {
    /// Flushes, syncs and atomically renames onto the target.
    fn commit(self: Box<Self>) -> Result<(), VfsError>;
}

/// A virtual file system back-end (VFS-001). Implementations are `Send + Sync` and stateless per call
/// (the CLI and GUI share one instance across threads).
pub trait Vfs: Send + Sync {
    /// Capabilities at `at` (may depend on the mount for local paths; M1 uses platform defaults).
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError>;
    /// Metadata of `spec`.
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError>;
    /// Lists a folder. Checks `cancel` at least every 1024 entries. Honours `opts.prefilter` (D-45).
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError>;
    /// Opens for streaming reads.
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError>;
    /// Opens an atomic writer.
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError>;
    /// The spec of child `name` inside folder `dir`.
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError>;
    /// A local path for `spec` when it is a local regular file (enables mmap). The default body names its
    /// parameter `_spec` (§0.4 rule 6).
    fn local_path(&self, _spec: &Spec) -> Option<PathBuf> {
        None
    }

    // ---- mutating primitives (M2 W3, D-92; contract §1.3.1) ---------------------------------------------

    /// Creates the folder `spec`. `parents = false`: a missing parent is `NotFound`, an existing item of any kind
    /// is `AlreadyExists`. `parents = true`: missing ancestors are created; an existing folder (or a link to one)
    /// is success; an existing non-folder (at `spec` or an ancestor) is `NotADirectory`. `Ok(true)` when `spec`
    /// was created, `Ok(false)` when it already was a folder. Default: `OperationUnsupported`.
    fn create_dir(&self, spec: &Spec, _parents: bool) -> Result<bool, VfsError> {
        Err(op_unsupported(spec, VfsOp::Mkdir))
    }

    /// Removes one item: a file, a special file, a symbolic link (the link itself, never its target; on Windows
    /// also a folder link or junction) or an **empty** folder. A non-empty folder is `NotEmpty`; a missing item
    /// `NotFound`. Default: `OperationUnsupported`.
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        Err(op_unsupported(spec, VfsOp::Remove))
    }

    /// Renames within this back-end (atomic where the platform renames atomically). The final components of
    /// `from` and `to` are never followed (a link is renamed, or replaced, as a link). `replace = false`: an
    /// existing `to` is `AlreadyExists`. `replace = true`: a file or link replaces a file or link; a folder
    /// replaces only an empty folder (else `NotEmpty`); folder ↔ non-folder is `IsADirectory`/`NotADirectory`.
    /// A rename whose `to` is the same item as `from` (case-only rename on a case-insensitive back-end) changes
    /// the spelling and always succeeds. Different file systems → `CrossDevice`. Default:
    /// `OperationUnsupported`.
    fn rename(&self, from: &Spec, _to: &Spec, _replace: bool) -> Result<(), VfsError> {
        Err(op_unsupported(from, VfsOp::Rename))
    }

    /// Sets the times of the item itself; a symbolic link gets its own times where the platform allows (else
    /// `OperationUnsupported`). `created` needs `SET_CREATED`, else `OperationUnsupported` (nothing changed).
    /// Default: `OperationUnsupported`.
    fn set_times(&self, spec: &Spec, _times: &SetTimes) -> Result<(), VfsError> {
        Err(op_unsupported(spec, VfsOp::SetTimes))
    }

    /// Changes the attributes of the item itself. On a symbolic link → `OperationUnsupported`; the wrong family
    /// (`Mode` on a DOS back-end, `Dos` on a Unix one) → `OperationUnsupported`. Default:
    /// `OperationUnsupported`.
    fn set_attrs(&self, spec: &Spec, _change: &AttrChange) -> Result<(), VfsError> {
        Err(op_unsupported(spec, VfsOp::SetAttrs))
    }

    /// Creates a symbolic link at `spec` whose target text is `target`, stored verbatim (never resolved or
    /// rewritten). `target_is_dir` makes a folder link on Windows and is ignored elsewhere. Existing item →
    /// `AlreadyExists`; no privilege (Windows) → `PermissionDenied`. Default: `OperationUnsupported`.
    fn create_symlink(
        &self,
        spec: &Spec,
        _target: &OsStr,
        _target_is_dir: bool,
    ) -> Result<(), VfsError> {
        Err(op_unsupported(spec, VfsOp::Symlink))
    }

    /// Moves one item (a file, a link as a link, or a whole folder) to the platform's recycle bin. Never falls
    /// back to a permanent delete: any failure is an error and the item stays (D-98). Default:
    /// `OperationUnsupported`.
    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        Err(op_unsupported(spec, VfsOp::Trash))
    }

    /// A back-end-specific copy of the regular file `from` to `to` (both on this back-end) with exactly the
    /// semantics of `open_write(to, opts)` + streaming + `commit`: atomic, target untouched on failure or
    /// cancellation. `Ok(None)`: no fast path for these specs (the caller streams). Default: `Ok(None)`.
    fn copy_file_fast(
        &self,
        _from: &Spec,
        _to: &Spec,
        _opts: &WriteOptions,
        _cancel: &CancelToken,
    ) -> Result<Option<u64>, VfsError> {
        Ok(None)
    }

    // ---- archives (M2 W5, D-136, D-139, D-145; contract §1.5.3) -------------------------------------------

    /// Does the slow part of reaching `spec` now, with cancellation and progress: for a spec inside an archive, the
    /// index of every archive around it (D-139). Default: nothing.
    fn prepare(&self, _spec: &Spec, _ctl: &Ctl<'_>) -> Result<(), VfsError> {
        Ok(())
    }

    /// [`Vfs::prepare`], then the members of the archive that directly holds `spec`, the folders it synthesizes
    /// included; `None` when `spec` is not inside an archive. A scan budgets by this before it opens one (D-151).
    fn archive_entries(&self, _spec: &Spec, _ctl: &Ctl<'_>) -> Result<Option<u64>, VfsError> {
        Ok(None)
    }

    /// `open_read` that can be cancelled while an archive member is decoded (`Cancelled`). Default: `open_read`.
    fn open_read_with(
        &self,
        spec: &Spec,
        _cancel: &CancelToken,
    ) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.open_read(spec)
    }

    /// The archive format of a file name per the masks; `None` on back-ends without archives.
    fn archive_format(&self, _name: &str) -> Option<ArchiveFormat> {
        None
    }

    /// Warnings raised since the last call (D-145). Default: none.
    fn take_warnings(&self) -> Vec<Warning> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::ErrorKind;

    fn report(message: &str) -> ErrorReport {
        ErrorReport {
            kind: ErrorKind::Io,
            message: message.into(),
            spec: None,
            causes: Vec::new(),
        }
    }

    #[test]
    fn sort_by_name_is_total() {
        let mut listing = Listing {
            entries: ["b", "a", "c"]
                .map(|n| Entry::new(FileName::new(n).unwrap(), EntryKind::File))
                .to_vec(),
            name_only: ["z", "y"]
                .map(|n| NameOnly {
                    name: FileName::new(n).unwrap(),
                    kind: EntryKind::Dir,
                })
                .to_vec(),
            errors: vec![
                EntryError {
                    name: FileName::new("e").unwrap(),
                    error: report("2"),
                },
                EntryError {
                    name: FileName::new("e").unwrap(),
                    error: report("1"),
                },
            ],
        };
        listing.sort_by_name();
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(listing.name_only[0].name.as_str(), "y");
        assert_eq!(listing.errors[0].error.message, "1");
        // Distinct names: 3 entries, 2 name-only, 1 failed name ("e" twice).
        assert_eq!(listing.len(), 6);
        assert!(!listing.is_empty());
        assert!(Listing::default().is_empty());
    }

    /// Regression: a followed broken link is in `entries` and in `errors` but counts as one name.
    #[test]
    fn len_counts_a_failed_described_name_once() {
        let link = FileName::new("broken").unwrap();
        let listing = Listing {
            entries: vec![
                Entry::new(link.clone(), EntryKind::Symlink),
                Entry::new(FileName::new("ok").unwrap(), EntryKind::File),
            ],
            name_only: Vec::new(),
            errors: vec![EntryError {
                name: link,
                error: report("not found"),
            }],
        };
        assert_eq!(listing.len(), 2);
        let only_errors = Listing {
            errors: listing.errors.clone(),
            ..Listing::default()
        };
        assert_eq!(only_errors.len(), 1);
        assert!(!only_errors.is_empty());
    }

    #[test]
    fn list_options_debug_hides_the_closure() {
        let accept = |_: &str, _: EntryKind| true;
        let opts = ListOptions {
            follow: FollowLinks::Yes,
            prefilter: Some(&accept),
        };
        assert_eq!(
            format!("{opts:?}"),
            "ListOptions { follow: Yes, prefilter: true }"
        );
    }
}
