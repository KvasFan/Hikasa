//! The overlay of a batch (M2 W5b, D-148; §1.5.10): a mutable tree over the member tree of one archive.
//!
//! On the first change of a batch the tree is cloned into a map from path to [`Item`]; primitives change only that map,
//! with the semantics of `MemoryVfs` (§1.3.1, D-94). What a rebuild needs from an item is its [`Origin`]: a kept
//! member names its node, so its raw record can be carried over; a new one holds its staged file. Nothing here reads
//! the archive or the disk.

use std::collections::BTreeMap;
use std::io;
use std::ops::Bound;
use std::sync::Arc;

use hikasa_core::{FileTime, Spec};

use super::ArchiveLimits;
use super::cache::CacheFile;
use super::format::ArchiveFormat;
use super::index::{MemberKind, NO_RECORD, Node, Tree, entry_name};
use super::name::NAME_MAX;
use super::zip_read::LINK_TARGET_MAX;
use crate::entry::{Attrs, DosAttrs, Entry, UnixMeta};
use crate::error::{VfsError, VfsOp};
use crate::vfs::AttrChange;

/// A new file's mode, a new folder's and a new link's (§1.5.10).
const FILE_MODE: u32 = 0o644;
const FOLDER_MODE: u32 = 0o755;
const LINK_MODE: u32 = 0o777;

/// Where an item comes from.
#[derive(Debug, Clone)]
pub(crate) enum Origin {
    /// Node `node` of the base tree. `moved`: its path changed; `touched`: its time or attributes changed.
    Kept {
        node: u32,
        moved: bool,
        touched: bool,
    },
    /// A folder that only exists through its members: it has no record.
    Implicit,
    /// A new folder or link.
    Fresh,
    /// New content, staged in the cache.
    Staged(Arc<CacheFile>),
}

/// One member of the overlay.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub(crate) origin: Origin,
    pub(crate) kind: MemberKind,
    lossy: bool,
    pub(crate) size: u64,
    pub(crate) mtime: Option<FileTime>,
    ctime: Option<FileTime>,
    pub(crate) attrs: Attrs,
    crc32: Option<u32>,
    pub(crate) link_target: Option<Box<str>>,
    /// Older records that came with this member from another archive, oldest first (D-156).
    pub(crate) older: Option<Arc<Vec<Item>>>,
}

impl Item {
    fn kept(at: u32, node: &Node) -> Item {
        let origin = if node.record == NO_RECORD {
            Origin::Implicit
        } else {
            Origin::Kept {
                node: at,
                moved: false,
                touched: false,
            }
        };
        Item {
            origin,
            kind: node.kind,
            lossy: node.lossy,
            size: node.size,
            mtime: node.mtime,
            ctime: node.ctime,
            attrs: node.attrs,
            crc32: node.crc32,
            link_target: node.link_target.clone(),
            older: None,
        }
    }

    /// An older record that comes from another archive.
    pub(crate) fn older(
        kind: MemberKind,
        origin: Origin,
        size: u64,
        mtime: Option<FileTime>,
        attrs: Attrs,
        link_target: Option<Box<str>>,
    ) -> Item {
        Item {
            origin,
            kind,
            lossy: false,
            size,
            mtime,
            ctime: None,
            attrs,
            crc32: None,
            link_target,
            older: None,
        }
    }

    fn fresh(kind: MemberKind, origin: Origin, mode: u32, now: Option<FileTime>) -> Item {
        Item {
            origin,
            kind,
            lossy: false,
            size: 0,
            mtime: now,
            ctime: None,
            attrs: Attrs::Unix {
                meta: UnixMeta {
                    mode,
                    uid: 0,
                    gid: 0,
                },
            },
            crc32: None,
            link_target: None,
            older: None,
        }
    }

    /// Notes a change of time or attributes: a kept member is rewritten, an implicit folder becomes explicit.
    fn touch(&mut self) {
        match &mut self.origin {
            Origin::Kept { touched, .. } => *touched = true,
            Origin::Implicit => self.origin = Origin::Fresh,
            Origin::Fresh | Origin::Staged(_) => {}
        }
    }
}

/// What a primitive runs against.
pub(crate) struct Scope<'a> {
    pub(crate) archive: &'a Spec,
    pub(crate) tree: &'a Tree,
    pub(crate) format: ArchiveFormat,
    pub(crate) limits: ArchiveLimits,
    /// The clock's now, the time of a new member.
    pub(crate) now: Option<FileTime>,
}

impl Scope<'_> {
    fn text(&self, path: &str) -> String {
        Spec::archive_member(self.archive.clone(), path).canonical()
    }

    fn unsupported(&self, path: &str, op: VfsOp) -> VfsError {
        VfsError::OperationUnsupported {
            spec: self.text(path),
            op,
        }
    }

    fn missing(&self, path: &str) -> VfsError {
        VfsError::NotFound {
            spec: self.text(path),
        }
    }

    fn not_a_folder(&self, path: &str) -> VfsError {
        VfsError::NotADirectory {
            spec: self.text(path),
        }
    }

    fn exists(&self, path: &str) -> VfsError {
        VfsError::AlreadyExists {
            spec: self.text(path),
        }
    }
}

/// `dir` as a key prefix: `""` for the root, `dir/` otherwise.
fn folder_prefix(dir: &str) -> String {
    if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    }
}

/// The folder that holds `path`; `None` at the root.
fn parent_of(path: &str) -> Option<&str> {
    path.rfind('/').and_then(|at| path.get(..at))
}

/// The paths of the folders above `path`, nearest the root first.
fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').filter_map(|(at, _)| path.get(..at))
}

/// True when `path` is a member's path a reader would find again: the limits of the index bind what is written too
/// (rule 13), and a longer name would come back refused.
pub(crate) fn check_path(
    path: &str,
    limits: &ArchiveLimits,
    archive: &Spec,
) -> Result<(), VfsError> {
    if path.len() > NAME_MAX {
        return Err(VfsError::InvalidName {
            name: path.to_owned(),
        });
    }
    if path.bytes().filter(|&b| b == b'/').count() >= limits.depth_max as usize {
        return Err(VfsError::ArchiveLimit {
            spec: Spec::archive_member(archive.clone(), path).canonical(),
            limit: "depth",
            value: u64::from(limits.depth_max),
        });
    }
    Ok(())
}

/// The mutable tree. Cloning shares what items hold; a clone only happens if a change meets a running rebuild.
#[derive(Debug, Clone)]
pub(crate) struct Overlay {
    nodes: BTreeMap<Box<str>, Item>,
}

impl Overlay {
    /// The tree of `tree`, unchanged.
    pub(crate) fn new(tree: &Tree) -> Overlay {
        let nodes = tree
            .nodes
            .iter()
            .enumerate()
            .map(|(at, node)| {
                let at = u32::try_from(at).unwrap_or(NO_RECORD);
                (Box::<str>::from(node.path()), Item::kept(at, node))
            })
            .collect();
        Overlay { nodes }
    }

    /// Every item, sorted by path.
    pub(crate) fn items(&self) -> impl Iterator<Item = (&str, &Item)> {
        self.nodes.iter().map(|(path, item)| (&**path, item))
    }

    /// The members of the tree, folders included.
    pub(crate) fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The item at `path`.
    pub(crate) fn get(&self, path: &str) -> Option<&Item> {
        self.nodes.get(path)
    }

    /// True when a component on the way to `path` is not a folder.
    fn blocked(&self, path: &str) -> bool {
        ancestors(path).any(|dir| {
            self.get(dir)
                .is_some_and(|item| item.kind != MemberKind::Dir)
        })
    }

    /// The item at `path`: `NotADirectory` when a component on the way is not a folder, else `NotFound`. `spec` is
    /// the canonical text for the error.
    pub(crate) fn lookup(&self, path: &str, spec: &Spec) -> Result<&Item, VfsError> {
        if let Some(item) = self.get(path) {
            return Ok(item);
        }
        let spec = spec.canonical();
        Err(if self.blocked(path) {
            VfsError::NotADirectory { spec }
        } else {
            VfsError::NotFound { spec }
        })
    }

    /// The items directly in `dir` (`""` is the root), sorted. Each subtree is skipped by one search, so the cost
    /// is the children's, not the tree's.
    pub(crate) fn children(&self, dir: &str) -> Vec<(&str, &Item)> {
        let prefix = folder_prefix(dir);
        let mut out = Vec::new();
        let mut from = prefix.clone();
        loop {
            let next = self
                .nodes
                .range::<str, _>((Bound::Included(from.as_str()), Bound::Unbounded))
                .next();
            let Some((path, item)) = next.filter(|(path, _)| path.starts_with(&prefix)) else {
                return out;
            };
            match path.get(prefix.len()..).and_then(|rest| rest.find('/')) {
                None => {
                    out.push((&**path, item));
                    from = format!("{path}\0");
                }
                // `0` is the character after `/`: the next key past this folder's subtree.
                Some(slash) => {
                    from = format!("{}0", path.get(..prefix.len() + slash).unwrap_or_default());
                }
            }
        }
    }

    /// True when something lies in the folder `dir`.
    pub(crate) fn has_children(&self, dir: &str) -> bool {
        let prefix = folder_prefix(dir);
        self.nodes
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .next()
            .is_some_and(|(path, _)| path.starts_with(&prefix))
    }

    /// The entry of the item at `path`.
    pub(crate) fn entry(path: &str, item: &Item) -> Entry {
        let name = path.rsplit('/').next().unwrap_or(path);
        let mut entry = Entry::new(entry_name(name, item.lossy), item.kind.entry_kind());
        entry.size = item.size;
        entry.mtime = item.mtime;
        entry.ctime = item.ctime;
        entry.attrs = item.attrs;
        entry.link_target = item.link_target.as_deref().map(str::to_owned);
        entry.crc32 = item.crc32;
        entry
    }

    /// False when the overlay equals the tree it started from: no rebuild is needed (§1.5.10 step 1).
    pub(crate) fn changed(&self, tree: &Tree) -> bool {
        let mut kept = 0usize;
        for item in self.nodes.values() {
            match item.origin {
                Origin::Kept {
                    moved: false,
                    touched: false,
                    ..
                } => kept += 1,
                Origin::Implicit => {}
                Origin::Kept { .. } | Origin::Fresh | Origin::Staged(_) => return true,
            }
        }
        kept != tree.nodes.iter().filter(|n| n.record != NO_RECORD).count()
    }

    fn insert(&mut self, path: &str, item: Item, scope: &Scope<'_>) -> Result<(), VfsError> {
        if self.nodes.len() as u64 >= scope.limits.entries_max && self.get(path).is_none() {
            return Err(VfsError::ArchiveLimit {
                spec: scope.text(path),
                limit: "entries",
                value: scope.limits.entries_max,
            });
        }
        self.nodes.insert(path.into(), item);
        Ok(())
    }

    /// Gives the new member `path` the older records that came with it. They are written before it, so only a member
    /// that is written after the kept ones can have them.
    pub(crate) fn put_older(
        &mut self,
        path: &str,
        older: Vec<Item>,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        let Some(item) = self.nodes.get_mut(path) else {
            return Err(scope.missing(path));
        };
        if !matches!(item.origin, Origin::Fresh | Origin::Staged(_)) {
            return Err(scope.unsupported(path, VfsOp::Write));
        }
        item.older = Some(Arc::new(older));
        Ok(())
    }

    fn folder(&mut self, path: &str, scope: &Scope<'_>) -> Result<(), VfsError> {
        let item = Item::fresh(MemberKind::Dir, Origin::Fresh, FOLDER_MODE, scope.now);
        self.insert(path, item, scope)
    }

    /// The folder that holds `path` exists, made here when `parents` says so: a missing one is `NotFound`, and a
    /// non-folder on the way `NotADirectory`.
    fn ready_parent(
        &mut self,
        path: &str,
        parents: bool,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        if self.blocked(path) {
            return Err(scope.not_a_folder(path));
        }
        let Some(parent) = parent_of(path) else {
            return Ok(());
        };
        if self.get(parent).is_some() {
            return Ok(());
        }
        if !parents {
            return Err(scope.missing(path));
        }
        for dir in ancestors(path) {
            if self.get(dir).is_none() {
                self.folder(dir, scope)?;
            }
        }
        Ok(())
    }

    /// `create_dir`: `Ok(true)` when `path` was created. An existing non-folder is `AlreadyExists`, with `parents`
    /// `NotADirectory`, as on every back-end (§1.3.1).
    pub(crate) fn create_dir(
        &mut self,
        path: &str,
        parents: bool,
        scope: &Scope<'_>,
    ) -> Result<bool, VfsError> {
        match self.get(path) {
            Some(item) if item.kind == MemberKind::Dir && parents => return Ok(false),
            Some(_) if parents => return Err(scope.not_a_folder(path)),
            Some(_) => return Err(scope.exists(path)),
            None => {}
        }
        self.ready_parent(path, parents, scope)?;
        self.folder(path, scope)?;
        Ok(true)
    }

    /// `remove`: a file, a link or an empty folder.
    pub(crate) fn remove(&mut self, path: &str, scope: &Scope<'_>) -> Result<(), VfsError> {
        let spec = Spec::archive_member(scope.archive.clone(), path);
        let item = self.lookup(path, &spec)?;
        if item.kind == MemberKind::Dir && self.has_children(path) {
            return Err(VfsError::NotEmpty {
                spec: scope.text(path),
            });
        }
        self.nodes.remove(path);
        Ok(())
    }

    /// `rename`, of a whole subtree (§1.3.1). What a rebuild could not carry over is refused here, before anything
    /// changes: a sparse member, or a shadowed one that must follow its winner (rule 3), keeps a block map no header
    /// rewrite can move, and a name that does not decode has no text to be written under.
    pub(crate) fn rename(
        &mut self,
        from: &str,
        to: &str,
        replace: bool,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        let spec = Spec::archive_member(scope.archive.clone(), from);
        let source = self.lookup(from, &spec)?;
        let moved_folder = source.kind == MemberKind::Dir;
        if from == to {
            return Ok(());
        }
        // The text of the name is not the name: writing it back would make up a new one.
        if source.lossy && to.contains(char::REPLACEMENT_CHARACTER) {
            return Err(scope.unsupported(from, VfsOp::Rename));
        }
        let prefix = format!("{from}/");
        if to.starts_with(&prefix) {
            return Err(VfsError::from_io(
                spec.canonical(),
                VfsOp::Rename,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "cannot move a folder into itself",
                ),
            ));
        }
        self.ready_parent(to, false, scope)?;
        let subtree: Vec<Box<str>> = self
            .nodes
            .range::<str, _>((Bound::Included(prefix.as_str()), Bound::Unbounded))
            .take_while(|(path, _)| path.starts_with(&prefix))
            .map(|(path, _)| path.clone())
            .collect();
        for path in &subtree {
            let below = self.get(path).is_some_and(|item| item.lossy);
            let new = format!("{to}{}", path.get(from.len()..).unwrap_or_default());
            check_path(&new, &scope.limits, scope.archive)?;
            if below || self.sparse(path) || self.stuck(path, scope.tree) {
                return Err(scope.unsupported(path, VfsOp::Rename));
            }
        }
        if self.sparse(from) || self.stuck(from, scope.tree) {
            return Err(scope.unsupported(from, VfsOp::Rename));
        }
        if let Some(existing) = self.get(to) {
            if !replace {
                return Err(scope.exists(to));
            }
            match (moved_folder, existing.kind == MemberKind::Dir) {
                (true, false) => return Err(scope.not_a_folder(to)),
                (false, true) => {
                    return Err(VfsError::IsADirectory {
                        spec: scope.text(to),
                    });
                }
                (true, true) if self.has_children(to) => {
                    return Err(VfsError::NotEmpty {
                        spec: scope.text(to),
                    });
                }
                _ => {}
            }
        }
        self.relocate(from, to, scope.tree);
        for path in subtree {
            let new = format!("{to}{}", path.get(from.len()..).unwrap_or_default());
            self.relocate(&path, &new, scope.tree);
        }
        Ok(())
    }

    fn sparse(&self, path: &str) -> bool {
        self.get(path)
            .is_some_and(|item| item.kind == MemberKind::Sparse)
    }

    /// True when the record at `path` has a shadow that cannot follow it.
    fn stuck(&self, path: &str, tree: &Tree) -> bool {
        match self.get(path).map(|item| &item.origin) {
            Some(Origin::Kept { node, .. }) => tree
                .nodes
                .get(*node as usize)
                .is_some_and(|node| tree.is_stuck(node.record)),
            _ => false,
        }
    }

    /// An implicit folder that does not decode has no record to edit, and a new one would need a name made up.
    fn check_can_touch(&self, path: &str, op: VfsOp, scope: &Scope<'_>) -> Result<(), VfsError> {
        match self.get(path) {
            Some(item) if item.lossy && matches!(item.origin, Origin::Implicit) => {
                Err(scope.unsupported(path, op))
            }
            _ => Ok(()),
        }
    }

    /// Moves one item to `to`, which it replaces. The item itself loses a name that does not decode: `to` is text.
    fn relocate(&mut self, from: &str, to: &str, tree: &Tree) {
        let Some(mut item) = self.nodes.remove(from) else {
            return;
        };
        if let Origin::Kept { node, moved, .. } = &mut item.origin {
            let original = tree.nodes.get(*node as usize).map(Node::path);
            *moved = original != Some(to);
        }
        item.lossy = false;
        self.nodes.insert(to.into(), item);
    }

    /// `set_times` of a modification time.
    pub(crate) fn set_mtime(
        &mut self,
        path: &str,
        modified: FileTime,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        let spec = Spec::archive_member(scope.archive.clone(), path);
        self.lookup(path, &spec)?;
        if self.sparse(path) {
            return Err(scope.unsupported(path, VfsOp::SetTimes));
        }
        self.check_can_touch(path, VfsOp::SetTimes, scope)?;
        if let Some(item) = self.nodes.get_mut(path)
            && item.mtime != Some(modified)
        {
            item.mtime = Some(modified);
            item.touch();
        }
        Ok(())
    }

    /// `set_attrs`: `Mode`, and in a zip `Dos` with the `R` bit only (§1.5.3). In a tar a mode keeps every bit; a zip
    /// reader gives `rwx` back, so that is what the overlay keeps.
    pub(crate) fn set_attrs(
        &mut self,
        path: &str,
        change: &AttrChange,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        change.validate()?;
        let spec = Spec::archive_member(scope.archive.clone(), path);
        let item = self.lookup(path, &spec)?;
        if matches!(item.kind, MemberKind::Symlink | MemberKind::Sparse) {
            return Err(scope.unsupported(path, VfsOp::SetAttrs));
        }
        self.check_can_touch(path, VfsOp::SetAttrs, scope)?;
        let zip = scope.format == ArchiveFormat::Zip;
        let unix = |mode: u32, was: &Attrs| {
            let (uid, gid) = was.unix().map_or((0, 0), |meta| (meta.uid, meta.gid));
            Attrs::Unix {
                meta: UnixMeta { mode, uid, gid },
            }
        };
        let new = match *change {
            AttrChange::Mode(mode) if zip => unix(mode & 0o777, &item.attrs),
            AttrChange::Mode(mode) => unix(mode & 0o7777, &item.attrs),
            AttrChange::Dos { set, clear } if zip && DosAttrs::READONLY.contains(set | clear) => {
                match item.attrs {
                    Attrs::Dos { attrs } => Attrs::Dos {
                        attrs: (attrs | set).difference(clear),
                    },
                    Attrs::Unix { meta } => {
                        let mut mode = meta.mode;
                        if !set.is_empty() {
                            mode &= !0o222;
                        }
                        if !clear.is_empty() {
                            mode |= 0o200;
                        }
                        unix(mode, &item.attrs)
                    }
                    Attrs::None => Attrs::Dos { attrs: set },
                }
            }
            AttrChange::Dos { .. } => return Err(scope.unsupported(path, VfsOp::SetAttrs)),
        };
        if let Some(item) = self.nodes.get_mut(path)
            && item.attrs != new
        {
            item.attrs = new;
            item.touch();
        }
        Ok(())
    }

    /// `create_symlink`. The target is stored as given; one that no reader would give back is refused.
    pub(crate) fn symlink(
        &mut self,
        path: &str,
        target: &str,
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        if self.get(path).is_some() {
            return Err(scope.exists(path));
        }
        if target.is_empty() || target.len() as u64 > LINK_TARGET_MAX {
            return Err(VfsError::InvalidName {
                name: target.to_owned(),
            });
        }
        self.ready_parent(path, false, scope)?;
        let mut item = Item::fresh(MemberKind::Symlink, Origin::Fresh, LINK_MODE, scope.now);
        item.link_target = Some(target.into());
        if scope.format == ArchiveFormat::Zip {
            item.size = target.len() as u64;
        }
        self.insert(path, item, scope)
    }

    /// Enters a staged file at `path` (the `commit` of `open_write`). A file or link at the target is replaced, never
    /// followed; a replaced file gives its attributes to the new one, as a local write keeps a file's permissions.
    pub(crate) fn put_file(
        &mut self,
        path: &str,
        file: Arc<CacheFile>,
        crc32: Option<u32>,
        (overwrite, parents): (bool, bool),
        scope: &Scope<'_>,
    ) -> Result<(), VfsError> {
        let mut attrs = None;
        match self.get(path) {
            Some(item) if item.kind == MemberKind::Dir => {
                return Err(VfsError::IsADirectory {
                    spec: scope.text(path),
                });
            }
            Some(_) if !overwrite => return Err(scope.exists(path)),
            Some(item) if item.kind == MemberKind::Symlink => {}
            Some(item) => attrs = Some(item.attrs),
            None => self.ready_parent(path, parents, scope)?,
        }
        let size = file.len();
        let mut item = Item::fresh(MemberKind::File, Origin::Staged(file), FILE_MODE, scope.now);
        item.size = size;
        item.crc32 = crc32;
        if let Some(attrs) = attrs {
            item.attrs = attrs;
        }
        self.insert(path, item, scope)
    }

    /// Puts the rebuilt bytes of a nested archive in the place of its member: its attributes stay, and so does a
    /// time the batch set on it. `false` when no such member exists: the write would be lost.
    pub(crate) fn put_rebuilt(
        &mut self,
        path: &str,
        file: Arc<CacheFile>,
        scope: &Scope<'_>,
    ) -> bool {
        let Some(item) = self.nodes.get_mut(path) else {
            return false;
        };
        let set = matches!(item.origin, Origin::Kept { touched: true, .. });
        item.size = file.len();
        item.crc32 = None;
        item.origin = Origin::Staged(file);
        item.kind = MemberKind::File;
        if !set {
            item.mtime = scope.now;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::index::{DataRef, NameBudget, RawMember, build_tree};
    use hikasa_core::{Ctl, LocalPath};
    use std::path::PathBuf;

    fn member(path: &str, kind: MemberKind, at: usize) -> RawMember {
        RawMember {
            path: Ok(path.to_owned()),
            kind,
            lossy: false,
            encrypted: false,
            size: 1,
            mtime: None,
            ctime: None,
            attrs: Attrs::None,
            crc32: None,
            link_target: None,
            hard_target: None,
            data: DataRef::Zip { index: at },
        }
    }

    fn tree_of(members: &[(&str, MemberKind)]) -> Tree {
        let pass = crate::archive::codec::Pass {
            spec: "t",
            limits: ArchiveLimits::default(),
            ctl: Ctl::detached(),
            zone: jiff::tz::TimeZone::UTC,
        };
        let raw: Vec<RawMember> = members
            .iter()
            .enumerate()
            .map(|(at, (path, kind))| member(path, *kind, at))
            .collect();
        build_tree(raw, NameBudget::new(&pass), &pass).unwrap()
    }

    fn archive() -> Spec {
        Spec::Local(
            LocalPath::new(PathBuf::from(if cfg!(windows) {
                "C:\\a.zip"
            } else {
                "/a.zip"
            }))
            .unwrap(),
        )
    }

    fn paths(overlay: &Overlay) -> Vec<&str> {
        overlay.items().map(|(path, _)| path).collect()
    }

    fn child_paths<'a>(overlay: &'a Overlay, dir: &str) -> Vec<&'a str> {
        overlay.children(dir).into_iter().map(|(p, _)| p).collect()
    }

    /// Children skip subtrees, and a name that sorts between a folder and its members does not hide anything.
    #[test]
    fn children_are_found_past_subtrees() {
        let tree = tree_of(&[
            ("a/x/1", MemberKind::File),
            ("a/x/2", MemberKind::File),
            ("a-b", MemberKind::File),
            ("a/y", MemberKind::File),
            ("a0", MemberKind::File),
            ("b", MemberKind::File),
        ]);
        let overlay = Overlay::new(&tree);
        assert_eq!(child_paths(&overlay, ""), ["a", "a-b", "a0", "b"]);
        assert_eq!(child_paths(&overlay, "a"), ["a/x", "a/y"]);
        assert_eq!(child_paths(&overlay, "a/x"), ["a/x/1", "a/x/2"]);
        assert!(overlay.has_children("a"));
        assert!(!overlay.has_children("b"));
    }

    #[test]
    fn an_unchanged_overlay_needs_no_rebuild() {
        let tree = tree_of(&[("d/x", MemberKind::File), ("y", MemberKind::File)]);
        let archive = archive();
        let scope = Scope {
            archive: &archive,
            tree: &tree,
            format: ArchiveFormat::Zip,
            limits: ArchiveLimits::default(),
            now: None,
        };
        let mut overlay = Overlay::new(&tree);
        assert!(!overlay.changed(&tree));
        overlay.create_dir("n", false, &scope).unwrap();
        assert!(overlay.changed(&tree));
        overlay.remove("n", &scope).unwrap();
        assert!(!overlay.changed(&tree), "made and removed");
        overlay.rename("y", "z", false, &scope).unwrap();
        assert!(overlay.changed(&tree));
        overlay.rename("z", "y", false, &scope).unwrap();
        assert!(!overlay.changed(&tree), "and back");
        overlay.remove("d/x", &scope).unwrap();
        assert!(overlay.changed(&tree));
    }

    #[test]
    fn a_subtree_moves_and_every_member_knows() {
        let tree = tree_of(&[("d/s/x", MemberKind::File), ("d/y", MemberKind::File)]);
        let archive = archive();
        let scope = Scope {
            archive: &archive,
            tree: &tree,
            format: ArchiveFormat::Zip,
            limits: ArchiveLimits::default(),
            now: None,
        };
        let mut overlay = Overlay::new(&tree);
        overlay.rename("d", "e", false, &scope).unwrap();
        assert_eq!(paths(&overlay), ["e", "e/s", "e/s/x", "e/y"]);
        let moved = |path| {
            matches!(
                overlay.get(path).unwrap().origin,
                Origin::Kept { moved: true, .. }
            )
        };
        assert!(moved("e/s/x") && moved("e/y"));
        let inside = overlay.clone().rename("e", "e/x", false, &scope);
        assert!(inside.is_err(), "{inside:?}");
    }

    /// What a rebuild cannot carry is refused before anything changes.
    #[test]
    fn a_sparse_member_stays_where_it_is() {
        let tree = tree_of(&[("d/s", MemberKind::Sparse), ("d/x", MemberKind::File)]);
        let archive = archive();
        let scope = Scope {
            archive: &archive,
            tree: &tree,
            format: ArchiveFormat::Tar,
            limits: ArchiveLimits::default(),
            now: None,
        };
        let mut overlay = Overlay::new(&tree);
        for result in [
            overlay.rename("d", "e", false, &scope),
            overlay.rename("d/s", "t", false, &scope),
        ] {
            assert!(matches!(result, Err(VfsError::OperationUnsupported { .. })));
        }
        let at = FileTime::from_unix(5, 0).unwrap();
        let times = overlay.set_mtime("d/s", at, &scope);
        assert!(matches!(times, Err(VfsError::OperationUnsupported { .. })));
        assert_eq!(paths(&overlay), ["d", "d/s", "d/x"]);
        assert!(!overlay.changed(&tree));
    }

    #[test]
    fn limits_bind_new_members() {
        let tree = tree_of(&[("a", MemberKind::File)]);
        let archive = archive();
        let limits = ArchiveLimits {
            entries_max: 2,
            depth_max: 2,
            ..ArchiveLimits::default()
        };
        let scope = Scope {
            archive: &archive,
            tree: &tree,
            format: ArchiveFormat::Zip,
            limits,
            now: None,
        };
        let mut overlay = Overlay::new(&tree);
        overlay.create_dir("b", false, &scope).unwrap();
        let full = overlay.create_dir("c", false, &scope);
        assert!(matches!(
            full,
            Err(VfsError::ArchiveLimit {
                limit: "entries",
                ..
            })
        ));
        assert!(check_path("a/b", &limits, &archive).is_ok());
        assert!(check_path("a/b/c", &limits, &archive).is_err());
        assert!(check_path(&"n".repeat(NAME_MAX + 1), &limits, &archive).is_err());
    }
}
