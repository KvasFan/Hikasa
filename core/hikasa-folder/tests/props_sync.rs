//! M2 W6c acceptance A6.29, property P-SYNC (T12): random trees × presets and custom matrices × options on `MemoryVfs`.
//!
//! The tree after a sync is compared, file by file, with what a small model of §1.6.14 says it must be, computed from the
//! tree before and the options alone, never from the plan. The model knows the row algorithm: scope (*Just selection*),
//! rows the comparison could not read (rule 2), a side a filter hides and a folder hidden on one side (rule 3), Ignored
//! rows (`same`), overrides, read-only sides, conflicts, case-insensitive alignment, links, and the empty-source refusal.
//! Also: planning writes nothing; the items a dry run shows are the items that run, with the status the dry run gave them;
//! after a run a re-comparison shows each copied row the same and each deleted row gone; a second plan, on the tree the
//! first left, is the model's again (and plans nothing that runs for a preset); a comparison that did not finish never
//! deletes; and a refresh of the comparison equals a fresh one (P-REFRESH). Shares of the cases must really run, delete,
//! hide one side of a pair, and hold links.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use common::{CancelAfter, config_with, default_now, engines, host, mspec, run_on, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, RelPath, Side};
use hikasa_folder::ops::{ConfirmMode, ConfirmPolicy, Confirmation, OpSettings, ReadOnlySides};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::sync::{
    OrphanReading, Situation, SyncAction, SyncMatrix, SyncOptions, SyncPreset, SyncRequest,
    plan_sync, sync_folder_rules,
};
use hikasa_folder::{
    CaseMode, Category, Cmp, FolderComparison, FolderError, FolderModel, FolderRules,
    FolderRulesPatch, NodeId, SizeFilter, TreeRead, compare_into, refresh_into,
};
use hikasa_vfs::ops::{self, ItemStatus, SkipReason};
use hikasa_vfs::{Attrs, DosAttrs, EntryKind, Fault, MemMeta, MemoryVfs, VfsOp};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

// ---- trees -----------------------------------------------------------------------------------------------------

const NAMES: [&str; 5] = ["a", "b", "c.txt", "d.txt", "e.log"];
const FOLDER_TIME: i64 = 500;
/// The size filter hides files of at most this many bytes: content 0.
const SMALL: u64 = 5;

#[derive(Debug, Clone)]
struct Node {
    name: usize,
    /// 0 a file, 1 a folder, per side.
    left: Option<u8>,
    right: Option<u8>,
    /// A link on each side it is on (a folder side becomes a link too): links pair with links only.
    link: bool,
    /// The right side spells the name with a capital first letter (one row under a case-insensitive comparison, two
    /// otherwise).
    upper: bool,
    content: [u8; 2],
    mtime: [u8; 2],
    children: Vec<Node>,
}

fn nodes() -> impl Strategy<Value = Vec<Node>> {
    let leaf = (
        0..NAMES.len(),
        proptest::option::weighted(0.75, 0u8..2),
        proptest::option::weighted(0.75, 0u8..2),
        (proptest::bool::weighted(0.12), proptest::bool::weighted(0.35)),
        (0u8..3, 0u8..3),
        (0u8..3, 0u8..3),
    )
        .prop_map(|(name, left, right, (link, upper), (cl, cr), (ml, mr))| Node {
            name,
            left,
            right,
            link,
            upper,
            content: [cl, cr],
            mtime: [ml, mr],
            children: Vec::new(),
        });
    let tree = leaf.prop_recursive(3, 24, 4, |inner| {
        (inner.clone(), proptest::collection::vec(inner, 0..4)).prop_map(|(mut node, children)| {
            node.children = children;
            node
        })
    });
    proptest::collection::vec(tree, 1..6)
}

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

/// The bytes of a content index: lengths differ, so equal sizes are equal contents; content 0 is at most [`SMALL`] bytes,
/// the others are larger.
fn data(name: &str, content: u8) -> Vec<u8> {
    format!("{name}{}", "x".repeat(8 * usize::from(content))).into_bytes()
}

fn spelt(node: &Node, side: usize) -> String {
    let name = NAMES[node.name];
    if side == 1 && node.upper {
        let mut chars = name.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    } else {
        name.to_owned()
    }
}

/// The folders made with the protected attributes (SYSTEM and HIDDEN): the default filter hides them on that side.
fn insert(vfs: &MemoryVfs, dir: &str, nodes: &[Node], side: usize, protect: &BTreeSet<String>) {
    let mut seen = BTreeSet::new();
    for node in nodes {
        // One entry per name and side, whatever its spelling: two names of one key are not generated.
        if !seen.insert(node.name) {
            continue;
        }
        let path = format!("{dir}/{}", spelt(node, side));
        let secs = 1_000_000 * (1 + i64::from(node.mtime[side]));
        match [node.left, node.right][side] {
            Some(_) if node.link => {
                let target = format!("t{}", node.content[side]);
                vfs.insert_symlink(&host(&path), &target, meta(secs)).unwrap();
            }
            Some(0) => {
                vfs.insert_file(&host(&path), data(NAMES[node.name], node.content[side]), meta(secs))
                    .unwrap();
            }
            Some(_) => {
                let attrs = protect.contains(&path).then_some(Attrs::Dos {
                    attrs: DosAttrs::SYSTEM | DosAttrs::HIDDEN,
                });
                vfs.insert_dir(
                    &host(&path),
                    MemMeta {
                        attrs,
                        ..meta(FOLDER_TIME)
                    },
                )
                .unwrap();
                insert(vfs, &path, &node.children, side, protect);
            }
            None => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum It {
    Dir,
    File { data: Vec<u8>, mtime: i64 },
    Link { target: String, mtime: i64 },
}

impl It {
    fn mtime(&self) -> Option<i64> {
        match self {
            It::Dir => None,
            It::File { mtime, .. } | It::Link { mtime, .. } => Some(*mtime),
        }
    }
}

type Tree = BTreeMap<String, It>;

fn plain(text: &str) -> String {
    let text = text.replace('\\', "/");
    text.strip_prefix("C:").map(str::to_owned).unwrap_or(text)
}

fn snapshot(vfs: &MemoryVfs) -> Tree {
    let mut tree = Tree::new();
    for entry in vfs.dump() {
        let path = plain(&entry.path);
        if path == "/" || path.is_empty() || path == "/l" || path == "/r" {
            continue;
        }
        let mtime = entry.mtime.map_or(0, |t| t.unix_seconds());
        let it = match entry.kind {
            EntryKind::Dir => It::Dir,
            EntryKind::Symlink => It::Link {
                target: entry.link_target.clone().unwrap_or_default(),
                mtime,
            },
            _ => It::File {
                data: vfs
                    .read_back(&entry.path)
                    .map(|b| b.to_vec())
                    .unwrap_or_default(),
                mtime,
            },
        };
        tree.insert(path, it);
    }
    tree
}

// ---- the model -------------------------------------------------------------------------------------------------

/// What the case asks of the sync.
#[derive(Debug, Clone)]
struct Ask {
    matrix: SyncMatrix,
    orphans: OrphanReading,
    create_empty: bool,
    /// A name filter: only `*.txt` files are rows.
    filtered: bool,
    /// A size filter: a file of at most [`SMALL`] bytes is hidden on its side.
    small: bool,
    include_hidden: bool,
    /// Names align without case.
    fold: bool,
    read_only: Option<Side>,
    /// One override: a path below the bases and its action.
    over: Option<(String, SyncAction)>,
    /// One Ignored path.
    ignored: Option<String>,
    /// *Just selection*: one path and what is below it.
    scope: Option<String>,
    /// Folders (actual paths) whose listing fails on their side.
    failed: BTreeSet<String>,
    /// The other sides of their pairs: what lies deeper than their own entries is not seen.
    dark: BTreeSet<String>,
    /// Folders (actual paths) hidden on their side by the protected attributes.
    protected: BTreeSet<String>,
    /// `no-to-all`: what has a question stays.
    no_to_all: bool,
}

impl Ask {
    /// A path as the comparison aligns it.
    fn key(&self, path: &str) -> String {
        if self.fold { path.to_lowercase() } else { path.to_owned() }
    }

    /// True when `rel` is `path` or below it, as aligned.
    fn at_or_below(&self, rel: &str, path: &str) -> bool {
        let (rel, path) = (self.key(rel), self.key(&format!("/{path}")));
        rel == path || rel.starts_with(&format!("{path}/"))
    }
}

/// One row of the model.
#[derive(Debug, Clone)]
struct MRow {
    /// The aligned path, from the base: `/a/b`.
    rel: String,
    folder: bool,
    action: SyncAction,
    conflict: bool,
    /// The actual paths of the row on each side (where it would be on a side that lacks it).
    paths: [String; 2],
    on: [bool; 2],
}

/// What holds for every row below a folder.
#[derive(Debug, Clone, Copy, Default)]
struct Above {
    failed: bool,
    hidden: bool,
}

struct Model<'t> {
    tree: &'t Tree,
    ask: &'t Ask,
    rows: Vec<MRow>,
}

fn children(tree: &Tree, path: &str) -> BTreeMap<String, String> {
    let prefix = format!("{path}/");
    tree.range(prefix.clone()..)
        .take_while(|(key, _)| key.starts_with(&prefix))
        .filter(|(key, _)| !key[prefix.len()..].contains('/'))
        .map(|(key, _)| key[prefix.len()..].to_owned())
        .map(|name| (name.clone(), name))
        .collect()
}

fn side_index(side: Side) -> usize {
    usize::from(side == Side::Right)
}

impl Model<'_> {
    fn is_small(&self, it: Option<&It>) -> bool {
        self.ask.small && matches!(it, Some(It::File { data, .. }) if data.len() as u64 <= SMALL)
    }

    fn situation(&self, row: &MRow) -> Situation {
        match row.on {
            [true, false] => match self.ask.orphans {
                OrphanReading::Added => Situation::AddedLeft,
                OrphanReading::Deleted => Situation::DeletedRight,
            },
            [false, true] => match self.ask.orphans {
                OrphanReading::Added => Situation::AddedRight,
                OrphanReading::Deleted => Situation::DeletedLeft,
            },
            _ => {
                let (l, r) = (&self.tree[&row.paths[0]], &self.tree[&row.paths[1]]);
                let (tl, tr) = (l.mtime().unwrap_or(0), r.mtime().unwrap_or(0));
                let differ = match (l, r) {
                    (It::File { data: dl, .. }, It::File { data: dr, .. }) => dl.len() != dr.len(),
                    (It::Link { target: a, .. }, It::Link { target: b, .. }) => a != b,
                    _ => true,
                };
                if (tl - tr).abs() > 2 {
                    if tl > tr { Situation::NewerLeft } else { Situation::NewerRight }
                } else if differ {
                    Situation::Different
                } else {
                    Situation::Same
                }
            }
        }
    }

    /// Rules 4–6 for a row that reads the matrix: the situation (Ignored is `same`), an override that names it or a folder
    /// above it, else the matrix; a read-only side leaves it. Returns the action and whether it is a conflict.
    fn by_matrix(&self, row: &MRow) -> (SyncAction, bool) {
        let ignored = self.ask.ignored.as_ref().is_some_and(|p| self.ask.at_or_below(&row.rel, p));
        let situation = if ignored { Situation::Same } else { self.situation(row) };
        let over = self.ask.over.as_ref().and_then(|(path, action)| {
            self.ask.at_or_below(&row.rel, path).then_some(*action)
        });
        let (action, overridden) = match over {
            Some(action) if situation.allows(action) => (action, true),
            _ => (self.ask.matrix.get(situation), false),
        };
        let conflict = situation == Situation::Different
            && !overridden
            && self.ask.matrix.different == SyncAction::Leave;
        let action = match action.writes() {
            Some(side) if Some(side) == self.ask.read_only => SyncAction::Leave,
            _ => action,
        };
        (action, conflict)
    }

    /// The rows below the folder at `paths` (present on `on`), decided, in post-order; returns their actions.
    fn folder(&mut self, rel: &str, paths: &[String; 2], on: [bool; 2], above: Above) -> Vec<SyncAction> {
        // Children by aligned key, per side: (actual name, kind class) where class 0 file or link, 1 folder.
        let mut keys: BTreeMap<String, [Option<(String, u8)>; 2]> = BTreeMap::new();
        // A listing that failed on a side: the comparison has nothing below the folder, on either side.
        if (0..2).any(|side| on[side] && self.ask.failed.contains(&paths[side])) {
            return Vec::new();
        }
        for side in 0..2 {
            if !on[side] {
                continue;
            }
            for name in children(self.tree, &paths[side]).into_keys() {
                let full = format!("{}/{name}", paths[side]);
                let class = u8::from(matches!(self.tree.get(&full), Some(It::Dir)));
                keys.entry(self.ask.key(&name)).or_default()[side] = Some((name.clone(), class));
            }
        }
        let mut out = Vec::new();
        for (key, sides) in keys {
            let same_class = matches!((&sides[0], &sides[1]), (Some((_, a)), Some((_, b))) if a == b);
            let groups: Vec<[Option<(String, u8)>; 2]> = if same_class || sides.iter().filter(|s| s.is_some()).count() == 1 {
                vec![sides]
            } else {
                vec![[sides[0].clone(), None], [None, sides[1].clone()]]
            };
            for group in groups {
                let name = |side: usize| {
                    group[side]
                        .as_ref()
                        .or(group[1 - side].as_ref())
                        .map(|(n, _)| n.clone())
                        .unwrap_or_default()
                };
                let row_paths = [format!("{}/{}", paths[0], name(0)), format!("{}/{}", paths[1], name(1))];
                let row_on = [group[0].is_some(), group[1].is_some()];
                let folder = group.iter().flatten().any(|(_, class)| *class == 1);
                let child_rel = format!("{rel}/{key}");
                out.push(self.row(&child_rel, folder, row_paths, row_on, above));
            }
        }
        out
    }

    fn row(&mut self, rel: &str, folder: bool, paths: [String; 2], on: [bool; 2], above: Above) -> SyncAction {
        let mut row = MRow {
            rel: rel.to_owned(),
            folder,
            action: SyncAction::Leave,
            conflict: false,
            paths,
            on,
        };
        let failed = above.failed || (0..2).any(|s| on[s] && self.ask.failed.contains(&row.paths[s]));
        let protected_here = (0..2).any(|s| on[s] && self.ask.protected.contains(&row.paths[s]));
        let below = Above {
            failed,
            hidden: above.hidden || protected_here,
        };
        // Post-order: the rows below are rows too, whatever this one is.
        let actions = if folder {
            self.folder(rel, &row.paths.clone(), on, below)
        } else {
            Vec::new()
        };
        let in_scope = self.ask.scope.as_ref().is_none_or(|p| self.ask.at_or_below(rel, p));
        let pair_of_folders = folder && on == [true, true];
        // A name filter hides both sides of a file-like row at once; a size filter one side; the protected attributes a
        // folder on its side.
        let named = self.ask.filtered && !folder && !rel.ends_with(".txt");
        let hidden_side = |s: usize| {
            on[s]
                && (named
                    || self.is_small(self.tree.get(&row.paths[s]))
                    || self.ask.protected.contains(&row.paths[s]))
        };
        let all_hidden = (0..2).filter(|&s| on[s]).all(hidden_side);
        let any_hidden = above.hidden || (0..2).any(hidden_side);
        if !in_scope || failed || pair_of_folders {
            // Rules 1 and 2; a pair of folders has no row of its own.
        } else if all_hidden || (any_hidden && !self.ask.include_hidden) {
            // Rule 3.
        } else if !folder {
            (row.action, row.conflict) = self.by_matrix(&row);
        } else {
            let side = if on[0] { Side::Left } else { Side::Right };
            let (gone, copied) = if side == Side::Left {
                (SyncAction::DeleteLeft, SyncAction::CopyRight)
            } else {
                (SyncAction::DeleteRight, SyncAction::CopyLeft)
            };
            if actions.is_empty() {
                let (action, _) = self.by_matrix(&row);
                if self.ask.create_empty {
                    row.action = action;
                }
            } else if actions.iter().all(|a| *a == gone) {
                row.action = gone;
            } else if actions.contains(&copied) {
                row.action = copied;
            }
        }
        let action = row.action;
        self.rows.push(row);
        action
    }
}

/// True when every folder above `path` (below its base) stands in `tree` as a folder; makes the missing ones, as a copy's
/// folders are made. False when a file stands where a folder must, or another spelling of a missing folder's name holds
/// it under a folding alignment.
fn make_parents(tree: &mut Tree, path: &str, ask: &Ask) -> bool {
    let mut missing = Vec::new();
    let mut at = path;
    while let Some((parent, _)) = at.rsplit_once('/') {
        if parent == "/l" || parent == "/r" || parent.is_empty() {
            break;
        }
        match tree.get(parent) {
            Some(It::Dir) => break,
            Some(_) => return false,
            None => missing.push(parent.to_owned()),
        }
        at = parent;
    }
    for dir in &missing {
        if let Some((up, name)) = dir.rsplit_once('/')
            && children(tree, up)
                .into_keys()
                .any(|other| other != name && ask.key(&other) == ask.key(name))
        {
            return false;
        }
    }
    for dir in missing {
        tree.insert(dir, It::Dir);
    }
    true
}

/// The tree a sync leaves and the rows, from the tree before and the case alone.
fn expected(before: &Tree, ask: &Ask) -> (Tree, Vec<MRow>) {
    let mut model = Model {
        tree: before,
        ask,
        rows: Vec::new(),
    };
    model.folder("", &["/l".to_owned(), "/r".to_owned()], [true, true], Above::default());
    let rows = model.rows;
    let mut tree = before.clone();
    // Rows were pushed children first: deletes in that order, of what is there.
    for row in &rows {
        let side = match row.action {
            SyncAction::DeleteLeft => Side::Left,
            SyncAction::DeleteRight => Side::Right,
            _ => continue,
        };
        let s = side_index(side);
        if row.on[s] {
            tree.remove(&row.paths[s]);
        }
    }
    // Copies right, then left, folders before what they hold.
    for action in [SyncAction::CopyRight, SyncAction::CopyLeft] {
        let mut copies: Vec<&MRow> = rows.iter().filter(|r| r.action == action).collect();
        copies.sort_by_key(|r| r.rel.matches('/').count());
        let (from, to) = if action == SyncAction::CopyRight { (0, 1) } else { (1, 0) };
        // A folder that could not be made: nothing below it runs (execution cuts it).
        let mut cut: Vec<String> = Vec::new();
        for row in copies {
            if cut.iter().any(|dir| row.paths[to].starts_with(&format!("{dir}/"))) {
                continue;
            }
            if !row.on[from] {
                continue; // An Ignored orphan whose matrix reads the other side: nothing to copy.
            }
            let target = row.paths[to].clone();
            // Under a folding alignment another spelling of the name, another row's, holds the name: the copy fails.
            if let Some((parent, name)) = target.rsplit_once('/')
                && children(&tree, parent)
                    .into_keys()
                    .any(|other| other != name && ask.key(&other) == ask.key(name))
            {
                cut.push(target);
                continue;
            }
            if !make_parents(&mut tree, &target, ask) {
                cut.push(target);
                continue;
            }
            if row.folder {
                match tree.get(&target) {
                    Some(It::Dir) => {}
                    Some(_) => cut.push(target),
                    None => {
                        tree.insert(target, It::Dir);
                    }
                }
                continue;
            }
            let source = tree[&row.paths[from]].clone();
            match tree.get(&target) {
                Some(It::Dir) => continue,
                Some(found) => {
                    if ask.no_to_all
                        && let (Some(theirs), Some(ours)) = (found.mtime(), source.mtime())
                        && theirs > ours + 2
                    {
                        continue;
                    }
                }
                None => {}
            }
            tree.insert(target, source);
        }
    }
    (tree, rows)
}

/// A base holds a file the filters let through (a folder whose listing fails proves none).
fn holds_file(tree: &Tree, ask: &Ask, base: &str) -> bool {
    let prefix = format!("{base}/");
    let protected = |path: &str| ask.protected.iter().any(|p| path.starts_with(&format!("{p}/")));
    // Nothing below a folder whose listing failed is seen; below the other side of its pair, only what it holds itself.
    let dark = |path: &str| {
        ask.failed.iter().any(|p| path.starts_with(&format!("{p}/")))
            || ask.dark.iter().any(|p| {
                path.strip_prefix(&format!("{p}/")).is_some_and(|rest| rest.contains('/'))
            })
    };
    tree.iter().any(|(path, it)| {
        path.starts_with(&prefix)
            && match it {
                It::Dir => false,
                It::Link { .. } => {
                    !(ask.filtered && !path.ends_with(".txt")) && !protected(path) && !dark(path)
                }
                It::File { data, .. } => {
                    !(ask.filtered && !path.ends_with(".txt"))
                        && !(ask.small && data.len() as u64 <= SMALL)
                        && !protected(path)
                        && !dark(path)
                }
            }
    })
}

// ---- the property --------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Case {
    nodes: Vec<Node>,
    preset: u8,
    cells: [u8; 8],
    orphans: bool,
    create_empty: bool,
    filtered: bool,
    include_hidden: bool,
    small: bool,
    fold: bool,
    read_only: u8,
    over: Option<(u8, u8)>,
    ignored: Option<u8>,
    scope: Option<u8>,
    failed: Option<(u8, bool)>,
    protected: Option<(u8, bool)>,
    incomplete: Option<u8>,
    no_to_all: bool,
    empty: u8,
    allow_empty: bool,
}

fn cases() -> impl Strategy<Value = Case> {
    (
        nodes(),
        (0u8..7, any::<[u8; 8]>()),
        (
            any::<bool>(),
            any::<bool>(),
            proptest::bool::weighted(0.2),
            proptest::bool::weighted(0.25),
            proptest::bool::weighted(0.35),
            proptest::bool::weighted(0.45),
        ),
        (
            0u8..8,
            proptest::option::weighted(0.3, (any::<u8>(), 0u8..5)),
            proptest::option::weighted(0.15, any::<u8>()),
            proptest::option::weighted(0.15, any::<u8>()),
        ),
        (
            proptest::option::weighted(0.12, (any::<u8>(), any::<bool>())),
            proptest::option::weighted(0.2, (any::<u8>(), any::<bool>())),
            proptest::option::weighted(0.2, Just(1u8)),
        ),
        (proptest::bool::weighted(0.25), 0u8..16, any::<bool>()),
    )
        .prop_map(
            |(
                nodes,
                (preset, cells),
                (orphans, create_empty, filtered, include_hidden, small, fold),
                (read_only, over, ignored, scope),
                (failed, protected, incomplete),
                (no_to_all, empty, allow_empty),
            )| Case {
                nodes,
                preset,
                cells,
                orphans,
                create_empty,
                filtered,
                include_hidden,
                small,
                fold,
                read_only,
                over,
                ignored,
                scope,
                failed,
                protected,
                incomplete,
                no_to_all,
                empty,
                allow_empty,
            },
        )
}

const PRESETS: [SyncPreset; 5] = [
    SyncPreset::UpdateRight,
    SyncPreset::UpdateLeft,
    SyncPreset::UpdateBoth,
    SyncPreset::MirrorRight,
    SyncPreset::MirrorLeft,
];

/// The presets as the model knows them: §1.6.14's table, written out here and not taken from the code under test.
fn model_matrix(preset: SyncPreset, custom: SyncMatrix) -> SyncMatrix {
    use SyncAction::{
        CopyLeft as L, CopyRight as R, DeleteLeft as DL, DeleteRight as DR, Leave as N,
    };
    // newer_left, newer_right, added_left, added_right, deleted_left, deleted_right, different, same
    let cells = match preset {
        SyncPreset::UpdateRight => [R, N, R, N, N, N, N, N],
        SyncPreset::UpdateLeft => [N, L, N, L, N, N, N, N],
        SyncPreset::UpdateBoth => [R, L, R, L, N, N, N, N],
        SyncPreset::MirrorRight => [R, R, R, DR, DR, R, R, N],
        SyncPreset::MirrorLeft => [L, L, DL, L, L, DL, L, N],
        SyncPreset::Custom => return custom,
    };
    let mut matrix = SyncMatrix::default();
    for (situation, action) in Situation::ALL.into_iter().zip(cells) {
        matrix.set(situation, action);
    }
    matrix
}

/// A valid custom matrix from random bytes: each cell one of the actions its situation allows.
fn custom(cells: [u8; 8]) -> SyncMatrix {
    let mut matrix = SyncMatrix::default();
    for (situation, byte) in Situation::ALL.into_iter().zip(cells) {
        let allowed: Vec<SyncAction> = SyncAction::ALL
            .into_iter()
            .filter(|a| situation.allows(*a))
            .collect();
        matrix.set(situation, allowed[usize::from(byte) % allowed.len()]);
    }
    matrix
}

fn compare(vfs: &MemoryVfs, rules: &FolderRules) -> FolderComparison {
    let engines = engines(1 << 30);
    with_env(vfs, &engines, &CancelToken::new(), |env| {
        let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
        compare_into(&model, env).unwrap()
    })
}

/// True when `spec` lies below the folder item `folder`.
fn below(spec: &hikasa_core::Spec, folder: &hikasa_core::Spec) -> bool {
    std::iter::successors(spec.parent(), hikasa_core::Spec::parent).any(|dir| dir == *folder)
}

/// The selection of *Just selection*: the row at `path`, both sides.
fn selection_of(read: &TreeRead<'_>, path: &str) -> Selection {
    let mut selection = Selection::default();
    if let Some(node) = RelPath::parse(path).ok().and_then(|rel| read.node_at(&rel)) {
        selection.insert(node, SideSet::all());
    }
    selection
}

/// The sync request of a case.
fn request<'a>(
    options: &'a SyncOptions,
    selection: Option<&'a Selection>,
    case: &Case,
    read_only: Option<Side>,
) -> SyncRequest<'a> {
    SyncRequest {
        options,
        just_selection: selection,
        visible: None,
        include_hidden: case.include_hidden,
        iu: false,
        read_only: ReadOnlySides {
            left: read_only == Some(Side::Left),
            right: read_only == Some(Side::Right),
        },
        allow_empty_source: case.allow_empty,
        settings: OpSettings::default(),
    }
}

/// The aligned path of a row of the comparison.
fn rel_of(read: &TreeRead<'_>, node: NodeId, ask: &Ask) -> String {
    ask.key(&format!("/{}", read.path(node).as_str()))
}

/// The plan's decisions, keyed as the model keys its rows (aligned path, folder): action and conflict.
fn decisions(read: &TreeRead<'_>, rows: &[hikasa_folder::sync::SyncRow], ask: &Ask) -> BTreeMap<(String, bool), (SyncAction, bool)> {
    rows.iter()
        .map(|row| {
            let view = read.node(row.node).unwrap();
            ((rel_of(read, row.node, ask), view.is_folder()), (row.action, row.conflict))
        })
        .collect()
}

/// The counters of what the cases exercised.
#[derive(Default)]
struct Seen {
    cases: AtomicU32,
    /// Cases that reached a plan of the whole comparison (not an incomplete one).
    planned: AtomicU32,
    /// Cases whose plan ran (not refused).
    executed: AtomicU32,
    ran: AtomicU32,
    deleted: AtomicU32,
    one_side_hidden: AtomicU32,
    links: AtomicU32,
    folded: AtomicU32,
    failed: AtomicU32,
    scoped: AtomicU32,
    ignored: AtomicU32,
    incomplete_refused: AtomicU32,
    second_custom: AtomicU32,
}

#[test]
fn p_sync_the_tree_is_what_the_model_says() {
    let config = config_with("tests/proptest-regressions/props_sync.txt", 384);
    let total = config.cases;
    let mut runner = TestRunner::new(config);
    let seen = Seen::default();
    runner
        .run(&cases(), |case| {
            seen.cases.fetch_add(1, Ordering::Relaxed);
            let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
            for b in ["/l", "/r"] {
                vfs.insert_dir(&host(b), meta(FOLDER_TIME)).unwrap();
            }
            // Folders of the tree on each side, to protect one or fail its listing.
            let mut folders: Vec<(String, String)> = Vec::new(); // (left path, right path) of folder pairs
            let mut one_side: Vec<String> = Vec::new();
            fn walk(nodes: &[Node], l: &str, r: &str, fold: bool, pairs: &mut Vec<(String, String)>, one: &mut Vec<String>) {
                let mut seen = BTreeSet::new();
                for node in nodes {
                    if !seen.insert(node.name) || node.link {
                        continue;
                    }
                    let (lp, rp) = (format!("{l}/{}", spelt(node, 0)), format!("{r}/{}", spelt(node, 1)));
                    let aligned = fold || !node.upper;
                    match (node.left, node.right) {
                        (Some(1), Some(1)) if aligned => {
                            pairs.push((lp.clone(), rp.clone()));
                            walk(&node.children, &lp, &rp, fold, pairs, one);
                        }
                        (left, right) => {
                            if left == Some(1) {
                                one.push(lp);
                            }
                            if right == Some(1) {
                                one.push(rp);
                            }
                        }
                    }
                }
            }
            walk(&case.nodes, "/l", "/r", case.fold, &mut folders, &mut one_side);
            let pick_side = |(pick, left): (u8, bool)| {
                folders.get(usize::from(pick) % folders.len().max(1)).map(|(l, r)| if left { l.clone() } else { r.clone() })
            };
            // A side of a pair hidden by the protected attributes (both sides are listed then, FC-054).
            let protected: BTreeSet<String> =
                case.protected.filter(|_| case.empty >= 2).and_then(pick_side).into_iter().collect();
            if case.empty != 0 {
                insert(&vfs, "/l", &case.nodes, 0, &protected);
            }
            if case.empty != 1 {
                insert(&vfs, "/r", &case.nodes, 1, &protected);
            }
            let before = snapshot(&vfs);
            let protected: BTreeSet<String> = protected.into_iter().filter(|p| before.contains_key(p)).collect();
            // A folder whose listing fails: one of a pair or one on one side.
            let mut failed = BTreeSet::new();
            if let Some((pick, left)) = case.failed {
                let all: Vec<String> = folders
                    .iter()
                    .map(|(l, r)| if left { l.clone() } else { r.clone() })
                    .chain(one_side.iter().cloned())
                    .filter(|p| before.contains_key(p) && !protected.contains(p))
                    .collect();
                if let Some(path) = all.get(usize::from(pick) % all.len().max(1)) {
                    vfs.inject(Fault { path: host(path), op: VfsOp::List, kind: ErrorKind::PermissionDenied });
                    failed.insert(path.clone());
                }
            }
            let preset = PRESETS.get(usize::from(case.preset)).copied().unwrap_or(SyncPreset::Custom);
            // Paths of the tree below the bases, by the left's spelling where it has one.
            let paths: Vec<String> = before
                .keys()
                .filter_map(|p| p.strip_prefix("/l/").or_else(|| p.strip_prefix("/r/")))
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let pick = |byte: u8| paths.get(usize::from(byte) % (paths.len() + 1)).cloned();
            let over = case.over.map(|(byte, action)| {
                (pick(byte).unwrap_or_else(|| "nowhere".to_owned()), SyncAction::ALL[usize::from(action)])
            });
            let ignored = case.ignored.and_then(pick);
            // A scope names a path that is one row: not a file and a folder of one name.
            let scope = case.scope.and_then(pick).filter(|p| {
                // As the comparison aligns: a case-insensitive one finds `A` for `a`.
                let class = |base: &str| {
                    before
                        .iter()
                        .find(|(path, _)| {
                            path.strip_prefix(base).is_some_and(|rest| {
                                if case.fold { rest.to_lowercase() == p.to_lowercase() } else { rest == p.as_str() }
                            })
                        })
                        .map(|(_, it)| matches!(it, It::Dir))
                };
                let (l, r) = (class("/l/"), class("/r/"));
                l.is_none() || r.is_none() || l == r
            });
            let mut options = SyncOptions {
                preset,
                matrix: custom(case.cells),
                orphans: if case.orphans { OrphanReading::Deleted } else { OrphanReading::Added },
                create_empty: case.create_empty,
                ..SyncOptions::default()
            };
            if let Some((path, action)) = &over {
                options.overrides.insert(path.clone(), *action);
            }
            let read_only = match case.read_only {
                0 => Some(Side::Left),
                1 => Some(Side::Right),
                _ => None,
            };
            let ask = Ask {
                matrix: model_matrix(preset, custom(case.cells)),
                orphans: options.orphans,
                create_empty: options.create_empty,
                filtered: case.filtered,
                small: case.small,
                include_hidden: case.include_hidden,
                fold: case.fold,
                read_only,
                over: over.clone(),
                ignored: ignored.clone(),
                scope: scope.clone(),
                failed: failed.clone(),
                dark: failed
                    .iter()
                    .filter_map(|f| {
                        let (l, r) = folders.iter().find(|(l, r)| l == f || r == f)?;
                        Some(if l == f { r.clone() } else { l.clone() })
                    })
                    .collect(),
                protected: protected.clone(),
                no_to_all: case.no_to_all,
            };
            let mut rules = FolderRules::default();
            if case.filtered {
                rules
                    .apply(&FolderRulesPatch {
                        name_filter: Some("*.txt".to_owned()),
                        ..FolderRulesPatch::default()
                    })
                    .unwrap();
            }
            if case.small {
                rules.filters.other.size = Some(SizeFilter { cmp: Cmp::Greater, bytes: SMALL });
            }
            if case.fold {
                rules.alignment.case = CaseMode::Insensitive;
            }
            if let Some(path) = &ignored {
                rules.ignored = vec![path.clone()];
            }
            let rules = sync_folder_rules(&rules, &options).unwrap();
            // A comparison stopped part-way never deletes: refused, or nothing to delete was reached.
            if let Some(after) = case.incomplete {
                let cancel = CancelToken::new();
                let observer = CancelAfter { after: usize::from(after), seen: AtomicUsize::new(0), cancel: &cancel };
                let cut = run_on(&vfs, &rules, None, &engines(1 << 30), &default_now(), &observer, &cancel, false)
                    .unwrap();
                if !cut.is_complete() {
                    // The case's sync, and a Mirror to Right, which deletes what the right alone has.
                    let mirror = SyncOptions { preset: SyncPreset::MirrorRight, ..options.clone() };
                    let mut refused = false;
                    for options in [&options, &mirror] {
                        match plan_sync(&cut.read(), &vfs, &request(options, None, &case, read_only), &Ctl::detached()) {
                            Err(FolderError::InvalidRules(text)) if text.contains("not complete") => refused = true,
                            Err(FolderError::EmptySource { .. }) => {}
                            Err(e) => return Err(TestCaseError::fail(format!("incomplete: {e}"))),
                            Ok(plan) => prop_assert_eq!(
                                plan.counts.delete_left + plan.counts.delete_right,
                                0,
                                "a comparison that did not finish deletes"
                            ),
                        }
                    }
                    if refused {
                        seen.incomplete_refused.fetch_add(1, Ordering::Relaxed);
                    }
                    prop_assert_eq!(&snapshot(&vfs), &before, "a plan wrote");
                    return Ok(());
                }
            }
            seen.planned.fetch_add(1, Ordering::Relaxed);
            let cmp = compare(&vfs, &rules);
            let selection = scope.as_ref().map(|p| selection_of(&cmp.read(), p));
            let first = request(&options, selection.as_ref(), &case, read_only);
            let (model_tree, model_rows) = expected(&before, &ask);
            let planned = plan_sync(&cmp.read(), &vfs, &first, &Ctl::detached());
            prop_assert_eq!(&snapshot(&vfs), &before, "a plan wrote");
            // An empty source (no file after filters) is refused exactly when the model deletes on the other side.
            let deletes = |action| model_rows.iter().any(|r| r.action == action);
            let refused = !case.allow_empty
                && ((!holds_file(&before, &ask, "/l") && deletes(SyncAction::DeleteRight))
                    || (!holds_file(&before, &ask, "/r") && deletes(SyncAction::DeleteLeft)));
            let plan = match planned {
                Err(FolderError::EmptySource { .. }) => {
                    prop_assert!(refused, "EmptySource where the model deletes nothing from an empty side");
                    return Ok(());
                }
                Err(e) => return Err(TestCaseError::fail(format!("{e}"))),
                Ok(plan) => {
                    prop_assert!(!refused, "an empty source was not refused");
                    plan
                }
            };
            // What the case exercised.
            if model_rows.iter().any(|r| r.action.deletes()) {
                seen.deleted.fetch_add(1, Ordering::Relaxed);
            }
            let model_ref = Model { tree: &before, ask: &ask, rows: Vec::new() };
            if model_rows.iter().any(|r| r.on == [true, true] && !r.folder
                && model_ref.is_small(before.get(&r.paths[0])) != model_ref.is_small(before.get(&r.paths[1])))
                || !protected.is_empty()
            {
                seen.one_side_hidden.fetch_add(1, Ordering::Relaxed);
            }
            if before.values().any(|it| matches!(it, It::Link { .. })) {
                seen.links.fetch_add(1, Ordering::Relaxed);
            }
            if case.fold && model_rows.iter().any(|r| r.on == [true, true] && r.paths[0][2..] != r.paths[1][2..]) {
                seen.folded.fetch_add(1, Ordering::Relaxed);
            }
            if !failed.is_empty() {
                seen.failed.fetch_add(1, Ordering::Relaxed);
            }
            if scope.is_some() {
                seen.scoped.fetch_add(1, Ordering::Relaxed);
            }
            if ignored.is_some() {
                seen.ignored.fetch_add(1, Ordering::Relaxed);
            }
            // Rows: the plan's decisions, and its conflicts, are the model's.
            let read = cmp.read();
            let decided = decisions(&read, &plan.rows, &ask);
            for row in &model_rows {
                let got = decided.get(&(row.rel.clone(), row.folder)).copied();
                prop_assert_eq!(got, Some((row.action, row.conflict)), "row {} {:?}", row.rel, row);
            }
            seen.executed.fetch_add(1, Ordering::Relaxed);
            let mode = if case.no_to_all { ConfirmMode::NoToAll } else { ConfirmMode::YesToAll };
            let op = Confirmation::new(&plan.op, &ConfirmPolicy::ALL, mode).finish(plan.op.clone()).unwrap();
            let dry = ops::dry_run(op.plan.clone());
            let report = ops::execute_with(&vfs, &vfs, op.plan.clone(), &op.exec, &Ctl::detached());
            prop_assert_eq!(report.plan.items.len(), dry.results.len(), "dry and run differ in items");
            let failed_dirs: Vec<&hikasa_vfs::ops::PlanItem> = report
                .plan
                .items
                .iter()
                .zip(&report.results)
                .filter(|(item, r)| r.status == ItemStatus::Failed && item.kind == EntryKind::Dir)
                .map(|(item, _)| item)
                .collect();
            for ((item, result), shown) in report.plan.items.iter().zip(&report.results).zip(&dry.results) {
                let line = format!("{:?} {} {result:?}", item.action, item.target.canonical());
                match (shown.status, result.status) {
                    (ItemStatus::Planned, ItemStatus::Done) if shown.skip.is_none() => {}
                    (ItemStatus::Planned, ItemStatus::Skipped) if shown.skip.is_some() => {}
                    (ItemStatus::Failed, ItemStatus::Failed) => {}
                    (ItemStatus::Planned, ItemStatus::NotRun) => {
                        let cut = failed_dirs.iter().any(|f| below(&item.target, &f.target));
                        prop_assert!(cut, "not run with nothing failed above it: {line}");
                    }
                    (ItemStatus::Planned, ItemStatus::Skipped)
                        if result.skip == Some(SkipReason::ChildrenFailed) => {}
                    _ => prop_assert!(false, "the dry run said {:?}: {line}", shown.status),
                }
            }
            if report.results.iter().any(|r| r.status == ItemStatus::Done) {
                seen.ran.fetch_add(1, Ordering::Relaxed);
            }
            let after = snapshot(&vfs);
            if after != model_tree {
                let extra: Vec<_> = after.iter().filter(|(k, v)| model_tree.get(*k) != Some(v)).collect();
                let missing: Vec<_> = model_tree.iter().filter(|(k, v)| after.get(*k) != Some(v)).collect();
                let items: Vec<String> = report
                    .plan
                    .items
                    .iter()
                    .zip(&report.results)
                    .map(|(i, r)| format!("{:?} {} {:?} {:?}", i.action, i.target.canonical(), r.status, r.skip.or(i.skip)))
                    .collect();
                prop_assert!(false, "{preset:?} {ask:?}\nnot in the model: {extra:#?}\nnot on disk: {missing:#?}\nitems {items:#?}");
            }
            // Each file row copied is the same in a re-comparison; each one deleted is gone.
            let again = compare(&vfs, &rules);
            let now = again.read();
            for row in model_rows.iter().filter(|r| !r.folder) {
                let at = now.preorder().map(|(id, _)| id).find(|&id| {
                    rel_of(&now, id, &ask) == row.rel && now.node(id).is_some_and(|v| !v.is_folder())
                });
                let (from, to) = match row.action {
                    SyncAction::CopyRight => (0, 1),
                    SyncAction::CopyLeft => (1, 0),
                    _ => (2, 2),
                };
                let landed = from < 2
                    && row.on[from]
                    && model_tree.get(&row.paths[to]) == model_tree.get(&row.paths[from]);
                if landed && let Some(id) = at {
                    // Both sides small now: the size filter hides the row, which is the same otherwise.
                    let small = (0..2).all(|s| model_ref.is_small(model_tree.get(&row.paths[s])));
                    let want = if small { Category::Filtered } else { Category::Same };
                    prop_assert_eq!(now.category(id, false), want, "{} copied, not the same", row.rel);
                }
                if let (Some(side), true) = (row.action.writes(), row.action.deletes())
                    && row.on[side_index(side)]
                {
                    let gone = at.is_none_or(|id| now.node(id).unwrap().side(side).is_none());
                    prop_assert!(gone, "{} deleted, still there", row.rel);
                }
            }
            drop(now);
            // P-REFRESH: the comparison refreshed where the plan wrote equals a fresh one.
            drop(read);
            let engines = engines(1 << 30);
            with_env(&vfs, &engines, &CancelToken::new(), |env| refresh_into(cmp.model(), &plan.op.refresh, env))
                .unwrap();
            prop_assert_eq!(cmp.to_canonical(), again.to_canonical(), "a refresh differs from a fresh comparison");
            // A second plan on what the first left is the model's on that tree: presets, custom matrices, overrides and
            // no-to-all alike.
            let selection2 = scope.as_ref().map(|p| selection_of(&again.read(), p));
            let request2 = request(&options, selection2.as_ref(), &case, read_only);
            let second = plan_sync(&again.read(), &vfs, &request2, &Ctl::detached());
            let (_, model_rows2) = expected(&after, &ask);
            let refused2 = !case.allow_empty
                && ((!holds_file(&after, &ask, "/l") && model_rows2.iter().any(|r| r.action == SyncAction::DeleteRight))
                    || (!holds_file(&after, &ask, "/r") && model_rows2.iter().any(|r| r.action == SyncAction::DeleteLeft)));
            let second = match second {
                Err(FolderError::EmptySource { .. }) => {
                    prop_assert!(refused2, "second run: EmptySource where the model deletes nothing from an empty side");
                    return Ok(());
                }
                other => other.map_err(|e| TestCaseError::fail(format!("second run: {e}")))?,
            };
            prop_assert!(!refused2, "second run: an empty source was not refused");
            let decided2 = decisions(&again.read(), &second.rows, &ask);
            for row in &model_rows2 {
                let got = decided2.get(&(row.rel.clone(), row.folder)).copied();
                prop_assert_eq!(got, Some((row.action, row.conflict)), "second run: row {} {:?}", row.rel, row);
            }
            if preset == SyncPreset::Custom || over.is_some() || case.no_to_all {
                seen.second_custom.fetch_add(1, Ordering::Relaxed);
            }
            // Idempotence of a preset: a second run plans nothing that runs (an item below a folder the plan cannot make
            // does not run either: execution cuts it; a question declined comes again).
            let leave_only = over.as_ref().is_none_or(|(_, a)| *a == SyncAction::Leave);
            if preset != SyncPreset::Custom && leave_only && !case.no_to_all {
                let cannot: Vec<&hikasa_core::Spec> = second
                    .op
                    .plan
                    .items
                    .iter()
                    .filter(|item| item.error.is_some() && item.kind == EntryKind::Dir)
                    .map(|item| &item.target)
                    .collect();
                let runs: Vec<String> = second
                    .op
                    .plan
                    .items
                    .iter()
                    .filter(|item| item.error.is_none() && item.skip.is_none())
                    .filter(|item| !cannot.iter().any(|folder| below(&item.target, folder)))
                    .map(|item| format!("{:?} {}", item.action, item.target.canonical()))
                    .collect();
                prop_assert!(runs.is_empty(), "a second {preset:?} plans {runs:?}");
            }
            Ok(())
        })
        .unwrap();
    let cases = seen.cases.load(Ordering::Relaxed).max(1).min(total.max(1));
    let planned = seen.planned.load(Ordering::Relaxed).max(1);
    let share = |counter: &AtomicU32| counter.load(Ordering::Relaxed);
    eprintln!(
        "P-SYNC: {} cases, {} planned whole, {} executed; ran {}, deleted {}, one side hidden {}, links {}, folded pairs {}, failed listings {}, \
         scoped {}, ignored {}, incomplete refused {}, second run checked with custom/override/no-to-all {}",
        cases,
        planned,
        share(&seen.executed),
        share(&seen.ran),
        share(&seen.deleted),
        share(&seen.one_side_hidden),
        share(&seen.links),
        share(&seen.folded),
        share(&seen.failed),
        share(&seen.scoped),
        share(&seen.ignored),
        share(&seen.incomplete_refused),
        share(&seen.second_custom),
    );
    // A property that runs nothing proves nothing: shares of what it must exercise.
    let at_least = |counter: &AtomicU32, percent: u32, what: &str| {
        let n = share(counter);
        assert!(n * 100 >= cases * percent, "only {n} of {cases} cases {what}");
    };
    // Of the plans that were not refused (an empty source, a comparison that did not finish), one in three runs an item.
    let (ran, executed) = (share(&seen.ran), share(&seen.executed).max(1));
    assert!(ran * 3 >= executed, "only {ran} of {executed} plans that ran ran an item");
    at_least(&seen.deleted, 10, "deleted");
    at_least(&seen.one_side_hidden, 3, "hid one side of a pair");
    at_least(&seen.links, 15, "held links");
    at_least(&seen.failed, 3, "had a failed listing");
    at_least(&seen.scoped, 3, "had a scope");
    at_least(&seen.ignored, 4, "had an Ignored path");
    at_least(&seen.second_custom, 10, "checked a second run of a custom matrix, an override or no-to-all");
    at_least(&seen.folded, 3, "aligned two spellings");
    at_least(&seen.incomplete_refused, 3, "refused a sync on a comparison that did not finish");
}
