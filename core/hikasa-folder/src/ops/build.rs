//! The one builder (§1.6.10, D-177): rows of the model become items of one batch plan. It reads the model; a row the model
//! has no contents for is expanded by `hikasa_vfs::ops` (rule 6). W6c's sync plan is made of the same pieces, so each rule
//! lives in one method here, and what a verb adds is its order of rows.

use std::collections::{BTreeMap, BTreeSet};

use hikasa_core::{
    Ctl, ErrorKind, ErrorReport, FileTime, Pair, RelPath, Side, Spec, Warning, WarningCode,
};
use hikasa_vfs::ops::{
    self, Action, BackupNames, BatchOptions, CopyOptions, ExecOptions, Expect, Fence, ItemStamp,
    MoveOptions, OpKind, OverwritePolicy, Plan, PlanItem, PlanOptions, RemoveOptions, SkipReason,
};
use hikasa_vfs::{
    Attrs, Capabilities, Entry, EntryKind, FileName, FollowLinks, SetTimes, Vfs, VfsError,
};

use super::conflict::Conflicts;
use super::{FolderOp, ItemMeta, OpPlan, OpScope, OpSettings, OpVerb, Recycle, TouchFrom};
use crate::error::FolderError;
use crate::model::{EntryView, NodeFlags, NodeId, NodeKind, NodeView, ScanStatus, TreeRead};
use crate::select::{Include, Resolved, SideSet, resolve};
use crate::state::CompareState;
use crate::time_cmp::{TimeRelation, compare_times};

/// `ancestry` bits: below a folder the comparison followed, per side.
const FOLLOWED_LEFT: u8 = 1;
/// The same on the right.
const FOLLOWED_RIGHT: u8 = 2;
/// Below a folder that failed, on either side.
const FAILED: u8 = 4;
/// The entry is computed.
const KNOWN: u8 = 128;

/// The bit of `side` in the `ancestry` bits.
const fn followed_bit(side: Side) -> u8 {
    match side {
        Side::Right => FOLLOWED_RIGHT,
        Side::Left | Side::Center | Side::Output => FOLLOWED_LEFT,
    }
}

/// The report of a VFS error: kind, message, the spec it names.
pub(super) fn report(error: &VfsError) -> ErrorReport {
    let mut report = ErrorReport::capture(error, None);
    report.spec = error.spec().map(str::to_owned);
    report
}

/// An item error with a kind of its own.
pub(super) fn report_with(kind: ErrorKind, spec: &Spec, message: String) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: Some(spec.canonical()),
        causes: Vec::new(),
    }
}

/// What a row saw on a side. A followed entry reads through its link, so a stamp of the row is the link's own only by kind.
pub(super) fn stamp_of(entry: &EntryView<'_>, as_source: bool) -> ItemStamp {
    if entry.followed && !as_source {
        return ItemStamp {
            kind: EntryKind::Symlink,
            size: None,
            mtime: None,
        };
    }
    match entry.kind {
        EntryKind::File | EntryKind::Symlink => ItemStamp {
            kind: entry.kind,
            size: Some(entry.size),
            mtime: entry.mtime,
        },
        kind => ItemStamp {
            kind,
            size: None,
            mtime: None,
        },
    }
}

/// What a stat saw.
pub(super) fn stamp_of_stat(entry: &Entry) -> ItemStamp {
    match entry.kind {
        EntryKind::File | EntryKind::Symlink => ItemStamp {
            kind: entry.kind,
            size: Some(entry.size),
            mtime: entry.mtime,
        },
        kind => ItemStamp {
            kind,
            size: None,
            mtime: None,
        },
    }
}

/// What stands where an item goes.
#[derive(Debug, Clone)]
pub(super) struct Found {
    /// Its kind (a followed link is a link: a replace acts on the link).
    pub(super) kind: EntryKind,
    /// What execution expects.
    pub(super) stamp: ItemStamp,
    /// Its attributes.
    pub(super) attrs: Attrs,
    /// Its time.
    pub(super) mtime: Option<FileTime>,
    /// The row it is the entry of.
    pub(super) row: Option<NodeId>,
}

/// One direction of a copy or a move.
#[derive(Debug, Clone, Copy)]
pub(super) struct Transfer {
    /// The side read.
    pub(super) from: Side,
    /// The side written.
    pub(super) to: Side,
    /// The source goes.
    pub(super) moving: bool,
    /// What an existing target means.
    pub(super) overwrite: OverwritePolicy,
}

/// A folder whose `remove_dir` comes after its subtree.
#[derive(Debug, Clone)]
pub(super) struct Open {
    /// Its level.
    pub(super) level: u32,
    /// Its row.
    pub(super) id: NodeId,
    /// The folder.
    pub(super) spec: Spec,
    /// Its depth in the plan.
    pub(super) depth: u32,
    /// Its side.
    pub(super) side: Side,
    /// What it needs asked.
    pub(super) conflicts: Conflicts,
}

/// One source and where it goes, for one item.
pub(super) struct Placed<'e> {
    /// The row.
    pub(super) id: NodeId,
    /// The source's entry.
    pub(super) entry: &'e EntryView<'e>,
    /// The source.
    pub(super) source: Spec,
    /// The target.
    pub(super) target: Spec,
    /// What stands there.
    pub(super) found: Option<Found>,
    /// The depth of the item.
    pub(super) depth: u32,
    /// The side of the target; `None` outside the bases.
    pub(super) side: Option<Side>,
}

/// The state of one plan being built.
pub(super) struct Builder<'a, 'r> {
    pub(super) read: &'a TreeRead<'r>,
    pub(super) vfs: &'a dyn Vfs,
    pub(super) ctl: &'a Ctl<'a>,
    pub(super) include: Include,
    pub(super) settings: &'a OpSettings,
    pub(super) bases: Pair<Spec>,
    pub(super) items: Vec<PlanItem>,
    pub(super) meta: Vec<ItemMeta>,
    pub(super) warnings: Vec<Warning>,
    warned: BTreeSet<String>,
    caps: BTreeMap<String, Capabilities>,
    ancestry: Vec<u8>,
    /// Folder rows with a folder item in the plan, and its depth.
    pub(super) depths: BTreeMap<NodeId, u32>,
    /// Rows an item above took care of, or whose folder was refused.
    pub(super) covered: Vec<bool>,
    /// Where the children of a row go, per side written.
    folders: BTreeMap<(NodeId, Side), Spec>,
    /// The entries of a folder on a side by name: rule 8's lookup, one map per folder asked.
    names: BTreeMap<(NodeId, Side), BTreeMap<String, NodeId>>,
    /// Names orphans of a folder took on a side.
    orphans: BTreeSet<(NodeId, Side, String)>,
    /// The target of `copyto`, `moveto`: what lies below it keeps its path in a backup folder.
    pub(super) outside: Option<Spec>,
}

/// Plans `op`; see [`super::plan_op`].
pub(super) fn plan(
    read: &TreeRead<'_>,
    vfs: &dyn Vfs,
    scope: &OpScope,
    op: &FolderOp,
    settings: &OpSettings,
    ctl: &Ctl<'_>,
) -> Result<OpPlan, FolderError> {
    let (Some(left), Some(right)) = (
        read.spec(NodeId::ROOT, Side::Left),
        read.spec(NodeId::ROOT, Side::Right),
    ) else {
        return Err(FolderError::InvalidRules(
            "a comparison needs both bases".to_owned(),
        ));
    };
    let mut builder = Builder::new(
        read,
        vfs,
        scope.include,
        settings,
        ctl,
        Pair::new(left, right),
    );
    let rows = resolve(read, &scope.selection, &scope.include);
    let verb = match op {
        FolderOp::Copy { to, overwrite } => {
            builder.copy_or_move(&rows, *to, false, *overwrite)?;
            OpVerb::Copy
        }
        FolderOp::Move { to, overwrite } => {
            builder.copy_or_move(&rows, *to, true, *overwrite)?;
            OpVerb::Move
        }
        FolderOp::Exchange => {
            builder.exchange(&rows)?;
            OpVerb::Exchange
        }
        FolderOp::Delete { sides, recycle } => {
            builder.delete(&rows, *sides, *recycle)?;
            OpVerb::Delete
        }
        FolderOp::Rename { sides, mask } => {
            builder.rename(&rows, scope, *sides, mask)?;
            OpVerb::Rename
        }
        FolderOp::Touch { sides, from } => {
            builder.touch(&rows, *sides, *from)?;
            OpVerb::Touch
        }
        FolderOp::Mkdir {
            sides,
            parent,
            name,
            folder,
        } => {
            builder.mkdir(*sides, *parent, name, *folder)?;
            OpVerb::Mkdir
        }
        FolderOp::CopyTo {
            sides,
            target,
            structure,
            overwrite,
        } => {
            builder.copy_to(&rows, *sides, target, *structure, *overwrite, false)?;
            OpVerb::CopyTo
        }
        FolderOp::MoveTo {
            sides,
            target,
            structure,
            overwrite,
        } => {
            builder.copy_to(&rows, *sides, target, *structure, *overwrite, true)?;
            OpVerb::MoveTo
        }
    };
    let writes = !matches!(verb, OpVerb::CopyTo | OpVerb::MoveTo);
    let moves = matches!(verb, OpVerb::Move | OpVerb::MoveTo | OpVerb::Exchange);
    Ok(builder.finish(verb, writes, moves))
}

impl<'a, 'r> Builder<'a, 'r> {
    /// An empty plan on `read`.
    pub(super) fn new(
        read: &'a TreeRead<'r>,
        vfs: &'a dyn Vfs,
        include: Include,
        settings: &'a OpSettings,
        ctl: &'a Ctl<'a>,
        bases: Pair<Spec>,
    ) -> Builder<'a, 'r> {
        let len = read.len() as usize;
        Builder {
            read,
            vfs,
            ctl,
            include,
            settings,
            bases,
            items: Vec::new(),
            meta: Vec::new(),
            warnings: Vec::new(),
            warned: BTreeSet::new(),
            caps: BTreeMap::new(),
            ancestry: vec![0; len],
            depths: BTreeMap::new(),
            covered: vec![false; len],
            folders: BTreeMap::new(),
            names: BTreeMap::new(),
            orphans: BTreeSet::new(),
            outside: None,
        }
    }

    // ---- what a row may do ---------------------------------------------------------------------------------------

    /// The base of `side`.
    pub(super) fn base(&self, side: Side) -> &Spec {
        match side {
            Side::Right => &self.bases.right,
            Side::Left | Side::Center | Side::Output => &self.bases.left,
        }
    }

    /// What a folder row hands down to what is below it.
    fn hands_down(&self, id: NodeId) -> u8 {
        let Some(view) = self.read.node(id) else {
            return 0;
        };
        let mut bits = 0;
        for side in [Side::Left, Side::Right] {
            if view.side(side).is_some_and(|entry| entry.followed) {
                bits |= followed_bit(side);
            }
        }
        let flagged = NodeFlags::LEFT_ERROR | NodeFlags::RIGHT_ERROR;
        if view.scan == ScanStatus::Failed || view.flags.intersects(flagged) {
            bits |= FAILED;
        }
        bits
    }

    /// What is above a row: folders followed and failed (fence rules 1 and 3). Memoized, so a tree costs one step per row.
    fn above(&mut self, id: NodeId) -> u8 {
        let mut chain = Vec::new();
        let mut at = self.read.node(id).and_then(|view| view.parent);
        let known = self.ancestry.get(id.0 as usize).copied().unwrap_or(0);
        if known & KNOWN != 0 {
            return known;
        }
        chain.push(id);
        let mut bits = 0;
        while let Some(node) = at {
            let slot = self.ancestry.get(node.0 as usize).copied().unwrap_or(KNOWN);
            if slot & KNOWN != 0 {
                bits = (slot & !KNOWN) | self.hands_down(node);
                break;
            }
            chain.push(node);
            at = self.read.node(node).and_then(|view| view.parent);
        }
        for node in chain.into_iter().rev() {
            if let Some(slot) = self.ancestry.get_mut(node.0 as usize) {
                *slot = bits | KNOWN;
            }
            bits |= self.hands_down(node);
        }
        self.ancestry.get(id.0 as usize).copied().unwrap_or(0)
    }

    /// True when `id` lies below a folder the comparison followed on `side` (fence rule 1).
    pub(super) fn below_followed(&mut self, id: NodeId, side: Side) -> bool {
        self.above(id) & followed_bit(side) != 0
    }

    /// True when the row failed in the comparison, or lies below a folder that failed (fence rule 3): a failure is not a
    /// difference (D-19).
    pub(super) fn compare_failed(&mut self, id: NodeId, view: &NodeView<'_>) -> bool {
        if self.above(id) & FAILED != 0 {
            return true;
        }
        let flagged = view
            .flags
            .intersects(NodeFlags::LEFT_ERROR | NodeFlags::RIGHT_ERROR);
        if view.is_folder() {
            flagged || view.scan == ScanStatus::Failed
        } else {
            flagged || matches!(view.state, CompareState::Unknown | CompareState::Error)
        }
    }

    /// Why a row is not touched: on `reads` it is read, on `writes` it is written or removed. Filtered and named by itself,
    /// failed, or below a folder followed on a side written (fence rules 1 and 3).
    pub(super) fn refusal(
        &mut self,
        id: NodeId,
        view: &NodeView<'_>,
        reads: Option<Side>,
        writes: &[Side],
    ) -> Option<SkipReason> {
        let filtered = reads
            .iter()
            .chain(writes)
            .any(|side| view.side(*side).is_some_and(|entry| entry.filtered));
        if filtered && !self.include.hidden {
            return Some(SkipReason::Filtered);
        }
        if self.compare_failed(id, view) {
            return Some(SkipReason::CompareError);
        }
        if writes.iter().any(|side| self.below_followed(id, *side)) {
            return Some(SkipReason::FollowedLink);
        }
        None
    }

    /// The capabilities at `spec`, memoized per archive and per base: what a side can do is asked once (rule 11). An unknown
    /// answer lets the row through, and execution says what it finds.
    pub(super) fn capabilities(&mut self, spec: &Spec) -> Capabilities {
        let key = spec
            .container()
            .map_or_else(|| spec.scheme_name().to_owned(), Spec::canonical);
        if let Some(found) = self.caps.get(&key) {
            return *found;
        }
        let found = self.vfs.capabilities(spec).unwrap_or(Capabilities::all());
        self.caps.insert(key, found);
        found
    }

    /// True when `spec` lacks `bit`: the row is `skip capability`, and one `capability_missing` warning names the side.
    pub(super) fn lacks(
        &mut self,
        spec: &Spec,
        bit: Capabilities,
        word: &str,
        side: Option<Side>,
    ) -> bool {
        if self.capabilities(spec).contains(bit) {
            return false;
        }
        let side = side.map_or("target", Side::as_str);
        if self.warned.insert(format!("{word} {side}")) {
            self.warnings.push(Warning {
                code: WarningCode::CapabilityMissing,
                message: format!(
                    "{side} side: {}: `{word}` is not possible here",
                    spec.canonical()
                ),
                spec: Some(spec.canonical()),
            });
        }
        true
    }

    /// Adds one warning with `code` per plan.
    fn warn_once(&mut self, code: WarningCode, spec: &Spec, message: String) {
        if self.warned.insert(code.as_str().to_owned()) {
            self.warnings.push(Warning {
                code,
                message,
                spec: Some(spec.canonical()),
            });
        }
    }

    // ---- items -------------------------------------------------------------------------------------------------------

    /// Adds `item`.
    pub(super) fn push(&mut self, item: PlanItem, meta: ItemMeta) {
        self.items.push(item);
        self.meta.push(meta);
    }

    /// The meta of a row's item.
    pub(super) fn meta_of(&self, id: NodeId, side: Option<Side>) -> ItemMeta {
        ItemMeta {
            side,
            node: Some(id),
            path: self.read.path(id).as_str().to_owned(),
            conflicts: Conflicts::empty(),
            answer: None,
        }
    }

    /// Marks everything below `id` as seen to.
    pub(super) fn cover(&mut self, id: NodeId) {
        if let Some(slot) = self.covered.get_mut(id.0 as usize) {
            *slot = true;
        }
    }

    /// True when an item above took care of the row; the mark goes down with it.
    pub(super) fn is_covered(&mut self, view: &NodeView<'_>) -> bool {
        let above = view
            .parent
            .and_then(|parent| self.covered.get(parent.0 as usize).copied())
            .unwrap_or(false);
        if above {
            self.cover(view.id);
        }
        above
    }

    /// The depth of what lies directly in the row `parent`.
    pub(super) fn child_depth(&self, parent: Option<NodeId>) -> u32 {
        parent
            .and_then(|parent| self.depths.get(&parent))
            .map_or(0, |depth| depth + 1)
    }

    /// A skipped item of a row.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn skip_row(
        &mut self,
        id: NodeId,
        kind: EntryKind,
        source: Option<Spec>,
        target: Spec,
        depth: u32,
        reason: SkipReason,
        side: Option<Side>,
    ) {
        let mut item = PlanItem::planned(Action::Skip, kind, source, target, depth);
        item.skip = Some(reason);
        let meta = self.meta_of(id, side);
        self.push(item, meta);
    }

    /// A failed item of a row.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fail_row(
        &mut self,
        id: NodeId,
        action: Action,
        kind: EntryKind,
        source: Option<Spec>,
        target: Spec,
        depth: u32,
        error: ErrorReport,
        side: Option<Side>,
    ) {
        let mut item = PlanItem::planned(action, kind, source, target, depth);
        item.error = Some(error);
        let meta = self.meta_of(id, side);
        self.push(item, meta);
    }

    /// Takes `plan`'s items, made from the disk for what the model has no contents for (rule 6), at `depth`.
    pub(super) fn splice(
        &mut self,
        plan: Plan,
        id: NodeId,
        root: &Spec,
        depth: u32,
        side: Option<Side>,
    ) {
        self.warnings.extend(plan.warnings);
        let base = self.read.path(id);
        for mut item in plan.items {
            item.depth += depth;
            let path = rel_below(root, item.source.as_ref().unwrap_or(&item.target))
                .and_then(|rel| {
                    rel.components()
                        .try_fold(base.clone(), |path, name| path.join(name).ok())
                })
                .unwrap_or_else(|| base.clone());
            let mut conflicts = Conflicts::empty();
            conflicts.set(
                Conflicts::REPLACE_DURING_MOVE,
                item.replaces && item.action == Action::Move,
            );
            self.push(
                item,
                ItemMeta {
                    side,
                    node: None,
                    path: path.as_str().to_owned(),
                    conflicts,
                    answer: None,
                },
            );
        }
    }

    /// The entry on `side` as a plan can stamp it: a name-only entry (D-45) is stat'd once.
    pub(super) fn source_stamp(
        &self,
        entry: &EntryView<'_>,
        source: &Spec,
    ) -> Result<ItemStamp, VfsError> {
        if entry.stat_skipped {
            let follow = if entry.followed {
                FollowLinks::Yes
            } else {
                FollowLinks::No
            };
            return Ok(stamp_of_stat(&self.vfs.stat(source, follow)?));
        }
        Ok(stamp_of(entry, true))
    }

    /// What stands at the entry of a row.
    pub(super) fn found_of(
        &self,
        id: NodeId,
        entry: &EntryView<'_>,
        spec: &Spec,
    ) -> Result<Found, VfsError> {
        if entry.stat_skipped {
            let stat = self.vfs.stat(spec, FollowLinks::No)?;
            return Ok(Found {
                kind: stat.kind,
                stamp: stamp_of_stat(&stat),
                attrs: stat.attrs.clone(),
                mtime: stat.mtime,
                row: Some(id),
            });
        }
        Ok(Found {
            kind: if entry.followed {
                EntryKind::Symlink
            } else {
                entry.kind
            },
            stamp: stamp_of(entry, false),
            attrs: entry.attrs,
            mtime: entry.mtime,
            row: Some(id),
        })
    }

    /// The row of the name `raw` among the children of `parent` on `side` (rule 8), by the comparison's case, or the back-end's
    /// when that one folds.
    fn lookup(&mut self, parent: NodeId, side: Side, raw: &str, at: &Spec) -> Option<NodeId> {
        let fold = self.folds(at);
        let read = self.read;
        let key = move |name: &str| key_of(name, fold);
        let names = self.names.entry((parent, side)).or_insert_with(|| {
            let mut names = BTreeMap::new();
            for child in read.children(parent) {
                if let Some(entry) = read.node(child).as_ref().and_then(|view| view.side(side)) {
                    names.entry(key(entry.name)).or_insert(child);
                }
            }
            names
        });
        names.get(&key(raw)).copied()
    }

    /// True when names at `spec` are compared folded.
    pub(super) fn folds(&mut self, spec: &Spec) -> bool {
        self.read.case() == Some(hikasa_core::CaseSensitivity::Insensitive)
            || !self
                .capabilities(spec)
                .contains(Capabilities::CASE_SENSITIVE)
    }

    // ---- targets -----------------------------------------------------------------------------------------------------

    /// Where the children of the row `id` go on `to`: the other side's own folder of the row (rule 2), or a new one by the
    /// raw name, under the folder of the row above. Nothing joins strings, so a name that is not UTF-8 survives.
    fn folder_spec(&mut self, tr: Transfer, id: NodeId) -> Result<Spec, VfsError> {
        if id == NodeId::ROOT {
            return Ok(self.base(tr.to).clone());
        }
        if let Some(known) = self.folders.get(&(id, tr.to)) {
            return Ok(known.clone());
        }
        let read = self.read;
        let view = read.node(id);
        let own = read.spec(id, tr.to);
        let spec = match (own, view) {
            (Some(spec), _) => spec,
            (None, Some(view)) => {
                let dir = self.folder_spec(tr, view.parent.unwrap_or(NodeId::ROOT))?;
                self.new_child(&dir, id, tr.from)?
            }
            (None, None) => self.base(tr.to).clone(),
        };
        self.folders.insert((id, tr.to), spec.clone());
        Ok(spec)
    }

    /// `dir`/<the raw name of the row on `side`>.
    pub(super) fn new_child(&self, dir: &Spec, id: NodeId, side: Side) -> Result<Spec, VfsError> {
        let raw = self.read.raw_name(id, side).unwrap_or_default();
        self.vfs.child(dir, &FileName::from_os(&raw))
    }

    /// The missing folder of the row `id` and those above it get a `create_dir`, once, going down from the base (rule 3).
    /// One that exists gets no item. `Err` when no folder can stand there.
    fn ensure_folder(&mut self, tr: Transfer, id: NodeId) {
        if id == NodeId::ROOT || self.depths.contains_key(&id) {
            return;
        }
        let read = self.read;
        let Some(view) = read.node(id) else { return };
        if view.side(tr.to).is_some() {
            return;
        }
        let parent = view.parent.unwrap_or(NodeId::ROOT);
        self.ensure_folder(tr, parent);
        let depth = self.child_depth(view.parent);
        self.depths.insert(id, depth);
        let Some(entry) = view.side(tr.from).copied() else {
            return;
        };
        let (Some(source), Ok(target)) = (read.spec(id, tr.from), self.folder_spec(tr, id)) else {
            return;
        };
        let found = self.found_by_name(tr, parent, id, &target);
        let placed = Placed {
            id,
            entry: &entry,
            source,
            target,
            found,
            depth,
            side: Some(tr.to),
        };
        self.create_dir(placed, tr.overwrite);
    }

    /// What stands on the side written under the name of a row that has no entry there (rule 8).
    fn found_by_name(
        &mut self,
        tr: Transfer,
        parent: NodeId,
        id: NodeId,
        target: &Spec,
    ) -> Option<Found> {
        let read = self.read;
        let view = read.node(id)?;
        let name = view.side(tr.from)?.name;
        let row = self.lookup(parent, tr.to, name, target)?;
        let other = read.node(row)?;
        let entry = other.side(tr.to)?;
        let spec = read.spec(row, tr.to)?;
        self.found_of(row, entry, &spec).ok()
    }

    /// A `create_dir` with its source, so its metadata is applied after its subtree; a file or link in its place is an error
    /// unless the policy is `Always` (rule 8).
    pub(super) fn create_dir(&mut self, placed: Placed<'_>, overwrite: OverwritePolicy) {
        let Placed {
            id,
            entry,
            source,
            target,
            found,
            depth,
            side,
        } = placed;
        let mut item = PlanItem::planned(
            Action::CreateDir,
            EntryKind::Dir,
            Some(source.clone()),
            target.clone(),
            depth,
        );
        item.follow_source = entry.followed;
        item.expect = Some(Expect {
            source: Some(ItemStamp {
                kind: EntryKind::Dir,
                size: None,
                mtime: None,
            }),
            target: None,
        });
        let mut meta = self.meta_of(id, side);
        if let Some(found) = found {
            if found.kind == EntryKind::Dir {
                // Another spelling of a folder that stands there: nothing to make.
                return;
            }
            if overwrite != OverwritePolicy::Always {
                let error = VfsError::AlreadyExists {
                    spec: target.canonical(),
                };
                item.error = Some(report(&error));
            } else {
                item.replaces = true;
                meta.conflicts = Conflicts::of_attrs(&found.attrs);
                if let Some(expect) = &mut item.expect {
                    expect.target = Some(found.stamp);
                }
            }
        }
        self.push(item, meta);
    }

    /// The target of a row on the side written: its own, or a new name under the folder of the row above, with what stands
    /// there.
    fn slot(
        &mut self,
        tr: Transfer,
        id: NodeId,
        view: &NodeView<'_>,
    ) -> Result<(Spec, Option<Found>), VfsError> {
        if let Some(entry) = view.side(tr.to)
            && let Some(spec) = self.read.spec(id, tr.to)
        {
            let found = self.found_of(id, entry, &spec)?;
            return Ok((spec, Some(found)));
        }
        let parent = view.parent.unwrap_or(NodeId::ROOT);
        if let Some(target) = self.flat_target(tr, view)? {
            return Ok((target, None));
        }
        let dir = self.folder_spec(tr, parent)?;
        let target = self.new_child(&dir, id, tr.from)?;
        let found = self.found_by_name(tr, parent, id, &target);
        Ok((target, found))
    }

    /// In a flat comparison a row's folder is no row: `<base>/<folder>/<name>`, made by what the disk holds (§1.6.25).
    fn flat_target(&mut self, tr: Transfer, view: &NodeView<'_>) -> Result<Option<Spec>, VfsError> {
        let Some(dir) = view.side(tr.from).and_then(|entry| entry.dir) else {
            return Ok(None);
        };
        let mut at = self.base(tr.to).clone();
        for name in dir.split('/').filter(|name| !name.is_empty()) {
            at = self.vfs.child(&at, &FileName::new(name)?)?;
        }
        self.new_child(&at, view.id, tr.from).map(Some)
    }

    // ---- files ---------------------------------------------------------------------------------------------------------

    /// True when the source of a row is strictly newer than `found`, and when `found` is strictly newer than the source, by
    /// the row's `TimeRelation`: unknown is neither (rule 9).
    pub(super) fn newer(
        &self,
        view: &NodeView<'_>,
        entry: &EntryView<'_>,
        found: &Found,
        from: Side,
    ) -> (bool, bool) {
        let rule = &self.read.rules().criteria.timestamps;
        let relation = if found.row == Some(view.id) {
            view.time
        } else {
            None
        };
        let relation = relation.or_else(|| {
            let (source, target) = (entry.mtime?, found.mtime?);
            let (left, right) = if from == Side::Left {
                (source, target)
            } else {
                (target, source)
            };
            Some(compare_times(left, right, rule))
        });
        let source_side = match from {
            Side::Right => TimeRelation::RightNewer,
            Side::Left | Side::Center | Side::Output => TimeRelation::LeftNewer,
        };
        (
            relation == Some(source_side),
            relation == Some(source_side.mirrored()),
        )
    }

    /// `check_nesting` of what replaces: the same file under two names, spelt or linked, is no replacement (bases that
    /// overlap, or a member that would replace its own archive), and a move would lose it. One `stat`, for items that sync
    /// a file anyway.
    pub(super) fn same_file(
        &self,
        source: &Spec,
        target: &Spec,
        follow: bool,
        moving: bool,
    ) -> Result<(), VfsError> {
        let follow = if follow {
            FollowLinks::Yes
        } else {
            FollowLinks::No
        };
        let entry = self.vfs.stat(source, follow)?;
        ops::check_nesting(self.vfs, self.vfs, source, &entry, target)?;
        if moving {
            ops::check_move_inside(self.vfs, self.vfs, source, target)?;
            ops::check_same_archive(self.vfs, self.vfs, source, target)?;
        }
        Ok(())
    }

    /// One file or link onto its target: the overwrite policy from the model (rule 9), stamps (rule 10), conflicts.
    pub(super) fn place_file(
        &mut self,
        placed: Placed<'_>,
        view: &NodeView<'_>,
        action: Action,
        tr: Transfer,
    ) {
        let Placed {
            id,
            entry,
            source,
            target,
            found,
            depth,
            side,
        } = placed;
        let kind = if entry.followed || view.kind == NodeKind::File {
            EntryKind::File
        } else {
            view.kind.entry_kind()
        };
        let mut item = PlanItem::planned(action, kind, Some(source.clone()), target.clone(), depth);
        item.follow_source = entry.followed;
        let stamp = match self.source_stamp(entry, &source) {
            Ok(stamp) => stamp,
            Err(e) => {
                item.error = Some(report(&e));
                let meta = self.meta_of(id, side);
                return self.push(item, meta);
            }
        };
        if kind == EntryKind::File {
            item.size = stamp.size.or(Some(entry.size));
        }
        let mut expect = Expect {
            source: Some(stamp),
            target: None,
        };
        let mut meta = self.meta_of(id, side);
        if let Some(found) = found {
            if found.kind == EntryKind::Dir {
                item.error = Some(report(&VfsError::IsADirectory {
                    spec: target.canonical(),
                }));
                return self.push(item, meta);
            }
            let (source_newer, target_newer) = self.newer(view, entry, &found, tr.from);
            match tr.overwrite {
                OverwritePolicy::Fail => {
                    item.error = Some(report(&VfsError::AlreadyExists {
                        spec: target.canonical(),
                    }));
                    return self.push(item, meta);
                }
                OverwritePolicy::Skip => {
                    return self.skip_row(
                        id,
                        kind,
                        Some(source),
                        target,
                        depth,
                        SkipReason::Exists,
                        side,
                    );
                }
                OverwritePolicy::Newer if !source_newer => {
                    return self.skip_row(
                        id,
                        kind,
                        Some(source),
                        target,
                        depth,
                        SkipReason::NotNewer,
                        side,
                    );
                }
                OverwritePolicy::Newer | OverwritePolicy::Always => {}
            }
            if let Err(e) = self.same_file(&source, &target, entry.followed, tr.moving) {
                item.error = Some(report(&e));
                return self.push(item, meta);
            }
            item.replaces = true;
            expect.target = Some(found.stamp);
            meta.conflicts = Conflicts::of_attrs(&found.attrs);
            meta.conflicts.set(Conflicts::OVERWRITE_NEWER, target_newer);
            meta.conflicts
                .set(Conflicts::REPLACE_DURING_MOVE, tr.moving);
        }
        item.expect = Some(expect);
        self.push(item, meta);
    }

    /// The action of a file-like row, or why it stays (rule 5). `to` may be `None`: a target outside the bases.
    pub(super) fn kind_action(
        &mut self,
        view: &NodeView<'_>,
        source: &Spec,
        target: &Spec,
        side: Option<Side>,
        moving: bool,
    ) -> Result<Action, SkipReason> {
        let file = if moving { Action::Move } else { Action::Copy };
        match view.kind {
            NodeKind::Other => Err(SkipReason::SpecialFile),
            NodeKind::Symlink => {
                if matches!(source, Spec::ArchiveMember { .. }) {
                    self.warn_once(
                        WarningCode::ArchiveSymlinkSkipped,
                        source,
                        format!(
                            "{}: link members of an archive are not copied out of it",
                            source.canonical()
                        ),
                    );
                    return Err(SkipReason::Symlink);
                }
                if self.settings.links == ops::LinkPolicy::Skip {
                    return Err(SkipReason::Symlink);
                }
                if self.lacks(
                    target,
                    Capabilities::CREATE_SYMLINKS,
                    "create_symlinks",
                    side,
                ) {
                    return Err(SkipReason::SymlinksUnsupported);
                }
                Ok(if moving { Action::Move } else { Action::Link })
            }
            NodeKind::File | NodeKind::Folder => Ok(file),
        }
    }

    // ---- copy and move -------------------------------------------------------------------------------------------------

    /// `Copy`, `Move`: the rows of the source side in pre-order; a source folder goes after its contents (rule 4).
    fn copy_or_move(
        &mut self,
        rows: &[Resolved],
        to: Side,
        moving: bool,
        overwrite: OverwritePolicy,
    ) -> Result<(), FolderError> {
        let from = to.opposite();
        let tr = Transfer {
            from,
            to,
            moving,
            overwrite,
        };
        let bit = SideSet::of(from);
        let mine: Vec<&Resolved> = rows.iter().filter(|row| row.sides.contains(bit)).collect();
        if mine.is_empty() {
            return Err(FolderError::InvalidRules(format!(
                "nothing selected on the {from}"
            )));
        }
        let mut open = Vec::new();
        for row in mine {
            self.ctl.check()?;
            self.close_open(&mut open, row.depth);
            self.transfer(tr, row, &mut open)?;
        }
        self.close_open(&mut open, 0);
        Ok(())
    }

    /// The `remove_dir` of each folder that ends before `level`.
    pub(super) fn close_open(&mut self, open: &mut Vec<Open>, level: u32) {
        while open.last().is_some_and(|folder| folder.level >= level) {
            let Some(folder) = open.pop() else { break };
            self.remove_dir(folder);
        }
    }

    /// One `remove_dir`, after its contents.
    fn remove_dir(&mut self, folder: Open) {
        let mut item = PlanItem::planned(
            Action::RemoveDir,
            EntryKind::Dir,
            None,
            folder.spec,
            folder.depth,
        );
        item.expect = Some(Expect {
            source: None,
            target: Some(ItemStamp {
                kind: EntryKind::Dir,
                size: None,
                mtime: None,
            }),
        });
        let mut meta = self.meta_of(folder.id, Some(folder.side));
        meta.conflicts = folder.conflicts;
        self.push(item, meta);
    }

    /// True when a folder's move can be one `rename`: D-97's conditions, one `stat` per such root (rule 7).
    pub(super) fn can_rename(&mut self, source: &Spec, target: &Spec) -> bool {
        if source.scheme_name() != target.scheme_name()
            || source.outermost_file() != target.outermost_file()
            || !self.capabilities(source).contains(Capabilities::RENAME)
            || !self.capabilities(target).contains(Capabilities::RENAME)
        {
            return false;
        }
        let device = |spec: &Spec, follow| {
            self.vfs
                .stat(spec, follow)
                .ok()
                .and_then(|entry| entry.file_id)
                .map(|id| id.dev)
        };
        let source_device = device(source, FollowLinks::No);
        // The nearest folder that stands: the plan may make the ones below it.
        let target_device = std::iter::successors(target.parent(), Spec::parent)
            .find_map(|dir| device(&dir, FollowLinks::Yes));
        match (source_device, target_device) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }

    /// One row of a copy or move.
    pub(super) fn transfer(
        &mut self,
        tr: Transfer,
        row: &Resolved,
        open: &mut Vec<Open>,
    ) -> Result<(), FolderError> {
        let read = self.read;
        let id = row.node;
        let Some(view) = read.node(id) else {
            return Ok(());
        };
        if self.is_covered(&view) {
            return Ok(());
        }
        let Some(entry) = view.side(tr.from).copied() else {
            return Ok(());
        };
        let Some(source) = read.spec(id, tr.from) else {
            return Ok(());
        };
        let parent = view.parent.unwrap_or(NodeId::ROOT);
        let (target, found) = match self.slot(tr, id, &view) {
            Ok(slot) => slot,
            Err(e) => {
                let action = if view.is_folder() {
                    Action::CreateDir
                } else if tr.moving {
                    Action::Move
                } else {
                    Action::Copy
                };
                let depth = self.child_depth(view.parent);
                self.fail_row(
                    id,
                    action,
                    view.kind.entry_kind(),
                    Some(source.clone()),
                    source,
                    depth,
                    report(&e),
                    Some(tr.to),
                );
                self.cover(id);
                return Ok(());
            }
        };
        let writes: &[Side] = if tr.moving {
            &[tr.to, tr.from]
        } else {
            std::slice::from_ref(&tr.to)
        };
        let side = Some(tr.to);
        let kind = view.kind.entry_kind();
        if let Some(reason) = self.refusal(id, &view, Some(tr.from), writes) {
            let depth = self.child_depth(view.parent);
            self.skip_row(id, kind, Some(source), target, depth, reason, side);
            self.cover(id);
            return Ok(());
        }
        // Two rows that reach one name: what stands there goes to its own pair, and the first orphan takes a free one.
        if view.side(tr.to).is_none() {
            let paired = found
                .as_ref()
                .and_then(|found| found.row)
                .and_then(|other| read.node(other))
                .is_some_and(|other| other.side(tr.from).is_some());
            let folded = key_of(entry.name, self.folds(&target));
            if paired || !self.orphans.insert((parent, tr.to, folded)) {
                let depth = self.child_depth(view.parent);
                let reason = SkipReason::DuplicateTarget;
                self.skip_row(id, kind, Some(source), target, depth, reason, side);
                self.cover(id);
                return Ok(());
            }
        }
        // A folder: a followed one cannot leave, its contents stay (fence rule 1).
        if view.is_folder() && tr.moving && entry.followed {
            let depth = self.child_depth(view.parent);
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::FollowedLink,
                side,
            );
            self.cover(id);
            return Ok(());
        }
        let need = if view.is_folder() {
            (Capabilities::MKDIR, "mkdir")
        } else {
            (Capabilities::WRITE, "write")
        };
        let gone = tr.moving && self.lacks(&source, Capabilities::REMOVE, "remove", Some(tr.from));
        if gone || self.lacks(&target, need.0, need.1, side) {
            let depth = self.child_depth(view.parent);
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::Capability,
                side,
            );
            self.cover(id);
            return Ok(());
        }
        if view.is_folder() {
            self.transfer_folder(tr, row, &view, &entry, source, target, found, open)
        } else {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            match self.kind_action(&view, &source, &target, side, tr.moving) {
                Ok(action) => {
                    let placed = Placed {
                        id,
                        entry: &entry,
                        source,
                        target,
                        found,
                        depth,
                        side,
                    };
                    self.place_file(placed, &view, action, tr);
                }
                Err(reason) => {
                    self.skip_row(id, kind, Some(source), target, depth, reason, side);
                }
            }
            Ok(())
        }
    }

    /// A folder row: rule 6 for one the model has no contents for, one `rename` for a whole one that can, else its
    /// `create_dir`; a source folder's `remove_dir` comes after its contents, for a whole one alone.
    #[allow(clippy::too_many_arguments)]
    fn transfer_folder(
        &mut self,
        tr: Transfer,
        row: &Resolved,
        view: &NodeView<'_>,
        entry: &EntryView<'_>,
        source: Spec,
        target: Spec,
        found: Option<Found>,
        open: &mut Vec<Open>,
    ) -> Result<(), FolderError> {
        let id = row.node;
        let side = Some(tr.to);
        let parent = view.parent.unwrap_or(NodeId::ROOT);
        let whole = row.whole.contains(SideSet::of(tr.from));
        let absent = view.side(tr.to).is_none() && found.is_none();
        if view.scan == ScanStatus::NotScanned {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            self.expand_unlisted(tr, id, entry, &source, &target, depth)?;
            self.cover(id);
            return Ok(());
        }
        // An opened archive that goes whole goes as the file it is.
        let opened = view.flags.contains(NodeFlags::archive_root(tr.from));
        if let (true, true, Spec::ArchiveMember { archive, .. }) =
            (opened, whole && absent, &source)
        {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            let file = (**archive).clone();
            self.place_archive(tr, id, file, target, depth);
            self.cover(id);
            return Ok(());
        }
        if tr.moving
            && whole
            && absent
            && !self.has_followed_below(row)
            && self.can_rename(&source, &target)
        {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            let mut item =
                PlanItem::planned(Action::Rename, EntryKind::Dir, Some(source), target, depth);
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
            self.cover(id);
            return Ok(());
        }
        if view.side(tr.to).is_none() {
            self.ensure_folder(tr, id);
        }
        if tr.moving && whole {
            let depth = match self.depths.get(&id) {
                Some(depth) => *depth,
                None => {
                    let depth = self.child_depth(view.parent);
                    self.depths.insert(id, depth);
                    depth
                }
            };
            open.push(Open {
                level: row.depth,
                id,
                spec: source,
                depth,
                side: tr.from,
                conflicts: Conflicts::empty(),
            });
        }
        Ok(())
    }

    /// An `Always` file in a folder's name is replaced by the folder only through its item; below a followed row nothing
    /// moves, so a subtree with one is no single item.
    fn has_followed_below(&self, row: &Resolved) -> bool {
        // `whole` holds for a followed row's own rows too; the walk stays linear because it stops at the first.
        let read = self.read;
        let mut stack: Vec<NodeId> = read.children(row.node).collect();
        while let Some(id) = stack.pop() {
            if let Some(view) = read.node(id) {
                if view.left.is_some_and(|e| e.followed) || view.right.is_some_and(|e| e.followed) {
                    return true;
                }
                stack.extend(read.children(id));
            }
        }
        false
    }

    /// Rule 6: a folder the model has no contents for is expanded by `hikasa_vfs::ops`, with that one root.
    pub(super) fn expand_unlisted(
        &mut self,
        tr: Transfer,
        id: NodeId,
        entry: &EntryView<'_>,
        source: &Spec,
        target: &Spec,
        depth: u32,
    ) -> Result<(), FolderError> {
        let pairs = [(source.clone(), target.clone())];
        let planned = if tr.moving {
            let options = MoveOptions {
                overwrite: tr.overwrite,
                create_parents: false,
            };
            ops::plan_move(self.vfs, self.vfs, &pairs, &options, self.ctl)
        } else {
            let options = CopyOptions {
                recursive: true,
                overwrite: tr.overwrite,
                links: self.settings.links,
                preserve: self.settings.preserve,
                follow_roots: entry.followed,
                create_parents: false,
            };
            ops::plan_copy(self.vfs, self.vfs, &pairs, &options, self.ctl)
        };
        let plan = planned.map_err(|e| vfs_stop(&e))?;
        self.splice(plan, id, source, depth, Some(tr.to));
        Ok(())
    }

    /// An opened archive that leaves as one file: copied or moved by its own stamp.
    fn place_archive(&mut self, tr: Transfer, id: NodeId, file: Spec, target: Spec, depth: u32) {
        let action = if tr.moving {
            Action::Move
        } else {
            Action::Copy
        };
        let mut item =
            PlanItem::planned(action, EntryKind::File, Some(file.clone()), target, depth);
        match self.vfs.stat(&file, FollowLinks::No) {
            Ok(stat) => {
                item.size = Some(stat.size);
                item.expect = Some(Expect {
                    source: Some(stamp_of_stat(&stat)),
                    target: None,
                });
            }
            Err(e) => item.error = Some(report(&e)),
        }
        let meta = self.meta_of(id, Some(tr.to));
        self.push(item, meta);
    }

    // ---- exchange ----------------------------------------------------------------------------------------------------

    /// `Exchange`: `exchange` on a pair, a move on an orphan, in pre-order over folder pairs (rule 13).
    fn exchange(&mut self, rows: &[Resolved]) -> Result<(), FolderError> {
        if rows.is_empty() {
            return Err(FolderError::InvalidRules("nothing selected".to_owned()));
        }
        let mut open = Vec::new();
        for row in rows {
            self.ctl.check()?;
            self.close_open(&mut open, row.depth);
            let read = self.read;
            let Some(view) = read.node(row.node) else {
                continue;
            };
            match (view.left, view.right) {
                (Some(left), Some(right)) => {
                    if !self.is_covered(&view) {
                        self.exchange_pair(row.node, &view, &left, &right);
                    }
                }
                (Some(_), None) | (None, Some(_)) => {
                    let from = if view.left.is_some() {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    let tr = Transfer {
                        from,
                        to: from.opposite(),
                        moving: true,
                        overwrite: OverwritePolicy::Fail,
                    };
                    self.transfer(tr, row, &mut open)?;
                }
                (None, None) => {}
            }
        }
        self.close_open(&mut open, 0);
        Ok(())
    }

    /// One pair of an exchange. A pair of folders is no item: its rows are. A followed side, and members of two archives,
    /// stay: the first has no file to swap, and the second cannot be made safe (§1.6.2).
    fn exchange_pair(
        &mut self,
        id: NodeId,
        view: &NodeView<'_>,
        left: &EntryView<'_>,
        right: &EntryView<'_>,
    ) {
        let read = self.read;
        let (Some(source), Some(target)) = (read.spec(id, Side::Left), read.spec(id, Side::Right))
        else {
            return;
        };
        let kind = view.kind.entry_kind();
        let depth = self.child_depth(view.parent);
        if let Some(reason) = self.refusal(id, view, None, &[Side::Left, Side::Right]) {
            self.skip_row(id, kind, Some(source), target, depth, reason, None);
            self.cover(id);
            return;
        }
        if view.is_folder() {
            if view.scan != ScanStatus::Scanned {
                self.skip_row(
                    id,
                    kind,
                    Some(source),
                    target,
                    depth,
                    SkipReason::CompareError,
                    None,
                );
                self.cover(id);
            }
            return;
        }
        if left.followed || right.followed {
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::FollowedLink,
                None,
            );
            return;
        }
        if view.kind == NodeKind::Other {
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::SpecialFile,
                None,
            );
            return;
        }
        // A link member would come out on a disk: a hostile archive's link must not (D-142).
        let member = matches!(source, Spec::ArchiveMember { .. })
            || matches!(target, Spec::ArchiveMember { .. });
        if view.kind == NodeKind::Symlink && member {
            let message = format!(
                "{}: link members of an archive are not copied out of it",
                source.canonical()
            );
            self.warn_once(WarningCode::ArchiveSymlinkSkipped, &source, message);
            self.skip_row(
                id,
                kind,
                Some(source),
                target,
                depth,
                SkipReason::Symlink,
                None,
            );
            return;
        }
        let mut item = PlanItem::planned(
            Action::Exchange,
            kind,
            Some(source.clone()),
            target.clone(),
            depth,
        );
        item.replaces = true;
        item.size = (kind == EntryKind::File).then_some(left.size);
        let two_archives = matches!(
            (source.outermost_file(), target.outermost_file()),
            (Some(a), Some(b)) if a != b
        );
        let mut meta = self.meta_of(id, None);
        if two_archives {
            item.error = Some(report_with(
                ErrorKind::Unsupported,
                &target,
                format!(
                    "{}: exchanging between two archives is not supported: copy, or move, instead",
                    target.canonical()
                ),
            ));
            return self.push(item, meta);
        }
        let stamps = self
            .source_stamp(left, &source)
            .and_then(|left| self.source_stamp(right, &target).map(|right| (left, right)));
        let checked = stamps.and_then(|stamps| {
            self.same_file(&source, &target, false, false)
                .map(|()| stamps)
        });
        match checked {
            Ok((left_stamp, right_stamp)) => {
                item.expect = Some(Expect {
                    source: Some(left_stamp),
                    target: Some(right_stamp),
                });
                meta.conflicts =
                    Conflicts::of_attrs(&left.attrs) | Conflicts::of_attrs(&right.attrs);
            }
            Err(e) => item.error = Some(report(&e)),
        }
        self.push(item, meta);
    }

    // ---- delete ------------------------------------------------------------------------------------------------------

    /// `Delete`: per side, post-order, from the model (§1.6.12).
    fn delete(
        &mut self,
        rows: &[Resolved],
        sides: SideSet,
        recycle: Recycle,
    ) -> Result<(), FolderError> {
        if !rows.iter().any(|row| row.sides.intersects(sides)) {
            return Err(FolderError::InvalidRules(nothing_on(sides)));
        }
        for side in sides.sides() {
            self.depths.clear();
            self.covered.iter_mut().for_each(|slot| *slot = false);
            let mut open = Vec::new();
            for row in rows
                .iter()
                .filter(|row| row.sides.contains(SideSet::of(side)))
            {
                self.ctl.check()?;
                self.close_open(&mut open, row.depth);
                self.delete_row(side, row, recycle, &mut open)?;
            }
            self.close_open(&mut open, 0);
        }
        Ok(())
    }

    /// One row of a delete.
    pub(super) fn delete_row(
        &mut self,
        side: Side,
        row: &Resolved,
        recycle: Recycle,
        open: &mut Vec<Open>,
    ) -> Result<(), FolderError> {
        let read = self.read;
        let id = row.node;
        let Some(view) = read.node(id) else {
            return Ok(());
        };
        if self.is_covered(&view) {
            return Ok(());
        }
        let (Some(entry), Some(spec)) = (view.side(side).copied(), read.spec(id, side)) else {
            return Ok(());
        };
        let depth = self.child_depth(view.parent);
        let kind = view.kind.entry_kind();
        if let Some(reason) = self.refusal(id, &view, None, &[side]) {
            self.skip_row(id, kind, None, spec, depth, reason, Some(side));
            self.cover(id);
            return Ok(());
        }
        let whole = row.whole.contains(SideSet::of(side));
        // An opened archive that goes whole goes as the file it is, whatever its format can do to a member.
        let opened = view.flags.contains(NodeFlags::archive_root(side));
        let file = match (&spec, opened && whole) {
            (Spec::ArchiveMember { archive, .. }, true) => Some((**archive).clone()),
            _ => None,
        };
        let asked = file.as_ref().unwrap_or(&spec).clone();
        if self.lacks(&asked, Capabilities::REMOVE, "remove", Some(side)) {
            self.skip_row(
                id,
                kind,
                None,
                spec,
                depth,
                SkipReason::Capability,
                Some(side),
            );
            self.cover(id);
            return Ok(());
        }
        // A followed row is its link (fence rule 2), whatever it leads to.
        if view.is_folder() && !entry.followed {
            if view.scan == ScanStatus::NotScanned {
                let options = RemoveOptions {
                    recursive: true,
                    trash: recycle.use_bin && self.has_bin(&spec),
                };
                let plan =
                    ops::plan_remove(self.vfs, std::slice::from_ref(&spec), &options, self.ctl)
                        .map_err(|e| vfs_stop(&e))?;
                let first = self.items.len();
                self.splice(plan, id, &spec, depth, Some(side));
                self.mark_trash(first, recycle);
                self.cover(id);
                return Ok(());
            }
            if !whole {
                return Ok(());
            }
            if let Some(file) = file {
                self.delete_one(
                    id,
                    &view,
                    &entry,
                    file,
                    EntryKind::File,
                    depth,
                    side,
                    recycle,
                );
                self.cover(id);
                return Ok(());
            }
            if recycle.use_bin && self.has_bin(&spec) {
                self.delete_one(
                    id,
                    &view,
                    &entry,
                    spec,
                    EntryKind::Dir,
                    depth,
                    side,
                    recycle,
                );
                self.cover(id);
                return Ok(());
            }
            self.depths.insert(id, depth);
            let mut conflicts = Conflicts::of_attrs(&entry.attrs);
            conflicts.set(Conflicts::NO_RECYCLE_BIN, recycle.use_bin);
            open.push(Open {
                level: row.depth,
                id,
                spec,
                depth,
                side,
                conflicts,
            });
            return Ok(());
        }
        let kind = if entry.followed {
            EntryKind::Symlink
        } else {
            kind
        };
        self.delete_one(id, &view, &entry, spec, kind, depth, side, recycle);
        self.cover(id);
        Ok(())
    }

    /// True when `spec` can go to the bin.
    fn has_bin(&mut self, spec: &Spec) -> bool {
        self.capabilities(spec).contains(Capabilities::TRASH)
    }

    /// The items of `first..`, from the disk: a `trash` may fall back, and what has no bin is its question.
    fn mark_trash(&mut self, first: usize, recycle: Recycle) {
        for (item, meta) in self.items.iter_mut().zip(&mut self.meta).skip(first) {
            item.trash_fallback = item.action == Action::Trash && recycle.fallback;
            meta.conflicts.set(
                Conflicts::NO_RECYCLE_BIN,
                recycle.use_bin && item.action != Action::Trash,
            );
        }
    }

    /// One `remove` or `trash` of a row (§1.6.12's table).
    #[allow(clippy::too_many_arguments)]
    fn delete_one(
        &mut self,
        id: NodeId,
        view: &NodeView<'_>,
        entry: &EntryView<'_>,
        spec: Spec,
        kind: EntryKind,
        depth: u32,
        side: Side,
        recycle: Recycle,
    ) {
        let binned = recycle.use_bin && self.has_bin(&spec);
        let action = if binned {
            Action::Trash
        } else {
            Action::Remove
        };
        let mut item = PlanItem::planned(action, kind, None, spec, depth);
        item.trash_fallback = binned && recycle.fallback;
        let stamp = if entry.stat_skipped {
            self.vfs
                .stat(&item.target, FollowLinks::No)
                .map(|stat| stamp_of_stat(&stat))
        } else if matches!(kind, EntryKind::Symlink) && entry.followed {
            Ok(stamp_of(entry, false))
        } else if view.is_folder() && kind == EntryKind::File {
            // The file of an opened archive: its own stamp.
            self.vfs
                .stat(&item.target, FollowLinks::No)
                .map(|stat| stamp_of_stat(&stat))
        } else {
            Ok(stamp_of(entry, true))
        };
        match stamp {
            Ok(stamp) => {
                item.expect = Some(Expect {
                    source: None,
                    target: Some(stamp),
                });
            }
            Err(e) => item.error = Some(report(&e)),
        }
        let mut meta = self.meta_of(id, Some(side));
        meta.conflicts = Conflicts::of_attrs(&entry.attrs);
        meta.conflicts
            .set(Conflicts::NO_RECYCLE_BIN, recycle.use_bin && !binned);
        self.push(item, meta);
    }

    // ---- touch and mkdir ---------------------------------------------------------------------------------------------

    /// `Touch` (rule 14): every resolved row, folders too; `OtherSide` the rows with both sides.
    fn touch(
        &mut self,
        rows: &[Resolved],
        sides: SideSet,
        from: TouchFrom,
    ) -> Result<(), FolderError> {
        let sides = match from {
            TouchFrom::At(_) => sides,
            TouchFrom::OtherSide { to } => SideSet::of(to),
        };
        if !rows.iter().any(|row| row.sides.intersects(sides)) {
            return Err(FolderError::InvalidRules(nothing_on(sides)));
        }
        for side in sides.sides() {
            for row in rows
                .iter()
                .filter(|row| row.sides.contains(SideSet::of(side)))
            {
                self.ctl.check()?;
                self.touch_row(side, row, from);
            }
        }
        Ok(())
    }

    /// One row of a touch.
    fn touch_row(&mut self, side: Side, row: &Resolved, from: TouchFrom) {
        let read = self.read;
        let id = row.node;
        let Some(view) = read.node(id) else { return };
        let (Some(entry), Some(spec)) = (view.side(side).copied(), read.spec(id, side)) else {
            return;
        };
        let time = match from {
            TouchFrom::At(time) => Some(time),
            TouchFrom::OtherSide { .. } => {
                let Some(other) = view.side(side.opposite()) else {
                    return;
                };
                other.mtime
            }
        };
        let Some(time) = time else { return };
        let kind = view.kind.entry_kind();
        if let Some(reason) = self.refusal(id, &view, None, &[side]) {
            return self.skip_row(id, kind, None, spec, 0, reason, Some(side));
        }
        if self.lacks(&spec, Capabilities::SET_TIMES, "set_times", Some(side)) {
            return self.skip_row(id, kind, None, spec, 0, SkipReason::Capability, Some(side));
        }
        let kind = if entry.followed {
            EntryKind::Symlink
        } else {
            kind
        };
        let mut item = PlanItem::planned(Action::Touch, kind, None, spec, 0);
        item.times = Some(SetTimes {
            modified: Some(time),
            created: None,
        });
        let found = self.found_of(id, &entry, &item.target);
        match found {
            Ok(found) => {
                item.expect = Some(Expect {
                    source: None,
                    target: Some(found.stamp),
                });
            }
            Err(e) => item.error = Some(report(&e)),
        }
        let meta = self.meta_of(id, Some(side));
        self.push(item, meta);
    }

    /// `Mkdir` (rule 15): one name per chosen side under `parent`'s own folder there; a parent missing on a side gets rule 3's
    /// chain, from the side that has it.
    fn mkdir(
        &mut self,
        sides: SideSet,
        parent: NodeId,
        name: &str,
        folder: bool,
    ) -> Result<(), FolderError> {
        let read = self.read;
        if read.node(parent).is_none() {
            return Err(FolderError::InvalidRules(
                "no such row to make a folder in".to_owned(),
            ));
        }
        for side in sides.sides() {
            self.ctl.check()?;
            let tr = Transfer {
                from: side.opposite(),
                to: side,
                moving: false,
                overwrite: OverwritePolicy::Fail,
            };
            let meta_path = read.path(parent);
            let shown = meta_path
                .join(name)
                .map_or_else(|_| name.to_owned(), |path| path.as_str().to_owned());
            let meta = ItemMeta {
                side: Some(side),
                node: None,
                path: shown,
                conflicts: Conflicts::empty(),
                answer: None,
            };
            let dir = match self.folder_spec(tr, parent) {
                Ok(dir) => dir,
                Err(e) => return Err(FolderError::InvalidRules(e.to_string())),
            };
            if let Some(view) = read.node(parent)
                && (self.compare_failed(parent, &view)
                    || self.below_followed(parent, side)
                    || view.side(side).is_some_and(|entry| entry.followed))
            {
                let mut item = PlanItem::planned(Action::CreateDir, EntryKind::Dir, None, dir, 0);
                item.action = Action::Skip;
                item.skip = Some(if self.compare_failed(parent, &view) {
                    SkipReason::CompareError
                } else {
                    SkipReason::FollowedLink
                });
                self.push(item, meta);
                continue;
            }
            if self.lacks(&dir, Capabilities::MKDIR, "mkdir", Some(side)) {
                let mut item = PlanItem::planned(Action::Skip, EntryKind::Dir, None, dir, 0);
                item.skip = Some(SkipReason::Capability);
                self.push(item, meta);
                continue;
            }
            let target = match name_problem(name).map_err(str::to_owned).and_then(|()| {
                FileName::new(name)
                    .and_then(|file| self.vfs.child(&dir, &file))
                    .map_err(|e| e.to_string())
            }) {
                Ok(target) => target,
                Err(message) => {
                    let mut item =
                        PlanItem::planned(Action::CreateDir, EntryKind::Dir, None, dir.clone(), 0);
                    item.error = Some(report_with(ErrorKind::InvalidInput, &dir, message));
                    self.push(item, meta);
                    continue;
                }
            };
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(Some(parent));
            let mut planned = ops::plan_mkdir(self.vfs, std::slice::from_ref(&target), false);
            for mut item in planned.items.drain(..) {
                item.depth += depth;
                if folder && item.action == Action::CreateArchive {
                    item.action = Action::CreateDir;
                    item.kind = EntryKind::Dir;
                }
                self.push(item, meta.clone());
            }
        }
        Ok(())
    }

    // ---- the end -----------------------------------------------------------------------------------------------------

    /// The plan: backups by their final names, around every path of the plan (§1.6.5), the fence.
    fn finish(mut self, verb: OpVerb, writes: bool, moves: bool) -> OpPlan {
        let mut exec = ExecOptions {
            fence: Some(Fence {
                bases: vec![self.bases.left.clone(), self.bases.right.clone()],
                writes,
            }),
            archive_backups: BTreeMap::new(),
        };
        if let Some(policy) = &self.settings.backup {
            self.assign_backups(policy, &mut exec);
        }
        let refresh = super::refresh_nodes(self.read, &self.meta);
        let plan = Plan {
            op: OpKind::Batch,
            options: PlanOptions::Batch(BatchOptions {
                preserve: self.settings.preserve,
                links: self.settings.links,
                moves,
            }),
            items: self.items,
            warnings: Vec::new(),
        };
        OpPlan {
            verb,
            bases: self.bases,
            plan,
            meta: self.meta,
            exec,
            warnings: self.warnings,
            refresh,
        }
    }

    /// `rel` of a spec: below the base it lies in, for `naming = "folder"`.
    fn rel_of(&self, spec: &Spec) -> Option<RelPath> {
        [&self.bases.left, &self.bases.right]
            .into_iter()
            .find_map(|base| rel_below(base, spec))
    }

    /// Reserves every path of the plan, then gives each replacing item outside an archive its backup, and each outermost
    /// file the plan rewrites its one backup: a name is final here, and execution refuses what clashes.
    fn assign_backups(&mut self, policy: &ops::BackupPolicy, exec: &mut ExecOptions) {
        let vfs = self.vfs;
        let mut names = BackupNames::default();
        let mut outers: Vec<Spec> = Vec::new();
        let mut known = BTreeSet::new();
        let mut created = BTreeSet::new();
        for item in &self.items {
            names.reserve(vfs, &item.target);
            if let Some(source) = &item.source {
                names.reserve(vfs, source);
            }
            if item.action == Action::CreateArchive {
                created.insert(item.target.canonical());
            }
        }
        for item in &self.items {
            if item.error.is_some() || item.skip.is_some() {
                continue;
            }
            let moved_out = matches!(
                item.action,
                Action::Move | Action::Rename | Action::Exchange
            );
            let source = item.source.as_ref().filter(|_| moved_out);
            let found = [Some(&item.target), source];
            for outer in found.into_iter().flatten().filter_map(Spec::outermost_file) {
                if known.insert(outer.canonical()) && !created.contains(&outer.canonical()) {
                    names.reserve(vfs, outer);
                    outers.push(outer.clone());
                }
            }
        }
        for item in &mut self.items {
            let backed = matches!(
                item.action,
                Action::Copy | Action::Move | Action::Rename | Action::Link | Action::CreateDir
            );
            if !backed
                || !item.replaces
                || item.error.is_some()
                || item.skip.is_some()
                || matches!(item.target, Spec::ArchiveMember { .. })
            {
                continue;
            }
            let rel = [&self.bases.left, &self.bases.right]
                .into_iter()
                .chain(&self.outside)
                .find_map(|base| rel_below(base, &item.target));
            match names.assign(policy, vfs, &item.target, rel.as_ref()) {
                Ok(backup) => item.backup = Some(backup),
                Err(e) => {
                    item.error = Some(report(&e));
                    item.replaces = false;
                }
            }
        }
        for outer in outers {
            let rel = self.rel_of(&outer);
            match names.assign(policy, vfs, &outer, rel.as_ref()) {
                Ok(backup) => {
                    exec.archive_backups.insert(outer.canonical(), backup);
                }
                Err(e) => self.fail_archive(&outer, &e),
            }
        }
    }

    /// Without its backup an archive is not rewritten: its items fail.
    fn fail_archive(&mut self, outer: &Spec, error: &VfsError) {
        for item in &mut self.items {
            let touches = item.target.outermost_file() == Some(outer)
                || item.source.as_ref().and_then(Spec::outermost_file) == Some(outer);
            if touches && item.error.is_none() && item.skip.is_none() {
                item.error = Some(report(error));
            }
        }
    }
}

/// A name as a folder compares it.
pub(super) fn key_of(name: &str, fold: bool) -> String {
    if fold {
        hikasa_core::case::fold(name).into_owned()
    } else {
        name.to_owned()
    }
}

/// The message of a selection empty on `sides`.
pub(super) fn nothing_on(sides: SideSet) -> String {
    match sides.sides().collect::<Vec<_>>().as_slice() {
        [one] => format!("nothing selected on the {one}"),
        _ => "nothing selected".to_owned(),
    }
}

/// What a planner's stop is: only cancellation comes out of `plan_copy`, `plan_move` and `plan_remove`.
pub(super) fn vfs_stop(error: &VfsError) -> FolderError {
    if error.is_cancelled() {
        FolderError::Cancelled
    } else {
        FolderError::InvalidRules(error.to_string())
    }
}

/// The checks of a new name that need no back-end (§1.6.13).
pub(super) fn name_problem(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        Err("a new name cannot be empty")
    } else if name == "." || name == ".." {
        Err("a new name cannot be `.` or `..`")
    } else if name.contains('/') {
        Err("a new name cannot hold a `/`")
    } else {
        Ok(())
    }
}

/// `spec` below `base`, by `Spec::parent`; `None` when it is not below it, or is `base`.
pub(super) fn rel_below(base: &Spec, spec: &Spec) -> Option<RelPath> {
    let base = base.canonical();
    let mut names = Vec::new();
    let mut at = spec.clone();
    loop {
        if at.canonical() == base {
            break;
        }
        names.push(at.file_name()?);
        at = at.parent()?;
    }
    names.reverse();
    if names.is_empty() {
        return None;
    }
    let mut rel = RelPath::root();
    for name in &names {
        rel = rel.join(name).ok()?;
    }
    Some(rel)
}
