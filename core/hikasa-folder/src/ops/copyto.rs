//! `copyto`, `moveto` (rule 16, FC-072): the target is no row, so what stands there is looked at; the rest is the builder's.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

use hikasa_core::{Side, Spec};
use hikasa_vfs::ops::{self, Action, OverwritePolicy, PlanItem, SkipReason};
use hikasa_vfs::{Capabilities, EntryKind, FileName, FollowLinks, Vfs, VfsError};

use super::ItemMeta;
use super::PathStructure;
use super::build::{
    Builder, Found, Open, Placed, Transfer, found_of_stat, key_of, nothing_on, report, view_of_entry,
};
use super::conflict::Conflicts;
use super::unlisted::Walk;
use crate::error::FolderError;
use crate::model::{NodeFlags, NodeId, ScanStatus};
use crate::select::{Resolved, SideSet};

/// The folders below a target, as the plan knows them.
#[derive(Default)]
struct Below {
    /// Folder (canonical) → whether it will exist only through this plan, with the depth of what lies directly in it.
    folders: BTreeMap<String, Option<u32>>,
    /// The folders planned only as the way to a source, by item: they stay only if something goes there.
    way: BTreeSet<usize>,
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
        Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => Ok(None),
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
                // In a flat comparison the folders of a row are no rows.
                let dir = self.read.node(row.node).and_then(|view| view.side(side)?.dir);
                let folders = dir.iter().flat_map(|dir| dir.rsplit('/'));
                names.extend(
                    folders
                        .filter(|name| !name.is_empty())
                        .map(OsString::from),
                );
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
    fn target_root(
        &mut self,
        target: &Spec,
        folder: bool,
        below: &mut Below,
    ) -> Result<Spec, FolderError> {
        let not_folder =
            || FolderError::InvalidRules(format!("{}: is not a folder", target.canonical()));
        // A member's root: the file around it says what there is.
        let file = match target {
            Spec::ArchiveMember { archive, member } if member.is_empty() => Some(&**archive),
            _ => None,
        };
        let on_disk = file.unwrap_or(target);
        let found =
            lstat(self.vfs, on_disk).map_err(|e| FolderError::InvalidRules(e.to_string()))?;
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
                let archive =
                    !folder && writes_archive && (file.is_some() || matches!(target, Spec::Local(_)));
                let action = if archive {
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
                below.way.insert(self.items.len());
                self.push_outside(item, on_disk);
                below.folders.insert(on_disk.canonical(), Some(0));
                if action == Action::CreateArchive {
                    let root = Spec::archive_member(on_disk.clone(), "");
                    below.folders.insert(root.canonical(), Some(0));
                    return Ok(root);
                }
                Ok(on_disk.clone())
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
            below.way.insert(self.items.len());
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

    /// `CopyTo`, `MoveTo`: each side in turn, left before right; a second source of one target path stays. A folder planned
    /// only as the way to a source goes when nothing goes there: a refused move leaves nothing behind.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn copy_to(
        &mut self,
        rows: &[Resolved],
        sides: SideSet,
        target: &Spec,
        folder: bool,
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
        let root = self.target_root(target, folder, &mut below)?;
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
        self.drop_unused_way(&below.way);
        Ok(())
    }

    /// What a folder renamed whole takes along is at its target as if each row had gone: a second source of one of those
    /// paths stays.
    fn claim_rows(&mut self, id: NodeId, side: Side, target: &Spec) {
        let read = self.read;
        let mut stack: Vec<(NodeId, Spec)> = read.children(id).map(|row| (row, target.clone())).collect();
        while let Some((row, dir)) = stack.pop() {
            let Some(view) = read.node(row) else { continue };
            let (Some(_), Some(name)) = (view.side(side), read.raw_name(row, side)) else {
                continue;
            };
            let Ok(at) = self.vfs.child(&dir, &FileName::from_os(&name)) else {
                continue;
            };
            if view.is_folder() {
                self.note_made(&at, 0);
                stack.extend(read.children(row).map(|child| (child, at.clone())));
            } else {
                let key = key_of(&at.canonical(), self.folds(&at));
                self.taken.insert(key);
            }
        }
    }

    /// Takes the folders of `way` no item needs out of the plan.
    fn drop_unused_way(&mut self, way: &BTreeSet<usize>) {
        let mut needed = BTreeSet::new();
        for (index, item) in self.items.iter().enumerate() {
            if way.contains(&index) || item.error.is_some() || item.skip.is_some() {
                continue;
            }
            for dir in std::iter::successors(item.target.parent(), Spec::parent) {
                if let Spec::ArchiveMember { archive, .. } = &dir {
                    needed.insert(archive.canonical());
                }
                if !needed.insert(dir.canonical()) {
                    break;
                }
            }
        }
        let unused: BTreeSet<usize> = way
            .iter()
            .copied()
            .filter(|index| {
                self.items
                    .get(*index)
                    .is_some_and(|item| !needed.contains(&item.target.canonical()))
            })
            .collect();
        if unused.is_empty() {
            return;
        }
        let mut at = 0;
        self.items.retain(|_| {
            at += 1;
            !unused.contains(&(at - 1))
        });
        at = 0;
        self.meta.retain(|_| {
            at += 1;
            !unused.contains(&(at - 1))
        });
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
        let first = source.names.len() - 1 - kept.len();
        for (index, folder) in kept.iter().enumerate() {
            at = self.vfs.child(&at, &FileName::from_os(folder))?;
            let key = at.canonical();
            // A file of this plan stands where this needs a folder.
            let folded = key_of(&key, self.folds(&at));
            if self.taken.contains(&folded) {
                return Err(VfsError::NotADirectory { spec: key });
            }
            let known = match below.folders.get(&key) {
                Some(known) => *known,
                None if self.is_made(&at) => {
                    let known = self.made_depth(&at);
                    below.folders.insert(key, known);
                    known
                }
                None => {
                    // What stands there: nothing, a folder, or a file that rule 8 replaces or keeps.
                    let stood = if new_parent { None } else { lstat(self.vfs, &at)? };
                    let made = !stood.as_ref().is_some_and(|it| it.kind == EntryKind::Dir);
                    let known = if made {
                        let at_item = self.items.len();
                        below.way.insert(at_item);
                        let found = stood.as_ref().map(found_of_stat);
                        self.folder_on_the_way(tr, source, first + index, &at, found, depth);
                        // A folder that cannot be made is no folder for the next source: it meets the same refusal.
                        if self.items.get(at_item).is_some_and(|item| item.error.is_some()) {
                            return Err(VfsError::NotADirectory { spec: key });
                        }
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
        found: Option<Found>,
        depth: u32,
    ) {
        let read = self.read;
        let mut id = source.row.node;
        let flat = read
            .node(id)
            .is_some_and(|view| view.side(tr.from).is_some_and(|entry| entry.dir.is_some()));
        if flat {
            // The folder is no row: the source's own path says which, and a `stat` what it is.
            let mut from = read.spec(id, tr.from);
            for _ in level + 1..source.names.len() {
                from = from.and_then(|spec| spec.parent());
            }
            let Some(from) = from else { return };
            if let Ok(stat) = self.vfs.stat(&from, FollowLinks::No) {
                let placed = Placed {
                    id,
                    entry: &view_of_entry(&stat),
                    source: from,
                    target: target.clone(),
                    found: found.clone(),
                    depth,
                    side: None,
                };
                self.create_dir(placed, tr);
            }
            return;
        }
        for _ in level + 1..source.names.len() {
            id = read
                .node(id)
                .and_then(|view| view.parent)
                .unwrap_or(NodeId::ROOT);
        }
        let Some(view) = read.node(id) else { return };
        let (Some(entry), Some(from)) = (view.side(tr.from).copied(), read.spec(id, tr.from))
        else {
            return;
        };
        let placed = Placed {
            id,
            entry: &entry,
            source: from,
            target: target.clone(),
            found: found.clone(),
            depth,
            side: None,
        };
        self.create_dir(placed, tr);
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
        let (Some(entry), Some(from)) = (view.side(tr.from).copied(), read.spec(id, tr.from))
        else {
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
                // At its row's depth, so the folder it is in stays when it moves.
                let depth = self.child_depth(view.parent);
                self.fail_row(
                    id,
                    Action::Copy,
                    kind,
                    Some(from),
                    root.clone(),
                    depth,
                    report(&e),
                    None,
                );
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
                // A file into the archive it is: a copy would rebuild it around itself.
                ops::check_move_inside(self.vfs, self.vfs, &from, &target)?;
                if tr.moving {
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
                self.fail_row(
                    id,
                    action,
                    kind,
                    Some(from),
                    target,
                    depth,
                    report(&e),
                    None,
                );
                self.cover(id);
                return Ok(());
            }
        }
        let bit = if view.is_folder() {
            (Capabilities::MKDIR, "mkdir")
        } else {
            (Capabilities::WRITE, "write")
        };
        let opened = view.flags.contains(NodeFlags::archive_root(tr.from))
            && source.row.whole.contains(SideSet::of(tr.from));
        let leaves = match (&from, opened) {
            (Spec::ArchiveMember { archive, .. }, true) => (**archive).clone(),
            _ => from.clone(),
        };
        let gone = tr.moving && self.lacks(&leaves, Capabilities::REMOVE, "remove", Some(tr.from));
        if gone || self.lacks(&target, bit.0, bit.1, None) {
            let reason = SkipReason::Capability;
            self.skip_row(id, kind, Some(from), target, depth, reason, None);
            self.cover(id);
            return Ok(());
        }
        // An opened archive that goes whole goes as the file it is.
        if let (true, Spec::ArchiveMember { archive, .. }) = (opened, &from)
            && (tr.moving || flat)
        {
            let key = target.canonical();
            let duplicate = !self.taken.insert(key);
            if duplicate {
                let reason = SkipReason::DuplicateTarget;
                self.skip_row(id, kind, Some(from), target, depth, reason, None);
            } else {
                self.place_archive(tr, id, (**archive).clone(), target, depth, None);
            }
            self.cover(id);
            return Ok(());
        }
        if view.is_folder() && !flat {
            return self.copy_to_folder(
                tr, source, &view, &entry, from, target, depth, under_new, below, open,
            );
        }
        if view.is_folder() && view.scan == ScanStatus::NotScanned {
            // Flat, and the model never listed it: rule 6 by name.
            let walk = Walk { tr, id, side: None };
            self.expand_flat(walk, &entry, &from, root, fold, depth)?;
            self.cover(id);
            return Ok(());
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
        // A second source of one path: a file, or a folder this plan makes there.
        if self.is_made(&target) || !self.taken.insert(key) {
            let reason = SkipReason::DuplicateTarget;
            self.skip_row(id, kind, Some(from), target, depth, reason, None);
            return Ok(());
        }
        let found = if under_new {
            None
        } else {
            match lstat(self.vfs, &target) {
                Ok(found) => found.as_ref().map(found_of_stat),
                Err(e) => {
                    self.fail_row(
                        id,
                        Action::Copy,
                        kind,
                        Some(from),
                        target,
                        depth,
                        report(&e),
                        None,
                    );
                    return Ok(());
                }
            }
        };
        match self.kind_action(view.kind, &from, &target, None, tr.moving) {
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
                self.place_file(placed, view.kind, action, tr);
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
        let key = key_of(&target.canonical(), self.folds(&target));
        if self.taken.contains(&key) {
            let reason = SkipReason::DuplicateTarget;
            self.skip_row(id, EntryKind::Dir, Some(from), target, depth, reason, None);
            self.cover(id);
            return Ok(());
        }
        let made = self.made_here(&target);
        let found = if under_new || made.is_some() {
            None
        } else {
            lstat(self.vfs, &target).map_err(|e| FolderError::InvalidRules(e.to_string()))?
        };
        if view.scan == ScanStatus::NotScanned {
            let walk = Walk { tr, id, side: None };
            let found = made.or_else(|| found.as_ref().map(found_of_stat));
            self.expand_unlisted(walk, entry, &from, &target, found, depth)?;
            self.cover(id);
            return Ok(());
        }
        let whole = source.row.whole.contains(SideSet::of(tr.from));
        // An opened archive that goes whole goes as the file it is.
        if let (true, true, None, Spec::ArchiveMember { archive, .. }) = (
            view.flags.contains(NodeFlags::archive_root(tr.from)),
            whole,
            &found,
            &from,
        ) {
            self.place_archive(tr, id, (**archive).clone(), target, depth, None);
            self.cover(id);
            return Ok(());
        }
        if tr.moving && whole && found.is_none() && made.is_none() && self.can_rename(&from, &target)
        {
            self.claim_rows(id, tr.from, &target);
            self.push_folder_rename(id, from, target, depth, None);
            self.cover(id);
            return Ok(());
        }
        let key = target.canonical();
        match found {
            _ if made.is_some() => {
                below.folders.entry(key).or_insert(None);
            }
            Some(stat) if stat.kind == EntryKind::Dir => {
                below.folders.entry(key).or_insert(None);
            }
            _ => {
                let found = found.as_ref().map(found_of_stat);
                if let Entry::Vacant(slot) = below.folders.entry(key.clone()) {
                    slot.insert(Some(depth));
                    let placed = Placed {
                        id,
                        entry,
                        source: from.clone(),
                        target,
                        found,
                        depth,
                        side: None,
                    };
                    let first = self.items.len();
                    self.create_dir(placed, tr);
                    self.depths.insert(id, depth);
                    // A folder that cannot be made is no folder for the next source: it meets the same refusal.
                    if self.items.get(first).is_some_and(|item| item.error.is_some()) {
                        below.folders.remove(&key);
                    }
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
}
