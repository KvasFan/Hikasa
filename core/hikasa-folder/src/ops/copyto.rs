//! `copyto`, `moveto` (rule 16, FC-072): the target is no row, so what stands there is looked at; the rest is the builder's.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

use hikasa_core::{Side, Spec};
use hikasa_vfs::ops::{
    self, Action, Expect, ItemStamp, OverwritePolicy, PlanItem, SkipReason,
};
use hikasa_vfs::{Capabilities, EntryKind, FileName, FollowLinks, Vfs, VfsError};

use super::PathStructure;
use super::build::{Builder, Found, Open, Placed, Transfer, nothing_on, report, stamp_of_stat};
use super::conflict::Conflicts;
use super::ItemMeta;
use crate::error::FolderError;
use crate::model::{NodeId, ScanStatus};
use crate::select::{Resolved, SideSet};

/// The folders below a target, as the plan knows them.
#[derive(Default)]
struct Below {
    /// Folder (canonical) → whether it will exist only through this plan, with the depth of what lies directly in it.
    folders: BTreeMap<String, Option<u32>>,
    /// The targets taken, as compared.
    taken: BTreeSet<String>,
}

/// One row with its own path below the base.
struct Source {
    row: Resolved,
    names: Vec<OsString>,
    /// No row above it counts: `check_nesting` looks at it.
    root: bool,
}

/// The number of leading folders common to all `paths`, in whole components.
fn common_prefix(paths: &[&[OsString]]) -> usize {
    let Some(first) = paths.first() else {
        return 0;
    };
    let mut common = first.len();
    for path in paths {
        let same = first
            .iter()
            .zip(path.iter())
            .take_while(|(a, b)| a == b)
            .count();
        common = common.min(same);
    }
    common
}

/// What stands at `spec`, unfollowed.
fn lstat(vfs: &dyn Vfs, spec: &Spec) -> Result<Option<hikasa_vfs::Entry>, VfsError> {
    match vfs.stat(spec, FollowLinks::No) {
        Ok(entry) => Ok(Some(entry)),
        Err(VfsError::NotFound { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Builder<'_, '_> {
    /// The rows of `side` with their paths by that side's own names; the roots are those whose parent row does not count.
    fn sources(&self, rows: &[Resolved], side: Side) -> Vec<Source> {
        let bit = SideSet::of(side);
        let mine: BTreeSet<NodeId> = rows
            .iter()
            .filter(|row| row.sides.contains(bit))
            .map(|row| row.node)
            .collect();
        rows.iter()
            .filter(|row| row.sides.contains(bit))
            .map(|row| {
                let mut names = Vec::new();
                let mut at = Some(row.node);
                let parent = self.read.node(row.node).and_then(|view| view.parent);
                while let Some(id) = at.filter(|id| *id != NodeId::ROOT) {
                    let view = self.read.node(id);
                    names.extend(self.read.raw_name(id, side));
                    at = view.and_then(|view| view.parent);
                }
                names.reverse();
                Source {
                    row: *row,
                    names,
                    root: !parent.is_some_and(|parent| mine.contains(&parent)),
                }
            })
            .collect()
    }

    /// The target folder: existing, or its chain at depth 0, ending in `create_archive` when its final name has a writable
    /// mask (D-150). What children go into: an archive's root for an archive.
    fn target_root(&mut self, target: &Spec, below: &mut Below) -> Result<Spec, FolderError> {
        let not_folder = || {
            FolderError::InvalidRules(format!("{}: is not a folder", target.canonical()))
        };
        // A member's root: the file around it says what there is.
        let file = match target {
            Spec::ArchiveMember { archive, member } if member.is_empty() => Some(&**archive),
            _ => None,
        };
        let on_disk = file.unwrap_or(target);
        let found = lstat(self.vfs, on_disk).map_err(|e| FolderError::InvalidRules(e.to_string()))?;
        let writes_archive = on_disk
            .file_name()
            .and_then(|name| self.vfs.archive_format(&name))
            .is_some_and(|format| format.writable());
        match found {
            Some(entry) if entry.kind == EntryKind::Dir => Ok(target.clone()),
            Some(_) if file.is_some() => Ok(target.clone()),
            Some(entry) if entry.kind == EntryKind::Symlink => {
                match self.vfs.stat(on_disk, FollowLinks::Yes) {
                    Ok(linked) if linked.kind == EntryKind::Dir => Ok(target.clone()),
                    _ => Err(not_folder()),
                }
            }
            Some(_) => Err(not_folder()),
            None => {
                self.target_chain(on_disk, below)?;
                let action = if writes_archive && (file.is_some() || matches!(target, Spec::Local(_)))
                {
                    Action::CreateArchive
                } else {
                    Action::CreateDir
                };
                let kind = if action == Action::CreateArchive {
                    EntryKind::File
                } else {
                    EntryKind::Dir
                };
                let item = PlanItem::planned(action, kind, None, on_disk.clone(), 0);
                self.push_outside(item, on_disk);
                below.folders.insert(on_disk.canonical(), Some(0));
                if action == Action::CreateArchive {
                    let root = Spec::archive_member(on_disk.clone(), "");
                    below.folders.insert(root.canonical(), Some(0));
                    return Ok(root);
                }
                Ok(target.clone())
            }
        }
    }

    /// The missing folders above `spec`.
    fn target_chain(&mut self, spec: &Spec, below: &mut Below) -> Result<(), FolderError> {
        let mut missing = Vec::new();
        let mut at = spec.parent();
        while let Some(dir) = at {
            match lstat(self.vfs, &dir) {
                Ok(None) => {
                    at = dir.parent();
                    missing.push(dir);
                }
                Ok(Some(_)) => break,
                Err(e) => return Err(FolderError::InvalidRules(e.to_string())),
            }
        }
        for dir in missing.into_iter().rev() {
            let item = PlanItem::planned(Action::CreateDir, EntryKind::Dir, None, dir.clone(), 0);
            self.push_outside(item, &dir);
            below.folders.insert(dir.canonical(), Some(0));
        }
        Ok(())
    }

    /// An item of the target's own chain.
    fn push_outside(&mut self, item: PlanItem, spec: &Spec) {
        let meta = ItemMeta {
            side: None,
            node: None,
            path: spec.canonical(),
            conflicts: Conflicts::empty(),
            answer: None,
        };
        self.push(item, meta);
    }

    /// `CopyTo`, `MoveTo`: each side in turn, left before right; a second source of one target path stays.
    pub(super) fn copy_to(
        &mut self,
        rows: &[Resolved],
        sides: SideSet,
        target: &Spec,
        structure: PathStructure,
        overwrite: OverwritePolicy,
        moving: bool,
    ) -> Result<(), FolderError> {
        if !rows.iter().any(|row| row.sides.intersects(sides)) {
            return Err(FolderError::InvalidRules(nothing_on(sides)));
        }
        if !matches!(target, Spec::Local(_) | Spec::ArchiveMember { .. }) {
            return Err(FolderError::InvalidRules(format!(
                "{}: nothing can be written there",
                target.canonical()
            )));
        }
        let mut below = Below::default();
        let root = self.target_root(target, &mut below)?;
        self.outside = Some(root.clone());
        let fold = self.folds(&root);
        for side in sides.sides() {
            self.depths.clear();
            self.covered.iter_mut().for_each(|slot| *slot = false);
            let sources = self.sources(rows, side);
            // The common prefix of a side: the folders of its roots.
            let parents: Vec<&[OsString]> = sources
                .iter()
                .filter(|source| source.root)
                .map(|source| source.names.split_last().map_or(&[][..], |(_, rest)| rest))
                .collect();
            let drop = match structure {
                PathStructure::Base => 0,
                PathStructure::Relative => common_prefix(&parents),
                PathStructure::None => usize::MAX,
            };
            let mut open = Vec::new();
            for source in &sources {
                self.ctl.check()?;
                self.close_open(&mut open, source.row.depth);
                let tr = Transfer {
                    from: side,
                    to: side,
                    moving,
                    overwrite,
                };
                self.copy_to_row(tr, source, &root, drop, fold, &mut below, &mut open)?;
            }
            self.close_open(&mut open, 0);
        }
        Ok(())
    }

    /// The target of a source: `<root>/<names without the first drop>`, every folder on the way planned, once.
    fn path_below(
        &mut self,
        tr: Transfer,
        source: &Source,
        root: &Spec,
        drop: usize,
        below: &mut Below,
    ) -> Result<(Spec, u32, bool), VfsError> {
        let Some((name, folders)) = source.names.split_last() else {
            return Ok((root.clone(), 0, false));
        };
        let kept = folders.get(drop.min(folders.len())..).unwrap_or_default();
        let mut at = root.clone();
        let mut depth = 0;
        let mut new_parent = below.folders.contains_key(&root.canonical());
        let mut level = source.names.len() - 1 - kept.len();
        for folder in kept {
            at = self.vfs.child(&at, &FileName::from_os(folder))?;
            let key = at.canonical();
            let known = match below.folders.get(&key) {
                Some(known) => *known,
                None => {
                    let made = !new_parent && lstat(self.vfs, &at)?.is_none() || new_parent;
                    let known = if made {
                        self.folder_on_the_way(tr, source, level, &at, depth);
                        Some(depth)
                    } else {
                        None
                    };
                    below.folders.insert(key, known);
                    known
                }
            };
            new_parent = known.is_some();
            depth = known.map_or(0, |depth| depth + 1);
            level += 1;
        }
        let target = self.vfs.child(&at, &FileName::from_os(name))?;
        Ok((target, depth, new_parent))
    }

    /// A folder a source needs below the target: its `create_dir`, from the folder of that level (rule 3).
    fn folder_on_the_way(
        &mut self,
        tr: Transfer,
        source: &Source,
        level: usize,
        target: &Spec,
        depth: u32,
    ) {
        let read = self.read;
        let mut id = source.row.node;
        for _ in level + 1..source.names.len() {
            id = read.node(id).and_then(|view| view.parent).unwrap_or(NodeId::ROOT);
        }
        let Some(view) = read.node(id) else { return };
        let (Some(entry), Some(from)) = (view.side(tr.from).copied(), read.spec(id, tr.from)) else {
            return;
        };
        let placed = Placed {
            id,
            entry: &entry,
            source: from,
            target: target.clone(),
            found: None,
            depth,
            side: None,
        };
        self.create_dir(placed, OverwritePolicy::Fail);
    }

    /// One row of a `copyto` or `moveto`.
    #[allow(clippy::too_many_arguments)]
    fn copy_to_row(
        &mut self,
        tr: Transfer,
        source: &Source,
        root: &Spec,
        drop: usize,
        fold: bool,
        below: &mut Below,
        open: &mut Vec<Open>,
    ) -> Result<(), FolderError> {
        let read = self.read;
        let id = source.row.node;
        let Some(view) = read.node(id) else {
            return Ok(());
        };
        if self.is_covered(&view) {
            return Ok(());
        }
        let (Some(entry), Some(from)) = (view.side(tr.from).copied(), read.spec(id, tr.from)) else {
            return Ok(());
        };
        let kind = view.kind.entry_kind();
        let flat = drop == usize::MAX;
        let writes: &[Side] = if tr.moving {
            std::slice::from_ref(&tr.from)
        } else {
            &[]
        };
        let (target, depth, under_new) = match self.path_below(tr, source, root, drop, below) {
            Ok(path) => path,
            Err(e) => {
                self.fail_row(id, Action::Copy, kind, Some(from), root.clone(), 0, report(&e), None);
                self.cover(id);
                return Ok(());
            }
        };
        // Below a folder that goes, an item blocks it by its depth.
        let depth = depth.max(self.child_depth(view.parent));
        if let Some(reason) = self.refusal(id, &view, Some(tr.from), writes) {
            self.skip_row(id, kind, Some(from), target, depth, reason, None);
            self.cover(id);
            return Ok(());
        }
        if source.root {
            let checked = self.vfs.stat(&from, FollowLinks::No).and_then(|stat| {
                ops::check_nesting(self.vfs, self.vfs, &from, &stat, &target)?;
                if tr.moving {
                    ops::check_move_inside(self.vfs, self.vfs, &from, &target)?;
                    ops::check_same_archive(self.vfs, self.vfs, &from, &target)?;
                }
                Ok(())
            });
            if let Err(e) = checked {
                let action = if view.is_folder() {
                    Action::CreateDir
                } else {
                    Action::Copy
                };
                self.fail_row(id, action, kind, Some(from), target, depth, report(&e), None);
                self.cover(id);
                return Ok(());
            }
        }
        let bit = if view.is_folder() {
            (Capabilities::MKDIR, "mkdir")
        } else {
            (Capabilities::WRITE, "write")
        };
        let gone = tr.moving && self.lacks(&from, Capabilities::REMOVE, "remove", Some(tr.from));
        if gone || self.lacks(&target, bit.0, bit.1, None) {
            let reason = SkipReason::Capability;
            self.skip_row(id, kind, Some(from), target, depth, reason, None);
            self.cover(id);
            return Ok(());
        }
        if view.is_folder() && !flat {
            return self.copy_to_folder(tr, source, &view, &entry, from, target, depth, under_new, below, open);
        }
        if view.is_folder() {
            // Flat: a folder is no item, and goes only when all of it has.
            if tr.moving && source.row.whole.contains(SideSet::of(tr.from)) && !entry.followed {
                self.depths.insert(id, depth);
                open.push(Open {
                    level: source.row.depth,
                    id,
                    spec: from,
                    depth,
                    side: tr.from,
                    conflicts: Conflicts::empty(),
                });
            }
            return Ok(());
        }
        let key = if fold {
            hikasa_core::case::fold(&target.canonical()).into_owned()
        } else {
            target.canonical()
        };
        if !below.taken.insert(key) {
            let reason = SkipReason::DuplicateTarget;
            self.skip_row(id, kind, Some(from), target, depth, reason, None);
            return Ok(());
        }
        let found = if under_new {
            None
        } else {
            match lstat(self.vfs, &target) {
                Ok(found) => found.map(|stat| Found {
                    kind: stat.kind,
                    stamp: stamp_of_stat(&stat),
                    attrs: stat.attrs.clone(),
                    mtime: stat.mtime,
                    row: None,
                }),
                Err(e) => {
                    self.fail_row(id, Action::Copy, kind, Some(from), target, depth, report(&e), None);
                    return Ok(());
                }
            }
        };
        match self.kind_action(&view, &from, &target, None, tr.moving) {
            Ok(action) => {
                let placed = Placed {
                    id,
                    entry: &entry,
                    source: from,
                    target,
                    found,
                    depth,
                    side: None,
                };
                self.place_file(placed, &view, action, tr);
            }
            Err(reason) => self.skip_row(id, kind, Some(from), target, depth, reason, None),
        }
        Ok(())
    }

    /// A folder row below the target: a `NotScanned` one by rule 6, a whole one of a move as one `rename` where it can be,
    /// else its `create_dir` once.
    #[allow(clippy::too_many_arguments)]
    fn copy_to_folder(
        &mut self,
        tr: Transfer,
        source: &Source,
        view: &crate::model::NodeView<'_>,
        entry: &crate::model::EntryView<'_>,
        from: Spec,
        target: Spec,
        depth: u32,
        under_new: bool,
        below: &mut Below,
        open: &mut Vec<Open>,
    ) -> Result<(), FolderError> {
        let id = source.row.node;
        let found = if under_new {
            None
        } else {
            lstat(self.vfs, &target).map_err(|e| FolderError::InvalidRules(e.to_string()))?
        };
        if view.scan == ScanStatus::NotScanned {
            self.expand_unlisted(tr, id, entry, &from, &target, depth)?;
            self.relabel_outside();
            self.cover(id);
            return Ok(());
        }
        let whole = source.row.whole.contains(SideSet::of(tr.from));
        if tr.moving && whole && found.is_none() && self.can_rename(&from, &target) {
            let mut item =
                PlanItem::planned(Action::Rename, EntryKind::Dir, Some(from), target, depth);
            item.expect = Some(Expect {
                source: Some(ItemStamp {
                    kind: EntryKind::Dir,
                    size: None,
                    mtime: None,
                }),
                target: None,
            });
            let meta = self.meta_of(id, None);
            self.push(item, meta);
            self.cover(id);
            return Ok(());
        }
        let key = target.canonical();
        match found {
            Some(stat) if stat.kind == EntryKind::Dir => {
                below.folders.entry(key).or_insert(None);
            }
            _ => {
                let found = found.map(|stat| Found {
                    kind: stat.kind,
                    stamp: stamp_of_stat(&stat),
                    attrs: stat.attrs.clone(),
                    mtime: stat.mtime,
                    row: None,
                });
                if !below.folders.contains_key(&key) {
                    let placed = Placed {
                        id,
                        entry,
                        source: from.clone(),
                        target,
                        found,
                        depth,
                        side: None,
                    };
                    self.create_dir(placed, tr.overwrite);
                    below.folders.insert(key, Some(depth));
                    self.depths.insert(id, depth);
                }
            }
        }
        if tr.moving && whole && !entry.followed {
            let at = match self.depths.get(&id) {
                Some(at) => *at,
                None => {
                    self.depths.insert(id, depth);
                    depth
                }
            };
            open.push(Open {
                level: source.row.depth,
                id,
                spec: from,
                depth: at,
                side: tr.from,
                conflicts: Conflicts::empty(),
            });
        }
        Ok(())
    }

    /// Spliced items of a target outside the bases have no side.
    fn relabel_outside(&mut self) {
        for meta in self.meta.iter_mut().rev() {
            if meta.node.is_some() || meta.side.is_none() {
                break;
            }
            meta.side = None;
        }
    }
}
