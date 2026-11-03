//! The one builder (§1.6.10, D-177): rows of the model become items of one batch plan. It reads the model; a row the model
//! has no contents for is expanded by `hikasa_vfs::ops` (rule 6). W6c's sync plan is made of the same pieces, so each rule
//! lives in one method here, and what a verb adds is its order of rows.

use std::collections::{BTreeMap, BTreeSet};

use hikasa_core::{
    Ctl, ErrorKind, ErrorReport, FileTime, Pair, RelPath, Side, Spec, Warning, WarningCode,
};
use hikasa_vfs::ops::{
    self, Action, BackupNames, BatchOptions, ExecOptions, Expect, Fence, ItemStamp, OpKind,
    OverwritePolicy, Plan, PlanItem, PlanOptions, SkipReason,
};
use hikasa_vfs::{
    Attrs, Capabilities, Entry, EntryKind, FileName, FollowLinks, SetTimes, Vfs, VfsError,
};

use super::conflict::Conflicts;
use super::unlisted::Walk;
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

/// What a stat saw stands where an item goes.
pub(super) fn found_of_stat(entry: &Entry) -> Found {
    Found {
        kind: entry.kind,
        stamp: stamp_of_stat(entry),
        attrs: entry.attrs,
        mtime: entry.mtime,
        row: None,
        filtered: false,
    }
}

/// An entry of a listing as a row's: what the items of rule 6 share with those of a row.
pub(super) fn view_of_entry(entry: &Entry) -> EntryView<'_> {
    EntryView {
        name: entry.name.as_str(),
        name_lossy: false,
        kind: entry.kind,
        size: entry.size,
        mtime: entry.mtime,
        ctime: entry.ctime,
        attrs: entry.attrs,
        link_target: entry.link_target.as_deref(),
        followed: false,
        crc32: None,
        filtered: false,
        stat_skipped: false,
        dir: None,
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
    /// A filter hides it: what a plan does not touch without `--include-hidden`.
    pub(super) filtered: bool,
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
    /// Folders a flat comparison needs on a side, by their path: the depth of what lies in one the plan makes (`None`: it
    /// stands).
    flat_folders: BTreeMap<(Side, String), Option<u32>>,
    /// Folders this plan makes (canonical), with the depth of their item: a second source of one folder merges into it.
    pub(super) made: BTreeMap<String, u32>,
    /// `copyto`, `moveto`: the file targets taken, keyed as `made` ([`Builder::taken_key`]): the first source of one
    /// path goes, the others stay.
    pub(super) taken: BTreeSet<String>,
    /// `copyto`, `moveto`: the folders planned only as the way to a source, by item: they stay only if something goes there.
    pub(super) way: BTreeSet<usize>,
    /// A sync: the entries of rows this plan deletes, by row (bit 1 left, 2 right). Their names are free for what the plan
    /// copies after its deletes (D-182: a file `x` replacing a folder `x`), so a copy expects no target there.
    pub(super) freed: Vec<u8>,
    /// A sync: what stands there is replaced only when it is the row's own entry. Another row's entry is no target of this
    /// row's action (Update never removes), and a file a delete frees is not there any more.
    pub(super) own_only: bool,
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
            folder,
            structure,
            overwrite,
        } => {
            builder.copy_to(
                &rows, *sides, target, *folder, *structure, *overwrite, false,
            )?;
            OpVerb::CopyTo
        }
        FolderOp::MoveTo {
            sides,
            target,
            folder,
            structure,
            overwrite,
        } => {
            builder.copy_to(&rows, *sides, target, *folder, *structure, *overwrite, true)?;
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
            flat_folders: BTreeMap::new(),
            made: BTreeMap::new(),
            taken: BTreeSet::new(),
            way: BTreeSet::new(),
            freed: Vec::new(),
            own_only: false,
        }
    }

    /// The overwrite policy of an item of the row `id` onto `found`: `own_only` turns it to `Fail` for another row's entry.
    fn policy_onto(&self, tr: Transfer, id: NodeId, found: &Found) -> OverwritePolicy {
        if self.own_only && found.row != Some(id) {
            OverwritePolicy::Fail
        } else {
            tr.overwrite
        }
    }

    /// True when this plan deletes the entry of `id` on `side` before it copies (a sync).
    fn is_freed(&self, id: NodeId, side: Side) -> bool {
        let bit = match side {
            Side::Right => 2,
            Side::Left | Side::Center | Side::Output => 1,
        };
        self.freed
            .get(id.0 as usize)
            .is_some_and(|bits| bits & bit != 0)
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
                row: Some(id),
                filtered: entry.filtered,
                ..found_of_stat(&stat)
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
            filtered: entry.filtered,
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
        self.create_dir(placed, tr);
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
        if self.is_freed(row, tr.to) {
            return None;
        }
        let other = read.node(row)?;
        let entry = other.side(tr.to)?;
        let spec = read.spec(row, tr.to)?;
        self.found_of(row, entry, &spec).ok()
    }

    /// A `create_dir` with its source, so its metadata is applied after its subtree; a file or link in its place is an error
    /// unless the policy is `Always` (rule 8), and then what it replaces asks what a file that replaces it would.
    pub(super) fn create_dir(&mut self, placed: Placed<'_>, tr: Transfer) {
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
            if found.filtered && !self.include.hidden {
                item.action = Action::Skip;
                item.skip = Some(SkipReason::Filtered);
                return self.push(item, meta);
            }
            if self.policy_onto(tr, id, &found) != OverwritePolicy::Always {
                let error = VfsError::AlreadyExists {
                    spec: target.canonical(),
                };
                item.error = Some(report(&error));
            } else {
                item.replaces = true;
                meta.conflicts = self.replacing(id, entry, &found, tr);
                if let Some(expect) = &mut item.expect {
                    expect.target = Some(found.stamp);
                }
            }
        }
        if item.error.is_none() {
            let made = item.target.clone();
            self.note_made(&made, depth);
        }
        self.push(item, meta);
    }

    /// The key of a folder of `made`: as the back-end compares names. Not as the comparison does: an aligned pair whose
    /// names differ is one folder by rule 16's choice of its name (D-194), and two folders the comparison folds together
    /// but a case-sensitive back-end keeps apart are two.
    pub(super) fn made_key(&mut self, spec: &Spec) -> String {
        let folds = !self
            .capabilities(spec)
            .contains(Capabilities::CASE_SENSITIVE);
        key_of(&spec.canonical(), folds)
    }

    /// The key of a file target of `taken`: as [`Builder::made_key`], by the target back-end's comparison, so a file and
    /// a folder meet where they would meet on the target (W7a note 1, review): a file `D` and a folder `d` the
    /// comparison folds together are two targets on a case-sensitive back-end, whichever comes first.
    pub(super) fn taken_key(&mut self, spec: &Spec) -> String {
        self.made_key(spec)
    }

    /// Records a folder this plan makes, with the depth of its item.
    pub(super) fn note_made(&mut self, spec: &Spec, depth: u32) {
        let key = self.made_key(spec);
        self.made.insert(key, depth);
    }

    /// True when this plan makes the folder `spec`.
    pub(super) fn is_made(&mut self, spec: &Spec) -> bool {
        let key = self.made_key(spec);
        self.made.contains_key(&key)
    }

    /// The depth of the item of a folder this plan makes.
    pub(super) fn made_depth(&mut self, spec: &Spec) -> Option<u32> {
        let key = self.made_key(spec);
        self.made.get(&key).copied()
    }

    /// What stands at a folder this plan makes: the folder, for what comes after it.
    pub(super) fn made_here(&mut self, spec: &Spec) -> Option<Found> {
        self.is_made(spec).then_some(Found {
            kind: EntryKind::Dir,
            stamp: ItemStamp {
                kind: EntryKind::Dir,
                size: None,
                mtime: None,
            },
            attrs: Attrs::None,
            mtime: None,
            row: None,
            filtered: false,
        })
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
            let found = match self.vfs.stat(&target, FollowLinks::No) {
                Ok(stat) => Some(found_of_stat(&stat)),
                Err(VfsError::NotFound { .. } | VfsError::NotADirectory { .. }) => None,
                Err(e) => return Err(e),
            };
            return Ok((target, found));
        }
        let dir = self.folder_spec(tr, parent)?;
        let target = self.new_child(&dir, id, tr.from)?;
        let found = self.found_by_name(tr, parent, id, &target);
        Ok((target, found))
    }

    /// In a flat comparison a row's folder is no row: `<base>/<folder>/<name>`, whose folders [`Builder::flat_depth`] plans.
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

    /// The folders on the way to a flat row, from the base: no row says which stand, so the disk does, once per folder. One
    /// that is missing gets rule 3's `create_dir`. The depth of what lies in the last one.
    fn flat_depth(&mut self, tr: Transfer, id: NodeId, dir: &str) -> u32 {
        let mut at = self.base(tr.to).clone();
        let mut from = self.base(tr.from).clone();
        let mut way = String::new();
        let mut depth = 0;
        for name in dir.split('/').filter(|name| !name.is_empty()) {
            way.push('/');
            way.push_str(name);
            let Ok(file) = FileName::new(name) else {
                return depth;
            };
            let (Ok(next), Ok(source)) = (self.vfs.child(&at, &file), self.vfs.child(&from, &file))
            else {
                return depth;
            };
            at = next;
            from = source;
            let key = (tr.to, way.clone());
            let known = match self.flat_folders.get(&key) {
                Some(known) => *known,
                None => {
                    let stood = self.vfs.stat(&at, FollowLinks::No).ok();
                    let made = !stood.as_ref().is_some_and(|it| it.kind == EntryKind::Dir);
                    if made && let Ok(stat) = self.vfs.stat(&from, FollowLinks::No) {
                        let placed = Placed {
                            id,
                            entry: &view_of_entry(&stat),
                            source: from.clone(),
                            target: at.clone(),
                            found: stood.as_ref().map(found_of_stat),
                            depth,
                            side: Some(tr.to),
                        };
                        self.create_dir(placed, tr);
                    }
                    let known = made.then_some(depth);
                    self.flat_folders.insert(key, known);
                    known
                }
            };
            depth = known.map_or(0, |depth| depth + 1);
        }
        depth
    }

    // ---- files ---------------------------------------------------------------------------------------------------------

    /// What an item that replaces `found` needs asked (§1.6.11), a file's or a folder's alike.
    fn replacing(
        &self,
        id: NodeId,
        entry: &EntryView<'_>,
        found: &Found,
        tr: Transfer,
    ) -> Conflicts {
        let mut conflicts = Conflicts::of_attrs(&found.attrs);
        if let Some(view) = self.read.node(id) {
            let (_, target_newer) = self.newer(&view, entry, found, tr.from);
            conflicts.set(Conflicts::OVERWRITE_NEWER, target_newer);
        }
        conflicts.set(Conflicts::REPLACE_DURING_MOVE, tr.moving);
        conflicts
    }

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
        kind: NodeKind,
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
        let kind = if entry.followed || kind == NodeKind::File {
            EntryKind::File
        } else {
            kind.entry_kind()
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
            if found.filtered && !self.include.hidden {
                item.action = Action::Skip;
                item.skip = Some(SkipReason::Filtered);
                return self.push(item, meta);
            }
            if found.kind == EntryKind::Dir {
                item.error = Some(report(&VfsError::IsADirectory {
                    spec: target.canonical(),
                }));
                return self.push(item, meta);
            }
            let source_newer = self
                .read
                .node(id)
                .is_some_and(|view| self.newer(&view, entry, &found, tr.from).0);
            match self.policy_onto(tr, id, &found) {
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
            meta.conflicts = self.replacing(id, entry, &found, tr);
        }
        item.expect = Some(expect);
        let respelt = item
            .replaces
            .then(|| self.respelling(id, entry, &item.target, stamp, tr))
            .flatten();
        self.push(item, meta);
        if let Some(rename) = respelt {
            let meta = self.meta_of(id, side);
            self.push(rename, meta);
        }
    }

    /// A sync that compares the case of names (`filename_case`) also gives a replaced counterpart the source's spelling:
    /// one `rename` after the copy, else a case-only difference is planned again on every run. The rename expects what the
    /// copy wrote, so a copy that did not run leaves the old spelling. `None` for `folder op`, the same spelling, or a
    /// side that cannot rename.
    fn respelling(
        &mut self,
        id: NodeId,
        entry: &EntryView<'_>,
        target: &Spec,
        stamp: ItemStamp,
        tr: Transfer,
    ) -> Option<PlanItem> {
        if !self.own_only || tr.moving || !self.read.rules().criteria.filename_case {
            return None;
        }
        let view = self.read.node(id)?;
        let there = view.side(tr.to)?;
        if there.name == entry.name || there.followed || entry.followed {
            return None;
        }
        let respelt = self.new_child(&target.parent()?, id, tr.from).ok()?;
        if self.lacks(target, Capabilities::RENAME, "rename", Some(tr.to)) {
            return None;
        }
        let mut rename = PlanItem::planned(
            Action::Rename,
            stamp.kind,
            Some(target.clone()),
            respelt,
            self.child_depth(view.parent),
        );
        rename.expect = Some(Expect {
            source: Some(ItemStamp {
                mtime: stamp.mtime.filter(|_| self.settings.preserve.times),
                ..stamp
            }),
            target: None,
        });
        Some(rename)
    }

    /// The action of a file-like row, or why it stays (rule 5). `to` may be `None`: a target outside the bases.
    pub(super) fn kind_action(
        &mut self,
        kind: NodeKind,
        source: &Spec,
        target: &Spec,
        side: Option<Side>,
        moving: bool,
    ) -> Result<Action, SkipReason> {
        let file = if moving { Action::Move } else { Action::Copy };
        match kind {
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
    pub(super) fn remove_dir(&mut self, folder: Open) {
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
            let spelt = match entry.dir {
                Some(dir) => format!("{dir}/{}", entry.name),
                None => entry.name.to_owned(),
            };
            let folded = key_of(&spelt, self.folds(&target));
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
        // An opened archive that goes whole goes as the file it is, whatever its format can do to a member.
        let whole = row.whole.contains(SideSet::of(tr.from));
        let file = match (
            &source,
            view.flags.contains(NodeFlags::archive_root(tr.from)) && whole,
        ) {
            (Spec::ArchiveMember { archive, .. }, true) => Some((**archive).clone()),
            _ => None,
        };
        let leaves = file.clone().unwrap_or_else(|| source.clone());
        let as_a_file = file.is_some() && (tr.moving || view.side(tr.to).is_none());
        let written = if as_a_file {
            outer_of(&target)
        } else {
            target.clone()
        };
        let need = if view.is_folder() && !as_a_file {
            (Capabilities::MKDIR, "mkdir")
        } else {
            (Capabilities::WRITE, "write")
        };
        let gone = tr.moving && self.lacks(&leaves, Capabilities::REMOVE, "remove", Some(tr.from));
        if gone || self.lacks(&written, need.0, need.1, side) {
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
            let depth = match entry.dir.filter(|_| view.side(tr.to).is_none()) {
                Some(dir) => self.flat_depth(tr, id, dir),
                None => self.child_depth(view.parent),
            };
            match self.kind_action(view.kind, &source, &target, side, tr.moving) {
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
                    self.place_file(placed, view.kind, action, tr);
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
        // A move of a folder onto or into itself: what `same_file` refuses for a file (rule 7 has no such check of its own).
        let opened = view.flags.contains(NodeFlags::archive_root(tr.from));
        if tr.moving
            && !(opened && whole)
            && let Err(e) = self.same_file(&source, &target, entry.followed, true)
        {
            let depth = self.child_depth(view.parent);
            self.fail_row(
                id,
                Action::Move,
                EntryKind::Dir,
                Some(source),
                target,
                depth,
                report(&e),
                side,
            );
            self.cover(id);
            return Ok(());
        }
        if view.scan == ScanStatus::NotScanned {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            let walk = Walk { tr, id, side };
            self.expand_unlisted(walk, entry, &source, &target, found, depth)?;
            self.cover(id);
            return Ok(());
        }
        // An opened archive that goes whole goes as the file it is, also onto its pair when it moves.
        if let (true, true, Spec::ArchiveMember { archive, .. }) =
            (opened, whole && (absent || tr.moving), &source)
        {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            let file = (**archive).clone();
            self.place_archive(tr, id, file, outer_of(&target), depth, side);
            self.cover(id);
            return Ok(());
        }
        if tr.moving
            && whole
            && absent
            && self.can_rename(&source, &target)
            && !self.has_followed_below(row)
        {
            self.ensure_folder(tr, parent);
            let depth = self.child_depth(view.parent);
            self.push_folder_rename(id, source, target, depth, side);
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

    /// True when a followed row lies below `row`: nothing below it moves (fence rule 1), so such a subtree is no single
    /// item.
    fn has_followed_below(&self, row: &Resolved) -> bool {
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

    /// One `rename` of a whole folder (rule 7).
    pub(super) fn push_folder_rename(
        &mut self,
        id: NodeId,
        source: Spec,
        target: Spec,
        depth: u32,
        side: Option<Side>,
    ) {
        self.note_made(&target, depth);
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
    }

    /// An opened archive that leaves as one file: copied or moved by its own stamp, onto what stands there as any file is.
    pub(super) fn place_archive(
        &mut self,
        tr: Transfer,
        id: NodeId,
        file: Spec,
        target: Spec,
        depth: u32,
        side: Option<Side>,
    ) {
        let action = if tr.moving {
            Action::Move
        } else {
            Action::Copy
        };
        let looked = self.vfs.stat(&file, FollowLinks::No).and_then(|stat| {
            match self.vfs.stat(&target, FollowLinks::No) {
                Ok(found) => Ok((stat, Some(found))),
                Err(VfsError::NotFound { .. }) => Ok((stat, None)),
                Err(e) => Err(e),
            }
        });
        match looked {
            Ok((stat, found)) => {
                let placed = Placed {
                    id,
                    entry: &view_of_entry(&stat),
                    source: file,
                    target,
                    found: found.as_ref().map(found_of_stat),
                    depth,
                    side,
                };
                self.place_file(placed, NodeKind::File, action, tr);
            }
            Err(e) => {
                let kind = EntryKind::File;
                self.fail_row(
                    id,
                    action,
                    kind,
                    Some(file),
                    target,
                    depth,
                    report(&e),
                    side,
                );
            }
        }
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
                self.delete_unlisted(side, id, &spec, depth, recycle)?;
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
                let first = self.items.len();
                self.delete_one(
                    id,
                    &view,
                    &entry,
                    spec.clone(),
                    EntryKind::Dir,
                    depth,
                    side,
                    recycle,
                );
                self.whole_trash(first, id, side);
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
    pub(super) fn has_bin(&mut self, spec: &Spec) -> bool {
        self.capabilities(spec).contains(Capabilities::TRASH)
    }

    /// What one `trash` of the whole folder `id`, the item at `index`, takes with it: its question asks for what the model
    /// knows below it, and each thing below is stamped. Execution checks that below the folder stands just that before
    /// it trashes (`verify_below`), and the fallback may delete nothing else.
    fn whole_trash(&mut self, index: usize, id: NodeId, side: Side) {
        let below = self.conflicts_below(id, side);
        let mut permanent = Vec::new();
        self.permanent_rows(id, side, 0, &mut permanent);
        if let Some(item) = self.items.get(index) {
            let mut own = PlanItem::planned(
                Action::RemoveDir,
                EntryKind::Dir,
                None,
                item.target.clone(),
                0,
            );
            own.expect = item.expect;
            permanent.push(own);
        }
        if let Some(meta) = self.meta.get_mut(index) {
            meta.conflicts |= below;
        }
        if let Some(item) = self.items.get_mut(index) {
            item.fallback = permanent;
            item.verify_below = true;
        }
    }

    /// What the attributes of everything below the row `id` on `side` ask of its removal.
    fn conflicts_below(&self, id: NodeId, side: Side) -> Conflicts {
        let read = self.read;
        let mut found = Conflicts::empty();
        let mut stack: Vec<NodeId> = read.children(id).collect();
        while let Some(row) = stack.pop() {
            let Some(view) = read.node(row) else { continue };
            let Some(entry) = view.side(side) else {
                continue;
            };
            found |= Conflicts::of_attrs(&entry.attrs);
            if view.is_folder() && !entry.followed {
                stack.extend(read.children(row));
            }
        }
        found
    }

    /// Everything below the row `id` for good, stamped, post-order (§1.6.12): a followed row is its link, an opened archive
    /// the file it is.
    fn permanent_rows(&self, id: NodeId, side: Side, depth: u32, out: &mut Vec<PlanItem>) {
        let read = self.read;
        let at = depth + 1;
        for row in read.children(id) {
            let Some(view) = read.node(row) else { continue };
            let (Some(entry), Some(spec)) = (view.side(side), read.spec(row, side)) else {
                continue;
            };
            let opened = view.flags.contains(NodeFlags::archive_root(side));
            let (target, kind, stamp) = match (&spec, opened) {
                (Spec::ArchiveMember { archive, .. }, true) => {
                    let file = (**archive).clone();
                    let stamp = self
                        .vfs
                        .stat(&file, FollowLinks::No)
                        .map(|stat| stamp_of_stat(&stat));
                    (file, EntryKind::File, stamp)
                }
                _ if view.is_folder() && !entry.followed => {
                    self.permanent_rows(row, side, at, out);
                    let stamp = ItemStamp {
                        kind: EntryKind::Dir,
                        size: None,
                        mtime: None,
                    };
                    let mut item =
                        PlanItem::planned(Action::RemoveDir, EntryKind::Dir, None, spec, at);
                    item.expect = Some(Expect {
                        source: None,
                        target: Some(stamp),
                    });
                    out.push(item);
                    continue;
                }
                _ if entry.followed => (spec, EntryKind::Symlink, Ok(stamp_of(entry, false))),
                _ => {
                    let stamp = if entry.stat_skipped {
                        self.vfs
                            .stat(&spec, FollowLinks::No)
                            .map(|stat| stamp_of_stat(&stat))
                    } else {
                        Ok(stamp_of(entry, true))
                    };
                    (spec, view.kind.entry_kind(), stamp)
                }
            };
            let mut item = PlanItem::planned(Action::Remove, kind, None, target, at);
            match stamp {
                Ok(stamp) => {
                    item.expect = Some(Expect {
                        source: None,
                        target: Some(stamp),
                    });
                }
                Err(e) => item.error = Some(report(&e)),
            }
            out.push(item);
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
    pub(super) fn finish(mut self, verb: OpVerb, writes: bool, moves: bool) -> OpPlan {
        self.ask_for_archives();
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
        // A plan is held across a confirmation and a dialog: it keeps what it holds, not what its growth reserved (§1.6.22).
        self.items.shrink_to_fit();
        self.meta.shrink_to_fit();
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
        let mut out = OpPlan {
            verb,
            bases: self.bases,
            plan,
            meta: self.meta,
            exec,
            warnings: self.warnings,
            refresh,
        };
        super::confirm::skip_below(&mut out);
        out
    }

    /// A change of a member rewrites the file around it: what that file's attributes ask, each item that changes it asks
    /// (§1.6.11). One `stat` per outermost file.
    fn ask_for_archives(&mut self) {
        let vfs = self.vfs;
        let mut files: BTreeMap<String, Conflicts> = BTreeMap::new();
        for (item, meta) in self.items.iter().zip(&mut self.meta) {
            if item.error.is_some() || item.skip.is_some() {
                continue;
            }
            for outer in changed_files(item) {
                meta.conflicts |= *files.entry(outer.canonical()).or_insert_with(|| {
                    vfs.stat(outer, FollowLinks::No)
                        .map_or(Conflicts::empty(), |stat| Conflicts::of_attrs(&stat.attrs))
                });
            }
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
        for (item, meta) in self.items.iter_mut().zip(&mut self.meta) {
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
                Ok(backup) => {
                    meta.conflicts
                        .set(Conflicts::OVERWRITE_BACKUP, stands(vfs, &backup));
                    item.backup = Some(backup);
                }
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
                    if stands(vfs, &backup) {
                        self.ask_for_backup(&outer);
                    }
                    exec.archive_backups.insert(outer.canonical(), backup);
                }
                Err(e) => self.fail_archive(&outer, &e),
            }
        }
    }

    /// What stands at the backup of `outer` is lost by its rewrite: each item that rewrites it asks.
    fn ask_for_backup(&mut self, outer: &Spec) {
        for (item, meta) in self.items.iter().zip(&mut self.meta) {
            if item.error.is_none() && item.skip.is_none() && changed_files(item).contains(&outer) {
                meta.conflicts |= Conflicts::OVERWRITE_BACKUP;
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

/// The outermost files `item` rewrites: its target's, and that of a source it takes away.
fn changed_files(item: &PlanItem) -> Vec<&Spec> {
    let moved_out = matches!(
        item.action,
        Action::Move | Action::Rename | Action::Exchange
    );
    let source = item.source.as_ref().filter(|_| moved_out);
    let mut files: Vec<&Spec> = [Some(&item.target), source]
        .into_iter()
        .flatten()
        .filter_map(Spec::outermost_file)
        .collect();
    files.dedup();
    files
}

/// True when something stands at a backup's name: this plan did not make it, for `BackupNames` keeps its own apart.
fn stands(vfs: &dyn Vfs, backup: &Spec) -> bool {
    vfs.stat(backup, FollowLinks::No).is_ok()
}

/// What a row of an opened archive stands for on the side written: the file, for an archive's root.
fn outer_of(target: &Spec) -> Spec {
    match target {
        Spec::ArchiveMember { archive, member } if member.is_empty() => (**archive).clone(),
        other => other.clone(),
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
