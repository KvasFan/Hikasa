//! Subtree refresh (§1.6.7, D-173): re-reads **one folder level** and only descends where a row changed, on the `Scan`
//! of `scan.rs`, so a refresh and a scan cannot disagree.
//!
//! Rows are matched by `(kind, left raw name, right raw name)`. A row whose facts are equal keeps its node, content
//! result, CRC and subtree; a folder of the same presence keeps its children, unlisted (a folder's own time says nothing,
//! §3). Anything else is a new record with the scan's own tasks. The same rows in the same order are rewritten in place
//! and every id stays; otherwise the folder's block moves to the tail of the arena and `renumbered` says how.

use std::collections::HashMap;
use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::RelPath;
use hikasa_vfs::{Entry, FollowLinks};

use super::{
    Items, Listed, NO, NodeFlags, NodeId, NodeKind, Planned, Scan, ScanStart, SideFolder, Task,
    side_of, stage,
};
use crate::error::FolderError;
use crate::model::store::{IgnoredMarks, Node, Row, SideRec, Store, bits, internal, path_key};
use crate::model::{FolderModel, ScanStatus};
use crate::resolve::path_verdict;
use crate::scan::FolderEnv;

/// What a refresh did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RefreshReport {
    /// Folders listed.
    pub folders: u32,
    /// Rows that are new, changed or gone.
    pub rows_changed: u32,
    /// Content tests scheduled.
    pub content_tests: u32,
    /// Ids that changed, `(old, new)`, in the order they did: only ids of the refreshed folders' own blocks.
    pub renumbered: Vec<(NodeId, NodeId)>,
    /// False when cancelled: what was not reached stays not scanned, and the verdict `incomplete`.
    pub complete: bool,
}

/// `(kind, left raw name, right raw name)`.
type RowKey = (NodeKind, Option<OsString>, Option<OsString>);

/// Refreshes the folders `nodes` (each a `Folder` node; file nodes count as their parent), on `env`'s pool. A second call
/// while one runs, or a model still scanning, → [`FolderError::RefreshBusy`]. In a flat comparison →
/// [`FolderError::RefreshFlat`]: the caller re-runs. Each folder is refreshed once, one level, parents before children.
pub fn refresh_into(
    model: &Arc<FolderModel>,
    nodes: &[NodeId],
    env: &FolderEnv<'_>,
) -> Result<RefreshReport, FolderError> {
    if model.rules().handling.flatten {
        return Err(FolderError::RefreshFlat);
    }
    let _claim = model.claim_refresh()?;
    let scan = Scan::for_model(env, model)?;
    let mut order = folders_of(model, nodes);
    let mut report = RefreshReport {
        complete: true,
        ..RefreshReport::default()
    };
    let mut moved: HashMap<NodeId, NodeId> = HashMap::new();
    while let Some(node) = order.pop() {
        let node = moved.get(&node).copied().unwrap_or(node);
        if scan.cancelled() {
            report.complete = false;
            break;
        }
        // A folder an earlier refresh of this call dropped, or one no scan would list.
        let Some((sides, depth)) = scan.sides_of(node) else {
            continue;
        };
        let listings = scan.list_pair(&sides);
        let Some(listed) = scan.collect(&sides, listings) else {
            report.complete = false;
            break;
        };
        report.folders += 1;
        let Some(tasks) = scan.replace(node, depth, &sides, listed, &mut report) else {
            report.complete = false;
            break;
        };
        for &(old, new) in &report.renumbered {
            moved.insert(old, new);
        }
        scan.execute(tasks);
    }
    scan.flush();
    scan.final_progress();
    let complete = report.complete && !scan.aborted.load(std::sync::atomic::Ordering::Acquire);
    report.complete = complete;
    let back_end = env.vfs.take_warnings();
    model.mutate(|s| {
        for w in back_end {
            s.warn(w);
        }
        let whole = s.complete && complete;
        s.finish(whole);
    });
    Ok(report)
}

/// The folders of `nodes`, without duplicates, the deepest first in the list: it is popped from the end.
fn folders_of(model: &FolderModel, nodes: &[NodeId]) -> Vec<NodeId> {
    let read = model.read();
    let mut folders: Vec<(usize, NodeId)> = nodes
        .iter()
        .filter_map(|&id| {
            let view = read.node(id)?;
            let folder = if view.is_folder() {
                id
            } else {
                view.parent.unwrap_or(NodeId::ROOT)
            };
            Some((read.ancestors(folder).len(), folder))
        })
        .collect();
    folders.sort_by(|a, b| b.cmp(a));
    folders.dedup();
    folders.into_iter().map(|(_, id)| id).collect()
}

/// True when the old side record and the listed entry state the same facts.
fn same_side(store: &Store, id: u32, side: usize, rec: &SideRec, entry: &Entry) -> bool {
    let link = store.link_target(side, id).map(|l| l.text.as_str());
    rec.size() == entry.size
        && rec.mtime() == entry.mtime
        && rec.attrs() == entry.attrs
        && rec.special() == entry.special
        && link == entry.link_target.as_deref()
}

impl<'a> Scan<'a> {
    /// The `Scan` of a finished model, from its rules and `Store::resolved`.
    pub(crate) fn for_model(
        env: &'a FolderEnv<'a>,
        model: &'a FolderModel,
    ) -> Result<Scan<'a>, FolderError> {
        let rules = model.rules();
        let read = model.read();
        let store = read.store();
        let bases = read.bases();
        let (Some(left), Some(right), Some(resolved), Some(align)) =
            (bases.left, bases.right, store.resolved.clone(), store.align)
        else {
            return Err(FolderError::RefreshBusy);
        };
        let caps = [left.capabilities, right.capabilities];
        let opened = rules.open_archives.iter().chain(&store.opened);
        let start = ScanStart {
            caps,
            resolved,
            align,
            follow: if rules.handling.follow_symlinks {
                FollowLinks::Yes
            } else {
                FollowLinks::No
            },
            open_archives: opened
                .map(|path| path_key(&align, path).into_owned())
                .collect(),
            ignored: IgnoredMarks::clone(&store.ignored),
            archive_entries: 0,
        };
        drop(read);
        Ok(Scan::new(env, model, start))
    }

    /// The two sides of the folder `id` as a scan would list them, and its depth; `None` for a folder no scan lists: it
    /// is gone, hidden on every side, or known by name only.
    fn sides_of(&self, id: NodeId) -> Option<([Option<SideFolder>; 2], u32)> {
        let read = self.model.read();
        let view = read.node(id)?;
        if !view.is_folder() {
            return None;
        }
        if id == NodeId::ROOT {
            let bases = read.bases();
            let side = |base: Option<&crate::model::BaseInfo>| {
                base.map(|b| SideFolder::root(b.spec.clone(), &b.entry, self.follow))
            };
            return Some(([side(bases.left), side(bases.right)], 0));
        }
        let entries = [view.left, view.right];
        if entries.iter().flatten().all(|entry| entry.filtered) {
            return None;
        }
        let depth = u32::try_from(read.ancestors(id).len()).unwrap_or(u32::MAX);
        let mut sides = [None, None];
        let mut chains: [Vec<_>; 2] = [Vec::new(), Vec::new()];
        for (index, entry) in entries.iter().enumerate() {
            let side = side_of(index);
            if entry.is_none() {
                continue;
            }
            let spec = read.spec(id, side)?;
            let rel = self.side_path(&read, id, index);
            let (rel, verdict) = path_verdict(&self.resolved, rel.as_str());
            if verdict == hikasa_core::FolderVerdict::Excluded {
                continue;
            }
            if self.follow == FollowLinks::Yes {
                let mut path: Vec<_> = read.ancestors(id);
                path.reverse();
                path.push(id);
                chains[index] = path.iter().filter_map(|&at| read.spec(at, side)).collect();
            }
            sides[index] = Some(SideFolder {
                spec,
                rel,
                verdict,
                chain: Vec::new(),
            });
        }
        drop(read);
        for (slot, chain) in sides.iter_mut().zip(chains) {
            if let Some(folder) = slot {
                folder.chain = chain
                    .iter()
                    .filter_map(|spec| self.env.vfs.stat(spec, FollowLinks::Yes).ok()?.file_id)
                    .collect();
            }
        }
        Some((sides, depth))
    }

    /// The path of `id` in its side's own names.
    fn side_path(&self, read: &crate::model::TreeRead<'_>, id: NodeId, side: usize) -> RelPath {
        let mut names = Vec::new();
        let mut at = id;
        while at != NodeId::ROOT {
            let Some(view) = read.node(at) else { break };
            let entry = if side == 0 { view.left } else { view.right };
            if let Some(entry) = entry {
                names.push(entry.name.to_owned());
            }
            at = view.parent.unwrap_or(NodeId::ROOT);
        }
        let mut path = RelPath::root();
        for name in names.iter().rev() {
            path = path.join(name).unwrap_or(path);
        }
        path
    }

    /// True when the row `p` states what the old row `id` does, so that the old record stays (§1.6.7, step 4).
    fn is_kept(&self, store: &Store, id: u32, old: &Node, p: &Planned, items: &[Items; 2]) -> bool {
        let lasting = NodeFlags::LEFT_ERROR
            | NodeFlags::RIGHT_ERROR
            | NodeFlags::SYMLINK_LOOP
            | NodeFlags::LEFT_ARCHIVE_ROOT
            | NodeFlags::RIGHT_ARCHIVE_ROOT
            | NodeFlags::IGNORED;
        let partial = internal::PARTIAL_UNKNOWN;
        if (old.flags() & lasting) != (p.flags & lasting)
            || old.internal & partial != p.internal & partial
            || old.internal & internal::CONTENT_PENDING != 0
        {
            return false;
        }
        for (side, rec) in old.sides.iter().enumerate() {
            let Some((entry, f)) = items[side].get(p.sides[side]) else {
                return false;
            };
            let flag = |bit: u8, new: bool| rec.has(bit) == new;
            if !flag(bits::FILTERED, f.filtered)
                || !flag(bits::STAT_SKIPPED, f.stat_skipped)
                || !flag(bits::NO_META, f.no_meta)
            {
                return false;
            }
            if p.kind != NodeKind::Folder {
                if !same_side(store, id, side, rec, entry) {
                    return false;
                }
            } else if f.opened && rec.mtime() != entry.mtime {
                // A new stamp below an opened archive: its index is another one.
                return false;
            }
        }
        if p.kind == NodeKind::Folder {
            // A row a cancelled refresh left, or a failure that has passed.
            let unlisted = old.scan() == ScanStatus::NotScanned && p.descend.is_some();
            let failed = old.scan() == ScanStatus::Failed;
            return !unlisted && failed == (p.scan == ScanStatus::Failed);
        }
        true
    }

    /// One folder's new rows: aligned and planned as in a scan, matched to the old rows, and put in their place. The tasks
    /// of what is new; `None` when cancelled.
    fn replace(
        &self,
        node: NodeId,
        depth: u32,
        sides: &[Option<SideFolder>; 2],
        listed: Listed,
        report: &mut RefreshReport,
    ) -> Option<Vec<Task>> {
        let Listed {
            mut items,
            partial,
            status,
            folder_flags,
            folder_errors,
            mut warnings,
        } = listed;
        let pairs = self
            .pair_items(&items, false, &[Vec::new(), Vec::new()])
            .ok()?;
        let spec_of = |side: usize, entry: &Entry, _dir: Option<u32>| match &sides[side] {
            Some(sf) => self.env.vfs.child(&sf.spec, &entry.name),
            None => Err(hikasa_vfs::VfsError::InvalidName {
                name: entry.name.as_str().to_owned(),
            }),
        };
        let mut planned = self
            .plan(
                &mut items,
                pairs,
                depth,
                partial,
                Some(sides),
                &spec_of,
                &mut warnings,
            )
            .ok()?;
        if !self.ignored.is_empty() {
            self.mark_ignored(node, &items, &mut planned, [&[], &[]]);
        }
        let order = self.canonical_order(&items, &planned, [&[], &[]])?;
        let name_of = |side: usize, p: &Planned| {
            (p.sides[side] != NO)
                .then(|| items[side].get(p.sides[side]))
                .flatten()
                .map(|(entry, _)| entry.name.to_os_string())
        };
        let mut rows = Vec::with_capacity(order.len());
        let old_count;
        {
            let read = self.model.read();
            let store = read.store();
            let old_rows: Vec<u32> = store
                .node(node)
                .map_or_else(Vec::new, |n| n.children().collect());
            old_count = old_rows.len();
            let mut by_key: HashMap<RowKey, u32> = HashMap::with_capacity(old_rows.len());
            for &id in &old_rows {
                let Some(old) = store.node(NodeId(id)) else {
                    continue;
                };
                let name = |side: usize| {
                    let rec = old.sides.get(side)?;
                    rec.present().then(|| store.raw_name(side, id, rec))
                };
                by_key.insert((old.kind(), name(0), name(1)), id);
            }
            for &index in &order {
                let p = planned.get_mut(index as usize)?;
                let key = (p.kind, name_of(0, p), name_of(1, p));
                let old = by_key.remove(&key);
                let kept = old
                    .and_then(|id| Some((id, store.node(NodeId(id))?)))
                    .is_some_and(|(id, record)| self.is_kept(store, id, record, p, &items));
                if kept {
                    p.content = None;
                    p.descend = None;
                }
                rows.push(Row { old, kept });
            }
        }
        let changed = rows.iter().filter(|row| !row.kept).count();
        let matched = rows.iter().filter(|row| row.old.is_some()).count();
        report.rows_changed += u32::try_from(changed + old_count - matched).unwrap_or(u32::MAX);
        let detached = Scan::detach(&items, &mut planned, &order);
        report.content_tests +=
            u32::try_from(detached.iter().filter(|d| d.content.is_some()).count())
                .unwrap_or(u32::MAX);
        let children = Scan::new_nodes(&items, &mut planned, &order);
        let staged = stage(
            node.0,
            self.rules.criteria.content.override_quick,
            order.len(),
            children,
            &|| self.cancelled(),
        )?;
        if self.cancelled() {
            return None;
        }
        let (first, renumbered) = self.model.mutate(|s| {
            for w in warnings.drain(..) {
                s.warn(w);
            }
            s.replace_children(node, status, folder_flags, folder_errors, staged, &rows)
        });
        report.renumbered.extend(renumbered);
        self.folders_done
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(self.tasks_of(detached, first, depth))
    }
}
