//! M2 W6b acceptance A6.20, property P-OPS (T12), the parts beside the model of `props_ops_model.rs`: conservation over
//! random trees with links, filters, both cases and unlisted folders; (c) a refresh equals a re-comparison; (d) cross-device
//! moves with a source-removal fault lose nothing; backups never clash with their plan.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{compared, config_with, engines, gen_tree, host, materialize_gen, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, Side, Spec};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, FolderOp, OpPlan, OpScope, OpSettings, PathStructure,
    ReadOnlySides, Recycle, RenameMask, TouchFrom, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{
    CaseMode, FolderComparison, FolderRules, FolderRulesPatch, NodeId, TreeRead, refresh_into,
};
use hikasa_vfs::ops::{
    self, BackupPolicy, BackupStyle, ItemStatus, OpReport, OverwritePolicy, SkipReason,
};
use hikasa_vfs::{EntryKind, Fault, FollowLinks, MemoryVfs, Vfs, VfsOp};
use proptest::prelude::*;

/// A choice of rows: each bit of `mask` picks a row of the tree, in pre-order, with one side or both.
fn selection(read: &TreeRead<'_>, picks: &[u8]) -> Selection {
    let mut sel = Selection::default();
    for (index, (id, _)) in read.preorder().enumerate() {
        let pick = picks.get(index % picks.len().max(1)).copied().unwrap_or(0);
        let sides = match pick % 4 {
            1 => SideSet::LEFT,
            2 => SideSet::RIGHT,
            3 => SideSet::all(),
            _ => SideSet::empty(),
        };
        sel.insert(id, sides);
    }
    sel
}

fn scope(read: &TreeRead<'_>, picks: &[u8]) -> OpScope {
    OpScope {
        selection: selection(read, picks),
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    }
}

fn operation(which: u8) -> FolderOp {
    match which % 8 {
        0 => FolderOp::Copy {
            to: Side::Right,
            overwrite: OverwritePolicy::Always,
        },
        1 => FolderOp::Copy {
            to: Side::Left,
            overwrite: OverwritePolicy::Newer,
        },
        2 => FolderOp::Move {
            to: Side::Right,
            overwrite: OverwritePolicy::Always,
        },
        3 => FolderOp::Move {
            to: Side::Left,
            overwrite: OverwritePolicy::Skip,
        },
        4 => FolderOp::Delete {
            sides: SideSet::LEFT,
            recycle: Recycle::default(),
        },
        5 => FolderOp::Delete {
            sides: SideSet::all(),
            recycle: Recycle {
                use_bin: true,
                fallback: false,
            },
        },
        6 => FolderOp::Exchange,
        _ => FolderOp::Touch {
            sides: SideSet::all(),
            from: TouchFrom::At(FileTime::from_unix(1_790_000_000, 0).unwrap()),
        },
    }
}

fn answered(plan: OpPlan, mode: ConfirmMode) -> OpPlan {
    let machine = Confirmation::new(&plan, &ConfirmPolicy::ALL, mode);
    machine.finish(plan).unwrap()
}

fn plan_for(vfs: &dyn Vfs, cmp: &FolderComparison, picks: &[u8], op: &FolderOp) -> Option<OpPlan> {
    let read = cmp.read();
    plan_op(
        &read,
        vfs,
        &scope(&read, picks),
        op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .ok()
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

proptest! {
    #![proptest_config(config_with("tests/proptest-regressions/props_ops.txt", 128))]

    /// Conservation on random trees with links, filters, both cases and folders left unlisted: what a verb may not touch
    /// stays, bytes are not lost, a declined item keeps its target, what a filter hides stays, and what the plan let
    /// through runs (from the review's hunt). The tree the model expects is `props_ops_model.rs`.
    #[test]
    fn p_ops_nothing_is_lost(
        nodes in gen_tree(),
        picks in proptest::collection::vec(any::<u8>(), 1..16),
        which in any::<u8>(),
        insensitive in any::<bool>(),
        no in any::<bool>(),
        hidden in any::<bool>(),
        filter in 0usize..common::FILTERS.len(),
        unlisted in 0u8..4,
    ) {
        let case = if insensitive { CaseSensitivity::Insensitive } else { CaseSensitivity::Sensitive };
        let vfs = materialize_gen(&nodes, case, true);
        vfs.insert_dir(&host("/o"), hikasa_vfs::MemMeta::default()).unwrap();
        let mut rules = FolderRules::default();
        if insensitive {
            rules.alignment.case = CaseMode::Insensitive;
        }
        let text = common::FILTERS[filter];
        if !text.is_empty() {
            let patch = FolderRulesPatch { name_filter: Some(text.to_owned()), ..FolderRulesPatch::default() };
            rules.apply(&patch).unwrap();
        }
        rules.handling.scan_orphan_folders = unlisted & 1 == 0;
        rules.handling.auto_scan_subfolders = unlisted & 2 == 0;
        let cmp = compared(&vfs, &rules);
        let op = hunted(which);
        let view = cmp.read();
        let mut include = Include::visible();
        include.hidden = hidden;
        // What a filter hides, by path, on a disk the plan must not touch for it.
        let mut hidden_files: Vec<String> = Vec::new();
        if !hidden {
            for (id, _) in view.preorder() {
                let Some(node) = view.node(id) else { continue };
                if node.is_folder() {
                    continue;
                }
                for side in [Side::Left, Side::Right] {
                    if node.side(side).is_some_and(|entry| entry.filtered)
                        && let Some(spec) = view.spec(id, side)
                    {
                        hidden_files.push(spec.canonical());
                    }
                }
            }
        }
        let scope = OpScope { selection: selection(&view, &picks), include, iu: false, read_only: ReadOnlySides::default() };
        let before_plan = vfs.dump();
        let planned = plan_op(&view, &vfs, &scope, &op, &OpSettings::default(), &Ctl::detached());
        drop(view);
        prop_assert_eq!(&vfs.dump(), &before_plan, "a plan wrote");
        let Ok(planned) = planned else { return Ok(()); };
        let mode = if no { ConfirmMode::NoToAll } else { ConfirmMode::YesToAll };
        let planned = Confirmation::new(&planned, &ConfirmPolicy::ALL, mode).finish(planned).unwrap();
        // Names as the tree compares them.
        let fold = |path: &str| if insensitive { fold(path) } else { path.to_owned() };
        let replaced: BTreeSet<String> =
            planned.plan.items.iter().filter(|i| i.replaces).map(|i| fold(&i.target.canonical())).collect();
        let before = files(&vfs);
        let before_all = all_paths(&vfs);
        let answered = planned.clone();
        let report = ops::execute_with(&vfs, &vfs, planned.plan, &planned.exec, &Ctl::detached());
        let after = files(&vfs);
        let after_all = all_paths(&vfs);
        // What the plan let through runs: `not_run` only below a folder that failed.
        let failed: Vec<&ops::PlanItem> = report.plan.items.iter().zip(&report.results)
            .filter(|(i, r)| r.status == ItemStatus::Failed && i.kind == EntryKind::Dir)
            .map(|(i, _)| i)
            .collect();
        for (item, result) in report.plan.items.iter().zip(&report.results) {
            if result.status == ItemStatus::NotRun {
                let cut = failed.iter().any(|folder| {
                    below(&item.target, &folder.target)
                        || folder.source.as_ref().is_some_and(|s| item.source.as_ref().is_some_and(|i| below(i, s)))
                });
                prop_assert!(
                    cut,
                    "{} not run with nothing failed above it: {:?}",
                    item.target.canonical(),
                    report.plan.items.iter().zip(&report.results).map(|(i, r)| (i.action, i.source.as_ref().map(Spec::canonical), i.target.canonical(), i.depth, r.status, r.skip.or(i.skip))).collect::<Vec<_>>()
                );
            }
        }
        let under = |path: &str, base: &str| {
            let base = host(base);
            path == base || path.strip_prefix(&base).is_some_and(|rest| rest.starts_with(['/', '\\']))
        };
        match &op {
            FolderOp::Copy { to, .. } => {
                let (src, dst) = if *to == Side::Right { ("/l", "/r") } else { ("/r", "/l") };
                for (path, data) in &before {
                    if under(path, src) {
                        prop_assert_eq!(after.get(path), Some(data), "source {} changed", path);
                    }
                    if under(path, dst) && after.get(path) != Some(data) {
                        prop_assert!(replaced.contains(&fold(path)), "target {} changed unplanned", path);
                    }
                }
            }
            FolderOp::Delete { sides, .. } if !sides.contains(SideSet::RIGHT) => {
                for (path, data) in before.iter().filter(|(p, _)| under(p, "/r")) {
                    prop_assert_eq!(after.get(path), Some(data), "other side {}", path);
                }
            }
            FolderOp::Exchange | FolderOp::Rename { .. } => {
                prop_assert_eq!(counts(&after), counts(&before), "bytes changed");
            }
            FolderOp::Move { overwrite: OverwritePolicy::Fail, .. }
            | FolderOp::MoveTo { overwrite: OverwritePolicy::Fail, .. } => {
                prop_assert_eq!(counts(&after), counts(&before), "bytes changed");
            }
            FolderOp::CopyTo { .. } | FolderOp::Touch { .. } => {
                for (path, data) in &before {
                    prop_assert_eq!(after.get(path), Some(data), "{} changed", path);
                }
            }
            FolderOp::Move { .. } | FolderOp::MoveTo { .. } => {
                let mut lost = counts(&before);
                for (data, n) in counts(&after) {
                    if let Some(count) = lost.get_mut(&data) {
                        *count = count.saturating_sub(n);
                    }
                }
                let known: BTreeSet<String> = before.keys().map(|p| fold(p)).collect();
                let replaced_known = replaced.iter().filter(|p| known.contains(*p)).count();
                let gone: usize = lost.values().sum();
                prop_assert!(
                    gone <= replaced_known,
                    "lost {} with {} replaced: {:?} -> {:?}: {:?}",
                    gone,
                    replaced_known,
                    before.keys().collect::<Vec<_>>(),
                    after.keys().collect::<Vec<_>>(),
                    report.plan.items.iter().zip(&report.results).map(|(i, r)| (i.action, i.source.as_ref().map(Spec::canonical), i.target.canonical(), i.replaces, r.status, r.skip.or(i.skip))).collect::<Vec<_>>()
                );
            }
            _ => {}
        }
        // What a filter hides stays where it is, as it is, unless its folder is renamed with it.
        if !matches!(op, FolderOp::Rename { .. }) {
            for path in &hidden_files {
                let host_path = plain_host(path);
                prop_assert_eq!(after.get(&host_path), before.get(&host_path), "hidden {} touched: {:?}", path, report.plan.items.iter().zip(&report.results).map(|(i, r)| (i.action, i.source.as_ref().map(Spec::canonical), i.target.canonical(), r.status, i.skip)).collect::<Vec<_>>());
            }
        }
        // A declined item keeps what stands at its target.
        for (item, meta) in answered.plan.items.iter().zip(&answered.meta) {
            if item.skip == Some(SkipReason::Declined) {
                let path = item.target.canonical();
                prop_assert_eq!(after_all.contains(&path), before_all.contains(&path), "declined {} {:?}", path, meta.conflicts);
            }
        }
    }

    /// (c) A refresh of what an operation names equals a re-comparison.
    #[test]
    fn p_ops_c_a_refresh_equals_a_recomparison(
        nodes in gen_tree(),
        picks in proptest::collection::vec(any::<u8>(), 1..16),
        which in any::<u8>(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, true);
        let rules = FolderRules::default();
        let cmp = compared(&vfs, &rules);
        let op = operation(which);
        let Some(planned) = plan_for(&vfs, &cmp, &picks, &op) else {
            return Ok(());
        };
        let nodes_to_refresh: Vec<NodeId> = planned.refresh.clone();
        let planned = answered(planned, ConfirmMode::YesToAll);
        run(&vfs, planned);
        let contents = engines(1 << 30);
        with_env(&vfs, &contents, &CancelToken::new(), |env| {
            refresh_into(cmp.model(), &nodes_to_refresh, env).unwrap();
        });
        let fresh = compared(&vfs, &rules);
        prop_assert_eq!(cmp.to_canonical(), fresh.to_canonical());
    }

    /// (d) Cross-device moves, a source that cannot be removed: every byte of the input is at its source or at its target.
    #[test]
    fn p_ops_d_cross_device_moves_lose_nothing(
        nodes in gen_tree(),
        picks in proptest::collection::vec(any::<u8>(), 1..16),
        faulted in proptest::collection::vec(any::<u8>(), 0..4),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, true);
        vfs.set_device(&host("/r"), 9);
        let rules = FolderRules::default();
        let cmp = compared(&vfs, &rules);
        let input: BTreeMap<String, Vec<u8>> = files_on_left(&vfs);
        let right: BTreeMap<String, Vec<u8>> = files_on_right(&vfs);
        let op = FolderOp::Move {
            to: Side::Right,
            overwrite: OverwritePolicy::Fail,
        };
        let Some(planned) = plan_for(&vfs, &cmp, &picks, &op) else {
            return Ok(());
        };
        let names: Vec<&String> = input.keys().collect();
        for pick in &faulted {
            if let Some(name) = names.get(usize::from(*pick) % names.len().max(1)) {
                vfs.inject(Fault {
                    path: host(&format!("/l/{name}")),
                    op: VfsOp::Remove,
                    kind: ErrorKind::PermissionDenied,
                });
            }
        }
        let planned = answered(planned, ConfirmMode::YesToAll);
        let report = run(&vfs, planned);
        let after_left = files_on_left(&vfs);
        let after_right = files_on_right(&vfs);
        for (name, bytes) in &input {
            let at_source = after_left.get(name) == Some(bytes);
            let at_target = after_right.get(name) == Some(bytes);
            prop_assert!(at_source || at_target, "{name} is lost: {:?}", report.results);
        }
        for (name, bytes) in &right {
            prop_assert_eq!(after_right.get(name), Some(bytes), "{} of the right side", name);
        }
    }
}

const STYLES: [BackupStyle; 7] = [
    BackupStyle::NameTilde,
    BackupStyle::StemTilde,
    BackupStyle::NameBak,
    BackupStyle::TildeName,
    BackupStyle::StemBak,
    BackupStyle::StemBakExt,
    BackupStyle::Folder,
];

proptest! {
    #![proptest_config(config_with("tests/proptest-regressions/props_ops_backups.txt", 128))]

    /// §1.6.5: the planner numbers around every path of the plan, so execution never refuses a clash, and what an item
    /// replaced is at its backup.
    #[test]
    fn p_ops_backups_never_clash_with_the_plan(
        nodes in gen_tree(),
        picks in proptest::collection::vec(any::<u8>(), 1..16),
        which in any::<u8>(),
        style in 0usize..STYLES.len(),
        case in any::<bool>(),
    ) {
        let case = if case {
            CaseSensitivity::Sensitive
        } else {
            CaseSensitivity::Insensitive
        };
        let vfs = materialize_gen(&nodes, case, true);
        let mut rules = FolderRules::default();
        if case == CaseSensitivity::Insensitive {
            rules.alignment.case = CaseMode::Insensitive;
        }
        let cmp = compared(&vfs, &rules);
        let style = STYLES[style];
        let folder = (style == BackupStyle::Folder).then(|| common::mspec("/l/kept"));
        let settings = OpSettings {
            backup: Some(BackupPolicy { style, folder }),
            ..OpSettings::default()
        };
        let op = if which.is_multiple_of(2) {
            FolderOp::Copy { to: Side::Right, overwrite: OverwritePolicy::Always }
        } else {
            FolderOp::Move { to: Side::Left, overwrite: OverwritePolicy::Always }
        };
        let view = cmp.read();
        let Ok(planned) = plan_op(&view, &vfs, &scope(&view, &picks), &op, &settings, &Ctl::detached())
        else {
            return Ok(());
        };
        drop(view);
        let planned = answered(planned, ConfirmMode::YesToAll);
        let kept: Vec<(usize, String, Vec<u8>)> = planned
            .plan
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                // A link replaced is a link at its backup, and reads as something else from there.
                vfs.stat(&item.target, FollowLinks::No)
                    .is_ok_and(|entry| entry.kind == EntryKind::File)
            })
            .filter_map(|(index, item)| {
                let backup = item.backup.as_ref()?.canonical();
                let before = vfs.read_back(&item.target.canonical())?.to_vec();
                Some((index, backup, before))
            })
            .collect();
        let report = run(&vfs, planned);
        for (result, item) in report.results.iter().zip(&report.plan.items) {
            if let Some(error) = &result.error {
                prop_assert!(
                    !error.message.contains("is a path of another item")
                        && !error.message.contains("is the item it backs up"),
                    "{}: {}", item.target, error.message
                );
            }
        }
        for (index, backup, before) in &kept {
            if report.results[*index].status == ItemStatus::Done {
                let found = vfs.read_back(backup).map(|bytes| bytes.to_vec());
                prop_assert_eq!(found.as_ref(), Some(before));
            }
        }
    }
}

/// The verbs of the conservation property.
fn hunted(which: u8) -> FolderOp {
    let target = common::mspec("/o/t");
    use OverwritePolicy::{Always, Fail, Skip};
    match which % 14 {
        0 => FolderOp::Copy { to: Side::Right, overwrite: Always },
        1 => FolderOp::Move { to: Side::Right, overwrite: Always },
        2 => FolderOp::Move { to: Side::Left, overwrite: Fail },
        3 => FolderOp::Delete { sides: SideSet::LEFT, recycle: Recycle::default() },
        4 => FolderOp::Delete { sides: SideSet::all(), recycle: Recycle { use_bin: true, fallback: false } },
        5 => FolderOp::Exchange,
        6 => FolderOp::Rename { sides: SideSet::all(), mask: RenameMask::Dos("?.*".to_owned()) },
        7 => FolderOp::Rename { sides: SideSet::LEFT, mask: RenameMask::Dos("b*".to_owned()) },
        8 => FolderOp::Rename {
            sides: SideSet::RIGHT,
            mask: RenameMask::Regex { old: "(.)(.*)".to_owned(), new: "$2$1".to_owned() },
        },
        9 => FolderOp::CopyTo { sides: SideSet::all(), target, folder: false, structure: PathStructure::None, overwrite: Skip },
        10 => FolderOp::MoveTo { sides: SideSet::LEFT, target, folder: false, structure: PathStructure::Relative, overwrite: Fail },
        11 => FolderOp::MoveTo { sides: SideSet::all(), target, folder: false, structure: PathStructure::Base, overwrite: Always },
        12 => FolderOp::MoveTo { sides: SideSet::all(), target, folder: false, structure: PathStructure::None, overwrite: Fail },
        _ => FolderOp::Touch {
            sides: SideSet::all(),
            from: TouchFrom::At(FileTime::from_unix(1_790_000_000, 0).unwrap()),
        },
    }
}

/// Every file with its bytes.
fn files(vfs: &MemoryVfs) -> BTreeMap<String, Vec<u8>> {
    vfs.dump()
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .filter_map(|entry| Some((entry.path.clone(), vfs.read_back(&entry.path)?.to_vec())))
        .collect()
}

fn all_paths(vfs: &MemoryVfs) -> BTreeSet<String> {
    vfs.dump().into_iter().map(|entry| entry.path).collect()
}

/// How many files hold each content.
fn counts(map: &BTreeMap<String, Vec<u8>>) -> BTreeMap<Vec<u8>, usize> {
    let mut out = BTreeMap::new();
    for data in map.values() {
        *out.entry(data.clone()).or_default() += 1;
    }
    out
}

fn fold(path: &str) -> String {
    hikasa_core::case::fold(path).into_owned()
}

/// A canonical spec's text as `MemoryVfs::dump` spells the path.
fn plain_host(canonical: &str) -> String {
    canonical.to_owned()
}

/// True when `spec` lies below `folder`.
fn below(spec: &Spec, folder: &Spec) -> bool {
    std::iter::successors(spec.parent(), Spec::parent).any(|dir| dir == *folder)
}

fn contents(vfs: &MemoryVfs, base: &str) -> BTreeMap<String, Vec<u8>> {
    let prefix = host(base);
    vfs.dump()
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .filter_map(|entry| {
            let rel = entry
                .path
                .strip_prefix(&prefix)?
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            let data = vfs.read_back(&entry.path)?;
            Some((rel, data.to_vec()))
        })
        .collect()
}

fn files_on_left(vfs: &MemoryVfs) -> BTreeMap<String, Vec<u8>> {
    contents(vfs, "/l")
}

fn files_on_right(vfs: &MemoryVfs) -> BTreeMap<String, Vec<u8>> {
    contents(vfs, "/r")
}
