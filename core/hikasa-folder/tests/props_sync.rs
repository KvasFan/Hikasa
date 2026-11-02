//! M2 W6c acceptance A6.29, property P-SYNC (T12): random trees × presets and custom matrices × options on `MemoryVfs`.
//!
//! The tree after a sync is compared, file by file, with what a small model of §1.6.14 says it must be, computed from the
//! tree before and the options alone, never from the plan. Also: planning writes nothing; the items a dry run shows are the
//! items that run, with the status the dry run gave them; after a run a re-comparison shows each copied row the same and
//! each deleted row gone; a second run of a preset plans nothing (idempotence); and a refresh of the comparison equals a
//! fresh one (P-REFRESH). A share of the cases must really run something.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU32, Ordering};

use common::{config_with, engines, host, mspec, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, Side};
use hikasa_folder::ops::{ConfirmMode, ConfirmPolicy, Confirmation, OpSettings, ReadOnlySides};
use hikasa_folder::sync::{
    OrphanReading, Situation, SyncAction, SyncMatrix, SyncOptions, SyncPreset, SyncRequest,
    plan_sync, sync_folder_rules,
};
use hikasa_folder::{
    Category, FolderComparison, FolderError, FolderModel, FolderRules, FolderRulesPatch, NodeId,
    compare_into, refresh_into,
};
use hikasa_vfs::ops::{self, ItemStatus, SkipReason};
use hikasa_vfs::{EntryKind, MemMeta, MemoryVfs};
use proptest::prelude::*;
use proptest::test_runner::TestRunner;

// ---- trees -----------------------------------------------------------------------------------------------------

const NAMES: [&str; 5] = ["a", "b", "c.txt", "d.txt", "e.log"];
const FOLDER_TIME: i64 = 500;

#[derive(Debug, Clone)]
struct Node {
    name: usize,
    /// 0 a file, 1 a folder, per side.
    left: Option<u8>,
    right: Option<u8>,
    content: [u8; 2],
    mtime: [u8; 2],
    children: Vec<Node>,
}

fn nodes() -> impl Strategy<Value = Vec<Node>> {
    let leaf = (
        0..NAMES.len(),
        proptest::option::weighted(0.75, 0u8..2),
        proptest::option::weighted(0.75, 0u8..2),
        (0u8..3, 0u8..3),
        (0u8..3, 0u8..3),
    )
        .prop_map(|(name, left, right, (cl, cr), (ml, mr))| Node {
            name,
            left,
            right,
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

/// The bytes of a content index: lengths differ, so equal sizes are equal contents.
fn data(name: &str, content: u8) -> Vec<u8> {
    format!("{name}{}", "x".repeat(usize::from(content))).into_bytes()
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
                let secs = 1_000_000 * (1 + i64::from(node.mtime[side]));
                vfs.insert_file(
                    &host(&path),
                    data(NAMES[node.name], node.content[side]),
                    meta(secs),
                )
                .unwrap();
            }
            Some(_) => {
                vfs.insert_dir(&host(&path), meta(FOLDER_TIME)).unwrap();
                insert(vfs, &path, &node.children, side);
            }
            None => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum It {
    Dir,
    File { data: Vec<u8>, mtime: i64 },
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
        let it = match entry.kind {
            EntryKind::Dir => It::Dir,
            _ => It::File {
                data: vfs
                    .read_back(&entry.path)
                    .map(|b| b.to_vec())
                    .unwrap_or_default(),
                mtime: entry.mtime.map_or(0, |t| t.unix_seconds()),
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
    filtered: bool,
    read_only: Option<Side>,
    /// One override: a path below the bases and its action.
    over: Option<(String, SyncAction)>,
    /// `no-to-all`: what has a question stays.
    no_to_all: bool,
}

/// One row of the model.
#[derive(Debug, Clone)]
struct MRow {
    rel: String,
    folder: bool,
    action: SyncAction,
}

struct Model<'t> {
    tree: &'t Tree,
    ask: &'t Ask,
    rows: Vec<MRow>,
}

fn kind_at(tree: &Tree, path: &str) -> Option<bool> {
    tree.get(path).map(|it| matches!(it, It::Dir))
}

fn children(tree: &Tree, path: &str) -> BTreeSet<String> {
    let prefix = format!("{path}/");
    tree.range(prefix.clone()..)
        .take_while(|(key, _)| key.starts_with(&prefix))
        .filter(|(key, _)| !key[prefix.len()..].contains('/'))
        .map(|(key, _)| key[prefix.len()..].to_owned())
        .collect()
}

fn base(side: Side) -> &'static str {
    if side == Side::Left { "/l" } else { "/r" }
}

impl Model<'_> {
    /// A name filter hides both sides of a row: no row, `--include-hidden` or not.
    fn hidden(&self, rel: &str, folder: bool) -> bool {
        self.ask.filtered && !folder && !rel.ends_with(".txt")
    }

    fn situation(&self, rel: &str, on: Option<Side>) -> Situation {
        match on {
            Some(Side::Left) => match self.ask.orphans {
                OrphanReading::Added => Situation::AddedLeft,
                OrphanReading::Deleted => Situation::DeletedRight,
            },
            Some(_) => match self.ask.orphans {
                OrphanReading::Added => Situation::AddedRight,
                OrphanReading::Deleted => Situation::DeletedLeft,
            },
            None => {
                let (
                    Some(It::File {
                        data: dl,
                        mtime: tl,
                    }),
                    Some(It::File {
                        data: dr,
                        mtime: tr,
                    }),
                ) = (
                    self.tree.get(&format!("/l{rel}")),
                    self.tree.get(&format!("/r{rel}")),
                )
                else {
                    unreachable!()
                };
                if (tl - tr).abs() > 2 {
                    if tl > tr {
                        Situation::NewerLeft
                    } else {
                        Situation::NewerRight
                    }
                } else if dl.len() != dr.len() {
                    Situation::Different
                } else {
                    Situation::Same
                }
            }
        }
    }

    /// The action of a row that reads the matrix: an override that names it or a folder above it, else the matrix; a
    /// read-only side leaves it.
    fn by_matrix(&self, rel: &str, situation: Situation) -> SyncAction {
        let over = self.ask.over.as_ref().and_then(|(path, action)| {
            (rel == format!("/{path}") || rel.starts_with(&format!("/{path}/"))).then_some(*action)
        });
        let action = match over {
            Some(action) if situation.allows(action) => action,
            _ => self.ask.matrix.get(situation),
        };
        match action.writes() {
            Some(side) if Some(side) == self.ask.read_only => SyncAction::Leave,
            _ => action,
        }
    }

    /// The rows below `rel` (a folder on both sides, or the base), decided; returns their actions.
    fn folder(&mut self, rel: &str, on: Option<Side>) -> Vec<SyncAction> {
        let mut names: BTreeSet<String> = BTreeSet::new();
        for side in [Side::Left, Side::Right] {
            if on.is_none_or(|s| s == side) {
                names.extend(children(self.tree, &format!("{}{rel}", base(side))));
            }
        }
        let mut out = Vec::new();
        for name in names {
            let child = format!("{rel}/{name}");
            let kl = kind_at(self.tree, &format!("/l{child}")).filter(|_| on != Some(Side::Right));
            let kr = kind_at(self.tree, &format!("/r{child}")).filter(|_| on != Some(Side::Left));
            if kl.is_some() && kl == kr {
                out.push(self.row(&child, kl == Some(true), None));
            } else {
                if let Some(folder) = kl {
                    out.push(self.row(&child, folder, Some(Side::Left)));
                }
                if let Some(folder) = kr {
                    out.push(self.row(&child, folder, Some(Side::Right)));
                }
            }
        }
        out
    }

    fn row(&mut self, rel: &str, folder: bool, on: Option<Side>) -> SyncAction {
        let action = match (folder, on) {
            (false, _) if self.hidden(rel, false) => SyncAction::Leave,
            (false, _) => {
                let situation = self.situation(rel, on);
                self.by_matrix(rel, situation)
            }
            (true, None) => {
                self.folder(rel, None);
                SyncAction::Leave
            }
            (true, Some(side)) => {
                let below = self.folder(rel, Some(side));
                let (gone, copied) = if side == Side::Left {
                    (SyncAction::DeleteLeft, SyncAction::CopyRight)
                } else {
                    (SyncAction::DeleteRight, SyncAction::CopyLeft)
                };
                if below.is_empty() {
                    let action = self.by_matrix(rel, self.situation(rel, Some(side)));
                    if self.ask.create_empty {
                        action
                    } else {
                        SyncAction::Leave
                    }
                } else if below.iter().all(|a| *a == gone) {
                    gone
                } else if below.contains(&copied) {
                    copied
                } else {
                    SyncAction::Leave
                }
            }
        };
        self.rows.push(MRow {
            rel: rel.to_owned(),
            folder,
            action,
        });
        action
    }
}

/// The tree a sync leaves, from the tree before and the case alone; also whether some delete has nothing on its source side
/// (an empty base), in which case the sync is refused.
fn expected(before: &Tree, ask: &Ask) -> (Tree, Vec<MRow>) {
    let mut model = Model {
        tree: before,
        ask,
        rows: Vec::new(),
    };
    model.folder("", None);
    let rows = model.rows;
    let mut tree = before.clone();
    // Rows were pushed children first: deletes in that order.
    for row in &rows {
        let side = match row.action {
            SyncAction::DeleteLeft => Side::Left,
            SyncAction::DeleteRight => Side::Right,
            _ => continue,
        };
        tree.remove(&format!("{}{}", base(side), row.rel));
    }
    // Copies right, then left, folders before what they hold.
    for action in [SyncAction::CopyRight, SyncAction::CopyLeft] {
        let mut copies: Vec<&MRow> = rows.iter().filter(|r| r.action == action).collect();
        copies.sort_by_key(|r| r.rel.matches('/').count());
        let (from, to) = if action == SyncAction::CopyRight {
            ("/l", "/r")
        } else {
            ("/r", "/l")
        };
        for row in copies {
            let target = format!("{to}{}", row.rel);
            let parent = target.rsplit_once('/').map_or("", |(p, _)| p);
            if parent != to && !matches!(tree.get(parent), Some(It::Dir)) {
                continue;
            }
            if row.folder {
                tree.entry(target).or_insert(It::Dir);
                continue;
            }
            let source = tree[&format!("{from}{}", row.rel)].clone();
            match tree.get(&target) {
                Some(It::Dir) => continue,
                Some(It::File { mtime, .. }) => {
                    let It::File { mtime: theirs, .. } = &source else {
                        continue;
                    };
                    if ask.no_to_all && *mtime > theirs + 2 {
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
    read_only: u8,
    over: Option<(u8, u8)>,
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
            proptest::bool::weighted(0.3),
            proptest::bool::weighted(0.2),
        ),
        (
            0u8..8,
            proptest::option::weighted(0.3, (any::<u8>(), 0u8..5)),
        ),
        (proptest::bool::weighted(0.25), 0u8..16, any::<bool>()),
    )
        .prop_map(
            |(
                nodes,
                (preset, cells),
                (orphans, create_empty, filtered, include_hidden),
                (read_only, over),
                (no_to_all, empty, allow_empty),
            )| {
                Case {
                    nodes,
                    preset,
                    cells,
                    orphans,
                    create_empty,
                    filtered,
                    include_hidden,
                    read_only,
                    over,
                    no_to_all,
                    empty,
                    allow_empty,
                }
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

#[test]
fn p_sync_the_tree_is_what_the_model_says() {
    let config = config_with("tests/proptest-regressions/props_sync.txt", 384);
    let total = config.cases;
    let mut runner = TestRunner::new(config);
    let ran = AtomicU32::new(0);
    let seen = AtomicU32::new(0);
    runner
        .run(&cases(), |case| {
            seen.fetch_add(1, Ordering::Relaxed);
            let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
            for b in ["/l", "/r"] {
                vfs.insert_dir(&host(b), meta(FOLDER_TIME)).unwrap();
            }
            if case.empty != 0 {
                insert(&vfs, "/l", &case.nodes, 0);
            }
            if case.empty != 1 {
                insert(&vfs, "/r", &case.nodes, 1);
            }
            let preset = PRESETS.get(usize::from(case.preset)).copied().unwrap_or(SyncPreset::Custom);
            let before = snapshot(&vfs);
            // The override names a path of the tree, or one that is not there.
            let paths: Vec<String> = before
                .keys()
                .filter_map(|p| p.strip_prefix("/l/").or_else(|| p.strip_prefix("/r/")))
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            let over = case.over.map(|(pick, action)| {
                let path = paths
                    .get(usize::from(pick) % (paths.len() + 1))
                    .cloned()
                    .unwrap_or_else(|| "nowhere".to_owned());
                (path, SyncAction::ALL[usize::from(action)])
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
                read_only,
                over: over.clone(),
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
            let rules = sync_folder_rules(&rules, &options).unwrap();
            let cmp = compare(&vfs, &rules);
            let request = SyncRequest {
                options: &options,
                just_selection: None,
                visible: None,
                include_hidden: case.include_hidden,
                iu: false,
                read_only: ReadOnlySides {
                    left: read_only == Some(Side::Left),
                    right: read_only == Some(Side::Right),
                },
                allow_empty_source: case.allow_empty,
                settings: OpSettings::default(),
            };
            let (model_tree, model_rows) = expected(&before, &ask);
            let planned = plan_sync(&cmp.read(), &vfs, &request, &Ctl::detached());
            prop_assert_eq!(&snapshot(&vfs), &before, "a plan wrote");
            // An empty source is refused exactly when the model deletes on the other side.
            let empty_left = !before.keys().any(|p| p.starts_with("/l/"));
            let empty_right = !before.keys().any(|p| p.starts_with("/r/"));
            let deletes = |action| model_rows.iter().any(|r| r.action == action);
            let refused = !case.allow_empty
                && ((empty_left && deletes(SyncAction::DeleteRight))
                    || (empty_right && deletes(SyncAction::DeleteLeft)));
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
            // Rows: the plan's decisions are the model's.
            let read = cmp.read();
            let mut decided: BTreeMap<(String, bool), SyncAction> = BTreeMap::new();
            for row in &plan.rows {
                let view = read.node(row.node).unwrap();
                decided.insert((format!("/{}", read.path(row.node).as_str()), view.is_folder()), row.action);
            }
            for row in &model_rows {
                let got = decided.get(&(row.rel.clone(), row.folder)).copied();
                prop_assert_eq!(got, Some(row.action), "row {} {:?}", row.rel, row);
            }
            let mode = if case.no_to_all { ConfirmMode::NoToAll } else { ConfirmMode::YesToAll };
            let op = Confirmation::new(&plan.op, &ConfirmPolicy::ALL, mode).finish(plan.op.clone()).unwrap();
            let dry = ops::dry_run(op.plan.clone());
            let report = ops::execute_with(&vfs, &vfs, op.plan.clone(), &op.exec, &Ctl::detached());
            prop_assert_eq!(report.plan.items.len(), dry.results.len(), "dry and run differ in items");
            let failed: Vec<&hikasa_vfs::ops::PlanItem> = report
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
                        let cut = failed.iter().any(|f| below(&item.target, &f.target));
                        prop_assert!(cut, "not run with nothing failed above it: {line}");
                    }
                    (ItemStatus::Planned, ItemStatus::Skipped)
                        if result.skip == Some(SkipReason::ChildrenFailed) => {}
                    _ => prop_assert!(false, "the dry run said {:?}: {line}", shown.status),
                }
            }
            if report.results.iter().any(|r| r.status == ItemStatus::Done) {
                ran.fetch_add(1, Ordering::Relaxed);
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
                let rel = hikasa_core::RelPath::parse(row.rel.trim_start_matches('/')).unwrap();
                let at = now.node_at(&rel).and_then(|id| {
                    let parent = now.node(id)?.parent.unwrap_or(NodeId::ROOT);
                    now.children(parent).find(|&c| now.path(c) == rel && now.node(c).is_some_and(|v| !v.is_folder()))
                });
                let copied = matches!(row.action, SyncAction::CopyRight | SyncAction::CopyLeft);
                let landed = model_tree.get(&format!("{}{}", if row.action == SyncAction::CopyRight { "/r" } else { "/l" }, row.rel))
                    == model_tree.get(&format!("{}{}", if row.action == SyncAction::CopyRight { "/l" } else { "/r" }, row.rel));
                if copied && landed {
                    let id = at.unwrap();
                    prop_assert_eq!(now.category(id, false), Category::Same, "{} copied, not the same", row.rel);
                }
                if let (Some(side), true) = (row.action.writes(), row.action.deletes()) {
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
            // Idempotence: a second run of a preset plans nothing that runs.
            let leave_only = over.as_ref().is_none_or(|(_, a)| *a == SyncAction::Leave);
            if preset != SyncPreset::Custom && leave_only && !case.no_to_all {
                let second = plan_sync(&again.read(), &vfs, &request, &Ctl::detached());
                let second = second.map_err(|e| TestCaseError::fail(format!("second run: {e}")))?;
                // An item below a folder the plan cannot make does not run either (execution cuts it).
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
    let seen = seen.load(Ordering::Relaxed).max(1);
    let ran = ran.load(Ordering::Relaxed);
    eprintln!("P-SYNC: {ran} of {seen} cases ran an item");
    // A property that runs nothing proves nothing.
    assert!(
        ran * 5 >= seen.min(total) * 2,
        "only {ran} of {seen} cases ran an item"
    );
}
