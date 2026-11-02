//! M2 W6b acceptance A6.20, property P-OPS (T12): random trees × selection × operation × answers on `MemoryVfs`. (a) The dry-run
//! items equal the executed items, field by field; (b) re-comparing gives the predicted state of each row; (c) a refresh
//! equals a re-comparison; (d) cross-device moves with a source-removal fault lose nothing.

mod common;

use std::collections::BTreeMap;

use common::{compared, config_with, engines, gen_tree, host, materialize_gen, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, RelPath, Side, Spec};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, FolderOp, OpPlan, OpScope, OpSettings, ReadOnlySides,
    Recycle, TouchFrom, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{CompareState, FolderComparison, FolderRules, NodeId, TreeRead, refresh_into};
use hikasa_vfs::ops::{self, Action, ItemStatus, OpReport, OverwritePolicy};
use hikasa_vfs::{EntryKind, Fault, MemoryVfs, Vfs, VfsOp};
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

fn mode(which: u8) -> ConfirmMode {
    if which.is_multiple_of(2) {
        ConfirmMode::YesToAll
    } else {
        ConfirmMode::NoToAll
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

fn side_rel(spec: &Spec) -> Option<(Side, String)> {
    let text = spec.canonical().replace('\\', "/");
    let text = text.strip_prefix("C:").unwrap_or(&text);
    if let Some(rel) = text.strip_prefix("/l/") {
        Some((Side::Left, rel.to_owned()))
    } else {
        text.strip_prefix("/r/")
            .map(|rel| (Side::Right, rel.to_owned()))
    }
}

fn state_of(cmp: &FolderComparison, rel: &str) -> Option<CompareState> {
    let read = cmp.read();
    let id = read.node_at(&RelPath::parse(rel).ok()?)?;
    read.node(id).map(|view| view.state)
}

fn has(vfs: &MemoryVfs, path: &str) -> bool {
    vfs.stat(&common::mspec(path), hikasa_vfs::FollowLinks::No)
        .is_ok()
}

proptest! {
    #![proptest_config(config_with("tests/proptest-regressions/props_ops.txt", 128))]

    /// (a) and (b), and what a plan never does: write before it runs.
    #[test]
    fn p_ops_a_and_b(
        nodes in gen_tree(),
        picks in proptest::collection::vec(any::<u8>(), 1..16),
        which in any::<u8>(),
        answer in any::<u8>(),
    ) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, true);
        let rules = FolderRules::default();
        let cmp = compared(&vfs, &rules);
        let before = vfs.dump();
        let op = operation(which);
        let Some(planned) = plan_for(&vfs, &cmp, &picks, &op) else {
            return Ok(());
        };
        prop_assert_eq!(&vfs.dump(), &before);
        prop_assert_eq!(planned.plan.items.len(), planned.meta.len());
        let planned = answered(planned, mode(answer));
        let dry = ops::dry_run(planned.plan.clone());
        prop_assert!(dry.results.iter().all(|r| r.status != ItemStatus::Done));
        prop_assert_eq!(&vfs.dump(), &before);
        let report = run(&vfs, planned);
        // (a) Field by field; a device never differs here, so nothing expands.
        prop_assert_eq!(&dry.plan.items, &report.plan.items);
        // (b) What ran is what a new comparison finds.
        let fresh = compared(&vfs, &rules);
        for (item, result) in report.plan.items.iter().zip(&report.results) {
            if result.status != ItemStatus::Done {
                continue;
            }
            let target = side_rel(&item.target);
            match item.action {
                Action::Copy => {
                    if let Some((_, rel)) = target {
                        let state = state_of(&fresh, &rel);
                        prop_assert!(
                            matches!(state, Some(CompareState::Same | CompareState::Error | CompareState::Unknown)),
                            "copied {rel}: {state:?}"
                        );
                    }
                }
                Action::Move | Action::Rename => {
                    let source = item.source.as_ref().and_then(side_rel);
                    if let (Some((_, rel)), Some((side, from))) = (target, source) {
                        let (there, gone) = match side {
                            Side::Left => (format!("/r/{rel}"), format!("/l/{from}")),
                            _ => (format!("/l/{rel}"), format!("/r/{from}")),
                        };
                        prop_assert!(has(&vfs, &there), "moved to {there}");
                        prop_assert!(!has(&vfs, &gone), "still at {gone}");
                    }
                }
                Action::Remove | Action::RemoveDir | Action::Trash => {
                    if let Some((side, rel)) = target {
                        let base = if side == Side::Left { "/l" } else { "/r" };
                        prop_assert!(!has(&vfs, &format!("{base}/{rel}")), "deleted {rel}");
                    }
                }
                _ => {}
            }
        }
        fresh.check_invariants().unwrap();
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
