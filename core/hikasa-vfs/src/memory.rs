//! [`MemoryVfs`]: a deterministic in-memory file system for tests (and future clipboard/stdin trees).
//!
//! Paths are `Spec::Local` absolute paths in the host's syntax (`/a/b` on Unix, `C:\a\b` on Windows).
//! Semantics follow a POSIX file system:
//!
//! - **Symlinks.** Intermediate symlinks are always followed; the final component is followed by
//!   [`FollowLinks::Yes`] (`stat`, `list`) and always by `open_read`, `list` of the folder itself and writes into
//!   a folder. A relative target is resolved from the link's folder, an absolute one from its root. More than
//!   40 links on one resolution is a loop: the same `io` error the local file system gives (the platform's
//!   `ELOOP`, "Too many levels of symbolic links", on Linux, macOS and the BSDs; `ERROR_CANT_RESOLVE_FILENAME`
//!   on Windows). A broken or looping link listed with `FollowLinks::Yes` appears unfollowed in `entries`
//!   **and** in `errors`.
//! - **Prefilter (D-45).** Every kind is known, so every non-symlink entry is passed to the prefilter; declined
//!   entries are reported in `Listing::name_only` and never stat'ed ([`MemoryVfs::stat_calls`] counts every
//!   metadata read, including the per-entry reads of `list`). [`MemoryVfs::set_kind_unknown`] simulates a
//!   directory record without a kind (`DT_UNKNOWN`): such an entry is always stat'ed and never passed to the
//!   prefilter. `list` checks the cancellation token every 1024 entries, while filtering and while describing.
//! - **Listing order** ([`ListingOrder`]) exposes bugs that depend on order (INV-VFS-1).
//! - **Atomic writes.** Nothing is visible until `commit`; dropping the writer discards the content
//!   (INV-VFS-3). Written files get [`MemMeta::default`] (no clock is read, T1); a replaced file keeps its
//!   times and attributes and loses its CRC.
//! - **Case.** With [`CaseSensitivity::Insensitive`] names are looked up by their simple 1:1 case fold
//!   (`hikasa_core::case::fold`); the spelling of the last insert is kept.
//!
//! # Fault injection
//!
//! A [`Fault`] makes one operation on one path fail; the error classifies as the fault's [`ErrorKind`]
//! (`protocol` and `internal`, which no file system error carries, classify as `io`). Paths are compared
//! lexically (after case folding when insensitive), as given to the operation:
//!
//! | `VfsOp` | Effect |
//! |---|---|
//! | `Stat` | `stat(path)` fails; listing the parent reports an [`EntryError`] for the entry (unless the prefilter declined it: name-only entries are never stat'ed) |
//! | `List` | `list(path)` fails |
//! | `Open` | `open_read(path)` fails |
//! | `Read` | `open_read(path)` succeeds; the first `read` fails with an `io::Error` of the kind |
//! | `Write` | `open_write(path)` fails |
//! | `Commit` | `commit` of a writer for `path` fails; nothing is written |
//! | `Map` | no effect: `MemoryVfs` has no local paths, so nothing is ever mapped |
//! | `Mkdir`, `Remove`, `Rename`, `SetTimes`, `SetAttrs`, `Symlink`, `Trash` | the primitive on that path fails with the kind (`Rename` matches the `from` path); nothing changes (M2 W3) |
//!
//! # Mutations (M2 W3, contract M2 §1.3.8)
//!
//! Every mutating primitive of [`Vfs`] is implemented with POSIX semantics and honours
//! [`MemoryVfs::set_capabilities`] (a primitive whose bit is removed returns `OperationUnsupported`). Test hooks:
//! [`MemoryVfs::inject_read_error_after`] and [`MemoryVfs::inject_write_error_after`] (a stream that fails after
//! N bytes), [`MemoryVfs::set_device`] (`FileId::dev` of a subtree; `rename` across devices is `CrossDevice`),
//! [`MemoryVfs::set_journal`]/[`MemoryVfs::journal`] (every successful mutating call in order, `commit` as
//! `Commit`), [`MemoryVfs::trashed`] (what `move_to_trash` took, in order) and [`MemoryVfs::dump`] (a
//! deterministic snapshot of the tree). A write without `replace_link` onto a symbolic link replaces the file the
//! link points to (a dangling link: the file it names is created, as `LocalVfs` does); with `replace_link` the
//! link is replaced.
//!
//! Errors of kind not found, permission denied, not a folder, is a folder and already exists use the
//! dedicated [`VfsError`] variants; cancelled is [`VfsError::Cancelled`]; every other kind is
//! [`VfsError::Io`] whose source has the matching `std::io::ErrorKind`.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Component, MAIN_SEPARATOR, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use hikasa_core::{
    CancelToken, Cancelled, CaseSensitivity, ErrorKind, ErrorReport, FileTime, LocalPath, Spec,
    case,
};

use crate::caps::Capabilities;
use crate::entry::{Attrs, Entry, EntryKind, FileId, FileName, UnixMeta};
use crate::error::{VfsError, VfsOp};
use crate::vfs::{
    AtomicWrite, AttrChange, EntryError, FollowLinks, ListOptions, Listing, NameOnly, ReadSeek,
    SetTimes, Vfs, WriteOptions, op_unsupported,
};

/// Most symlinks followed in one resolution (Linux `MAXSYMLINKS`).
const MAX_LINK_HOPS: u32 = 40;

/// Deterministic in-memory file system for tests (and future clipboard/stdin trees). Paths are
/// `Spec::Local` absolute paths. Thread-safe (std `RwLock` inside).
pub struct MemoryVfs {
    shared: Arc<Shared>,
}

/// Metadata for inserted entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemMeta {
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// Attributes (`None` = `Attrs::None`).
    pub attrs: Option<Attrs>,
    /// Back-end CRC (simulates archives).
    pub crc32: Option<u32>,
}

/// Listing order produced by `MemoryVfs::list`. It exposes bugs that depend on order (INV-VFS-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListingOrder {
    /// Byte order of names.
    #[default]
    Sorted,
    /// Reverse byte order.
    Reversed,
    /// Deterministic shuffle (xorshift64* seeded with `seed`).
    Shuffled {
        /// The seed (0 is replaced by a fixed non-zero constant: xorshift needs a non-zero state).
        seed: u64,
    },
}

impl ListingOrder {
    /// Reorders `items` (given in byte order of names).
    fn apply<T>(self, items: &mut [T]) {
        match self {
            ListingOrder::Sorted => {}
            ListingOrder::Reversed => items.reverse(),
            ListingOrder::Shuffled { seed } => {
                let mut state = if seed == 0 {
                    0x9E37_79B9_7F4A_7C15
                } else {
                    seed
                };
                for i in (1..items.len()).rev() {
                    state ^= state >> 12;
                    state ^= state << 25;
                    state ^= state >> 27;
                    let r = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
                    let j = (r % (i as u64 + 1)) as usize;
                    items.swap(i, j);
                }
            }
        }
    }
}

/// An injected failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// Affected path.
    pub path: String,
    /// Operation that fails.
    pub op: VfsOp,
    /// Resulting error kind.
    pub kind: ErrorKind,
}

/// The `io::Error` an injected fault of `kind` produces.
fn fault_io(kind: ErrorKind) -> io::Error {
    let io_kind = match kind {
        ErrorKind::NotFound => io::ErrorKind::NotFound,
        ErrorKind::PermissionDenied => io::ErrorKind::PermissionDenied,
        ErrorKind::AlreadyExists => io::ErrorKind::AlreadyExists,
        ErrorKind::NotADirectory => io::ErrorKind::NotADirectory,
        ErrorKind::IsADirectory => io::ErrorKind::IsADirectory,
        ErrorKind::InvalidInput => io::ErrorKind::InvalidInput,
        ErrorKind::InvalidData => io::ErrorKind::InvalidData,
        ErrorKind::Unsupported => io::ErrorKind::Unsupported,
        ErrorKind::TooLarge => io::ErrorKind::FileTooLarge,
        ErrorKind::NotEmpty => io::ErrorKind::DirectoryNotEmpty,
        ErrorKind::Cancelled => return io::Error::other(Cancelled),
        // Io, and kinds no file system error carries (protocol, internal).
        _ => io::ErrorKind::Other,
    };
    io::Error::new(io_kind, "injected fault")
}

/// The `VfsError` an injected fault produces.
fn fault_error(spec: &str, op: VfsOp, kind: ErrorKind) -> VfsError {
    VfsError::from_io(spec, op, fault_io(kind))
}

/// One component of a parsed path.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Comp {
    Name(String),
    Parent,
}

/// An absolute path split into its root text and components.
#[derive(Debug, Clone)]
struct Parsed {
    root: String,
    comps: Vec<Comp>,
}

/// Splits an absolute path; `None` for relative paths.
fn parse_path(path: &Path) -> Option<Parsed> {
    let mut root = String::new();
    let mut rooted = false;
    let mut comps = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => root.push_str(&prefix.as_os_str().to_string_lossy()),
            Component::RootDir => {
                root.push(MAIN_SEPARATOR);
                rooted = true;
            }
            Component::CurDir => {}
            Component::ParentDir => comps.push(Comp::Parent),
            Component::Normal(name) => comps.push(Comp::Name(name.to_string_lossy().into_owned())),
        }
    }
    rooted.then_some(Parsed { root, comps })
}

/// A symlink target: an absolute path (`Some(root)`) or components relative to the link's folder.
fn parse_target(target: &str) -> (Option<String>, Vec<Comp>) {
    match parse_path(Path::new(target)) {
        Some(parsed) => (Some(parsed.root), parsed.comps),
        None => {
            let comps = Path::new(target)
                .components()
                .filter_map(|c| match c {
                    Component::ParentDir => Some(Comp::Parent),
                    Component::Normal(name) => {
                        Some(Comp::Name(name.to_string_lossy().into_owned()))
                    }
                    Component::CurDir | Component::Prefix(_) | Component::RootDir => None,
                })
                .collect();
            (None, comps)
        }
    }
}

/// Why a path did not resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Miss {
    NotFound,
    NotADirectory,
    Loop,
}

fn miss_error(miss: Miss, spec: String, op: VfsOp) -> VfsError {
    match miss {
        Miss::NotFound => VfsError::NotFound { spec },
        Miss::NotADirectory => VfsError::NotADirectory { spec },
        Miss::Loop => VfsError::Io {
            spec,
            op,
            source: link_loop_error(),
        },
    }
}

/// The error the local file system gives for a symlink loop: the OS error `ELOOP` (or Windows'
/// `ERROR_CANT_RESOLVE_FILENAME`), so that its `std::io::ErrorKind` and message match `LocalVfs` (§5.5 rule 3).
/// On a platform whose code is not known here: an `Other` error with the Unix message.
fn link_loop_error() -> io::Error {
    // ELOOP = 40 in the generic Linux errno table (MIPS, SPARC, Alpha and PA-RISC differ).
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        not(any(
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "mips32r6",
            target_arch = "mips64r6",
            target_arch = "sparc",
            target_arch = "sparc64"
        ))
    ))]
    const CODE: Option<i32> = Some(40);
    // ELOOP = 62 in the BSD errno table.
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    const CODE: Option<i32> = Some(62);
    // ERROR_CANT_RESOLVE_FILENAME.
    #[cfg(windows)]
    const CODE: Option<i32> = Some(1921);
    #[cfg(not(any(
        all(
            any(target_os = "linux", target_os = "android"),
            not(any(
                target_arch = "mips",
                target_arch = "mips64",
                target_arch = "mips32r6",
                target_arch = "mips64r6",
                target_arch = "sparc",
                target_arch = "sparc64"
            ))
        ),
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        windows
    )))]
    const CODE: Option<i32> = None;
    match CODE {
        Some(code) => io::Error::from_raw_os_error(code),
        None => io::Error::other("Too many levels of symbolic links"),
    }
}

#[derive(Debug, Clone)]
enum NodeKind {
    Dir,
    File(Arc<[u8]>),
    Symlink(String),
}

impl NodeKind {
    fn entry_kind(&self) -> EntryKind {
        match self {
            NodeKind::Dir => EntryKind::Dir,
            NodeKind::File(_) => EntryKind::File,
            NodeKind::Symlink(_) => EntryKind::Symlink,
        }
    }
}

#[derive(Debug)]
struct Node {
    /// Name as last inserted (a root: its path text).
    name: String,
    /// Parent folder (`None` for a root).
    parent: Option<usize>,
    kind: NodeKind,
    meta: MemMeta,
    /// Lookup key of the name → child node.
    children: BTreeMap<String, usize>,
}

struct Shared {
    tree: RwLock<Tree>,
    stat_calls: AtomicU64,
}

#[derive(Debug)]
struct Tree {
    case: CaseSensitivity,
    caps: Capabilities,
    order: ListingOrder,
    /// Lookup key of a root's text → root node.
    roots: BTreeMap<String, usize>,
    nodes: Vec<Node>,
    /// Faults with the lexical key of their path (`None` when the path is not absolute: never matches).
    faults: Vec<(Fault, Option<String>)>,
    /// Nodes whose directory record states no kind (`MemoryVfs::set_kind_unknown`).
    kind_unknown: BTreeSet<usize>,
    /// Streams that fail after a number of bytes: (path key, bytes, kind).
    read_errors: Vec<(String, u64, ErrorKind)>,
    /// Writers that fail after a number of bytes: (path key, bytes, kind).
    write_errors: Vec<(String, u64, ErrorKind)>,
    /// Device numbers of subtrees: (path key of the prefix, dev).
    devices: Vec<(String, u64)>,
    /// `Some` while the journal records.
    journal: Option<Vec<(VfsOp, String)>>,
    /// Original paths moved to the trash, in order.
    trashed: Vec<String>,
}

/// One item of [`MemoryVfs::dump`]: everything a "target untouched" assertion compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemDump {
    /// The path (host syntax), with the kept spelling.
    pub path: String,
    /// The kind (a link is `Symlink`).
    pub kind: EntryKind,
    /// Content length (files), target length (links), 0 (folders).
    pub len: u64,
    /// CRC-32 of the content (files only).
    pub crc32: Option<u32>,
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// Attributes.
    pub attrs: Attrs,
    /// Link target (links only).
    pub link_target: Option<String>,
}

impl Tree {
    fn key(&self, name: &str) -> String {
        match self.case {
            CaseSensitivity::Sensitive => name.to_owned(),
            CaseSensitivity::Insensitive => case::fold(name).into_owned(),
        }
    }

    /// The lexical key of a parsed path (fault matching).
    fn path_key(&self, parsed: &Parsed) -> String {
        let mut key = self.key(&parsed.root);
        for comp in &parsed.comps {
            key.push('\0');
            match comp {
                Comp::Name(name) => key.push_str(&self.key(name)),
                Comp::Parent => key.push_str(".."),
            }
        }
        key
    }

    /// The kind of the first fault for `op` at `parsed`.
    fn fault(&self, parsed: &Parsed, op: VfsOp) -> Option<ErrorKind> {
        if self.faults.is_empty() {
            return None;
        }
        let key = self.path_key(parsed);
        self.faults
            .iter()
            .find(|(fault, fault_key)| fault.op == op && fault_key.as_deref() == Some(key.as_str()))
            .map(|(fault, _)| fault.kind)
    }

    fn node(&self, idx: usize) -> Option<&Node> {
        self.nodes.get(idx)
    }

    fn is_dir(&self, idx: usize) -> bool {
        self.node(idx)
            .is_some_and(|n| matches!(n.kind, NodeKind::Dir))
    }

    fn root(&self, text: &str) -> Result<usize, Miss> {
        self.roots
            .get(&self.key(text))
            .copied()
            .ok_or(Miss::NotFound)
    }

    /// Walks `comps` from folder `start`. Intermediate symlinks are followed; the final one when
    /// `follow_final`. `hops` counts the links followed so far.
    fn walk(
        &self,
        start: usize,
        comps: &[Comp],
        follow_final: bool,
        hops: &mut u32,
    ) -> Result<usize, Miss> {
        let mut cur = start;
        let mut pending: Vec<Comp> = comps.iter().rev().cloned().collect();
        while let Some(comp) = pending.pop() {
            let node = self.node(cur).ok_or(Miss::NotFound)?;
            if !matches!(node.kind, NodeKind::Dir) {
                return Err(Miss::NotADirectory);
            }
            match comp {
                Comp::Parent => cur = node.parent.unwrap_or(cur),
                Comp::Name(name) => {
                    let child = *node.children.get(&self.key(&name)).ok_or(Miss::NotFound)?;
                    let last = pending.is_empty();
                    match self.node(child).map(|n| &n.kind) {
                        Some(NodeKind::Symlink(target)) if !last || follow_final => {
                            *hops += 1;
                            if *hops > MAX_LINK_HOPS {
                                return Err(Miss::Loop);
                            }
                            if target.is_empty() {
                                return Err(Miss::NotFound);
                            }
                            let (root, target_comps) = parse_target(target);
                            if let Some(root) = root {
                                cur = self.root(&root)?;
                            }
                            pending.extend(target_comps.into_iter().rev());
                        }
                        _ => cur = child,
                    }
                }
            }
        }
        Ok(cur)
    }

    fn resolve(&self, parsed: &Parsed, follow_final: bool) -> Result<usize, Miss> {
        let root = self.root(&parsed.root)?;
        self.walk(root, &parsed.comps, follow_final, &mut 0)
    }

    /// Where a write through the link `link` lands when the chain of links ends at a missing item (a dangling
    /// link names the file to create, as `LocalVfs` resolves it): the parent folder and the name of the first
    /// path of the chain that is not a link. `Miss::NotFound` when that parent is missing.
    fn dangling_end(&self, link: usize) -> Result<(Parsed, String), Miss> {
        let mut cur = link;
        for _ in 0..MAX_LINK_HOPS {
            let node = self.node(cur).ok_or(Miss::NotFound)?;
            let NodeKind::Symlink(target) = &node.kind else {
                return Err(Miss::NotFound);
            };
            if target.is_empty() {
                return Err(Miss::NotFound);
            }
            let (root, comps) = parse_target(target);
            let mut base = match root {
                Some(root) => Parsed {
                    root,
                    comps: Vec::new(),
                },
                None => {
                    let dir = node.parent.ok_or(Miss::NotFound)?;
                    parse_path(&self.node_path(dir)).ok_or(Miss::NotFound)?
                }
            };
            base.comps.extend(comps);
            let Some(Comp::Name(name)) = base.comps.pop() else {
                return Err(Miss::NotFound);
            };
            let dir = self.resolve(&base, true)?;
            if !self.is_dir(dir) {
                return Err(Miss::NotADirectory);
            }
            match self
                .node(dir)
                .and_then(|n| n.children.get(&self.key(&name)).copied())
            {
                Some(next)
                    if matches!(self.node(next).map(|n| &n.kind), Some(NodeKind::Symlink(_))) =>
                {
                    cur = next;
                }
                _ => return Ok((base, name)),
            }
        }
        Err(Miss::Loop)
    }

    /// The node a symlink node points to (fully followed).
    fn follow_link(&self, link: usize) -> Result<usize, Miss> {
        let node = self.node(link).ok_or(Miss::NotFound)?;
        let NodeKind::Symlink(target) = &node.kind else {
            return Ok(link);
        };
        if target.is_empty() {
            return Err(Miss::NotFound);
        }
        let (root, comps) = parse_target(target);
        let start = match root {
            Some(root) => self.root(&root)?,
            None => node.parent.ok_or(Miss::NotFound)?,
        };
        self.walk(start, &comps, true, &mut 1)
    }

    fn entry_of(&self, idx: usize, name: FileName, followed: bool, link: Option<String>) -> Entry {
        let Some(node) = self.node(idx) else {
            return Entry::new(name, EntryKind::Other);
        };
        let (kind, size, link_target) = match &node.kind {
            NodeKind::Dir => (EntryKind::Dir, 0, link),
            NodeKind::File(content) => (EntryKind::File, content.len() as u64, link),
            NodeKind::Symlink(target) => (
                EntryKind::Symlink,
                target.len() as u64,
                Some(target.clone()),
            ),
        };
        Entry {
            name,
            kind,
            size,
            mtime: node.meta.mtime,
            ctime: node.meta.ctime,
            attrs: node.meta.attrs.unwrap_or(Attrs::None),
            link_target,
            // MemoryVfs targets are UTF-8 strings: never lossy.
            link_target_raw: None,
            followed,
            crc32: node.meta.crc32,
            file_id: Some(FileId {
                dev: self.dev_of(idx),
                ino: idx as u64 + 1,
            }),
        }
    }

    /// Describes node `idx` named `name`; a symlink is followed per `follow`. A link that cannot be followed is
    /// described unfollowed, and the miss returned as well.
    fn describe(&self, idx: usize, name: FileName, follow: FollowLinks) -> (Entry, Option<Miss>) {
        let link = match self.node(idx).map(|n| &n.kind) {
            Some(NodeKind::Symlink(target)) if follow == FollowLinks::Yes => target.clone(),
            _ => return (self.entry_of(idx, name, false, None), None),
        };
        match self.follow_link(idx) {
            Ok(target) => (self.entry_of(target, name, true, Some(link)), None),
            Err(miss) => (self.entry_of(idx, name, false, None), Some(miss)),
        }
    }

    fn push_node(
        &mut self,
        name: &str,
        parent: Option<usize>,
        kind: NodeKind,
        meta: MemMeta,
    ) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(Node {
            name: name.to_owned(),
            parent,
            kind,
            meta,
            children: BTreeMap::new(),
        });
        if let Some(parent) = parent {
            let key = self.key(name);
            if let Some(p) = self.nodes.get_mut(parent) {
                p.children.insert(key, idx);
            }
        }
        idx
    }

    fn root_or_create(&mut self, text: &str) -> usize {
        match self.root(text) {
            Ok(idx) => idx,
            Err(_) => {
                let idx = self.push_node(text, None, NodeKind::Dir, MemMeta::default());
                let key = self.key(text);
                self.roots.insert(key, idx);
                idx
            }
        }
    }

    /// Walks `comps` from `start`, creating missing folders (`mkdir -p`; existing symlinks are followed).
    fn make_dirs(&mut self, start: usize, comps: &[Comp]) -> Result<usize, Miss> {
        let mut cur = start;
        for comp in comps {
            if !self.is_dir(cur) {
                return Err(Miss::NotADirectory);
            }
            match comp {
                Comp::Parent => cur = self.node(cur).and_then(|n| n.parent).unwrap_or(cur),
                Comp::Name(name) => {
                    let key = self.key(name);
                    let existing = self.node(cur).and_then(|n| n.children.get(&key).copied());
                    cur = match existing {
                        Some(child) => match self.node(child).map(|n| &n.kind) {
                            Some(NodeKind::Symlink(_)) => self.follow_link(child)?,
                            _ => child,
                        },
                        None => self.push_node(name, Some(cur), NodeKind::Dir, MemMeta::default()),
                    };
                }
            }
        }
        if self.is_dir(cur) {
            Ok(cur)
        } else {
            Err(Miss::NotADirectory)
        }
    }

    /// Inserts `kind` at `parsed` (creating parents). An existing entry of the same category (folder, or
    /// file/symlink) is replaced; a folder keeps its children.
    fn insert(
        &mut self,
        parsed: &Parsed,
        kind: NodeKind,
        meta: MemMeta,
        spec: &str,
    ) -> Result<(), VfsError> {
        let root = self.root_or_create(&parsed.root);
        let Some((last, parents)) = parsed.comps.split_last() else {
            // The root itself.
            return match kind {
                NodeKind::Dir => {
                    if let Some(node) = self.nodes.get_mut(root) {
                        node.meta = meta;
                    }
                    Ok(())
                }
                NodeKind::File(_) | NodeKind::Symlink(_) => Err(VfsError::IsADirectory {
                    spec: spec.to_owned(),
                }),
            };
        };
        let Comp::Name(name) = last else {
            return Err(VfsError::InvalidName {
                name: spec.to_owned(),
            });
        };
        let dir = self
            .make_dirs(root, parents)
            .map_err(|miss| miss_error(miss, spec.to_owned(), VfsOp::Write))?;
        let key = self.key(name);
        let existing = self.node(dir).and_then(|n| n.children.get(&key).copied());
        match existing {
            None => {
                self.push_node(name, Some(dir), kind, meta);
                Ok(())
            }
            Some(idx) => {
                let Some(node) = self.nodes.get_mut(idx) else {
                    return Err(VfsError::NotFound {
                        spec: spec.to_owned(),
                    });
                };
                let same_category =
                    matches!(node.kind, NodeKind::Dir) == matches!(kind, NodeKind::Dir);
                if !same_category {
                    return Err(VfsError::AlreadyExists {
                        spec: spec.to_owned(),
                    });
                }
                node.name = name.clone();
                node.meta = meta;
                if !matches!(kind, NodeKind::Dir) {
                    node.kind = kind;
                }
                Ok(())
            }
        }
    }
}

impl Tree {
    /// The path key of node `idx` (the same key `path_key` builds for its path).
    fn node_key(&self, idx: usize) -> String {
        let mut names = Vec::new();
        let mut cur = Some(idx);
        while let Some(i) = cur {
            let Some(node) = self.node(i) else { break };
            names.push(node.name.as_str());
            cur = node.parent;
        }
        let mut key = String::new();
        for (n, name) in names.iter().rev().enumerate() {
            if n > 0 {
                key.push('\0');
            }
            key.push_str(&self.key(name));
        }
        key
    }

    /// The host path of node `idx` (its kept spelling).
    fn node_path(&self, idx: usize) -> PathBuf {
        let mut names = Vec::new();
        let mut cur = Some(idx);
        while let Some(i) = cur {
            let Some(node) = self.node(i) else { break };
            names.push(node.name.clone());
            cur = node.parent;
        }
        let mut path = PathBuf::new();
        for name in names.iter().rev() {
            path.push(name);
        }
        path
    }

    /// The device of node `idx` (`set_device`; the longest matching prefix, else 0).
    fn dev_of(&self, idx: usize) -> u64 {
        if self.devices.is_empty() {
            return 0;
        }
        let key = self.node_key(idx);
        self.devices
            .iter()
            .filter(|(prefix, _)| {
                key == *prefix
                    || (key.starts_with(prefix.as_str())
                        && key.as_bytes().get(prefix.len()) == Some(&0))
            })
            .max_by_key(|(prefix, _)| prefix.len())
            .map_or(0, |(_, dev)| *dev)
    }

    /// The first stream fault of `list` at `key`.
    fn after_fault(list: &[(String, u64, ErrorKind)], key: &str) -> Option<(u64, ErrorKind)> {
        list.iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, bytes, kind)| (*bytes, *kind))
    }

    /// Records a successful mutating call.
    fn record(&mut self, op: VfsOp, spec: &str) {
        if let Some(journal) = &mut self.journal {
            journal.push((op, spec.to_owned()));
        }
    }

    /// Detaches node `idx` from its parent (the node stays in `nodes`, unreachable).
    fn detach(&mut self, idx: usize) {
        let Some(node) = self.node(idx) else { return };
        let key = self.key(&node.name);
        let Some(parent) = node.parent else { return };
        if let Some(p) = self.nodes.get_mut(parent)
            && p.children.get(&key) == Some(&idx)
        {
            p.children.remove(&key);
        }
        if let Some(node) = self.nodes.get_mut(idx) {
            node.parent = None;
        }
    }

    /// True when `idx` is `ancestor` or below it.
    fn is_within(&self, mut idx: usize, ancestor: usize) -> bool {
        loop {
            if idx == ancestor {
                return true;
            }
            match self.node(idx).and_then(|n| n.parent) {
                Some(parent) => idx = parent,
                None => return false,
            }
        }
    }

    /// The folder that holds the final component of `parsed` (followed) and that component's name: `NotFound` /
    /// `NotADirectory` when the parent does not resolve to a folder, `None` for a root.
    fn parent_of(&self, parsed: &Parsed) -> Result<Option<(usize, String)>, Miss> {
        let Some((last, parents)) = parsed.comps.split_last() else {
            return Ok(None);
        };
        let Comp::Name(name) = last else {
            return Err(Miss::NotFound);
        };
        let root = self.root(&parsed.root)?;
        let dir = self.walk(root, parents, true, &mut 0)?;
        if !self.is_dir(dir) {
            return Err(Miss::NotADirectory);
        }
        Ok(Some((dir, name.clone())))
    }

    /// The child of `dir` named `name` (not followed).
    fn child_of(&self, dir: usize, name: &str) -> Option<usize> {
        self.node(dir)
            .and_then(|n| n.children.get(&self.key(name)).copied())
    }
}

impl Shared {
    fn read(&self) -> RwLockReadGuard<'_, Tree> {
        // The tree is consistent after every completed mutation; a panic elsewhere cannot corrupt it.
        self.tree.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Tree> {
        self.tree.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn count_stat(&self) {
        self.stat_calls.fetch_add(1, Ordering::Relaxed);
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

/// The parsed form of a spec's path (a `LocalPath` is always absolute).
fn parsed(path: &LocalPath, spec: &str) -> Result<Parsed, VfsError> {
    parse_path(path.as_path()).ok_or_else(|| VfsError::NotFound {
        spec: spec.to_owned(),
    })
}

/// The entry name of `path`: its last component, or the whole path for a root.
fn name_of(path: &LocalPath) -> FileName {
    FileName::from_os(path.file_name().unwrap_or(path.as_path().as_os_str()))
}

/// Validates an insert path (absolute and normalised for the host).
fn insert_path(path: &str) -> Result<Parsed, VfsError> {
    let invalid = || VfsError::InvalidName {
        name: path.to_owned(),
    };
    let local = LocalPath::new(PathBuf::from(path)).map_err(|_| invalid())?;
    parse_path(local.as_path()).ok_or_else(invalid)
}

/// The lexical fault key of `path` (`None` when it is not an absolute path).
fn lexical_key(tree: &Tree, path: &str) -> Option<String> {
    LocalPath::new(PathBuf::from(path))
        .ok()
        .and_then(|p| parse_path(p.as_path()))
        .map(|p| tree.path_key(&p))
}

impl MemoryVfs {
    /// Empty file system with the given name case sensitivity.
    pub fn new(case: CaseSensitivity) -> MemoryVfs {
        let mut caps = Capabilities::READ
            | Capabilities::WRITE
            | Capabilities::LIST
            | Capabilities::SEEK
            | Capabilities::MTIME
            | Capabilities::CTIME
            | Capabilities::DOS_ATTRS
            | Capabilities::UNIX_MODE
            | Capabilities::UNIX_OWNER
            | Capabilities::SYMLINKS
            | Capabilities::CRC_PROVIDED
            | Capabilities::SET_TIMES
            | Capabilities::MKDIR
            | Capabilities::REMOVE
            | Capabilities::RENAME
            | Capabilities::SET_ATTRS
            | Capabilities::TRASH
            | Capabilities::CREATE_SYMLINKS
            | Capabilities::SET_CREATED;
        caps.set(
            Capabilities::CASE_SENSITIVE,
            case == CaseSensitivity::Sensitive,
        );
        MemoryVfs {
            shared: Arc::new(Shared {
                tree: RwLock::new(Tree {
                    case,
                    caps,
                    order: ListingOrder::Sorted,
                    roots: BTreeMap::new(),
                    nodes: Vec::new(),
                    faults: Vec::new(),
                    kind_unknown: BTreeSet::new(),
                    read_errors: Vec::new(),
                    write_errors: Vec::new(),
                    devices: Vec::new(),
                    journal: None,
                    trashed: Vec::new(),
                }),
                stat_calls: AtomicU64::new(0),
            }),
        }
    }

    /// Inserts a file (creating parents).
    pub fn insert_file(
        &self,
        path: &str,
        content: impl Into<Arc<[u8]>>,
        meta: MemMeta,
    ) -> Result<(), VfsError> {
        let parsed = insert_path(path)?;
        self.shared
            .write()
            .insert(&parsed, NodeKind::File(content.into()), meta, path)
    }

    /// Inserts a folder (creating parents).
    pub fn insert_dir(&self, path: &str, meta: MemMeta) -> Result<(), VfsError> {
        let parsed = insert_path(path)?;
        self.shared
            .write()
            .insert(&parsed, NodeKind::Dir, meta, path)
    }

    /// Inserts a symlink (creating parents). `target` is stored verbatim; a relative target is resolved from
    /// the link's folder.
    pub fn insert_symlink(&self, path: &str, target: &str, meta: MemMeta) -> Result<(), VfsError> {
        let parsed = insert_path(path)?;
        self.shared
            .write()
            .insert(&parsed, NodeKind::Symlink(target.to_owned()), meta, path)
    }

    /// Sets the listing order.
    pub fn set_listing_order(&self, order: ListingOrder) {
        self.shared.write().order = order;
    }

    /// Replaces the capabilities reported for every local spec. The default is every capability (all 21 bits:
    /// read, write, list, seek, mtime, ctime, DOS attributes, Unix mode and owner, symlinks, CRC provided, the
    /// seven mutation bits of M2 W3 and set times), case-sensitive only when the file system is. A mutating
    /// primitive whose bit is removed returns `OperationUnsupported`.
    pub fn set_capabilities(&self, caps: Capabilities) {
        self.shared.write().caps = caps;
    }

    /// Adds a fault.
    pub fn inject(&self, fault: Fault) {
        let mut tree = self.shared.write();
        let key = LocalPath::new(PathBuf::from(&fault.path))
            .ok()
            .and_then(|p| parse_path(p.as_path()))
            .map(|p| tree.path_key(&p));
        tree.faults.push((fault, key));
    }

    /// Removes every fault, the streams that fail after a number of bytes included.
    pub fn clear_faults(&self) {
        let mut tree = self.shared.write();
        tree.faults.clear();
        tree.read_errors.clear();
        tree.write_errors.clear();
    }

    /// Makes the directory record of the entry at `path` (the entry itself; a final symlink is not followed)
    /// state no kind, like `DT_UNKNOWN` on some file systems: `list` then always stat's it and never passes it
    /// to the prefilter (D-45). `NotFound` (or `NotADirectory`) when `path` does not resolve.
    pub fn set_kind_unknown(&self, path: &str) -> Result<(), VfsError> {
        let parsed = insert_path(path)?;
        let mut tree = self.shared.write();
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, path.to_owned(), VfsOp::Stat))?;
        tree.kind_unknown.insert(idx);
        Ok(())
    }

    /// Content of a file written through `open_write` (for asserting report outputs). Returns the content of
    /// any file at `path` (symlinks followed), whether written or inserted.
    pub fn read_back(&self, path: &str) -> Option<Arc<[u8]>> {
        let parsed = insert_path(path).ok()?;
        let tree = self.shared.read();
        let idx = tree.resolve(&parsed, true).ok()?;
        match &tree.node(idx)?.kind {
            NodeKind::File(content) => Some(Arc::clone(content)),
            NodeKind::Dir | NodeKind::Symlink(_) => None,
        }
    }

    /// `open_read(path)` succeeds; its reads return the first `bytes` bytes, then fail with `kind` (M2 W3).
    pub fn inject_read_error_after(&self, path: &str, bytes: u64, kind: ErrorKind) {
        let mut tree = self.shared.write();
        if let Some(key) = lexical_key(&tree, path) {
            tree.read_errors.push((key, bytes, kind));
        }
    }

    /// `open_write(path)` succeeds; its `write` fails with `kind` once `bytes` bytes were accepted (the caller
    /// then drops the writer: nothing becomes visible; M2 W3).
    pub fn inject_write_error_after(&self, path: &str, bytes: u64, kind: ErrorKind) {
        let mut tree = self.shared.write();
        if let Some(key) = lexical_key(&tree, path) {
            tree.write_errors.push((key, bytes, kind));
        }
    }

    /// Items at and below `prefix` report `FileId { dev, .. }`; `rename` between different devices is
    /// `CrossDevice` (M2 W3). The longest matching prefix wins; everything else is device 0.
    pub fn set_device(&self, prefix: &str, dev: u64) {
        let mut tree = self.shared.write();
        if let Some(key) = lexical_key(&tree, prefix) {
            tree.devices.retain(|(k, _)| *k != key);
            tree.devices.push((key, dev));
        }
    }

    /// Starts (`true`, clearing it) or stops (`false`) the journal of mutating calls (M2 W3).
    pub fn set_journal(&self, on: bool) {
        self.shared.write().journal = on.then(Vec::new);
    }

    /// Every successful mutating call since `set_journal(true)`, in order: the primitive and the canonical spec
    /// (`rename`: the `from` spec; a committed write: `Commit` and the target spec).
    pub fn journal(&self) -> Vec<(VfsOp, String)> {
        self.shared.read().journal.clone().unwrap_or_default()
    }

    /// The original paths moved to the trash, in order (the items left the tree).
    pub fn trashed(&self) -> Vec<String> {
        self.shared.read().trashed.clone()
    }

    /// A deterministic snapshot of the reachable tree, sorted by path: kind, length and CRC-32 of the content,
    /// times, attributes and link targets.
    pub fn dump(&self) -> Vec<MemDump> {
        let tree = self.shared.read();
        let mut out = Vec::new();
        let mut pending: Vec<usize> = tree.roots.values().copied().collect();
        while let Some(idx) = pending.pop() {
            let Some(node) = tree.node(idx) else { continue };
            pending.extend(node.children.values().copied());
            let (kind, len, crc32, link_target) = match &node.kind {
                NodeKind::Dir => (EntryKind::Dir, 0, None, None),
                NodeKind::File(content) => (
                    EntryKind::File,
                    content.len() as u64,
                    Some(crc32fast::hash(content)),
                    None,
                ),
                NodeKind::Symlink(target) => (
                    EntryKind::Symlink,
                    target.len() as u64,
                    None,
                    Some(target.clone()),
                ),
            };
            out.push(MemDump {
                path: tree.node_path(idx).to_string_lossy().into_owned(),
                kind,
                len,
                crc32,
                mtime: node.meta.mtime,
                ctime: node.meta.ctime,
                attrs: node.meta.attrs.unwrap_or(Attrs::None),
                link_target,
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// The parsed path and canonical text of a mutation's spec; `OperationUnsupported` when `cap` was removed.
    fn mutation(
        &self,
        spec: &Spec,
        op: VfsOp,
        cap: Capabilities,
    ) -> Result<(Parsed, String), VfsError> {
        let path = local(spec)?;
        if !self.shared.read().caps.contains(cap) {
            return Err(op_unsupported(spec, op));
        }
        let spec_text = spec.canonical();
        let parsed = parsed(path, &spec_text)?;
        Ok((parsed, spec_text))
    }

    /// Number of metadata reads so far: `stat` calls plus the per-entry reads of `list` (entries the
    /// prefilter declined are not read, D-45).
    pub fn stat_calls(&self) -> u64 {
        self.shared.stat_calls.load(Ordering::Relaxed)
    }
}

impl Default for MemoryVfs {
    /// An empty, case-sensitive file system.
    fn default() -> MemoryVfs {
        MemoryVfs::new(CaseSensitivity::Sensitive)
    }
}

impl fmt::Debug for MemoryVfs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tree = self.shared.read();
        f.debug_struct("MemoryVfs")
            .field("case", &tree.case)
            .field("order", &tree.order)
            .field("nodes", &tree.nodes.len())
            .field("faults", &tree.faults.len())
            .field("stat_calls", &self.stat_calls())
            .finish()
    }
}

impl Vfs for MemoryVfs {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        local(at)?;
        Ok(self.shared.read().caps)
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        let path = local(spec)?;
        let spec_text = spec.canonical();
        let parsed = parsed(path, &spec_text)?;
        self.shared.count_stat();
        let tree = self.shared.read();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Stat) {
            return Err(fault_error(&spec_text, VfsOp::Stat, kind));
        }
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Stat))?;
        match tree.describe(idx, name_of(path), follow) {
            (entry, None) => Ok(entry),
            (_, Some(miss)) => Err(miss_error(miss, spec_text, VfsOp::Stat)),
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
        let spec_text = dir.canonical();
        let dir_parsed = parsed(path, &spec_text)?;
        // Snapshot the children (byte order of names, then the listing order) without holding the lock while
        // the prefilter runs.
        let children: Vec<(String, usize, Option<EntryKind>)> = {
            let tree = self.shared.read();
            if let Some(kind) = tree.fault(&dir_parsed, VfsOp::List) {
                return Err(fault_error(&spec_text, VfsOp::List, kind));
            }
            let idx = tree
                .resolve(&dir_parsed, true)
                .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::List))?;
            let Some(node) = tree.node(idx).filter(|n| matches!(n.kind, NodeKind::Dir)) else {
                return Err(VfsError::NotADirectory { spec: spec_text });
            };
            // The kind as the directory record states it (`None`: unknown, see `set_kind_unknown`).
            let mut children: Vec<(String, usize, Option<EntryKind>)> = node
                .children
                .values()
                .filter_map(|&c| {
                    let child = tree.node(c)?;
                    let kind = (!tree.kind_unknown.contains(&c)).then(|| child.kind.entry_kind());
                    Some((child.name.clone(), c, kind))
                })
                .collect();
            children.sort_by(|a, b| a.0.cmp(&b.0));
            tree.order.apply(&mut children);
            children
        };
        let mut listing = Listing::default();
        let mut described = Vec::with_capacity(children.len());
        for (index, (name, idx, kind)) in children.into_iter().enumerate() {
            if index % 1024 == 1023 {
                cancel.check()?;
            }
            let file_name = FileName::from_os(OsStr::new(&name));
            // Symlinks and entries of unknown kind are always stat'ed and never passed to the prefilter.
            if let Some(kind @ (EntryKind::Dir | EntryKind::File | EntryKind::Other)) = kind
                && let Some(prefilter) = opts.prefilter
                && !prefilter(file_name.as_str(), kind)
            {
                listing.name_only.push(NameOnly {
                    name: file_name,
                    kind,
                });
                continue;
            }
            described.push((file_name, idx));
        }
        let tree = self.shared.read();
        for (index, (name, idx)) in described.into_iter().enumerate() {
            if index % 1024 == 1023 {
                cancel.check()?;
            }
            self.shared.count_stat();
            let child = path.join_name(&name.to_os_string()).ok();
            let child_text = child.as_ref().map_or_else(
                || name.as_str().to_owned(),
                |c| c.as_path().to_string_lossy().into_owned(),
            );
            let child_spec = child.map(Spec::Local);
            let report = |error: VfsError| ErrorReport::capture(&error, child_spec.as_ref());
            let mut child_parsed = dir_parsed.clone();
            child_parsed
                .comps
                .push(Comp::Name(name.as_str().to_owned()));
            if let Some(kind) = tree.fault(&child_parsed, VfsOp::Stat) {
                listing.errors.push(EntryError {
                    name,
                    error: report(fault_error(&child_text, VfsOp::Stat, kind)),
                });
                continue;
            }
            let (entry, miss) = tree.describe(idx, name, opts.follow);
            if let Some(miss) = miss {
                listing.errors.push(EntryError {
                    name: entry.name.clone(),
                    error: report(miss_error(miss, child_text, VfsOp::Stat)),
                });
            }
            listing.entries.push(entry);
        }
        Ok(listing)
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let path = local(spec)?;
        let spec_text = spec.canonical();
        let parsed = parsed(path, &spec_text)?;
        let tree = self.shared.read();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Open) {
            return Err(fault_error(&spec_text, VfsOp::Open, kind));
        }
        let idx = tree
            .resolve(&parsed, true)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Open))?;
        let content = match tree.node(idx).map(|n| &n.kind) {
            Some(NodeKind::File(content)) => Arc::clone(content),
            Some(NodeKind::Dir) => return Err(VfsError::IsADirectory { spec: spec_text }),
            Some(NodeKind::Symlink(_)) | None => {
                return Err(VfsError::NotFound { spec: spec_text });
            }
        };
        if let Some(kind) = tree.fault(&parsed, VfsOp::Read) {
            return Ok(Box::new(FaultReader { kind }));
        }
        if let Some((bytes, kind)) = Tree::after_fault(&tree.read_errors, &tree.path_key(&parsed)) {
            return Ok(Box::new(LimitedReader {
                inner: Cursor::new(content),
                limit: bytes,
                kind,
            }));
        }
        Ok(Box::new(Cursor::new(content)))
    }

    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        let path = local(spec)?;
        let spec_text = spec.canonical();
        let target = parsed(path, &spec_text)?;
        let (Some(Comp::Name(name)), Some(parent)) = (target.comps.last().cloned(), path.parent())
        else {
            return Err(VfsError::IsADirectory { spec: spec_text });
        };
        let parent = parsed(&parent, &spec_text)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&target, VfsOp::Write) {
            return Err(fault_error(&spec_text, VfsOp::Write, kind));
        }
        let dir = match tree.resolve(&parent, true) {
            Ok(dir) if tree.is_dir(dir) => dir,
            Ok(_) => return Err(VfsError::NotADirectory { spec: spec_text }),
            Err(Miss::NotFound) if opts.create_parents => {
                let root = tree.root_or_create(&parent.root);
                tree.make_dirs(root, &parent.comps)
                    .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Write))?
            }
            Err(miss) => return Err(miss_error(miss, spec_text, VfsOp::Write)),
        };
        let key = tree.key(&name);
        let (mut write_parent, mut write_name) = (parent, name);
        if let Some(existing) = tree.node(dir).and_then(|n| n.children.get(&key).copied()) {
            if tree.is_dir(existing) {
                return Err(VfsError::IsADirectory { spec: spec_text });
            }
            if !opts.overwrite {
                return Err(VfsError::AlreadyExists { spec: spec_text });
            }
            // Without `replace_link` a link is followed: the file it points to is replaced (M1 save semantics).
            let is_link = matches!(
                tree.node(existing).map(|n| &n.kind),
                Some(NodeKind::Symlink(_))
            );
            if is_link && !opts.replace_link {
                match tree.follow_link(existing) {
                    Ok(resolved) => {
                        if tree.is_dir(resolved) {
                            return Err(VfsError::IsADirectory { spec: spec_text });
                        }
                        let resolved_path = tree.node_path(resolved);
                        if let (Some(dir_path), Some(file_name)) =
                            (resolved_path.parent(), resolved_path.file_name())
                            && let Some(dir_parsed) = parse_path(dir_path)
                        {
                            write_parent = dir_parsed;
                            write_name = file_name.to_string_lossy().into_owned();
                        }
                    }
                    // A dangling link: the file it names is created (as `LocalVfs`), the link stays.
                    Err(Miss::NotFound) => match tree.dangling_end(existing) {
                        Ok((dir, name)) => {
                            write_parent = dir;
                            write_name = name;
                        }
                        Err(miss) => return Err(miss_error(miss, spec_text, VfsOp::Write)),
                    },
                    Err(miss) => return Err(miss_error(miss, spec_text, VfsOp::Write)),
                }
            }
        }
        let fail_after = Tree::after_fault(&tree.write_errors, &tree.path_key(&target));
        Ok(Box::new(MemWriter {
            shared: Arc::clone(&self.shared),
            target,
            parent: write_parent,
            name: write_name,
            spec: spec_text,
            overwrite: opts.overwrite,
            buf: Vec::new(),
            pos: 0,
            fail_after,
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

    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        let (parsed, spec_text) = self.mutation(spec, VfsOp::Mkdir, Capabilities::MKDIR)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Mkdir) {
            return Err(fault_error(&spec_text, VfsOp::Mkdir, kind));
        }
        let miss = |miss: Miss| miss_error(miss, spec_text.clone(), VfsOp::Mkdir);
        if parents {
            match tree.resolve(&parsed, true) {
                Ok(idx) if tree.is_dir(idx) => return Ok(false),
                Ok(_) => return Err(VfsError::NotADirectory { spec: spec_text }),
                Err(Miss::NotFound) => {}
                Err(other) => return Err(miss(other)),
            }
            let root = tree.root_or_create(&parsed.root);
            // `make_dirs` meets a missing item only through a broken link (missing folders are created): an
            // existing non-folder, like a file at `spec` or an ancestor.
            tree.make_dirs(root, &parsed.comps).map_err(|m| match m {
                Miss::NotFound => VfsError::NotADirectory {
                    spec: spec_text.clone(),
                },
                other => miss(other),
            })?;
            tree.record(VfsOp::Mkdir, &spec_text);
            return Ok(true);
        }
        let Some((dir, name)) = tree.parent_of(&parsed).map_err(miss)? else {
            return match tree.root(&parsed.root) {
                Ok(_) => Err(VfsError::AlreadyExists { spec: spec_text }),
                Err(_) => {
                    tree.root_or_create(&parsed.root);
                    tree.record(VfsOp::Mkdir, &spec_text);
                    Ok(true)
                }
            };
        };
        if tree.child_of(dir, &name).is_some() {
            return Err(VfsError::AlreadyExists { spec: spec_text });
        }
        tree.push_node(&name, Some(dir), NodeKind::Dir, MemMeta::default());
        tree.record(VfsOp::Mkdir, &spec_text);
        Ok(true)
    }

    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        let (parsed, spec_text) = self.mutation(spec, VfsOp::Remove, Capabilities::REMOVE)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Remove) {
            return Err(fault_error(&spec_text, VfsOp::Remove, kind));
        }
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Remove))?;
        let Some(node) = tree.node(idx) else {
            return Err(VfsError::NotFound { spec: spec_text });
        };
        if node.parent.is_none() {
            return Err(VfsError::InvalidTarget {
                spec: spec_text,
                reason: "is a root folder",
            });
        }
        if matches!(node.kind, NodeKind::Dir) && !node.children.is_empty() {
            return Err(VfsError::NotEmpty { spec: spec_text });
        }
        tree.detach(idx);
        tree.record(VfsOp::Remove, &spec_text);
        Ok(())
    }

    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        let (from_parsed, from_text) = self.mutation(from, VfsOp::Rename, Capabilities::RENAME)?;
        let to_path = local(to)?;
        let to_text = to.canonical();
        let to_parsed = parsed(to_path, &to_text)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&from_parsed, VfsOp::Rename) {
            return Err(fault_error(&from_text, VfsOp::Rename, kind));
        }
        let idx = tree
            .resolve(&from_parsed, false)
            .map_err(|miss| miss_error(miss, from_text.clone(), VfsOp::Rename))?;
        if tree.node(idx).is_none_or(|n| n.parent.is_none()) {
            return Err(VfsError::InvalidTarget {
                spec: from_text,
                reason: "is a root folder",
            });
        }
        let Some((dir, name)) = tree
            .parent_of(&to_parsed)
            .map_err(|miss| miss_error(miss, to_text.clone(), VfsOp::Rename))?
        else {
            return Err(VfsError::AlreadyExists { spec: to_text });
        };
        if tree.dev_of(idx) != tree.dev_of(dir) {
            return Err(VfsError::CrossDevice {
                from: from_text,
                to: to_text,
            });
        }
        let from_is_dir = tree.is_dir(idx);
        if from_is_dir && tree.is_within(dir, idx) {
            return Err(VfsError::from_io(
                from_text,
                VfsOp::Rename,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot move a folder into itself",
                ),
            ));
        }
        match tree.child_of(dir, &name) {
            Some(existing) if existing == idx => {}
            Some(existing) => {
                if !replace {
                    return Err(VfsError::AlreadyExists { spec: to_text });
                }
                let existing_is_dir = tree.is_dir(existing);
                match (from_is_dir, existing_is_dir) {
                    (true, false) => return Err(VfsError::NotADirectory { spec: to_text }),
                    (false, true) => return Err(VfsError::IsADirectory { spec: to_text }),
                    (true, true) if tree.node(existing).is_some_and(|n| !n.children.is_empty()) => {
                        return Err(VfsError::NotEmpty { spec: to_text });
                    }
                    _ => {}
                }
                tree.detach(existing);
            }
            None => {}
        }
        tree.detach(idx);
        let key = tree.key(&name);
        if let Some(node) = tree.nodes.get_mut(idx) {
            node.name = name;
            node.parent = Some(dir);
        }
        if let Some(parent) = tree.nodes.get_mut(dir) {
            parent.children.insert(key, idx);
        }
        tree.record(VfsOp::Rename, &from_text);
        Ok(())
    }

    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        let (parsed, spec_text) = self.mutation(spec, VfsOp::SetTimes, Capabilities::SET_TIMES)?;
        let mut tree = self.shared.write();
        if times.created.is_some() && !tree.caps.contains(Capabilities::SET_CREATED) {
            return Err(op_unsupported(spec, VfsOp::SetTimes));
        }
        if let Some(kind) = tree.fault(&parsed, VfsOp::SetTimes) {
            return Err(fault_error(&spec_text, VfsOp::SetTimes, kind));
        }
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::SetTimes))?;
        if let Some(node) = tree.nodes.get_mut(idx) {
            if let Some(modified) = times.modified {
                node.meta.mtime = Some(modified);
            }
            if let Some(created) = times.created {
                node.meta.ctime = Some(created);
            }
        }
        tree.record(VfsOp::SetTimes, &spec_text);
        Ok(())
    }

    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        let (parsed, spec_text) = self.mutation(spec, VfsOp::SetAttrs, Capabilities::SET_ATTRS)?;
        change.validate()?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&parsed, VfsOp::SetAttrs) {
            return Err(fault_error(&spec_text, VfsOp::SetAttrs, kind));
        }
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::SetAttrs))?;
        let caps = tree.caps;
        let Some(node) = tree.nodes.get_mut(idx) else {
            return Err(VfsError::NotFound { spec: spec_text });
        };
        if matches!(node.kind, NodeKind::Symlink(_)) {
            return Err(op_unsupported(spec, VfsOp::SetAttrs));
        }
        let current = node.meta.attrs.unwrap_or(Attrs::None);
        let new = match (*change, current) {
            (AttrChange::Mode(mode), Attrs::Unix { meta }) => Attrs::Unix {
                meta: UnixMeta {
                    mode: mode & 0o7777,
                    ..meta
                },
            },
            (AttrChange::Mode(mode), Attrs::None) if caps.contains(Capabilities::UNIX_MODE) => {
                Attrs::Unix {
                    meta: UnixMeta {
                        mode: mode & 0o7777,
                        uid: 0,
                        gid: 0,
                    },
                }
            }
            (AttrChange::Dos { set, clear }, Attrs::Dos { attrs }) => Attrs::Dos {
                attrs: (attrs | set).difference(clear),
            },
            (AttrChange::Dos { set, .. }, Attrs::None)
                if caps.contains(Capabilities::DOS_ATTRS) =>
            {
                Attrs::Dos { attrs: set }
            }
            _ => return Err(op_unsupported(spec, VfsOp::SetAttrs)),
        };
        node.meta.attrs = Some(new);
        tree.record(VfsOp::SetAttrs, &spec_text);
        Ok(())
    }

    fn create_symlink(
        &self,
        spec: &Spec,
        target: &OsStr,
        _target_is_dir: bool,
    ) -> Result<(), VfsError> {
        let (parsed, spec_text) =
            self.mutation(spec, VfsOp::Symlink, Capabilities::CREATE_SYMLINKS)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Symlink) {
            return Err(fault_error(&spec_text, VfsOp::Symlink, kind));
        }
        let Some((dir, name)) = tree
            .parent_of(&parsed)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Symlink))?
        else {
            return Err(VfsError::AlreadyExists { spec: spec_text });
        };
        if tree.child_of(dir, &name).is_some() {
            return Err(VfsError::AlreadyExists { spec: spec_text });
        }
        let target = target.to_string_lossy().into_owned();
        tree.push_node(
            &name,
            Some(dir),
            NodeKind::Symlink(target),
            MemMeta::default(),
        );
        tree.record(VfsOp::Symlink, &spec_text);
        Ok(())
    }

    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        let (parsed, spec_text) = self.mutation(spec, VfsOp::Trash, Capabilities::TRASH)?;
        let mut tree = self.shared.write();
        if let Some(kind) = tree.fault(&parsed, VfsOp::Trash) {
            return Err(fault_error(&spec_text, VfsOp::Trash, kind));
        }
        let idx = tree
            .resolve(&parsed, false)
            .map_err(|miss| miss_error(miss, spec_text.clone(), VfsOp::Trash))?;
        if tree.node(idx).is_none_or(|n| n.parent.is_none()) {
            return Err(VfsError::InvalidTarget {
                spec: spec_text,
                reason: "is a root folder",
            });
        }
        let path = tree.node_path(idx).to_string_lossy().into_owned();
        tree.detach(idx);
        tree.trashed.push(path);
        tree.record(VfsOp::Trash, &spec_text);
        Ok(())
    }
}

/// A reader whose first read fails with an injected fault.
#[derive(Debug)]
struct FaultReader {
    kind: ErrorKind,
}

impl Read for FaultReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(fault_io(self.kind))
    }
}

impl Seek for FaultReader {
    fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
        Ok(0)
    }
}

/// A reader that returns the first `limit` bytes of `inner`, then fails with `kind`.
#[derive(Debug)]
struct LimitedReader {
    inner: Cursor<Arc<[u8]>>,
    limit: u64,
    kind: ErrorKind,
}

impl Read for LimitedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let room = self.limit.saturating_sub(self.inner.position());
        if room == 0 && !buf.is_empty() {
            return Err(fault_io(self.kind));
        }
        let take = buf.len().min(usize::try_from(room).unwrap_or(usize::MAX));
        let slice = buf.get_mut(..take).unwrap_or_default();
        self.inner.read(slice)
    }
}

impl Seek for LimitedReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// The atomic writer of `MemoryVfs`: the content is buffered and inserted by `commit`.
struct MemWriter {
    shared: Arc<Shared>,
    target: Parsed,
    parent: Parsed,
    name: String,
    spec: String,
    overwrite: bool,
    buf: Vec<u8>,
    /// Where the next write lands.
    pos: usize,
    /// `inject_write_error_after`: the bytes accepted before writes fail, and the kind.
    fail_after: Option<(u64, ErrorKind)>,
}

impl Write for MemWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let take = match self.fail_after {
            Some((limit, kind)) => {
                let room = limit.saturating_sub(self.buf.len() as u64);
                if room == 0 && !buf.is_empty() {
                    return Err(fault_io(kind));
                }
                buf.len().min(usize::try_from(room).unwrap_or(usize::MAX))
            }
            None => buf.len(),
        };
        let bytes = buf.get(..take).unwrap_or(buf);
        let end = self.pos + bytes.len();
        if self.buf.len() < end {
            self.buf.resize(end, 0);
        }
        if let Some(room) = self.buf.get_mut(self.pos..end) {
            room.copy_from_slice(bytes);
        }
        self.pos = end;
        Ok(take)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for MemWriter {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(delta) => (self.pos as u64).checked_add_signed(delta),
            SeekFrom::End(delta) => (self.buf.len() as u64).checked_add_signed(delta),
        };
        let target = target.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the file",
            )
        })?;
        self.pos = usize::try_from(target)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "seek past memory"))?;
        Ok(target)
    }
}

impl AtomicWrite for MemWriter {
    fn commit(self: Box<Self>) -> Result<(), VfsError> {
        let writer = *self;
        let mut tree = writer.shared.write();
        if let Some(kind) = tree.fault(&writer.target, VfsOp::Commit) {
            return Err(fault_error(&writer.spec, VfsOp::Commit, kind));
        }
        let dir = tree
            .resolve(&writer.parent, true)
            .map_err(|miss| miss_error(miss, writer.spec.clone(), VfsOp::Commit))?;
        if !tree.is_dir(dir) {
            return Err(VfsError::NotADirectory { spec: writer.spec });
        }
        let key = tree.key(&writer.name);
        let content: Arc<[u8]> = writer.buf.into();
        match tree.node(dir).and_then(|n| n.children.get(&key).copied()) {
            Some(existing) if tree.is_dir(existing) => {
                Err(VfsError::IsADirectory { spec: writer.spec })
            }
            Some(_) if !writer.overwrite => Err(VfsError::AlreadyExists { spec: writer.spec }),
            Some(existing) => {
                if let Some(node) = tree.nodes.get_mut(existing) {
                    node.kind = NodeKind::File(content);
                    node.meta.crc32 = None;
                    node.name = writer.name;
                }
                tree.record(VfsOp::Commit, &writer.spec);
                Ok(())
            }
            None => {
                tree.push_node(
                    &writer.name,
                    Some(dir),
                    NodeKind::File(content),
                    MemMeta::default(),
                );
                tree.record(VfsOp::Commit, &writer.spec);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffles_are_permutations_and_deterministic() {
        for seed in [0, 1, 42, u64::MAX] {
            let mut a: Vec<u32> = (0..100).collect();
            let mut b = a.clone();
            ListingOrder::Shuffled { seed }.apply(&mut a);
            ListingOrder::Shuffled { seed }.apply(&mut b);
            assert_eq!(a, b);
            let mut sorted = a.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, (0..100).collect::<Vec<_>>());
            assert_ne!(a, sorted, "seed {seed} shuffles");
        }
        let mut r = vec![1, 2, 3];
        ListingOrder::Reversed.apply(&mut r);
        assert_eq!(r, [3, 2, 1]);
    }

    #[test]
    fn targets_parse() {
        let (root, comps) = parse_target("../x/y");
        assert_eq!(root, None);
        assert_eq!(
            comps,
            [Comp::Parent, Comp::Name("x".into()), Comp::Name("y".into())]
        );
        #[cfg(unix)]
        {
            let (root, comps) = parse_target("/a");
            assert_eq!(root.as_deref(), Some("/"));
            assert_eq!(comps, [Comp::Name("a".into())]);
        }
    }

    #[test]
    fn fault_kinds_map_to_io_kinds() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::PermissionDenied,
            ErrorKind::AlreadyExists,
            ErrorKind::NotADirectory,
            ErrorKind::IsADirectory,
            ErrorKind::InvalidInput,
            ErrorKind::InvalidData,
            ErrorKind::Unsupported,
            ErrorKind::Cancelled,
            ErrorKind::Io,
            ErrorKind::TooLarge,
        ] {
            assert_eq!(hikasa_core::io_error_kind(&fault_io(kind)), kind);
        }
        assert_eq!(
            hikasa_core::io_error_kind(&fault_io(ErrorKind::Protocol)),
            ErrorKind::Io
        );
    }
}
