//! The sync plan (§1.6.14, D-182): each row gets a situation and an action by the matrix, the overrides and what the side
//! written allows; then the builder of the folder operations turns the actions into one batch, deletes first. Nothing
//! here writes, and nothing plans an item the builder would not plan for a `delete` or a `copy` of the same row.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hikasa_core::{Ctl, Pair, Side, Warning, WarningCode};
use hikasa_vfs::ops::OverwritePolicy;
use hikasa_vfs::{Capabilities, Vfs};

use super::build::{Builder, Transfer};
use super::{OpVerb, Recycle};
use crate::error::FolderError;
use crate::model::store::path_key;
use crate::model::{NodeFlags, NodeId, NodeView, ScanStatus, TreeRead};
use crate::select::{Include, Resolved, SideSet, resolve};
use crate::state::Category;
use crate::sync::{
    LeaveReason, OrphanReading, Situation, SyncAction, SyncCounts, SyncMatrix, SyncPlan,
    SyncRequest, SyncRow, override_path,
};

/// The bit of a side in the per-row side bits.
const fn bit(side: Side) -> u8 {
    match side {
        Side::Right => 2,
        Side::Left | Side::Center | Side::Output => 1,
    }
}

/// The overrides of FS-004, keyed as the comparison aligns names: the longest path that names a row or an ancestor
/// decides, as for Ignored marks (§1.6.6).
struct Overrides {
    /// Key → (the path as written, the action).
    keys: BTreeMap<String, (String, SyncAction)>,
}

impl Overrides {
    /// The override that decides the row at `key`, with the path as written.
    fn decides(&self, key: &str) -> Option<(&str, SyncAction)> {
        let mut at = key;
        loop {
            if let Some((path, action)) = self.keys.get(at) {
                return Some((path.as_str(), *action));
            }
            at = at.rsplit_once('/')?.0;
        }
    }
}

/// What the planner keeps per row while it walks the tree.
struct Planner<'a, 'r> {
    read: &'a TreeRead<'r>,
    request: &'a SyncRequest<'a>,
    matrix: SyncMatrix,
    builder: Builder<'a, 'r>,
    /// The level of each row (the base's children are 0).
    depth: Vec<u32>,
    /// The sides on which a folder above the row is hidden by a filter; either hides the row.
    hidden_above: Vec<u8>,
    /// Rows in scope (`None`: all).
    scope: Option<Vec<bool>>,
    /// The overrides (`None`: none).
    overrides: Option<Overrides>,
    /// Override paths dropped for a row they cannot apply to, each warned once.
    dropped: BTreeSet<String>,
    /// Override paths that decided some row.
    used: BTreeSet<String>,
    /// Read-only sides warned.
    warned_read_only: u8,
    /// The index in `rows` of each row's decision.
    row_of: Vec<u32>,
    /// The row's subtree goes as one item: everything below has its action.
    whole: Vec<bool>,
    rows: Vec<SyncRow>,
    warnings: Vec<Warning>,
}

/// Plans a sync; see [`crate::sync::plan_sync`].
pub(crate) fn plan(
    read: &TreeRead<'_>,
    vfs: &dyn Vfs,
    request: &SyncRequest<'_>,
    ctl: &Ctl<'_>,
) -> Result<SyncPlan, FolderError> {
    let options = request.options;
    options.validate()?;
    if read.rules().handling.flatten {
        return Err(FolderError::InvalidRules(
            "a sync needs the folder structure: it cannot run on a flat comparison".to_owned(),
        ));
    }
    let (Some(left), Some(right)) = (
        read.spec(NodeId::ROOT, Side::Left),
        read.spec(NodeId::ROOT, Side::Right),
    ) else {
        return Err(FolderError::InvalidRules(
            "a comparison needs both bases".to_owned(),
        ));
    };
    let include = Include {
        hidden: request.include_hidden,
        display: request.visible.unwrap_or(Include::visible().display),
    };
    let builder = Builder::new(
        read,
        vfs,
        include,
        &request.settings,
        ctl,
        Pair::new(left, right),
    );
    let len = read.len() as usize;
    let mut planner = Planner {
        read,
        request,
        matrix: options.effective_matrix(),
        builder,
        depth: vec![0; len],
        hidden_above: vec![0; len],
        scope: None,
        overrides: None,
        dropped: BTreeSet::new(),
        used: BTreeSet::new(),
        warned_read_only: 0,
        row_of: vec![u32::MAX; len],
        whole: vec![false; len],
        rows: Vec::with_capacity(len.saturating_sub(1)),
        warnings: Vec::new(),
    };
    planner.prepare(include)?;
    let (counts, folders) = planner.decide(ctl)?;
    planner.refuse(&counts)?;
    planner.unused_overrides();
    if counts.unresolved > 0 {
        planner.warnings.push(Warning {
            code: WarningCode::SyncIncomplete,
            message: format!(
                "{} rows could not be compared; the sync leaves them as they are",
                counts.unresolved
            ),
            spec: None,
        });
    }
    let matrix = planner.matrix;
    let (op, rows, warnings) = planner.build(ctl)?;
    Ok(SyncPlan {
        rows,
        op,
        counts,
        warnings,
        preset: options.preset,
        matrix,
        orphans: options.orphans,
        folders,
    })
}

impl<'a, 'r> Planner<'a, 'r> {
    /// Depths, what filters hide above each row, the scope and the overrides: one pass in pre-order.
    fn prepare(&mut self, include: Include) -> Result<(), FolderError> {
        let read = self.read;
        for (id, depth) in read.preorder() {
            let index = id.0 as usize;
            let Some(view) = read.node(id) else { continue };
            if let Some(slot) = self.depth.get_mut(index) {
                *slot = depth;
            }
            let parent = view.parent.unwrap_or(NodeId::ROOT);
            let mut hidden = self
                .hidden_above
                .get(parent.0 as usize)
                .copied()
                .unwrap_or(0);
            if let Some(parent_view) = read.node(parent).filter(|_| parent != NodeId::ROOT) {
                for side in [Side::Left, Side::Right] {
                    if parent_view.side(side).is_some_and(|entry| entry.filtered) {
                        hidden |= bit(side);
                    }
                }
            }
            if let Some(slot) = self.hidden_above.get_mut(index) {
                *slot = hidden;
            }
        }
        let request = self.request;
        if request.just_selection.is_some() || request.visible.is_some() {
            let len = read.len() as usize;
            let mut scope = vec![true; len];
            if let Some(selection) = request.just_selection {
                let mut chosen = vec![false; len];
                for row in resolve(read, selection, &include) {
                    if let Some(slot) = chosen.get_mut(row.node.0 as usize) {
                        *slot = true;
                    }
                }
                for (slot, chosen) in scope.iter_mut().zip(chosen) {
                    *slot &= chosen;
                }
            }
            if let Some(display) = &request.visible {
                let mut shown = vec![false; len];
                let mut rows = Vec::new();
                read.visible_rows(display, &|_| true, &mut rows);
                for row in rows {
                    if let Some(slot) = shown.get_mut(row.node.0 as usize) {
                        *slot = true;
                    }
                }
                for (slot, shown) in scope.iter_mut().zip(shown) {
                    *slot &= shown;
                }
            }
            self.scope = Some(scope);
        }
        if !request.options.overrides.is_empty() {
            let align = read.store().marks_align();
            let mut keys = BTreeMap::new();
            for (path, action) in &request.options.overrides {
                let checked = override_path(path)?;
                let key = path_key(&align, checked.as_str()).into_owned();
                keys.insert(key, (path.clone(), *action));
            }
            self.overrides = Some(Overrides { keys });
        }
        Ok(())
    }

    /// True when the row is in scope.
    fn in_scope(&self, id: NodeId) -> bool {
        self.scope
            .as_ref()
            .is_none_or(|scope| scope.get(id.0 as usize).copied().unwrap_or(false))
    }

    /// True when the row's comparison failed or did not reach it, or a folder above it failed (rule 2, fence rule 3).
    fn failed(&mut self, id: NodeId, view: &NodeView<'_>) -> bool {
        if self.builder.compare_failed(id, view) {
            return true;
        }
        if view.is_folder() {
            return view.scan != ScanStatus::Scanned;
        }
        matches!(
            self.read.category(id, self.request.iu),
            Category::Unknown | Category::Error
        )
    }

    /// True when a side of the row, or a folder above it on either side, is hidden by a filter (rule 3).
    fn hidden(&self, id: NodeId, view: &NodeView<'_>) -> bool {
        // A folder hidden on either side hides what lies below it on both: a row whose counterpart folder is hidden is no
        // orphan to delete.
        let above = self.hidden_above.get(id.0 as usize).copied().unwrap_or(0);
        above != 0
            || [Side::Left, Side::Right]
                .into_iter()
                .any(|side| view.side(side).is_some_and(|entry| entry.filtered))
    }

    /// The situation of a row with its sides (rule 4); `None` when the comparison cannot tell.
    fn situation(&self, id: NodeId, view: &NodeView<'_>) -> Option<Situation> {
        let read = self.read;
        let category = read.category(id, self.request.iu);
        if view.flags.contains(NodeFlags::IGNORED) && category != Category::Filtered {
            // An Ignored row reads `same`: a mark changes the verdict, and a sync keeps its hands off it.
            return Some(Situation::Same);
        }
        match (view.left.is_some(), view.right.is_some()) {
            (true, true) => match category {
                Category::Same => Some(Situation::Same),
                Category::LeftNewer => Some(Situation::NewerLeft),
                Category::RightNewer => Some(Situation::NewerRight),
                Category::Different => Some(Situation::Different),
                _ => None,
            },
            (true, false) => Some(match self.request.options.orphans {
                OrphanReading::Added => Situation::AddedLeft,
                OrphanReading::Deleted => Situation::DeletedRight,
            }),
            (false, true) => Some(match self.request.options.orphans {
                OrphanReading::Added => Situation::AddedRight,
                OrphanReading::Deleted => Situation::DeletedLeft,
            }),
            (false, false) => None,
        }
    }

    /// The action of `situation` at the row (rule 5): the override of the longest path, else the matrix. An override the
    /// row cannot take is dropped, with one `switch_ignored` per path. Returns the action and whether an override chose it.
    fn action(&mut self, id: NodeId, situation: Situation) -> (SyncAction, bool) {
        let found = self.overrides.as_ref().and_then(|overrides| {
            let align = self.read.store().marks_align();
            let path = self.read.path(id);
            let key = path_key(&align, path.as_str());
            overrides
                .decides(&key)
                .map(|(path, action)| (path.to_owned(), action))
        });
        if let Some((path, action)) = found {
            if situation.allows(action) {
                self.used.insert(path);
                return (action, true);
            }
            if self.dropped.insert(path.clone()) {
                self.warnings.push(Warning {
                    code: WarningCode::SwitchIgnored,
                    message: format!(
                        "override {path}={action}: a {situation} row has no side for it; the matrix decides"
                    ),
                    spec: None,
                });
            }
        }
        (self.matrix.get(situation), false)
    }

    /// Why the side written cannot take `action` (rule 6): read-only, below a followed folder, or a capability missing.
    fn infeasible(
        &mut self,
        id: NodeId,
        view: &NodeView<'_>,
        action: SyncAction,
    ) -> Option<LeaveReason> {
        let side = action.writes()?;
        if self.request.read_only.get(side) {
            if self.warned_read_only & bit(side) == 0 {
                self.warned_read_only |= bit(side);
                let base = self.builder.base(side).canonical();
                self.warnings.push(Warning {
                    code: WarningCode::ReadOnlySide,
                    message: format!(
                        "the {side} side is read-only: the sync leaves what it would write or delete there"
                    ),
                    spec: Some(base),
                });
            }
            return Some(LeaveReason::ReadOnlySide);
        }
        if self.builder.below_followed(id, side) {
            return Some(LeaveReason::FollowedLink);
        }
        let read = self.read;
        if action.deletes() {
            let spec = read.spec(id, side)?;
            // An opened archive that goes whole goes as the file it is.
            let spec = match spec {
                hikasa_core::Spec::ArchiveMember { archive, member }
                    if member.is_empty() && view.flags.contains(NodeFlags::archive_root(side)) =>
                {
                    *archive
                }
                other => other,
            };
            if self
                .builder
                .lacks(&spec, Capabilities::REMOVE, "remove", Some(side))
            {
                return Some(LeaveReason::Capability);
            }
            return None;
        }
        // A copy: where it lands is the row's own entry, or the nearest folder above that stands on that side.
        let at = std::iter::once(id)
            .chain(read.ancestors(id))
            .find_map(|row| read.spec(row, side))?;
        let (need, word) = if view.is_folder() {
            (Capabilities::MKDIR, "mkdir")
        } else {
            (Capabilities::WRITE, "write")
        };
        if self.builder.lacks(&at, need, word, Some(side)) {
            return Some(LeaveReason::Capability);
        }
        None
    }

    /// The decision of a row whose side or sides read the matrix: a file-like row, an empty folder on one side, or the
    /// link of a followed folder that goes. Rules 4–6.
    fn by_matrix(&mut self, id: NodeId, view: &NodeView<'_>, row: &mut SyncRow) {
        let Some(situation) = self.situation(id, view) else {
            row.reason = Some(LeaveReason::CompareError);
            return;
        };
        row.situation = Some(situation);
        let (action, overridden) = self.action(id, situation);
        row.overridden = overridden;
        row.conflict = situation == Situation::Different
            && !overridden
            && self.matrix.different == SyncAction::Leave;
        if action == SyncAction::Leave {
            row.reason = Some(if situation == Situation::Same {
                LeaveReason::Same
            } else {
                LeaveReason::Matrix
            });
            return;
        }
        match self.infeasible(id, view, action) {
            Some(reason) => row.reason = Some(reason),
            None => row.action = action,
        }
    }

    /// The decision of one row; its children are decided (post-order).
    fn row(&mut self, id: NodeId, view: &NodeView<'_>) -> SyncRow {
        let mut row = SyncRow {
            node: id,
            situation: None,
            action: SyncAction::Leave,
            reason: None,
            overridden: false,
            conflict: false,
        };
        let pair_of_folders = view.is_folder() && view.left.is_some() && view.right.is_some();
        // Rule 1.
        if !self.in_scope(id) {
            row.reason = Some(LeaveReason::OutOfScope);
            return row;
        }
        // Rule 2: what failed is no difference, and below it nothing is an orphan.
        if self.failed(id, view) {
            row.reason = Some(LeaveReason::CompareError);
            return row;
        }
        if pair_of_folders {
            // A pair of folders has no row of its own in the matrix: its rows have.
            return row;
        }
        // Rule 3. A row every present side of which a filter hides is no row, as for the display: `--include-hidden` brings
        // the hidden side of a pair, never an item the filters hide altogether.
        let all_hidden = [view.left, view.right]
            .iter()
            .flatten()
            .all(|entry| entry.filtered);
        if all_hidden || (self.hidden(id, view) && !self.request.include_hidden) {
            row.reason = Some(LeaveReason::Filtered);
            return row;
        }
        if !view.is_folder() {
            self.by_matrix(id, view, &mut row);
            self.set_whole(id, row.action != SyncAction::Leave);
            return row;
        }
        let Some(side) = [Side::Left, Side::Right]
            .into_iter()
            .find(|side| view.side(*side).is_some())
        else {
            return row;
        };
        let followed = view.side(side).is_some_and(|entry| entry.followed);
        let empty = self.read.children(id).next().is_none();
        if followed {
            // A followed folder is its link (fence rule 2): it goes as one, and nothing below it is touched there.
            let mut link = row;
            self.by_matrix(id, view, &mut link);
            if link.action.deletes() {
                return link;
            }
        }
        if empty {
            self.by_matrix(id, view, &mut row);
            if row.action != SyncAction::Leave && !self.request.options.create_empty {
                // FS-005 *Sync empty folders* is off: an empty folder is neither made nor deleted.
                row.action = SyncAction::Leave;
                row.reason = Some(LeaveReason::Matrix);
            }
            self.set_whole(id, row.action != SyncAction::Leave);
            return row;
        }
        // A folder on one side with contents: what its rows do.
        row.situation = self.situation(id, view);
        let children: Vec<SyncRow> = self
            .read
            .children(id)
            .filter_map(|child| self.decided(child))
            .collect();
        let gone = if side == Side::Left {
            SyncAction::DeleteLeft
        } else {
            SyncAction::DeleteRight
        };
        let copied = if side == Side::Left {
            SyncAction::CopyRight
        } else {
            SyncAction::CopyLeft
        };
        let all_whole = self
            .read
            .children(id)
            .all(|child| self.whole.get(child.0 as usize).copied().unwrap_or(false));
        if !followed && !children.is_empty() && children.iter().all(|child| child.action == gone) {
            // Every row of a folder it alone has goes: so does the folder (no empty shells).
            row.action = gone;
            self.set_whole(id, all_whole);
        } else if children.iter().any(|child| child.action == copied) {
            row.action = copied;
            self.set_whole(id, all_whole && children.iter().all(|c| c.action == copied));
        } else {
            row.reason = Some(strongest(&children));
        }
        row.overridden = !children.is_empty() && children.iter().all(|c| c.overridden);
        row
    }

    /// The decision of a row already decided.
    fn decided(&self, id: NodeId) -> Option<SyncRow> {
        let index = *self.row_of.get(id.0 as usize)?;
        self.rows.get(index as usize).copied()
    }

    fn set_whole(&mut self, id: NodeId, whole: bool) {
        if let Some(slot) = self.whole.get_mut(id.0 as usize) {
            *slot = whole;
        }
    }

    /// Every row in post-order, with the counts of the base and of each folder.
    fn decide(
        &mut self,
        ctl: &Ctl<'_>,
    ) -> Result<(SyncCounts, HashMap<NodeId, SyncCounts>), FolderError> {
        let read = self.read;
        let mut folders: HashMap<NodeId, SyncCounts> = HashMap::new();
        for (n, id) in read.store().post_order().into_iter().enumerate() {
            if n % 4096 == 0 {
                ctl.check()?;
            }
            let id = NodeId(id);
            if id == NodeId::ROOT {
                continue;
            }
            let Some(view) = read.node(id) else { continue };
            let row = self.row(id, &view);
            let bytes = match (row.action.reads(), view.is_folder()) {
                (Some(side), false) => view.side(side).map_or(0, |entry| entry.size),
                _ => 0,
            };
            let mut counts = SyncCounts::of_row(&row, bytes);
            if let Some(below) = folders.get(&id) {
                counts.add(below);
            }
            folders
                .entry(view.parent.unwrap_or(NodeId::ROOT))
                .or_default()
                .add(&counts);
            if let Some(slot) = self.row_of.get_mut(id.0 as usize) {
                *slot = u32::try_from(self.rows.len()).unwrap_or(u32::MAX);
            }
            self.rows.push(row);
        }
        let counts = folders.get(&NodeId::ROOT).copied().unwrap_or_default();
        Ok((counts, folders))
    }

    /// `EmptySource` and an incomplete comparison: what a sync that deletes must not run on.
    fn refuse(&self, counts: &SyncCounts) -> Result<(), FolderError> {
        let read = self.read;
        let deletes = counts.delete_left + counts.delete_right > 0;
        if deletes && !read.is_complete() {
            return Err(FolderError::InvalidRules(
                "the comparison is not complete: a sync that deletes needs a complete comparison"
                    .to_owned(),
            ));
        }
        if self.request.allow_empty_source {
            return Ok(());
        }
        let empty = |side: Side| {
            read.children(NodeId::ROOT).all(|child| {
                read.node(child)
                    .is_none_or(|view| view.side(side).is_none())
            })
        };
        for (deleted, source) in [
            (counts.delete_right, Side::Left),
            (counts.delete_left, Side::Right),
        ] {
            if deleted > 0 && empty(source) {
                return Err(FolderError::EmptySource { side: source });
            }
        }
        Ok(())
    }

    /// One `switch_ignored` for an override that named no row.
    fn unused_overrides(&mut self) {
        let Some(overrides) = &self.overrides else {
            return;
        };
        for (path, _) in overrides.keys.values() {
            if !self.used.contains(path) && !self.dropped.contains(path) {
                self.warnings.push(Warning {
                    code: WarningCode::SwitchIgnored,
                    message: format!("override {path}: no row of this comparison is there"),
                    spec: None,
                });
            }
        }
    }

    /// The rows of `action`, in pre-order, as the builder takes them.
    fn resolved(&self, action: SyncAction) -> Vec<Resolved> {
        let mut out = Vec::new();
        let Some(side) = action.writes() else {
            return out;
        };
        let on = if action.deletes() {
            side
        } else {
            side.opposite()
        };
        for (id, depth) in self.read.preorder() {
            let Some(row) = self.decided(id) else {
                continue;
            };
            if row.action != action {
                continue;
            }
            let whole = self.whole.get(id.0 as usize).copied().unwrap_or(false);
            out.push(Resolved {
                node: id,
                sides: SideSet::of(on),
                whole: if whole {
                    SideSet::of(on)
                } else {
                    SideSet::empty()
                },
                depth,
            });
        }
        out
    }

    /// The batch (D-182): deletes left, deletes right (post-order), copies right, copies left (pre-order); with the rows
    /// and the warnings.
    #[allow(clippy::type_complexity)]
    fn build(
        mut self,
        ctl: &Ctl<'_>,
    ) -> Result<(super::OpPlan, Vec<SyncRow>, Vec<Warning>), FolderError> {
        let mut freed = vec![0u8; self.read.len() as usize];
        for row in &self.rows {
            if let (true, Some(side)) = (row.action.deletes(), row.action.writes())
                && let Some(slot) = freed.get_mut(row.node.0 as usize)
            {
                *slot |= bit(side);
            }
        }
        self.builder.freed = freed;
        self.builder.own_only = true;
        let recycle = Recycle {
            use_bin: self.request.options.recycle,
            fallback: self.request.options.recycle_fallback,
        };
        for (action, side) in [
            (SyncAction::DeleteLeft, Side::Left),
            (SyncAction::DeleteRight, Side::Right),
        ] {
            let rows = self.resolved(action);
            self.fresh();
            let mut open = Vec::new();
            for row in &rows {
                ctl.check()?;
                self.builder.close_open(&mut open, row.depth);
                self.builder.delete_row(side, row, recycle, &mut open)?;
            }
            self.builder.close_open(&mut open, 0);
        }
        for action in [SyncAction::CopyRight, SyncAction::CopyLeft] {
            let (Some(from), Some(to)) = (action.reads(), action.writes()) else {
                continue;
            };
            let tr = Transfer {
                from,
                to,
                moving: false,
                overwrite: OverwritePolicy::Always,
            };
            let rows = self.resolved(action);
            self.fresh();
            let mut open = Vec::new();
            for row in &rows {
                ctl.check()?;
                self.builder.close_open(&mut open, row.depth);
                self.builder.transfer(tr, row, &mut open)?;
            }
            self.builder.close_open(&mut open, 0);
        }
        let op = self.builder.finish(OpVerb::Sync, true, false);
        self.rows.shrink_to_fit();
        Ok((op, self.rows, self.warnings))
    }

    /// A new phase: the depths and covers of the last one say nothing about this one.
    fn fresh(&mut self) {
        self.builder.depths.clear();
        self.builder
            .covered
            .iter_mut()
            .for_each(|slot| *slot = false);
    }
}

/// The reason a folder on one side stays, from those of its rows: the one that says most.
fn strongest(children: &[SyncRow]) -> LeaveReason {
    const ORDER: [LeaveReason; 8] = [
        LeaveReason::CompareError,
        LeaveReason::ReadOnlySide,
        LeaveReason::Capability,
        LeaveReason::FollowedLink,
        LeaveReason::Filtered,
        LeaveReason::OutOfScope,
        LeaveReason::Matrix,
        LeaveReason::Same,
    ];
    ORDER
        .into_iter()
        .find(|reason| children.iter().any(|child| child.reason == Some(*reason)))
        .unwrap_or(LeaveReason::Matrix)
}
