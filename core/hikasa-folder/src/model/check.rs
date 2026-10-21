//! INV-FOLDER-1..10 (Appendix B), checked on a live or finished model (T4).

use std::collections::BTreeMap;

use hikasa_core::{ContentKind, FolderVerdict, InvariantViolation, Pair, RelPath, Side};
use hikasa_vfs::EntryKind;

use crate::align::{ChildKey, kind_rank};
use crate::counts::{Aggregates, SideTotals, Slot};
use crate::display::{
    DisplayOptions, FolderDisplayFilter, FolderMode, reference_rows, visible_rows_into,
};
use crate::model::TreeRead;
use crate::model::node::{NodeFlags, NodeId, NodeKind};
use crate::model::store::{NONE, Node, Store, bits};
use crate::resolve::{path_verdict, side_of};
use crate::state::{Category, CompareState};
use crate::stats::ContentCounts;

fn fail(id: &'static str, detail: String) -> Result<(), InvariantViolation> {
    Err(InvariantViolation { id, detail })
}

/// Checks INV-FOLDER-1..10.
pub(crate) fn check(tree: &TreeRead<'_>) -> Result<(), InvariantViolation> {
    let store = tree.store();
    structure(store)?;
    kinds_and_orphans(store)?;
    canonical_order(store)?;
    counters(store)?;
    filters(store)?;
    content(store)?;
    visibility(store)?;
    monotonicity(store)
}

fn nodes(store: &Store) -> impl Iterator<Item = (u32, &Node)> + '_ {
    (0..store.len()).filter_map(|id| store.node(NodeId(id)).map(|n| (id, n)))
}

/// INV-FOLDER-1: parent and child links agree; the root has no parent.
fn structure(store: &Store) -> Result<(), InvariantViolation> {
    let len = store.len();
    for (id, node) in nodes(store) {
        if id == 0 {
            if node.parent != NONE {
                return fail("INV-FOLDER-1", "the root has a parent".into());
            }
        } else {
            if node.parent >= id {
                return fail(
                    "INV-FOLDER-1",
                    format!("node {id}: parent {} is not older", node.parent),
                );
            }
            let Some(parent) = store.node(NodeId(node.parent)) else {
                return fail("INV-FOLDER-1", format!("node {id}: missing parent"));
            };
            if !parent.children().contains(&id) {
                return fail(
                    "INV-FOLDER-1",
                    format!("node {id} is not among its parent's children"),
                );
            }
        }
        if node.children().end > len {
            return fail("INV-FOLDER-1", format!("node {id}: children out of range"));
        }
        if !node.is_folder() && node.child_count > 0 {
            return fail("INV-FOLDER-1", format!("file-like node {id} has children"));
        }
        for child in node.children() {
            if store.node(NodeId(child)).map(|c| c.parent) != Some(id) {
                return fail(
                    "INV-FOLDER-1",
                    format!("child {child} of {id} names another parent"),
                );
            }
        }
    }
    Ok(())
}

/// INV-FOLDER-2 (kinds never mix) and INV-FOLDER-4 (an orphan has exactly one present side); node flags agree
/// with the side records.
fn kinds_and_orphans(store: &Store) -> Result<(), InvariantViolation> {
    for (id, node) in nodes(store).skip(1) {
        for (index, rec) in node.sides.iter().enumerate() {
            let side = side_of(index);
            if rec.present() && !rec.has(bits::NO_META) && NodeKind::of(rec.kind()) != node.kind() {
                return fail(
                    "INV-FOLDER-2",
                    format!(
                        "node {id}: {side} entry kind {} in a {} node",
                        rec.kind(),
                        node.kind().as_str()
                    ),
                );
            }
            let flags = node.flags();
            if flags.contains(NodeFlags::filtered(side))
                != (rec.present() && rec.has(bits::FILTERED))
            {
                return fail(
                    "INV-FOLDER-7",
                    format!("node {id}: {side} filtered flag disagrees"),
                );
            }
            if flags.contains(NodeFlags::stat_skipped(side))
                != (rec.present() && rec.has(bits::STAT_SKIPPED))
            {
                return fail(
                    "INV-FOLDER-7",
                    format!("node {id}: {side} stat-skipped flag disagrees"),
                );
            }
        }
        let state = store.state_of(node);
        let present = [node.sides[0].present(), node.sides[1].present()];
        let ok = match state {
            CompareState::LeftOrphan => present == [true, false],
            CompareState::RightOrphan => present == [false, true],
            CompareState::Unknown
            | CompareState::Same
            | CompareState::LeftNewer
            | CompareState::RightNewer
            | CompareState::Different
            | CompareState::Error
            | CompareState::Filtered => present.iter().any(|&p| p),
        };
        if !ok {
            return fail(
                "INV-FOLDER-4",
                format!("node {id}: state {state:?} with sides {present:?}"),
            );
        }
    }
    Ok(())
}

/// The canonical key of a node, computed from the store.
fn key_of(store: &Store, id: u32, node: &Node) -> ChildKey {
    let side_parts = |side: usize| {
        let rec = &node.sides[side];
        let text = store.name(side, rec).to_owned();
        let exact = store.raw_name(side, id, rec).into_encoded_bytes();
        let dir = rec
            .dir()
            .and_then(|d| store.dir_info(side, d))
            .map_or_else(String::new, |d| d.text.clone());
        (text, exact, dir)
    };
    let repr_side = if node.sides[0].present() { 0 } else { 1 };
    let (text, exact, dir) = side_parts(repr_side);
    let right = node.sides[1].present().then(|| side_parts(1));
    ChildKey::new(
        kind_rank(node.kind().entry_kind()),
        &text,
        &exact,
        &dir,
        right.as_ref().map(|(_, e, d)| (&e[..], d.as_str())),
    )
}

/// INV-FOLDER-3: children are one contiguous block (by construction) in strictly increasing canonical order.
fn canonical_order(store: &Store) -> Result<(), InvariantViolation> {
    for (id, node) in nodes(store) {
        let mut previous: Option<ChildKey> = None;
        for child in node.children() {
            let Some(c) = store.node(NodeId(child)) else {
                return fail("INV-FOLDER-3", format!("child {child} of {id} missing"));
            };
            let key = key_of(store, child, c);
            if previous.as_ref().is_some_and(|p| *p >= key) {
                return fail(
                    "INV-FOLDER-3",
                    format!("children of {id} not in canonical order at {child}"),
                );
            }
            previous = Some(key);
        }
    }
    Ok(())
}

/// INV-FOLDER-5 (every counter and total equals a recount, statistics too) and INV-FOLDER-6 (a folder's state is
/// `own_category(F, false)` of its counters).
fn counters(store: &Store) -> Result<(), InvariantViolation> {
    let len = store.len() as usize;
    let mut agg: Vec<Aggregates> = vec![Aggregates::default(); len];
    let mut totals: Vec<Pair<SideTotals>> = vec![Pair::default(); len];
    let mut content = ContentCounts::default();
    for id in (1..len).rev() {
        let Some(node) = store.node(NodeId(id as u32)) else {
            continue;
        };
        let (cats, slot) = if node.is_folder() {
            let own = agg.get(id).copied().unwrap_or_default();
            let cats = [
                store.own_category_from(node, Some(&own), false),
                store.own_category_from(node, Some(&own), true),
            ];
            for iu in [false, true] {
                if store.own_category(node, iu) != cats[usize::from(iu)] {
                    return fail(
                        "INV-FOLDER-6",
                        format!("folder {id}: own category disagrees with a recount"),
                    );
                }
            }
            (cats, Slot::Folders)
        } else {
            (
                [
                    store.category_of(node, false),
                    store.category_of(node, true),
                ],
                Slot::Files,
            )
        };
        if let Some(c) = node.content() {
            let slot = match c.kind {
                ContentKind::BinarySame => &mut content.binary_same,
                ContentKind::RulesSame => &mut content.rules_same,
                ContentKind::UnimportantOnly => &mut content.unimportant_only,
                ContentKind::ImportantDifferences => &mut content.important_differences,
                ContentKind::BinaryDifferent => &mut content.binary_different,
                ContentKind::Error => &mut content.error,
            };
            *slot += 1;
        }
        if node.flags().contains(NodeFlags::CONTENT_SKIPPED) {
            content.not_compared += 1;
        }
        let parent = node.parent as usize;
        let own_agg = if node.is_folder() {
            agg.get(id).copied()
        } else {
            None
        };
        let own_totals = if node.is_folder() {
            totals.get(id).copied()
        } else {
            None
        };
        let contribution = Store::totals_contribution(node);
        if let Some(p) = agg.get_mut(parent) {
            for (i, iu) in [false, true].into_iter().enumerate() {
                let a = p.get_mut(iu);
                match slot {
                    Slot::Files => a.files.add_one(cats[i]),
                    Slot::Folders => a.folders.add_one(cats[i]),
                }
                if let Some(child) = own_agg {
                    let c = child.get(iu);
                    a.files = a.files.plus(&c.files);
                    a.folders = a.folders.plus(&c.folders);
                }
            }
        }
        if let Some(t) = totals.get_mut(parent) {
            t.left.add(&contribution.left);
            t.right.add(&contribution.right);
            if let Some(child) = own_totals {
                t.left.add(&child.left);
                t.right.add(&child.right);
            }
        }
    }
    for (id, node) in nodes(store) {
        if !node.is_folder() {
            continue;
        }
        let i = id as usize;
        if agg.get(i).copied().unwrap_or_default() != store.aggregates(node) {
            return fail(
                "INV-FOLDER-5",
                format!("folder {id}: aggregates differ from a recount"),
            );
        }
        if totals.get(i).copied().unwrap_or_default() != store.totals(node) {
            return fail(
                "INV-FOLDER-5",
                format!("folder {id}: totals differ from a recount"),
            );
        }
    }
    if content != store.content_counts {
        return fail(
            "INV-FOLDER-5",
            format!(
                "content counts {:?} differ from a recount {content:?}",
                store.content_counts
            ),
        );
    }
    Ok(())
}

/// INV-FOLDER-7: `filtered` flags agree with re-evaluating the filters; name-only entries occur only on sides
/// filtered by the name masks.
fn filters(store: &Store) -> Result<(), InvariantViolation> {
    let Some(rules) = store.resolved.as_ref() else {
        return Ok(());
    };
    // Path and verdict of every folder side (top-down: parents have smaller ids).
    let mut folders: BTreeMap<u32, [Option<(RelPath, FolderVerdict)>; 2]> = BTreeMap::new();
    folders.insert(
        0,
        [
            Some((RelPath::root(), FolderVerdict::Base)),
            Some((RelPath::root(), FolderVerdict::Base)),
        ],
    );
    for (id, node) in nodes(store).skip(1) {
        let mut own: [Option<(RelPath, FolderVerdict)>; 2] = [None, None];
        for (side, rec) in node.sides.iter().enumerate() {
            if !rec.present() {
                continue;
            }
            let parent = if store.flatten {
                let dir = rec
                    .dir()
                    .and_then(|d| store.dir_info(side, d))
                    .map_or("", |d| d.text.as_str());
                Some(path_verdict(rules, dir))
            } else {
                folders
                    .get(&node.parent)
                    .and_then(|p| p.get(side).cloned().flatten())
            };
            let Some((parent_path, parent_verdict)) = parent else {
                return fail(
                    "INV-FOLDER-7",
                    format!("node {id}: {} side without a parent side", side_of(side)),
                );
            };
            let name = store.name(side, rec);
            if rec.has(bits::NO_META) {
                // Only the name is known: the name masks decide (kind unknown, so the file masks); never name-only.
                let names_ok =
                    rules.names_accept(&parent_path, parent_verdict, name, EntryKind::Other);
                if rec.has(bits::STAT_SKIPPED) || rec.has(bits::FILTERED) == names_ok {
                    return fail(
                        "INV-FOLDER-7",
                        format!(
                            "node {id}: an entry without metadata has filtered = {} but the name masks say {}",
                            rec.has(bits::FILTERED),
                            !names_ok
                        ),
                    );
                }
                continue;
            }
            let kind = rec.kind();
            let names_ok = rules.names_accept(&parent_path, parent_verdict, name, kind);
            let other = !rec.has(bits::STAT_SKIPPED)
                && rules.other_filtered(kind, rec.size(), rec.mtime(), &rec.attrs());
            if rec.has(bits::FILTERED) != (!names_ok || other) {
                return fail(
                    "INV-FOLDER-7",
                    format!(
                        "node {id}: {} filtered = {} but the filters say {}",
                        side_of(side),
                        rec.has(bits::FILTERED),
                        !names_ok || other
                    ),
                );
            }
            if rec.has(bits::STAT_SKIPPED) && names_ok {
                return fail(
                    "INV-FOLDER-7",
                    format!("node {id}: a name-only entry the name masks accept"),
                );
            }
            if node.is_folder() && kind == EntryKind::Dir {
                let verdict = rules.folder_verdict(&parent_path, name, parent_verdict);
                let path = parent_path.join(name).unwrap_or(parent_path);
                if let Some(slot) = own.get_mut(side) {
                    *slot = Some((path, verdict));
                }
            }
        }
        if node.is_folder() {
            folders.insert(id, own);
        }
    }
    Ok(())
}

/// INV-FOLDER-8: a content result implies a file pair with both sides present, stat'ed and not filtered.
fn content(store: &Store) -> Result<(), InvariantViolation> {
    for (id, node) in nodes(store) {
        if node.content().is_none() {
            continue;
        }
        let ok = node.kind() == NodeKind::File
            && node.sides.iter().all(|s| {
                s.present()
                    && !s.has(bits::FILTERED)
                    && !s.has(bits::STAT_SKIPPED)
                    && !s.has(bits::NO_META)
            });
        if !ok {
            return fail("INV-FOLDER-8", format!("node {id} has a content result"));
        }
    }
    Ok(())
}

/// INV-FOLDER-9: `visible_rows` equals the recursive reference (two filters, every mode, both ignore-unimportant
/// values, Suppress Filters on and off).
fn visibility(store: &Store) -> Result<(), InvariantViolation> {
    for filter in [FolderDisplayFilter::All, FolderDisplayFilter::Differences] {
        for folders in FolderMode::ALL {
            for ignore_unimportant in [false, true] {
                for suppress_filters in [false, true] {
                    let opts = DisplayOptions {
                        filter,
                        folders,
                        ignore_unimportant,
                        suppress_filters,
                    };
                    let mut rows = Vec::new();
                    visible_rows_into(store, &opts, &|_| true, &mut rows);
                    if rows != reference_rows(store, &opts) {
                        return fail(
                            "INV-FOLDER-9",
                            format!("visible rows differ from the reference for {opts:?}"),
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

/// INV-FOLDER-10: a difference under ignore-unimportant is a difference without it; a node that is not a
/// difference without it has the same category with it.
fn monotonicity(store: &Store) -> Result<(), InvariantViolation> {
    for (id, node) in nodes(store).skip(1) {
        let without = store.category_of(node, false);
        let with = store.category_of(node, true);
        if with.is_difference() && !without.is_difference() {
            return fail(
                "INV-FOLDER-10",
                format!("node {id}: {with:?} with ignore-unimportant, {without:?} without"),
            );
        }
        if !without.is_difference() && with != without {
            return fail(
                "INV-FOLDER-10",
                format!("node {id}: {without:?} becomes {with:?}"),
            );
        }
        let unknown_or_error = match without {
            Category::Unknown | Category::Error => true,
            Category::Same
            | Category::LeftNewer
            | Category::RightNewer
            | Category::Different
            | Category::LeftOrphan
            | Category::RightOrphan
            | Category::Filtered => false,
        };
        if unknown_or_error && !node.is_folder() && with != without {
            return fail(
                "INV-FOLDER-10",
                format!("node {id}: unknown/error depends on ignore-unimportant"),
            );
        }
    }
    let _ = Side::Left;
    Ok(())
}
