//! M2 W9a acceptance A9.9 (core, D-243): `SyncOptions::swapped`. The table of every preset, a custom matrix and the
//! overrides; property: swapping twice is the identity, and the swapped options planned on the swapped comparison give
//! every row the mirrored action, so a Swap Sides performs the same writes on the same folders.

mod common;

use std::collections::BTreeMap;

use common::{config, engines, gen_tree, materialize_gen, mspec, with_env};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, NullProgress};
use hikasa_folder::ops::{OpSettings, ReadOnlySides};
use hikasa_folder::sync::{
    OrphanReading, Situation, SyncAction, SyncMatrix, SyncOptions, SyncPreset, SyncRequest,
    plan_sync, sync_folder_rules,
};
use hikasa_folder::{FolderComparison, FolderModel, FolderRules, TreeRead, compare_into};
use hikasa_vfs::MemoryVfs;
use proptest::prelude::*;

#[test]
fn every_preset_a_custom_matrix_and_the_overrides() {
    let pairs = [
        (SyncPreset::UpdateLeft, SyncPreset::UpdateRight),
        (SyncPreset::UpdateRight, SyncPreset::UpdateLeft),
        (SyncPreset::UpdateBoth, SyncPreset::UpdateBoth),
        (SyncPreset::MirrorLeft, SyncPreset::MirrorRight),
        (SyncPreset::MirrorRight, SyncPreset::MirrorLeft),
        (SyncPreset::Custom, SyncPreset::Custom),
    ];
    for (preset, mirror) in pairs {
        let options = SyncOptions {
            preset,
            ..SyncOptions::default()
        };
        let swapped = options.swapped();
        assert_eq!(swapped.preset, mirror, "{preset:?}");
        if let (Some(a), Some(b)) = (SyncMatrix::of(preset), SyncMatrix::of(mirror)) {
            // The preset's matrix mirrored is the mirrored preset's matrix.
            assert_eq!(a.mirrored(), b, "{preset:?}");
        }
    }
    // A custom matrix: newer_left=copy_right, added_right=delete_right, deleted_left=copy_left, different=copy_left.
    let custom = SyncMatrix::parse(
        "newer-left=copy-right,added-right=delete-right,deleted-left=copy-left,different=copy-left",
        SyncMatrix::default(),
    )
    .unwrap();
    let options = SyncOptions {
        preset: SyncPreset::Custom,
        matrix: custom,
        orphans: OrphanReading::Deleted,
        recycle: true,
        overrides: BTreeMap::from([
            ("a/b".to_owned(), SyncAction::CopyRight),
            ("c".to_owned(), SyncAction::DeleteLeft),
            ("d".to_owned(), SyncAction::Leave),
        ]),
        ..SyncOptions::default()
    };
    let swapped = options.swapped();
    let m = swapped.matrix;
    assert_eq!(m.newer_right, SyncAction::CopyLeft);
    assert_eq!(m.added_left, SyncAction::DeleteLeft);
    assert_eq!(m.deleted_right, SyncAction::CopyRight);
    assert_eq!(m.different, SyncAction::CopyRight);
    assert_eq!(m.newer_left, SyncAction::Leave);
    m.validate().unwrap();
    assert_eq!(
        swapped.overrides,
        BTreeMap::from([
            ("a/b".to_owned(), SyncAction::CopyLeft),
            ("c".to_owned(), SyncAction::DeleteRight),
            ("d".to_owned(), SyncAction::Leave),
        ])
    );
    // Not side-specific: kept.
    assert_eq!(swapped.orphans, OrphanReading::Deleted);
    assert!(swapped.recycle);
    assert_eq!(swapped.swapped(), options);
    for s in Situation::ALL {
        assert_eq!(s.mirrored().mirrored(), s);
    }
    for a in SyncAction::ALL {
        assert_eq!(a.mirrored().mirrored(), a);
    }
}

/// The comparison of `left` against `right`.
fn compare(vfs: &MemoryVfs, left: &str, right: &str, rules: &FolderRules) -> FolderComparison {
    let engines = engines(1 << 30);
    with_env(vfs, &engines, &CancelToken::new(), |env| {
        let model = FolderModel::new(mspec(left), mspec(right), rules.clone()).unwrap();
        compare_into(&model, env).unwrap()
    })
}

/// The actions of a plan by (path, folder); `None` when the plan is refused.
fn actions(
    read: &TreeRead<'_>,
    vfs: &MemoryVfs,
    options: &SyncOptions,
) -> Option<BTreeMap<(String, bool), SyncAction>> {
    let request = SyncRequest {
        options,
        just_selection: None,
        visible: None,
        include_hidden: false,
        iu: false,
        read_only: ReadOnlySides::default(),
        allow_empty_source: true,
        settings: OpSettings::default(),
    };
    let cancel = CancelToken::never();
    let progress = NullProgress;
    let ctl = Ctl {
        cancel: &cancel,
        progress: &progress,
    };
    let plan = plan_sync(read, vfs, &request, &ctl).ok()?;
    Some(
        plan.rows
            .iter()
            .map(|row| {
                let folder = read.node(row.node).is_some_and(|n| n.is_folder());
                (
                    (read.path(row.node).as_str().to_owned(), folder),
                    row.action,
                )
            })
            .collect(),
    )
}

fn gen_options() -> impl Strategy<Value = (SyncOptions, Vec<(usize, u8)>)> {
    (
        0..SyncPreset::ALL.len(),
        proptest::array::uniform8(0..SyncAction::ALL.len()),
        any::<bool>(),
        proptest::collection::vec((0..64usize, 0..5u8), 0..4),
    )
        .prop_map(|(preset, cells, deleted, picks)| {
            let mut matrix = SyncMatrix::default();
            for (situation, cell) in Situation::ALL.into_iter().zip(cells) {
                let action = SyncAction::ALL[cell];
                // Invalid cells (a side the item is not on) are left alone.
                if situation.allows(action) {
                    matrix.set(situation, action);
                }
            }
            let options = SyncOptions {
                preset: SyncPreset::ALL[preset],
                matrix,
                orphans: if deleted {
                    OrphanReading::Deleted
                } else {
                    OrphanReading::Added
                },
                ..SyncOptions::default()
            };
            (options, picks)
        })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/sync_swapped.txt"))]

    /// A9.9: `swapped` twice is the identity; the swapped options on the swapped comparison mirror every row's action.
    #[test]
    fn p_swapped_options_do_the_same_writes(nodes in gen_tree(), (mut options, picks) in gen_options()) {
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, true);
        let rules = sync_folder_rules(&FolderRules::default(), &options).unwrap();
        let ab = compare(&vfs, "/l", "/r", &rules);
        let ba = compare(&vfs, "/r", "/l", &rules);
        // Overrides on rows of the tree, by path.
        let paths: Vec<String> = {
            let read = ab.read();
            read.preorder().map(|(id, _)| read.path(id).as_str().to_owned()).collect()
        };
        if !paths.is_empty() {
            for (pick, action) in picks {
                let path = paths[pick % paths.len()].clone();
                options.overrides.insert(path, SyncAction::ALL[usize::from(action)]);
            }
        }
        prop_assert_eq!(options.swapped().swapped(), options.clone());
        let forward = actions(&ab.read(), &vfs, &options);
        let backward = actions(&ba.read(), &vfs, &options.swapped());
        match (forward, backward) {
            (Some(forward), Some(backward)) => {
                let mirrored: BTreeMap<_, _> =
                    forward.into_iter().map(|(k, a)| (k, a.mirrored())).collect();
                prop_assert_eq!(mirrored, backward);
            }
            (None, None) => {}
            (f, b) => prop_assert!(false, "one side refused: {:?} / {:?}", f.is_some(), b.is_some()),
        }
    }
}
