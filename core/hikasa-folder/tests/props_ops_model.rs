//! M2 W6b acceptance A6.20, property P-OPS (T12), against a model: random trees × selection × operation × answers on
//! `MemoryVfs`. The tree after execution is compared, file by file, with what a small model of each verb says it must be,
//! computed from the tree before and the selection alone, never from the plan. A plan shows what runs: an item that the
//! plan let through and that did not run is a failure, and a share of the cases must run something.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};

use common::{config_with, engines, host, mspec, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, Side, Spec};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, FolderOp, OpScope, OpSettings, PathStructure,
    ReadOnlySides, Recycle, RenameMask, Reply, TouchFrom, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{
    FolderComparison, FolderModel, FolderRules, FolderRulesPatch, NodeId, ScanStatus, compare_into,
};
use hikasa_vfs::ops::{self, ItemStatus, OverwritePolicy, SkipReason};
use hikasa_vfs::{Attrs, EntryKind, MemMeta, MemoryVfs, UnixMeta};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

// ---- trees -----------------------------------------------------------------------------------------------------

const NAMES: [&str; 5] = ["a", "b", "c.txt", "d.txt", "e.log"];
/// The time of every folder: older than every file, so a folder that replaces a file replaces something newer.
const FOLDER_TIME: i64 = 500;
/// The time a touch sets.
const TOUCHED: i64 = 7_000_000;

#[derive(Debug, Clone)]
struct Node {
    name: usize,
    /// 0 a file, 1 a folder, per side.
    left: Option<u8>,
    right: Option<u8>,
    content: [u8; 2],
    mtime: [u8; 2],
    read_only: [bool; 2],
    children: Vec<Node>,
}

fn nodes() -> impl Strategy<Value = Vec<Node>> {
    let leaf = (
        0..NAMES.len(),
        proptest::option::weighted(0.8, 0u8..2),
        proptest::option::weighted(0.8, 0u8..2),
        any::<[u8; 2]>(),
        any::<[u8; 2]>(),
        (
            proptest::bool::weighted(0.15),
            proptest::bool::weighted(0.15),
        ),
    )
        .prop_map(|(name, left, right, content, mtime, (l, r))| Node {
            name,
            left,
            right,
            content,
            mtime,
            read_only: [l, r],
            children: Vec::new(),
        });
    let tree = leaf.prop_recursive(3, 24, 4, |inner| {
        (inner.clone(), proptest::collection::vec(inner, 0..4)).prop_map(|(mut node, children)| {
            node.children = children;
            node
        })
    });
    proptest::collection::vec(tree, 1..5)
}

fn meta(secs: i64, read_only: bool) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: if read_only { 0o444 } else { 0o644 },
                uid: 0,
                gid: 0,
            },
        }),
        ..MemMeta::default()
    }
}

fn insert(vfs: &MemoryVfs, dir: &str, nodes: &[Node], side: usize) {
    let mut seen = BTreeSet::new();
    for node in nodes {
        if !seen.insert(node.name) {
            continue;
        }
        let path = format!("{dir}/{}", NAMES[node.name]);
        match [node.left, node.right][side] {
            Some(0) => {
                let bytes = format!("{}{}", NAMES[node.name], node.content[side] % 3).into_bytes();
                let secs = 1_000_000 * (1 + i64::from(node.mtime[side] % 3));
                vfs.insert_file(&host(&path), bytes, meta(secs, node.read_only[side]))
                    .unwrap();
            }
            Some(_) => {
                vfs.insert_dir(&host(&path), meta(FOLDER_TIME, false))
                    .unwrap();
                insert(vfs, &path, &node.children, side);
            }
            None => {}
        }
    }
}

/// What the model compares: kind, bytes, a file's time and mode.
#[derive(Debug, Clone, PartialEq, Eq)]
enum It {
    Dir,
    File { data: Vec<u8>, mtime: i64, ro: bool },
}

type Tree = BTreeMap<String, It>;

/// `/l/a/b` from a host path or a spec.
fn plain(text: &str) -> String {
    let text = text.replace('\\', "/");
    text.strip_prefix("C:").map(str::to_owned).unwrap_or(text)
}

fn snapshot(vfs: &MemoryVfs) -> Tree {
    let mut tree = Tree::new();
    for entry in vfs.dump() {
        let path = plain(&entry.path);
        if path == "/" || path.is_empty() || path == "/l" || path == "/r" || path == "/o" {
            continue;
        }
        let it = match entry.kind {
            EntryKind::Dir => It::Dir,
            _ => It::File {
                data: vfs
                    .read_back(&entry.path)
                    .map(|b| b.to_vec())
                    .unwrap_or_default(),
                mtime: entry.mtime.map_or(0, |t| t.unix_seconds()),
                ro: matches!(entry.attrs, Attrs::Unix { meta } if meta.mode & 0o222 == 0),
            },
        };
        tree.insert(path, it);
    }
    tree
}

/// `copyto`: a folder at a file's target is a second source of that path when this operation made it; one that stood
/// there is rule 8. Either way the file stays.
fn made_folder(tree: &Tree, to: &str) -> bool {
    matches!(tree.get(to), Some(It::Dir))
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

// ---- the model -------------------------------------------------------------------------------------------------

/// How items are answered.
#[derive(Debug, Clone)]
enum Answers {
    Yes,
    /// `NoToAll`: whatever has a conflict stays.
    NoToConflicts,
    /// These targets were answered no.
    Declined(BTreeSet<String>),
}

struct Model {
    tree: Tree,
    filtered: bool,
    /// Folders the comparison did not list.
    unlisted: BTreeSet<String>,
    answers: Answers,
    /// `copyto`: file targets taken.
    taken: Option<BTreeSet<String>>,
}

impl Model {
    fn hidden(&self, path: &str) -> bool {
        self.filtered
            && !path.starts_with("/o/")
            && matches!(self.tree.get(path), Some(It::File { .. }))
            && !name_of(path).ends_with(".txt")
    }

    /// The names in the folder `path`, in the order of a comparison.
    fn children(&self, path: &str) -> Vec<String> {
        let prefix = format!("{path}/");
        let mut names: Vec<String> = self
            .tree
            .range(prefix.clone()..)
            .take_while(|(key, _)| key.starts_with(&prefix))
            .filter(|(key, _)| !key[prefix.len()..].contains('/'))
            .map(|(key, _)| key[prefix.len()..].to_owned())
            .collect();
        // The order of a comparison: folders first, then by name.
        names.sort_by_key(|name| {
            let file = !matches!(self.tree.get(&format!("{path}/{name}")), Some(It::Dir));
            (file, name.clone())
        });
        names
    }

    fn subtree(&self, path: &str) -> Vec<String> {
        let prefix = format!("{path}/");
        let mut all = vec![path.to_owned()];
        all.extend(
            self.tree
                .range(prefix.clone()..)
                .take_while(|(key, _)| key.starts_with(&prefix))
                .map(|(key, _)| key.clone()),
        );
        all
    }

    /// Nothing below `path` stays out of a whole-folder item: no file a filter hides, no folder the comparison did not list.
    fn whole(&self, path: &str) -> bool {
        self.subtree(path)
            .iter()
            .skip(1)
            .all(|p| !self.hidden(p) && !self.unlisted.contains(p))
    }

    fn declines(&self, target: &str, conflict: bool) -> bool {
        match &self.answers {
            Answers::Yes => false,
            Answers::NoToConflicts => conflict,
            Answers::Declined(set) => set.contains(target),
        }
    }

    fn remove_all(&mut self, path: &str) {
        for p in self.subtree(path) {
            self.tree.remove(&p);
        }
    }

    /// The folders above `to` below its base, made where missing; a file in the way of one is rule 8.
    fn parents(&mut self, to: &str, policy: OverwritePolicy, moving: bool) -> bool {
        let base_len = 1;
        let parts: Vec<&str> = to.split('/').filter(|p| !p.is_empty()).collect();
        let mut at = String::new();
        for (index, part) in parts.iter().enumerate().take(parts.len().saturating_sub(1)) {
            at.push('/');
            at.push_str(part);
            if index < base_len {
                continue;
            }
            match self.tree.get(&at).cloned() {
                Some(It::Dir) => {}
                None => {
                    self.tree.insert(at.clone(), It::Dir);
                }
                Some(It::File { .. }) if self.taken.as_ref().is_some_and(|t| t.contains(&at)) => {
                    return false;
                }
                Some(It::File { .. }) if self.hidden(&at) => return false,
                Some(It::File { mtime, ro, .. }) => {
                    let conflict = ro || mtime > FOLDER_TIME || moving;
                    if policy != OverwritePolicy::Always || self.declines(&at, conflict) {
                        return false;
                    }
                    self.tree.insert(at.clone(), It::Dir);
                }
            }
        }
        true
    }

    /// `from` onto `to`. True when nothing of the source is left.
    fn transfer(&mut self, from: &str, to: &str, moving: bool, policy: OverwritePolicy) -> bool {
        let Some(source) = self.tree.get(from).cloned() else {
            return true;
        };
        match source {
            It::File { mtime, .. } => {
                if self.hidden(from) {
                    return false;
                }
                if let Some(taken) = &mut self.taken
                    && (!taken.insert(to.to_owned()) || made_folder(&self.tree, to))
                {
                    return false;
                }
                match self.tree.get(to).cloned() {
                    Some(It::Dir) => return false,
                    Some(It::File {
                        mtime: theirs, ro, ..
                    }) => {
                        let replace = match policy {
                            OverwritePolicy::Fail | OverwritePolicy::Skip => false,
                            OverwritePolicy::Always => true,
                            OverwritePolicy::Newer => mtime > theirs + 2,
                        };
                        let conflict = ro || theirs > mtime + 2 || moving;
                        if !replace || self.declines(to, conflict) {
                            return false;
                        }
                    }
                    None => {
                        if !self.parents(to, policy, moving) {
                            return false;
                        }
                    }
                }
                self.tree.insert(to.to_owned(), source);
                if moving {
                    self.tree.remove(from);
                }
                true
            }
            It::Dir => {
                if self.taken.as_ref().is_some_and(|taken| taken.contains(to)) {
                    return false;
                }
                match self.tree.get(to).cloned() {
                    Some(It::Dir) => {}
                    Some(It::File { .. }) if self.hidden(to) => return false,
                    Some(It::File { mtime, ro, .. }) => {
                        let conflict = ro || mtime > FOLDER_TIME + 2 || moving;
                        if policy != OverwritePolicy::Always || self.declines(to, conflict) {
                            return false;
                        }
                        self.tree.insert(to.to_owned(), It::Dir);
                    }
                    None => {
                        if !self.parents(to, policy, moving) {
                            return false;
                        }
                        self.tree.insert(to.to_owned(), It::Dir);
                    }
                }
                let mut gone = true;
                for name in self.children(from) {
                    let child = format!("{from}/{name}");
                    let listed = !self.unlisted.contains(&child);
                    gone &=
                        self.transfer(&child, &format!("{to}/{name}"), moving, policy) && listed;
                }
                if moving && gone {
                    self.tree.remove(from);
                }
                moving && gone
            }
        }
    }

    /// A delete of `path`. True when it is gone.
    fn delete(&mut self, path: &str, bin: bool) -> bool {
        match self.tree.get(path).cloned() {
            None => true,
            Some(It::File { ro, .. }) => {
                if self.hidden(path) || self.declines(path, ro) {
                    return false;
                }
                self.tree.remove(path);
                true
            }
            Some(It::Dir) => {
                if bin && self.whole(path) {
                    let read_only = self
                        .subtree(path)
                        .iter()
                        .any(|p| matches!(self.tree.get(p), Some(It::File { ro: true, .. })));
                    if self.declines(path, read_only) {
                        return false;
                    }
                    self.remove_all(path);
                    return true;
                }
                let mut gone = true;
                for name in self.children(path) {
                    let child = format!("{path}/{name}");
                    let listed = !self.unlisted.contains(&child);
                    gone &= self.delete(&child, bin) && listed;
                }
                if gone {
                    self.tree.remove(path);
                }
                gone
            }
        }
    }

    /// An exchange of the row `rel` (with that kind) on the sides selected.
    fn exchange(&mut self, rel: &str, folder: bool, sides: SideSet) {
        let (l, r) = (format!("/l{rel}"), format!("/r{rel}"));
        let kind = |it: Option<&It>| it.map(|it| matches!(it, It::Dir));
        let (left, right) = (kind(self.tree.get(&l)), kind(self.tree.get(&r)));
        let mine = |k: Option<bool>| k == Some(folder);
        match (mine(left), mine(right)) {
            (true, true) if folder => {
                // A pair of folders the model never listed has no rows to swap: `skip compare_error`, as the plan says.
                if self.unlisted.contains(&l) || self.unlisted.contains(&r) {
                    return;
                }
                let mut names: BTreeSet<String> = self.children(&l).into_iter().collect();
                names.extend(self.children(&r));
                for name in names {
                    let child = format!("{rel}/{name}");
                    let kinds = [
                        kind(self.tree.get(&format!("/l{child}"))),
                        kind(self.tree.get(&format!("/r{child}"))),
                    ];
                    for wanted in [false, true] {
                        if kinds.contains(&Some(wanted)) {
                            self.exchange(&child, wanted, sides);
                        }
                    }
                }
            }
            (true, true) => {
                let (Some(It::File { ro: a, .. }), Some(It::File { ro: b, .. })) =
                    (self.tree.get(&l).cloned(), self.tree.get(&r).cloned())
                else {
                    return;
                };
                if self.hidden(&l) || self.declines(&r, a || b) {
                    return;
                }
                let (x, y) = (self.tree.remove(&l).unwrap(), self.tree.remove(&r).unwrap());
                self.tree.insert(l, y);
                self.tree.insert(r, x);
            }
            (true, false) if sides.contains(SideSet::LEFT) => {
                self.transfer(&l, &r, true, OverwritePolicy::Fail);
            }
            (false, true) if sides.contains(SideSet::RIGHT) => {
                self.transfer(&r, &l, true, OverwritePolicy::Fail);
            }
            _ => {}
        }
    }

    /// Renames of the rows `rows` (paths on one side, in plan order) by `rename`, by the table of §1.6.13.
    fn rename(&mut self, rows: &[String], rename: &dyn Fn(&str) -> String) {
        let mut folders: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for row in rows {
            let new = rename(name_of(row));
            if self.hidden(row) || new == name_of(row) {
                continue;
            }
            folders
                .entry(parent_of(row).to_owned())
                .or_default()
                .push((name_of(row).to_owned(), new));
        }
        for (parent, movers) in folders {
            let taken: BTreeSet<String> = self.children(&parent).into_iter().collect();
            let mut live: Vec<bool> = vec![true; movers.len()];
            let mut seen = BTreeSet::new();
            for (index, (_, to)) in movers.iter().enumerate() {
                if !seen.insert(to.clone()) {
                    live[index] = false;
                }
            }
            loop {
                let moving: BTreeSet<&String> = movers
                    .iter()
                    .zip(&live)
                    .filter(|(_, l)| **l)
                    .map(|((from, _), _)| from)
                    .collect();
                let blocked: Vec<usize> = movers
                    .iter()
                    .enumerate()
                    .filter(|(i, (_, to))| live[*i] && taken.contains(to) && !moving.contains(to))
                    .map(|(i, _)| i)
                    .collect();
                if blocked.is_empty() {
                    break;
                }
                for i in blocked {
                    live[i] = false;
                }
            }
            let mut parked = Vec::new();
            for ((from, to), live) in movers.iter().zip(&live) {
                if *live {
                    let path = format!("{parent}/{from}");
                    let held: Vec<(String, It)> = self
                        .subtree(&path)
                        .into_iter()
                        .filter_map(|p| {
                            let it = self.tree.remove(&p)?;
                            Some((format!("{parent}/{to}{}", &p[path.len()..]), it))
                        })
                        .collect();
                    parked.extend(held);
                }
            }
            self.tree.extend(parked);
        }
    }
}

// ---- the property --------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Case {
    nodes: Vec<Node>,
    picks: Vec<u8>,
    which: u8,
    answer: u8,
    replies: Vec<u8>,
    filtered: bool,
    unlisted: u8,
    outside: Vec<usize>,
}

fn cases() -> impl Strategy<Value = Case> {
    (
        nodes(),
        proptest::collection::vec(any::<u8>(), 1..12),
        any::<u8>(),
        any::<u8>(),
        proptest::collection::vec(any::<u8>(), 0..24),
        proptest::bool::weighted(0.25),
        0u8..4,
        proptest::collection::vec(0..NAMES.len(), 0..3),
    )
        .prop_map(
            |(nodes, picks, which, answer, replies, filtered, unlisted, outside)| Case {
                nodes,
                picks,
                which,
                answer,
                replies,
                filtered,
                unlisted,
                outside,
            },
        )
}

/// The verbs; `None` for a target folder: `/o/t`.
fn operation(which: u8, mkdir_parent: NodeId, mkdir_name: &str) -> FolderOp {
    let target = mspec("/o/t");
    let to = |structure, overwrite, moving| {
        if moving {
            FolderOp::MoveTo {
                sides: SideSet::all(),
                target: target.clone(),
                folder: false,
                structure,
                overwrite,
            }
        } else {
            FolderOp::CopyTo {
                sides: SideSet::all(),
                target: target.clone(),
                folder: false,
                structure,
                overwrite,
            }
        }
    };
    use OverwritePolicy::{Always, Fail, Newer, Skip};
    match which % 19 {
        0 => FolderOp::Copy {
            to: Side::Right,
            overwrite: Always,
        },
        1 => FolderOp::Copy {
            to: Side::Left,
            overwrite: Newer,
        },
        2 => FolderOp::Copy {
            to: Side::Right,
            overwrite: Skip,
        },
        3 => FolderOp::Copy {
            to: Side::Left,
            overwrite: Fail,
        },
        4 => FolderOp::Move {
            to: Side::Right,
            overwrite: Always,
        },
        5 => FolderOp::Move {
            to: Side::Left,
            overwrite: Fail,
        },
        6 => FolderOp::Move {
            to: Side::Right,
            overwrite: Newer,
        },
        7 => FolderOp::Delete {
            sides: SideSet::LEFT,
            recycle: Recycle::default(),
        },
        8 => FolderOp::Delete {
            sides: SideSet::all(),
            recycle: Recycle {
                use_bin: true,
                fallback: false,
            },
        },
        9 => FolderOp::Exchange,
        10 => FolderOp::Rename {
            sides: SideSet::LEFT,
            mask: RenameMask::Regex {
                old: "(.)(.*)".into(),
                new: "$2$1".into(),
            },
        },
        11 => FolderOp::Rename {
            sides: SideSet::all(),
            mask: RenameMask::Regex {
                old: "(.*)".into(),
                new: "a".into(),
            },
        },
        12 => FolderOp::Touch {
            sides: SideSet::all(),
            from: TouchFrom::At(FileTime::from_unix(TOUCHED, 0).unwrap()),
        },
        13 => FolderOp::Mkdir {
            sides: SideSet::all(),
            parent: mkdir_parent,
            name: mkdir_name.to_owned(),
            folder: true,
        },
        14 => to(PathStructure::Base, Always, false),
        15 => to(PathStructure::Relative, Fail, false),
        16 => to(PathStructure::None, Always, true),
        17 => to(PathStructure::Relative, Skip, true),
        _ => to(PathStructure::None, Skip, false),
    }
}

/// One row: its path below the bases, whether it is a folder, the sides selected.
#[derive(Debug, Clone)]
struct Row {
    id: NodeId,
    rel: String,
    folder: bool,
    sides: SideSet,
}

/// Rows picked by `picks`, none below another: what each one stands for is all of it.
fn pick(cmp: &FolderComparison, picks: &[u8]) -> Vec<Row> {
    let read = cmp.read();
    let mut rows: Vec<Row> = Vec::new();
    for (index, (id, _)) in read.preorder().enumerate() {
        let pick = picks[index % picks.len()];
        let sides = match pick % 5 {
            1 => SideSet::LEFT,
            2 => SideSet::RIGHT,
            3 | 4 => SideSet::all(),
            _ => continue,
        };
        let rel = format!("/{}", read.path(id).as_str());
        if rows
            .iter()
            .any(|row| rel.starts_with(&format!("{}/", row.rel)))
        {
            continue;
        }
        let Some(view) = read.node(id) else { continue };
        rows.push(Row {
            id,
            rel,
            folder: view.is_folder(),
            sides,
        });
        if rows.len() >= 3 {
            break;
        }
    }
    rows
}

/// What a row is on `side`: present, of its own kind.
fn on(model: &Model, base: &str, row: &Row) -> Option<String> {
    let path = format!("{base}{}", row.rel);
    let it = model.tree.get(&path)?;
    (matches!(it, It::Dir) == row.folder).then_some(path)
}

/// The tree the model says `op` leaves.
fn expected(model: &mut Model, op: &FolderOp, rows: &[Row], mkdir: (&str, &str)) {
    let base = |side: Side| if side == Side::Left { "/l" } else { "/r" };
    match op {
        FolderOp::Copy { to, overwrite } | FolderOp::Move { to, overwrite } => {
            let moving = matches!(op, FolderOp::Move { .. });
            let from = to.opposite();
            for row in rows.iter().filter(|r| r.sides.contains(SideSet::of(from))) {
                if let Some(path) = on(model, base(from), row) {
                    model.transfer(
                        &path,
                        &format!("{}{}", base(*to), row.rel),
                        moving,
                        *overwrite,
                    );
                }
            }
        }
        FolderOp::Delete { sides, recycle } => {
            for side in sides.sides() {
                for row in rows.iter().filter(|r| r.sides.contains(SideSet::of(side))) {
                    if let Some(path) = on(model, base(side), row) {
                        model.delete(&path, recycle.use_bin);
                    }
                }
            }
        }
        FolderOp::Exchange => {
            for row in rows {
                model.exchange(&row.rel, row.folder, row.sides);
            }
        }
        FolderOp::Rename { sides, mask } => {
            let rename: Box<dyn Fn(&str) -> String> = match mask {
                RenameMask::Regex { new, .. } if new == "a" => Box::new(|_: &str| "a".to_owned()),
                _ => Box::new(|name: &str| {
                    let mut chars = name.chars();
                    let first = chars.next().map(String::from).unwrap_or_default();
                    format!("{}{first}", chars.as_str())
                }),
            };
            for side in sides.sides() {
                let paths: Vec<String> = rows
                    .iter()
                    .filter(|r| r.sides.contains(SideSet::of(side)))
                    .filter_map(|r| on(model, base(side), r))
                    .collect();
                model.rename(&paths, &*rename);
            }
        }
        FolderOp::Touch { sides, .. } => {
            for side in sides.sides() {
                for row in rows.iter().filter(|r| r.sides.contains(SideSet::of(side))) {
                    let Some(path) = on(model, base(side), row) else {
                        continue;
                    };
                    for p in model.subtree(&path) {
                        // Rule 14 touches rows: what a folder the model never listed holds is no row (§1.6.25).
                        let unlisted = model
                            .unlisted
                            .iter()
                            .any(|u| p.starts_with(&format!("{u}/")));
                        let hidden = model.hidden(&p) || unlisted;
                        if let Some(It::File { mtime, .. }) = model.tree.get_mut(&p)
                            && !hidden
                        {
                            *mtime = TOUCHED;
                        }
                    }
                }
            }
        }
        FolderOp::Mkdir { sides, .. } => {
            let (parent, name) = mkdir;
            for side in sides.sides() {
                let target = format!("{}{parent}/{name}", base(side));
                let source = format!("{}{parent}", base(side.opposite()));
                if !parent.is_empty()
                    && !matches!(
                        model.tree.get(&format!("{}{parent}", base(side))),
                        Some(It::Dir)
                    )
                {
                    // Rule 3's chain from the side that has the parent.
                    if !matches!(model.tree.get(&source), Some(It::Dir))
                        || !model.parents(&target, OverwritePolicy::Fail, false)
                    {
                        continue;
                    }
                }
                model.tree.entry(target).or_insert(It::Dir);
            }
        }
        FolderOp::CopyTo {
            sides,
            structure,
            overwrite,
            ..
        }
        | FolderOp::MoveTo {
            sides,
            structure,
            overwrite,
            ..
        } => {
            let moving = matches!(op, FolderOp::MoveTo { .. });
            model.taken = Some(BTreeSet::new());
            for side in sides.sides() {
                let mine: Vec<(String, Vec<String>)> = rows
                    .iter()
                    .filter(|r| r.sides.contains(SideSet::of(side)))
                    .filter_map(|r| on(model, base(side), r).map(|p| (p, r.rel.clone())))
                    .map(|(p, rel)| {
                        (
                            p,
                            rel.split('/')
                                .filter(|n| !n.is_empty())
                                .map(str::to_owned)
                                .collect(),
                        )
                    })
                    .collect();
                let parents: Vec<&[String]> = mine.iter().map(|(_, n)| &n[..n.len() - 1]).collect();
                let common = match parents.first() {
                    Some(first) => parents
                        .iter()
                        .map(|p| {
                            p.iter()
                                .zip(first.iter())
                                .take_while(|(a, b)| a == b)
                                .count()
                        })
                        .min()
                        .unwrap_or(0),
                    None => 0,
                };
                for (path, names) in &mine {
                    match structure {
                        PathStructure::None => {
                            model.flat(path, moving, *overwrite);
                        }
                        _ => {
                            let drop = if *structure == PathStructure::Base {
                                0
                            } else {
                                common
                            };
                            let to = format!("/o/t/{}", names[drop..].join("/"));
                            model.transfer(path, &to, moving, *overwrite);
                        }
                    }
                }
            }
        }
    }
}

impl Model {
    /// `structure none`: every file of `path` by its name in `/o/t`, the first of a name only; a folder goes when all of it
    /// has.
    fn flat(&mut self, path: &str, moving: bool, policy: OverwritePolicy) -> bool {
        match self.tree.get(path).cloned() {
            None => true,
            Some(It::File { .. }) => {
                let to = format!("/o/t/{}", name_of(path));
                self.transfer(path, &to, moving, policy)
            }
            Some(It::Dir) => {
                let mut gone = true;
                for name in self.children(path) {
                    let child = format!("{path}/{name}");
                    let listed = !self.unlisted.contains(&child);
                    gone &= self.flat(&child, moving, policy) && listed;
                }
                if moving && gone {
                    self.tree.remove(path);
                }
                moving && gone
            }
        }
    }
}

fn rules_of(case: &Case) -> FolderRules {
    let mut rules = FolderRules::default();
    if case.filtered {
        let patch = FolderRulesPatch {
            name_filter: Some("*.txt".to_owned()),
            ..FolderRulesPatch::default()
        };
        rules.apply(&patch).unwrap();
    }
    rules.handling.scan_orphan_folders = case.unlisted & 1 == 0;
    rules.handling.auto_scan_subfolders = case.unlisted & 2 == 0;
    rules
}

fn compared(vfs: &MemoryVfs, rules: &FolderRules) -> FolderComparison {
    let engines = engines(1 << 30);
    with_env(vfs, &engines, &CancelToken::new(), |env| {
        let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
        compare_into(&model, env).unwrap()
    })
}

fn reply(byte: u8) -> Reply {
    match byte % 4 {
        0 => Reply::Yes,
        1 => Reply::No,
        2 => Reply::YesToAll,
        _ => Reply::NoToAll,
    }
}

/// True when `spec` lies below the folder item `folder`.
fn below(spec: &Spec, folder: &Spec) -> bool {
    std::iter::successors(spec.parent(), Spec::parent).any(|dir| dir == *folder)
}

#[test]
fn p_ops_the_tree_is_what_the_model_says() {
    let config = config_with("tests/proptest-regressions/props_ops_model.txt", 384);
    let total = config.cases;
    let mut runner = TestRunner::new(config);
    let ran = AtomicU32::new(0);
    let seen = AtomicU32::new(0);
    runner
        .run(&cases(), |case| {
            seen.fetch_add(1, Ordering::Relaxed);
            let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
            for base in ["/l", "/r", "/o"] {
                vfs.insert_dir(&host(base), meta(FOLDER_TIME, false)).unwrap();
            }
            insert(&vfs, "/l", &case.nodes, 0);
            insert(&vfs, "/r", &case.nodes, 1);
            for name in &case.outside {
                let path = format!("/o/t/{}", NAMES[*name]);
                vfs.insert_dir(&host("/o/t"), meta(FOLDER_TIME, false)).ok();
                vfs.insert_file(&host(&path), b"outside".to_vec(), meta(4_000_000, false))
                    .ok();
            }
            let rules = rules_of(&case);
            let cmp = compared(&vfs, &rules);
            let rows = pick(&cmp, &case.picks);
            let read = cmp.read();
            let unlisted: BTreeSet<String> = read
                .preorder()
                .filter(|(id, _)| read.node(*id).is_some_and(|v| v.is_folder() && v.scan == ScanStatus::NotScanned))
                .flat_map(|(id, _)| {
                    let rel = read.path(id).as_str().to_owned();
                    let view = read.node(id);
                    [(Side::Left, "/l"), (Side::Right, "/r")]
                        .into_iter()
                        .filter(|(side, _)| view.as_ref().is_some_and(|v| v.side(*side).is_some()))
                        .map(|(_, base)| format!("{base}/{rel}"))
                        .collect::<Vec<_>>()
                })
                .collect();
            // `mkdir`: into the first folder row picked, or the base.
            let parent = rows.iter().find(|r| r.folder);
            let mkdir_parent = parent.map_or(NodeId::ROOT, |r| r.id);
            let mkdir_rel = parent.map_or(String::new(), |r| r.rel.clone());
            let mkdir_name = if case.which % 2 == 0 { "new" } else { NAMES[usize::from(case.answer) % NAMES.len()] };
            let op = operation(case.which, mkdir_parent, mkdir_name);
            let mut selection = Selection::default();
            for row in &rows {
                selection.insert(row.id, row.sides);
            }
            let scope = OpScope {
                selection,
                include: Include::visible(),
                iu: false,
                read_only: ReadOnlySides::default(),
            };
            let before = snapshot(&vfs);
            let planned = plan_op(&read, &vfs, &scope, &op, &OpSettings::default(), &Ctl::detached());
            drop(read);
            prop_assert_eq!(&snapshot(&vfs), &before, "a plan wrote");
            let Ok(planned) = planned else {
                return Ok(());
            };
            let (answers, planned) = match case.answer % 3 {
                0 => {
                    let machine = Confirmation::new(&planned, &ConfirmPolicy::ALL, ConfirmMode::YesToAll);
                    (Answers::Yes, machine.finish(planned).unwrap())
                }
                1 => {
                    let machine = Confirmation::new(&planned, &ConfirmPolicy::ALL, ConfirmMode::NoToAll);
                    (Answers::NoToConflicts, machine.finish(planned).unwrap())
                }
                _ => {
                    let mut machine = Confirmation::new(&planned, &ConfirmPolicy::ALL, ConfirmMode::Prompt);
                    let mut replies = case.replies.iter();
                    while let Some(question) = machine.next() {
                        let answer = match question.kind {
                            hikasa_folder::ops::QuestionKind::Operation => Reply::Yes,
                            _ => replies.next().map_or(Reply::No, |byte| reply(*byte)),
                        };
                        machine.reply(answer);
                    }
                    let done = machine.finish(planned).unwrap();
                    let declined = done
                        .plan
                        .items
                        .iter()
                        .zip(&done.meta)
                        .filter(|(_, meta)| meta.answer == Some(false))
                        .map(|(item, _)| plain(&item.target.canonical()))
                        .collect();
                    (Answers::Declined(declined), done)
                }
            };
            let dry = ops::dry_run(planned.plan.clone());
            let report = ops::execute_with(&vfs, &vfs, planned.plan, &planned.exec, &Ctl::detached());
            // What the plan let through runs: no fault is injected, so nothing fails that the plan did not show.
            let failed: Vec<&hikasa_vfs::ops::PlanItem> = report
                .plan
                .items
                .iter()
                .zip(&report.results)
                .filter(|(item, result)| {
                    result.status == ItemStatus::Failed && item.kind == EntryKind::Dir
                })
                .map(|(item, _)| item)
                .collect();
            for ((item, result), shown) in report.plan.items.iter().zip(&report.results).zip(&dry.results) {
                let line = format!("{:?} {} {:?}", item.action, item.target.canonical(), result);
                match result.status {
                    ItemStatus::NotRun => {
                        let cut = failed.iter().any(|folder| {
                            below(&item.target, &folder.target)
                                || folder.source.as_ref().is_some_and(|s| item.source.as_ref().is_some_and(|i| below(i, s)))
                        });
                        prop_assert!(cut, "not run with nothing failed above it: {line}");
                    }
                    ItemStatus::Failed => {
                        prop_assert!(shown.status == ItemStatus::Failed, "failed, and the plan did not say so: {line}");
                    }
                    ItemStatus::Skipped => {
                        let why = result.skip.or(item.skip);
                        prop_assert!(
                            shown.status == ItemStatus::Planned
                                && (item.skip.is_some() || why == Some(SkipReason::ChildrenFailed)),
                            "skipped where the plan ran it: {line}"
                        );
                    }
                    ItemStatus::Done | ItemStatus::Planned => {}
                }
            }
            if report.results.iter().any(|r| r.status == ItemStatus::Done) {
                ran.fetch_add(1, Ordering::Relaxed);
            }
            let mut model = Model {
                tree: before,
                filtered: case.filtered,
                unlisted,
                answers,
                taken: None,
            };
            expected(&mut model, &op, &rows, (&mkdir_rel, mkdir_name));
            let after = snapshot(&vfs);
            if after != model.tree {
                let extra: Vec<_> = after.iter().filter(|(k, v)| model.tree.get(*k) != Some(v)).collect();
                let missing: Vec<_> = model.tree.iter().filter(|(k, v)| after.get(*k) != Some(v)).collect();
                let items: Vec<String> = report
                    .plan
                    .items
                    .iter()
                    .zip(&report.results)
                    .map(|(i, r)| format!("{:?} {:?} -> {} {:?} {:?}", i.action, i.source.as_ref().map(Spec::canonical), i.target.canonical(), r.status, r.skip.or(i.skip)))
                    .collect();
                prop_assert!(false, "{op:?}\nrows {rows:?}\nnot in the model: {extra:#?}\nnot on disk: {missing:#?}\nitems {items:#?}");
            }
            Ok(())
        })
        .unwrap();
    let seen = seen.load(Ordering::Relaxed).max(1);
    let ran = ran.load(Ordering::Relaxed);
    eprintln!("P-OPS: {ran} of {seen} cases ran an item");
    // A property that runs nothing proves nothing: a good share of the cases must run an item (about 55 % do).
    assert!(
        ran * 5 >= seen.min(total) * 2,
        "only {ran} of {seen} cases ran an item"
    );
}

// ---- aligned folder pairs (D-194) -----------------------------------------------------------------------------

/// Folder names by their two spellings (left, right): a case pair, an NFC/NFD pair, and one spelled alike.
const PAIRS: [(&str, &str); 3] = [("D", "d"), ("caf\u{e9}", "cafe\u{301}"), ("e", "e")];
/// File names: the same on both sides.
const FILES: [&str; 2] = ["x.txt", "y.txt"];

/// A node of a pair tree: a folder (one of `PAIRS`, spelled by `spell` per side) or a file (one of `FILES`).
#[derive(Debug, Clone)]
struct PairNode {
    folder: bool,
    name: usize,
    /// Present on the left, on the right.
    on: [bool; 2],
    /// Per side, whether the folder is spelled the other way.
    spell: [bool; 2],
    content: [u8; 2],
    children: Vec<PairNode>,
}

impl PairNode {
    fn name(&self, side: usize) -> &'static str {
        if self.folder {
            let (a, b) = PAIRS[self.name];
            if self.spell[side] { b } else { a }
        } else {
            FILES[self.name]
        }
    }
}

fn pair_nodes() -> impl Strategy<Value = Vec<PairNode>> {
    let leaf = (
        any::<bool>(),
        0..PAIRS.len(),
        prop_oneof![
            Just([true, true]),
            Just([true, false]),
            Just([false, true]),
            Just([true, true])
        ],
        any::<[bool; 2]>(),
        any::<[u8; 2]>(),
    )
        .prop_map(|(folder, name, on, spell, content)| PairNode {
            folder,
            name: if folder { name } else { name % FILES.len() },
            on,
            spell,
            content,
            children: Vec::new(),
        });
    let tree = leaf.prop_recursive(3, 16, 4, |inner| {
        (inner.clone(), proptest::collection::vec(inner, 0..4)).prop_map(|(mut node, children)| {
            if node.folder {
                node.children = children;
            }
            node
        })
    });
    proptest::collection::vec(tree, 1..4).prop_map(|mut nodes| {
        for node in &mut nodes {
            within(node, [true, true]);
        }
        nodes
    })
}

/// A node is on a side only where its folder is.
fn within(node: &mut PairNode, parent: [bool; 2]) {
    node.on = [node.on[0] && parent[0], node.on[1] && parent[1]];
    let on = node.on;
    for child in &mut node.children {
        within(child, on);
    }
}

/// Writes the nodes of `side` below `dir`; siblings of one name (whatever the spelling) count once, on both sides alike.
fn pair_insert(vfs: &MemoryVfs, dir: &str, nodes: &[PairNode], side: usize) {
    for node in pair_unique(nodes) {
        if !node.on[side] {
            continue;
        }
        let path = format!("{dir}/{}", node.name(side));
        if node.folder {
            vfs.insert_dir(&host(&path), meta(FOLDER_TIME, false))
                .unwrap();
            pair_insert(vfs, &path, &node.children, side);
        } else {
            let bytes = format!("{}{}", side, node.content[side] % 3).into_bytes();
            let secs = 1_000_000 * (1 + i64::from(node.content[side] % 2));
            vfs.insert_file(&host(&path), bytes, meta(secs, false))
                .unwrap();
        }
    }
}

/// The first node of each (kind, name).
fn pair_unique(nodes: &[PairNode]) -> Vec<&PairNode> {
    let mut seen = BTreeSet::new();
    nodes
        .iter()
        .filter(|node| seen.insert((node.folder, node.name)))
        .collect()
}

/// The model of `copyto`/`moveto` of whole rows below `p`, overwrite always, onto a case-sensitive `/o/t` that holds
/// folders only: each aligned pair is one folder, named by rule 16 (D-194); a second file of one path stays.
struct PairModel<'a> {
    before: &'a Tree,
    tree: Tree,
    taken: BTreeSet<String>,
    /// The name chosen for a pair below a target folder.
    chosen: BTreeMap<(String, usize), &'static str>,
    sides: SideSet,
    moving: bool,
}

impl PairModel<'_> {
    /// The folder of `node` in the target folder `to`: rule 16's choice when the two sides spell it differently.
    fn folder_name(&mut self, node: &PairNode, to: &str, side: usize) -> &'static str {
        let (left, right) = (node.name(0), node.name(1));
        if !(node.on[0] && node.on[1]) || left == right {
            return node.name(side);
        }
        let key = (to.to_owned(), node.name);
        if let Some(name) = self.chosen.get(&key) {
            return name;
        }
        let stands = |name: &str| matches!(self.before.get(&format!("{to}/{name}")), Some(It::Dir));
        let name = if stands(left) {
            left
        } else if stands(right) {
            right
        } else if self.sides.contains(SideSet::LEFT) {
            left
        } else {
            right
        };
        self.chosen.insert(key, name);
        name
    }

    /// True when nothing of the source is left.
    fn place(&mut self, node: &PairNode, from: &str, to_dir: &str, side: usize) -> bool {
        if node.folder {
            let to = format!("{to_dir}/{}", self.folder_name(node, to_dir, side));
            self.tree.entry(to.clone()).or_insert(It::Dir);
            let mut gone = true;
            for child in pair_unique(&node.children) {
                if child.on[side] {
                    gone &= self.place(child, &format!("{from}/{}", child.name(side)), &to, side);
                }
            }
            if self.moving && gone {
                self.tree.remove(from);
            }
            self.moving && gone
        } else {
            let to = format!("{to_dir}/{}", node.name(side));
            if !self.taken.insert(to.clone()) {
                return false;
            }
            let it = self.tree.get(from).cloned().expect("a source file");
            self.tree.insert(to, it);
            if self.moving {
                self.tree.remove(from);
            }
            true
        }
    }
}

#[derive(Debug, Clone)]
struct PairRun {
    nodes: Vec<PairNode>,
    /// Per top-level node: 0 nothing at the target, 1 the left's spelling, 2 the right's, 3 both.
    stands: Vec<u8>,
    sides: u8,
    relative: bool,
    moving: bool,
}

fn pair_runs() -> impl Strategy<Value = PairRun> {
    (
        pair_nodes(),
        proptest::collection::vec(0u8..4, 4),
        0u8..3,
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(nodes, stands, sides, relative, moving)| PairRun {
            nodes,
            stands,
            sides,
            relative,
            moving,
        })
}

/// A7.1, P-OPS for aligned folder pairs: trees whose folders differ in case and NFC/NFD between the sides, compared
/// case-insensitively with `align_normalization`, `copyto`/`moveto` with structure `base` or `relative` onto a case-sensitive
/// target that may already hold one spelling or both. The tree after execution is the model's, which places both sides of
/// a pair under the one folder rule 16 chooses; nothing fails.
#[test]
fn p_ops_aligned_pairs_land_in_one_folder() {
    let config = config_with("tests/proptest-regressions/props_ops_model_pairs.txt", 256);
    let mut runner = TestRunner::new(config);
    let paired = AtomicU32::new(0);
    runner
        .run(&pair_runs(), |run| {
            let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
            for dir in ["/l", "/r", "/o", "/l/p", "/r/p"] {
                vfs.insert_dir(&host(dir), meta(FOLDER_TIME, false)).unwrap();
            }
            pair_insert(&vfs, "/l/p", &run.nodes, 0);
            pair_insert(&vfs, "/r/p", &run.nodes, 1);
            let above = if run.relative { "/o/t" } else { "/o/t/p" };
            for (node, stands) in pair_unique(&run.nodes).into_iter().zip(&run.stands) {
                if !node.folder {
                    continue;
                }
                for (bit, side) in [(1, 0), (2, 1)] {
                    if stands & bit != 0 {
                        for dir in ["/o/t", "/o/t/p"].into_iter().filter(|d| above.starts_with(d)) {
                            vfs.insert_dir(&host(dir), meta(FOLDER_TIME, false)).ok();
                        }
                        let path = format!("{above}/{}", node.name(side));
                        vfs.insert_dir(&host(&path), meta(FOLDER_TIME, false)).ok();
                    }
                }
            }
            let mut rules = FolderRules::default();
            rules.alignment.case = hikasa_folder::CaseMode::Insensitive;
            rules.alignment.align_normalization = true;
            let cmp = compared(&vfs, &rules);
            let read = cmp.read();
            let sides = match run.sides {
                0 => SideSet::all(),
                1 => SideSet::LEFT,
                _ => SideSet::RIGHT,
            };
            let p = read.children(NodeId::ROOT).next().expect("p");
            let mut selection = Selection::default();
            for child in read.children(p) {
                selection.insert(child, sides);
            }
            let structure = if run.relative { PathStructure::Relative } else { PathStructure::Base };
            let target = mspec("/o/t");
            let overwrite = OverwritePolicy::Always;
            let op = if run.moving {
                FolderOp::MoveTo { sides, target, folder: false, structure, overwrite }
            } else {
                FolderOp::CopyTo { sides, target, folder: false, structure, overwrite }
            };
            let scope = OpScope {
                selection,
                include: Include::visible(),
                iu: false,
                read_only: ReadOnlySides::default(),
            };
            let before = snapshot(&vfs);
            let planned = plan_op(&read, &vfs, &scope, &op, &OpSettings::default(), &Ctl::detached());
            drop(read);
            let Ok(planned) = planned else {
                // Nothing on the sides asked for.
                return Ok(());
            };
            let machine = Confirmation::new(&planned, &ConfirmPolicy::ALL, ConfirmMode::YesToAll);
            let planned = machine.finish(planned).unwrap();
            let report = ops::execute_with(&vfs, &vfs, planned.plan, &planned.exec, &Ctl::detached());
            let items: Vec<String> = report
                .plan
                .items
                .iter()
                .zip(&report.results)
                .map(|(i, r)| format!("{:?} {:?} -> {} {:?} {:?}", i.action, i.source.as_ref().map(Spec::canonical), i.target.canonical(), r.status, r.skip.or(i.skip)))
                .collect();
            prop_assert!(
                report.results.iter().all(|r| r.status != ItemStatus::Failed),
                "an item failed: {items:#?}"
            );
            let mut model = PairModel {
                before: &before,
                tree: before.clone(),
                taken: BTreeSet::new(),
                chosen: BTreeMap::new(),
                sides,
                moving: run.moving,
            };
            for (side, base) in [(0, "/l/p"), (1, "/r/p")] {
                if !sides.contains(if side == 0 { SideSet::LEFT } else { SideSet::RIGHT }) {
                    continue;
                }
                for node in pair_unique(&run.nodes) {
                    if node.on[side] {
                        model.tree.entry("/o/t".to_owned()).or_insert(It::Dir);
                        if !run.relative {
                            model.tree.entry("/o/t/p".to_owned()).or_insert(It::Dir);
                        }
                        let from = format!("{base}/{}", node.name(side));
                        model.place(node, &from, above, side);
                    }
                }
            }
            if !model.chosen.is_empty() {
                paired.fetch_add(1, Ordering::Relaxed);
            }
            let after = snapshot(&vfs);
            if after != model.tree {
                let extra: Vec<_> = after.iter().filter(|(k, v)| model.tree.get(*k) != Some(v)).collect();
                let missing: Vec<_> = model.tree.iter().filter(|(k, v)| after.get(*k) != Some(v)).collect();
                prop_assert!(false, "{op:?}\nnot in the model: {extra:#?}\nnot on disk: {missing:#?}\nitems {items:#?}");
            }
            Ok(())
        })
        .unwrap();
    // A property that meets no pair proves nothing.
    assert!(
        paired.load(Ordering::Relaxed) > 0,
        "no case held an aligned pair"
    );
}
