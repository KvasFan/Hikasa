//! Rule 6 of §1.6.10: a folder the model has no contents for is listed here, once, at planning, through the filters the
//! comparison used. From then on it is what a scanned row is: the same overwrite policy, stamps and conflicts, so a plan
//! asks for it what it asks for a row, and execution holds it to what was seen. What the filters hide stays, and a folder
//! goes as one `rename`, `trash` or `remove_dir` only when nothing below it stays.

use hikasa_core::{ErrorReport, FolderVerdict, RelPath, Side, Spec};
use hikasa_vfs::ops::{self, Action, Expect, ItemStamp, PlanItem, SkipReason};
use hikasa_vfs::{Entry, EntryKind, FollowLinks, ListOptions, VfsError};

use super::Recycle;
use super::build::{
    Builder, Found, Open, Placed, Transfer, found_of_stat, key_of, report, stamp_of_stat,
    view_of_entry,
};
use super::conflict::Conflicts;
use crate::align::{ChildKey, exact_bytes, kind_rank};
use crate::error::FolderError;
use crate::model::{EntryView, NodeId, NodeKind};
use crate::resolve::{ResolvedRules, path_verdict};

/// One folder as a listing at planning saw it, with what the filters left of it.
pub(super) struct Listed {
    /// The folder.
    spec: Spec,
    /// What it is.
    entry: Entry,
    /// What the filters leave, by name.
    children: Vec<Listed>,
    /// Its listing failed: nothing below it is known.
    error: Option<ErrorReport>,
    /// Entries the listing could not read.
    failures: Vec<(Spec, ErrorReport)>,
    /// Its path and verdict, for the filters of what stands on the side written.
    place: Option<(RelPath, FolderVerdict)>,
    /// Nothing below it stays: no filter hid anything, and all of it could be read.
    whole: bool,
    /// What removing all of it needs asked.
    inside: Conflicts,
}

/// One expansion of a copy or move: the row it stands for, and the side of its target.
#[derive(Clone, Copy)]
pub(super) struct Walk {
    pub(super) tr: Transfer,
    pub(super) id: NodeId,
    pub(super) side: Option<Side>,
}

/// One expansion of a delete.
#[derive(Clone, Copy)]
struct Removal {
    id: NodeId,
    side: Side,
    recycle: Recycle,
    /// The spec can go to the bin.
    binned: bool,
}

/// The kind of a row for a kind of entry.
fn node_kind(kind: EntryKind) -> NodeKind {
    match kind {
        EntryKind::File => NodeKind::File,
        EntryKind::Dir => NodeKind::Folder,
        EntryKind::Symlink => NodeKind::Symlink,
        EntryKind::Other => NodeKind::Other,
    }
}

/// An item that removes what a listing saw, with its stamp.
fn removal_of(spec: Spec, entry: &Entry, action: Action, depth: u32) -> PlanItem {
    let mut item = PlanItem::planned(action, entry.kind, None, spec, depth);
    item.expect = Some(Expect {
        source: None,
        target: Some(stamp_of_stat(entry)),
    });
    item
}

/// `folder`, for good: its contents stamped, then the folder, post-order. What `--recycle-fallback` may delete.
fn permanent(folder: &Listed, depth: u32, out: &mut Vec<PlanItem>) {
    for child in &folder.children {
        if child.entry.kind == EntryKind::Dir {
            permanent(child, depth + 1, out);
        } else {
            let item = removal_of(child.spec.clone(), &child.entry, Action::Remove, depth + 1);
            out.push(item);
        }
    }
    let mut item = removal_of(folder.spec.clone(), &folder.entry, Action::RemoveDir, depth);
    item.set_seen(folder.entry.file_id);
    out.push(item);
}

impl Builder<'_, '_> {
    /// What the filters of the comparison say, unless `--include-hidden` takes all.
    fn filters(&self) -> Option<&ResolvedRules> {
        if self.include.hidden {
            return None;
        }
        self.read.store().resolved.as_ref()
    }

    /// The filter's path and verdict of the row `id`.
    fn place_of(&self, id: NodeId) -> Option<(RelPath, FolderVerdict)> {
        let path = self.read.path(id);
        self.filters()
            .map(|rules| path_verdict(rules, path.as_str()))
    }

    /// True when a filter hides `entry` in the folder at `place`.
    fn hides(&self, place: Option<&(RelPath, FolderVerdict)>, entry: &Entry) -> bool {
        let (Some(rules), Some((rel, verdict))) = (self.filters(), place) else {
            return false;
        };
        !rules.names_accept(rel, *verdict, entry.name.as_str(), entry.kind)
            || rules.other_filtered(
                entry.kind,
                entry.size,
                entry.mtime,
                &entry.attrs,
                entry.special,
            )
    }

    /// Lists `spec`, all that is below it. `Err` only for a stop.
    fn listed(
        &self,
        spec: Spec,
        entry: Entry,
        place: Option<(RelPath, FolderVerdict)>,
    ) -> Result<Listed, FolderError> {
        self.ctl.check()?;
        ops::scanning(self.ctl, 0, &spec);
        let mut folder = Listed {
            spec,
            entry,
            children: Vec::new(),
            error: None,
            failures: Vec::new(),
            place,
            whole: true,
            inside: Conflicts::empty(),
        };
        let options = ListOptions {
            follow: FollowLinks::No,
            prefilter: None,
        };
        let mut listing = match self.vfs.list(&folder.spec, &options, self.ctl.cancel) {
            Ok(listing) => listing,
            Err(e) if e.is_cancelled() => return Err(FolderError::Cancelled),
            Err(e) => {
                folder.error = Some(report(&e));
                folder.whole = false;
                return Ok(folder);
            }
        };
        // In the order of a comparison's rows, so that the first of two sources of one target is the same one as for rows.
        listing.entries.sort_by_cached_key(|entry| {
            let exact = exact_bytes(&entry.name);
            ChildKey::new(kind_rank(entry.kind), entry.name.as_str(), &exact, "", None)
        });
        for failed in &listing.errors {
            let child = self
                .vfs
                .child(&folder.spec, &failed.name)
                .unwrap_or_else(|_| folder.spec.clone());
            folder.failures.push((child, failed.error.clone()));
            folder.whole = false;
        }
        for entry in listing.entries {
            if self.hides(folder.place.as_ref(), &entry) {
                folder.whole = false;
                continue;
            }
            let spec = match self.vfs.child(&folder.spec, &entry.name) {
                Ok(spec) => spec,
                Err(e) => {
                    folder.failures.push((folder.spec.clone(), report(&e)));
                    folder.whole = false;
                    continue;
                }
            };
            folder.inside |= Conflicts::of_attrs(&entry.attrs);
            if entry.kind != EntryKind::Dir {
                folder.children.push(Listed {
                    spec,
                    entry,
                    children: Vec::new(),
                    error: None,
                    failures: Vec::new(),
                    place: None,
                    whole: true,
                    inside: Conflicts::empty(),
                });
                continue;
            }
            let place = match (self.filters(), &folder.place) {
                (Some(rules), Some((rel, verdict))) => {
                    let name = entry.name.as_str();
                    let below = rules.folder_verdict(rel, name, *verdict);
                    Some((rel.join(name).unwrap_or_else(|_| rel.clone()), below))
                }
                _ => None,
            };
            let child = self.listed(spec, entry, place)?;
            folder.whole &= child.whole;
            folder.inside |= child.inside;
            folder.children.push(child);
        }
        Ok(folder)
    }

    /// Items `first..` lie below the row they came from: their path is `path`, and no row names them.
    fn relabel(&mut self, first: usize, path: &RelPath) {
        for meta in self.meta.iter_mut().skip(first) {
            meta.node = None;
            meta.path = path.as_str().to_owned();
        }
    }

    /// The identity a listing saw of the folder of the last item: execution holds it to that (D-94 rule 3).
    fn remember_seen(&mut self, entry: &Entry) {
        if let Some(item) = self.items.last_mut() {
            item.set_seen(entry.file_id);
        }
    }

    /// Rule 6 of a copy or a move: `source`, onto `target` with `found` standing there, at `depth`. One `rename` where nothing
    /// below it stays and rule 7 allows it, else item by item.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn expand_unlisted(
        &mut self,
        walk: Walk,
        entry: &EntryView<'_>,
        source: &Spec,
        target: &Spec,
        found: Option<Found>,
        depth: u32,
    ) -> Result<(), FolderError> {
        let Walk { tr, id, side } = walk;
        let follow = if entry.followed {
            FollowLinks::Yes
        } else {
            FollowLinks::No
        };
        let first = self.items.len();
        let stat = self.vfs.stat(source, follow).and_then(|stat| {
            ops::check_nesting(self.vfs, self.vfs, source, &stat, target).map(|()| stat)
        });
        let stat = match stat {
            Ok(stat) => stat,
            Err(e) if e.is_cancelled() => return Err(FolderError::Cancelled),
            Err(e) => {
                self.fail_row(
                    id,
                    Action::CreateDir,
                    EntryKind::Dir,
                    Some(source.clone()),
                    target.clone(),
                    depth,
                    report(&e),
                    side,
                );
                return Ok(());
            }
        };
        let tree = self.listed(source.clone(), stat, self.place_of(id))?;
        if tr.moving && tree.whole && found.is_none() && self.can_rename(source, target) {
            if self.outside.is_some() {
                self.claim_listed(&tree, target);
            }
            self.push_folder_rename(id, source.clone(), target.clone(), depth, side);
            return Ok(());
        }
        let path = self.read.path(id);
        self.transfer_listed(walk, &tree, target, found, depth, &path, entry.followed)?;
        if let Some(meta) = self.meta.get_mut(first) {
            meta.node = Some(id);
        }
        Ok(())
    }

    /// A folder of an expansion, its contents, and a source folder's `remove_dir` when nothing of it stays.
    #[allow(clippy::too_many_arguments)]
    fn transfer_listed(
        &mut self,
        walk: Walk,
        folder: &Listed,
        target: &Spec,
        found: Option<Found>,
        depth: u32,
        path: &RelPath,
        followed: bool,
    ) -> Result<(), FolderError> {
        self.ctl.check()?;
        let Walk { tr, id, side } = walk;
        let first = self.items.len();
        if let Some(error) = &folder.error {
            self.fail_row(
                id,
                Action::CreateDir,
                EntryKind::Dir,
                Some(folder.spec.clone()),
                target.clone(),
                depth,
                error.clone(),
                side,
            );
            self.relabel(first, path);
            return Ok(());
        }
        let key = key_of(&target.canonical(), self.folds(target));
        if self.outside.is_some() && self.taken.contains(&key) {
            let source = Some(folder.spec.clone());
            let reason = SkipReason::DuplicateTarget;
            self.skip_row(
                id,
                EntryKind::Dir,
                source,
                target.clone(),
                depth,
                reason,
                side,
            );
            self.relabel(first, path);
            return Ok(());
        }
        let stands = found.as_ref().is_some_and(|it| it.kind == EntryKind::Dir);
        let mut shown = view_of_entry(&folder.entry);
        shown.followed = followed;
        if stands {
            let mut item = PlanItem::planned(
                Action::KeepDir,
                EntryKind::Dir,
                Some(folder.spec.clone()),
                target.clone(),
                depth,
            );
            item.follow_source = followed;
            item.expect = Some(Expect {
                source: Some(ItemStamp {
                    kind: EntryKind::Dir,
                    size: None,
                    mtime: None,
                }),
                target: None,
            });
            let meta = self.meta_of(id, side);
            self.push(item, meta);
        } else {
            let placed = Placed {
                id,
                entry: &shown,
                source: folder.spec.clone(),
                target: target.clone(),
                found,
                depth,
                side,
            };
            self.create_dir(placed, tr);
        }
        self.remember_seen(&folder.entry);
        self.relabel(first, path);
        // A file in its place the policy keeps: nothing below it can go.
        if self.items.get(first).is_some_and(|it| it.error.is_some()) {
            return Ok(());
        }
        for (spec, error) in &folder.failures {
            let first = self.items.len();
            let action = if tr.moving {
                Action::Move
            } else {
                Action::Copy
            };
            self.fail_row(
                id,
                action,
                EntryKind::Other,
                Some(spec.clone()),
                target.clone(),
                depth + 1,
                error.clone(),
                side,
            );
            self.relabel(first, path);
        }
        for child in &folder.children {
            let below = path
                .join(child.entry.name.as_str())
                .unwrap_or_else(|_| path.clone());
            let child_target = self.vfs.child(target, &child.entry.name).and_then(|spec| {
                let stood = if stands && self.made_here(&spec).is_none() {
                    self.stood_at(&spec)?
                } else {
                    None
                };
                Ok((spec, stood))
            });
            let (spec, stood) = match child_target {
                Ok(pair) => pair,
                Err(e) => {
                    let first = self.items.len();
                    let action = if tr.moving {
                        Action::Move
                    } else {
                        Action::Copy
                    };
                    self.fail_row(
                        id,
                        action,
                        child.entry.kind,
                        Some(child.spec.clone()),
                        target.clone(),
                        depth + 1,
                        report(&e),
                        side,
                    );
                    self.relabel(first, &below);
                    continue;
                }
            };
            if child.entry.kind == EntryKind::Dir {
                // A filter that hides what stands there keeps it, as `skip filtered` keeps a row.
                let found = self.made_here(&spec).or_else(|| {
                    stood.as_ref().map(|entry| Found {
                        filtered: side.is_some() && self.hides(folder.place.as_ref(), entry),
                        ..found_of_stat(entry)
                    })
                });
                self.transfer_listed(walk, child, &spec, found, depth + 1, &below, false)?;
            } else {
                self.transfer_entry(walk, folder, child, spec, stood, depth + 1, &below);
            }
        }
        if tr.moving && folder.whole {
            let first = self.items.len();
            self.remove_dir(Open {
                level: 0,
                id,
                spec: folder.spec.clone(),
                depth,
                side: tr.from,
                conflicts: Conflicts::empty(),
            });
            self.remember_seen(&folder.entry);
            self.relabel(first, path);
        }
        Ok(())
    }

    /// Rule 6 of `structure none`: each file below `source` by its name in `root`, the first of a name only, and a source
    /// folder's `remove_dir` when nothing of it stays.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn expand_flat(
        &mut self,
        walk: Walk,
        entry: &EntryView<'_>,
        source: &Spec,
        root: &Spec,
        fold: bool,
        depth: u32,
    ) -> Result<(), FolderError> {
        let follow = if entry.followed {
            FollowLinks::Yes
        } else {
            FollowLinks::No
        };
        let first = self.items.len();
        let stat = match self.vfs.stat(source, follow) {
            Ok(stat) => stat,
            Err(e) if e.is_cancelled() => return Err(FolderError::Cancelled),
            Err(e) => {
                let kind = EntryKind::Dir;
                let (source, root) = (Some(source.clone()), root.clone());
                self.fail_row(
                    walk.id,
                    Action::Copy,
                    kind,
                    source,
                    root,
                    depth,
                    report(&e),
                    None,
                );
                return Ok(());
            }
        };
        let tree = self.listed(source.clone(), stat, self.place_of(walk.id))?;
        let path = self.read.path(walk.id);
        self.flat_listed(walk, &tree, root, fold, depth, &path)?;
        if let Some(meta) = self.meta.get_mut(first) {
            meta.node = Some(walk.id);
        }
        Ok(())
    }

    /// One folder of [`Builder::expand_flat`].
    #[allow(clippy::too_many_arguments)]
    fn flat_listed(
        &mut self,
        walk: Walk,
        folder: &Listed,
        root: &Spec,
        fold: bool,
        depth: u32,
        path: &RelPath,
    ) -> Result<(), FolderError> {
        self.ctl.check()?;
        let Walk { tr, id, side } = walk;
        let action = if tr.moving {
            Action::Move
        } else {
            Action::Copy
        };
        if let Some(error) = &folder.error {
            let first = self.items.len();
            let source = Some(folder.spec.clone());
            let kind = EntryKind::Dir;
            self.fail_row(
                id,
                action,
                kind,
                source,
                root.clone(),
                depth,
                error.clone(),
                side,
            );
            self.relabel(first, path);
            return Ok(());
        }
        for (spec, error) in &folder.failures {
            let first = self.items.len();
            let source = Some(spec.clone());
            let kind = EntryKind::Other;
            self.fail_row(
                id,
                action,
                kind,
                source,
                root.clone(),
                depth + 1,
                error.clone(),
                side,
            );
            self.relabel(first, path);
        }
        for child in &folder.children {
            let below = path
                .join(child.entry.name.as_str())
                .unwrap_or_else(|_| path.clone());
            if child.entry.kind == EntryKind::Dir {
                self.flat_listed(walk, child, root, fold, depth + 1, &below)?;
                continue;
            }
            let first = self.items.len();
            let looked = self.vfs.child(root, &child.entry.name).and_then(|target| {
                let stood = self.stood_at(&target)?;
                Ok((target, stood))
            });
            match looked {
                Ok((target, stood)) => {
                    let key = key_of(&target.canonical(), fold);
                    if !self.taken.contains(&key) && !self.holds_folder(&target) {
                        self.transfer_entry(walk, folder, child, target, stood, depth + 1, &below);
                    } else {
                        let source = Some(child.spec.clone());
                        let reason = SkipReason::DuplicateTarget;
                        let kind = child.entry.kind;
                        self.skip_row(id, kind, source, target, depth + 1, reason, side);
                        self.relabel(first, &below);
                    }
                }
                Err(e) => {
                    let source = Some(child.spec.clone());
                    let kind = child.entry.kind;
                    self.fail_row(
                        id,
                        action,
                        kind,
                        source,
                        root.clone(),
                        depth + 1,
                        report(&e),
                        side,
                    );
                    self.relabel(first, &below);
                }
            }
        }
        if tr.moving && folder.whole {
            let first = self.items.len();
            self.remove_dir(Open {
                level: 0,
                id,
                spec: folder.spec.clone(),
                depth,
                side: tr.from,
                conflicts: Conflicts::empty(),
            });
            self.remember_seen(&folder.entry);
            self.relabel(first, path);
        }
        Ok(())
    }

    /// What a folder renamed whole takes along is at its target: a second source of one of those paths stays.
    fn claim_listed(&mut self, folder: &Listed, target: &Spec) {
        for child in &folder.children {
            let Ok(at) = self.vfs.child(target, &child.entry.name) else {
                continue;
            };
            if child.entry.kind == EntryKind::Dir {
                self.note_made(&at, 0);
                self.claim_listed(child, &at);
            } else {
                let key = key_of(&at.canonical(), self.folds(&at));
                self.taken.insert(key);
            }
        }
    }

    /// What stands at `spec`, unfollowed.
    fn stood_at(&self, spec: &Spec) -> Result<Option<Entry>, VfsError> {
        match self.vfs.stat(spec, FollowLinks::No) {
            Ok(entry) => Ok(Some(entry)),
            // A file in the place of a folder the plan makes: nothing stands below it.
            Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A file, link or special file of an expansion: rule 5, then what a row gets. A filter that hides what stands on the
    /// side written keeps it, as it keeps a row (`skip filtered`).
    #[allow(clippy::too_many_arguments)]
    fn transfer_entry(
        &mut self,
        walk: Walk,
        folder: &Listed,
        child: &Listed,
        target: Spec,
        stood: Option<Entry>,
        depth: u32,
        path: &RelPath,
    ) {
        let Walk { tr, id, side } = walk;
        let first = self.items.len();
        let kind = child.entry.kind;
        let source = child.spec.clone();
        let hidden = side.is_some()
            && stood
                .as_ref()
                .is_some_and(|entry| self.hides(folder.place.as_ref(), entry));
        if hidden {
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::Filtered,
                side,
            );
            return self.relabel(first, path);
        }
        // `copyto`, `moveto`: a second source of one target path stays (rule 16).
        if self.outside.is_some() {
            let key = key_of(&target.canonical(), self.folds(&target));
            if self.holds_folder(&target) || !self.taken.insert(key) {
                let reason = SkipReason::DuplicateTarget;
                self.skip_row(id, kind, Some(source), target, depth, reason, side);
                return self.relabel(first, path);
            }
        }
        match self.kind_action(node_kind(kind), &source, &target, side, tr.moving) {
            Ok(action) => {
                let shown = view_of_entry(&child.entry);
                let placed = Placed {
                    id,
                    entry: &shown,
                    source,
                    target,
                    found: stood.as_ref().map(found_of_stat),
                    depth,
                    side,
                };
                self.place_file(placed, node_kind(kind), action, tr);
            }
            Err(reason) => self.skip_row(id, kind, Some(source), target, depth, reason, side),
        }
        self.relabel(first, path);
    }

    /// Rule 6 of a delete: post-order, by §1.6.12's table, from a listing.
    pub(super) fn delete_unlisted(
        &mut self,
        side: Side,
        id: NodeId,
        spec: &Spec,
        depth: u32,
        recycle: Recycle,
    ) -> Result<(), FolderError> {
        let first = self.items.len();
        let stat = match self.vfs.stat(spec, FollowLinks::No) {
            Ok(stat) => stat,
            Err(e) if e.is_cancelled() => return Err(FolderError::Cancelled),
            Err(e) => {
                self.fail_row(
                    id,
                    Action::RemoveDir,
                    EntryKind::Dir,
                    None,
                    spec.clone(),
                    depth,
                    report(&e),
                    Some(side),
                );
                return Ok(());
            }
        };
        let tree = self.listed(spec.clone(), stat, self.place_of(id))?;
        let removal = Removal {
            id,
            side,
            recycle,
            binned: recycle.use_bin && self.has_bin(spec),
        };
        let path = self.read.path(id);
        self.delete_listed(removal, &tree, depth, &path)?;
        if let Some(meta) = self.meta.get_mut(first) {
            meta.node = Some(id);
        }
        Ok(())
    }

    /// A folder of a delete: one `trash` when nothing below it stays, else item by item; its `remove_dir` only then.
    fn delete_listed(
        &mut self,
        removal: Removal,
        folder: &Listed,
        depth: u32,
        path: &RelPath,
    ) -> Result<(), FolderError> {
        self.ctl.check()?;
        let Removal {
            id,
            side,
            recycle,
            binned,
        } = removal;
        let first = self.items.len();
        if let Some(error) = &folder.error {
            self.fail_row(
                id,
                Action::RemoveDir,
                EntryKind::Dir,
                None,
                folder.spec.clone(),
                depth,
                error.clone(),
                Some(side),
            );
            self.relabel(first, path);
            return Ok(());
        }
        if binned && folder.whole {
            let mut item = removal_of(folder.spec.clone(), &folder.entry, Action::Trash, depth);
            item.trash_fallback = recycle.fallback;
            // What the listing saw is checked right before the trash, and is all the fallback may delete.
            permanent(folder, 0, &mut item.fallback);
            item.verify_below = true;
            let mut meta = self.meta_of(id, Some(side));
            meta.conflicts = Conflicts::of_attrs(&folder.entry.attrs) | folder.inside;
            self.push(item, meta);
            self.relabel(first, path);
            return Ok(());
        }
        for (spec, error) in &folder.failures {
            let first = self.items.len();
            self.fail_row(
                id,
                Action::Remove,
                EntryKind::Other,
                None,
                spec.clone(),
                depth + 1,
                error.clone(),
                Some(side),
            );
            self.relabel(first, path);
        }
        for child in &folder.children {
            let below = path
                .join(child.entry.name.as_str())
                .unwrap_or_else(|_| path.clone());
            if child.entry.kind == EntryKind::Dir {
                self.delete_listed(removal, child, depth + 1, &below)?;
                continue;
            }
            let first = self.items.len();
            let action = if binned {
                Action::Trash
            } else {
                Action::Remove
            };
            let mut item = removal_of(child.spec.clone(), &child.entry, action, depth + 1);
            item.trash_fallback = binned && recycle.fallback;
            let mut meta = self.meta_of(id, Some(side));
            meta.conflicts = Conflicts::of_attrs(&child.entry.attrs);
            meta.conflicts
                .set(Conflicts::NO_RECYCLE_BIN, recycle.use_bin && !binned);
            self.push(item, meta);
            self.relabel(first, &below);
        }
        if folder.whole {
            let first = self.items.len();
            let mut conflicts = Conflicts::of_attrs(&folder.entry.attrs);
            conflicts.set(Conflicts::NO_RECYCLE_BIN, recycle.use_bin);
            self.remove_dir(Open {
                level: 0,
                id,
                spec: folder.spec.clone(),
                depth,
                side,
                conflicts,
            });
            self.remember_seen(&folder.entry);
            self.relabel(first, path);
        }
        Ok(())
    }
}
