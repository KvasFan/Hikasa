//! M2 W6c acceptance A6.26 (D-181): the sync matrix. Eight situations × five actions on `MemoryVfs`, in both orphan
//! readings: each valid cell does its action and nothing else, each invalid one is refused. The five presets as data and as
//! runs, `--custom` words, and a round trip through TOML.

mod common;

use std::collections::BTreeMap;

use common::{compared, host, mspec};
use hikasa_core::{CaseSensitivity, Ctl, FileTime};
use hikasa_folder::FolderRules;
use hikasa_folder::ops::{OpSettings, ReadOnlySides};
use hikasa_folder::sync::{
    OrphanReading, Situation, SyncAction, SyncMatrix, SyncOptions, SyncPreset, SyncRequest,
    SyncRules, plan_sync, sync_folder_rules,
};
use hikasa_vfs::ops::{self, ItemStatus};
use hikasa_vfs::{EntryKind, MemMeta, MemoryVfs};

const OLD: i64 = 1_000_000;
const NEW: i64 = 2_000_000;

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

/// One file per situation: `nl` newer left, `nr` newer right, `lo` left only, `ro` right only, `df` different at the same
/// time, `sm` the same.
fn fixture() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(base), meta(OLD)).unwrap();
    }
    let file = |path: &str, bytes: &[u8], secs: i64| {
        vfs.insert_file(&host(path), bytes.to_vec(), meta(secs))
            .unwrap();
    };
    file("/l/nl.txt", b"left newer", NEW);
    file("/r/nl.txt", b"right older", OLD);
    file("/l/nr.txt", b"left older", OLD);
    file("/r/nr.txt", b"right newer", NEW);
    file("/l/lo.txt", b"left only", OLD);
    file("/r/ro.txt", b"right only", OLD);
    file("/l/df.txt", b"left differs", OLD);
    file("/r/df.txt", b"right is different", OLD);
    file("/l/sm.txt", b"same", OLD);
    file("/r/sm.txt", b"same", OLD);
    vfs
}

/// The file of a situation in [`fixture`].
fn file_of(situation: Situation, orphans: OrphanReading) -> &'static str {
    use OrphanReading::{Added, Deleted};
    match (situation, orphans) {
        (Situation::NewerLeft, _) => "nl.txt",
        (Situation::NewerRight, _) => "nr.txt",
        (Situation::AddedLeft, Added) | (Situation::DeletedRight, Deleted) => "lo.txt",
        (Situation::AddedRight, Added) | (Situation::DeletedLeft, Deleted) => "ro.txt",
        (Situation::Different, _) => "df.txt",
        (Situation::Same, _) => "sm.txt",
        _ => "",
    }
}

type Tree = BTreeMap<String, (Vec<u8>, i64)>;

fn snapshot(vfs: &MemoryVfs) -> Tree {
    vfs.dump()
        .into_iter()
        .filter(|entry| entry.kind == EntryKind::File)
        .map(|entry| {
            let path = entry.path.replace('\\', "/");
            let path = path.strip_prefix("C:").map_or(path.clone(), str::to_owned);
            let bytes = vfs.read_back(&entry.path).unwrap().to_vec();
            (path, (bytes, entry.mtime.map_or(0, |t| t.unix_seconds())))
        })
        .collect()
}

/// What `action` makes of `name` in `tree`.
fn apply(tree: &mut Tree, name: &str, action: SyncAction) {
    let (l, r) = (format!("/l/{name}"), format!("/r/{name}"));
    match action {
        SyncAction::Leave => {}
        SyncAction::CopyRight => {
            let it = tree[&l].clone();
            tree.insert(r, it);
        }
        SyncAction::CopyLeft => {
            let it = tree[&r].clone();
            tree.insert(l, it);
        }
        SyncAction::DeleteLeft => {
            tree.remove(&l);
        }
        SyncAction::DeleteRight => {
            tree.remove(&r);
        }
    }
}

fn options(preset: SyncPreset, matrix: SyncMatrix, orphans: OrphanReading) -> SyncOptions {
    SyncOptions {
        preset,
        matrix,
        orphans,
        ..SyncOptions::default()
    }
}

/// Plans and runs a sync on `vfs`; every item it lets through runs.
fn run(vfs: &MemoryVfs, options: &SyncOptions) -> hikasa_folder::sync::SyncPlan {
    let rules = sync_folder_rules(&FolderRules::default(), options).unwrap();
    let cmp = compared(vfs, &rules);
    let read = cmp.read();
    let request = SyncRequest {
        options,
        just_selection: None,
        visible: None,
        include_hidden: false,
        iu: false,
        read_only: ReadOnlySides::default(),
        allow_empty_source: false,
        settings: OpSettings::default(),
    };
    let plan = plan_sync(&read, vfs, &request, &Ctl::detached()).unwrap();
    drop(read);
    let report = ops::execute_with(
        vfs,
        vfs,
        plan.op.plan.clone(),
        &plan.op.exec,
        &Ctl::detached(),
    );
    for result in &report.results {
        assert!(
            matches!(result.status, ItemStatus::Done | ItemStatus::Skipped),
            "{result:?}"
        );
    }
    plan
}

#[test]
fn a6_26_every_cell_in_both_readings() {
    let mut valid = 0;
    let mut refused = 0;
    for orphans in [OrphanReading::Added, OrphanReading::Deleted] {
        for situation in Situation::ALL {
            for action in SyncAction::ALL {
                let mut matrix = SyncMatrix::default();
                matrix.set(situation, action);
                let only = situation.only_on();
                let writes_missing_side = only.is_some()
                    && action != SyncAction::Leave
                    && (action.reads() != only && !(action.deletes() && action.writes() == only));
                if writes_missing_side {
                    assert!(matrix.validate().is_err(), "{situation} {action}");
                    let opts = options(SyncPreset::Custom, matrix, orphans);
                    assert!(
                        sync_folder_rules(&FolderRules::default(), &opts).is_err(),
                        "{situation}={action} reached a scan"
                    );
                    refused += 1;
                    continue;
                }
                matrix.validate().unwrap();
                valid += 1;
                let vfs = fixture();
                let mut expected = snapshot(&vfs);
                let name = file_of(situation, orphans);
                if !name.is_empty() {
                    apply(&mut expected, name, action);
                }
                let plan = run(&vfs, &options(SyncPreset::Custom, matrix, orphans));
                assert_eq!(
                    snapshot(&vfs),
                    expected,
                    "{situation}={action} with orphans {orphans:?}"
                );
                // The row of the situation says so; the others are left.
                let read_situations: Vec<_> =
                    plan.rows.iter().filter_map(|row| row.situation).collect();
                if !name.is_empty() {
                    assert!(read_situations.contains(&situation));
                    let row = plan
                        .rows
                        .iter()
                        .find(|row| row.situation == Some(situation))
                        .unwrap();
                    assert_eq!(row.action, action, "{situation}");
                }
                for row in plan.rows.iter().filter(|r| r.situation != Some(situation)) {
                    assert_eq!(row.action, SyncAction::Leave, "{row:?}");
                }
            }
        }
    }
    // 4 one-sided situations × 2 impossible actions, in two readings; the rest are valid.
    assert_eq!(refused, 2 * 4 * 2);
    assert_eq!(valid, 2 * (8 * 5 - 8));
}

/// §1.6.14's table, row by row: `→` copy right, `←` copy left, `dL`/`dR` delete, `—` leave.
const TABLE: [(Situation, [&str; 5]); 8] = [
    (Situation::NewerLeft, ["→", "—", "→", "→", "←"]),
    (Situation::NewerRight, ["—", "←", "←", "→", "←"]),
    (Situation::AddedLeft, ["→", "—", "→", "→", "dL"]),
    (Situation::AddedRight, ["—", "←", "←", "dR", "←"]),
    (Situation::DeletedLeft, ["—", "—", "—", "dR", "←"]),
    (Situation::DeletedRight, ["—", "—", "—", "→", "dL"]),
    (Situation::Different, ["—", "—", "—", "→", "←"]),
    (Situation::Same, ["—", "—", "—", "—", "—"]),
];
const PRESETS: [SyncPreset; 5] = [
    SyncPreset::UpdateRight,
    SyncPreset::UpdateLeft,
    SyncPreset::UpdateBoth,
    SyncPreset::MirrorRight,
    SyncPreset::MirrorLeft,
];

fn word(action: &str) -> SyncAction {
    match action {
        "→" => SyncAction::CopyRight,
        "←" => SyncAction::CopyLeft,
        "dL" => SyncAction::DeleteLeft,
        "dR" => SyncAction::DeleteRight,
        _ => SyncAction::Leave,
    }
}

#[test]
fn a6_26_presets_as_data() {
    for (column, preset) in PRESETS.into_iter().enumerate() {
        let matrix = SyncMatrix::of(preset).unwrap();
        matrix.validate().unwrap();
        for (situation, cells) in TABLE {
            assert_eq!(
                matrix.get(situation),
                word(cells[column]),
                "{preset:?} {situation}"
            );
        }
    }
    assert_eq!(SyncMatrix::of(SyncPreset::Custom), None);
    assert_eq!(SyncPreset::default(), SyncPreset::UpdateRight);
}

#[test]
fn a6_26_presets_as_runs() {
    for orphans in [OrphanReading::Added, OrphanReading::Deleted] {
        for (column, preset) in PRESETS.into_iter().enumerate() {
            let vfs = fixture();
            let mut expected = snapshot(&vfs);
            for (situation, cells) in TABLE {
                let name = file_of(situation, orphans);
                if !name.is_empty() {
                    apply(&mut expected, name, word(cells[column]));
                }
            }
            let plan = run(&vfs, &options(preset, SyncMatrix::default(), orphans));
            assert_eq!(snapshot(&vfs), expected, "{preset:?} {orphans:?}");
            // Update never overwrites a newer file and never removes; `different` is its conflict.
            if matches!(
                preset,
                SyncPreset::UpdateRight | SyncPreset::UpdateLeft | SyncPreset::UpdateBoth
            ) {
                assert_eq!(plan.counts.delete_left + plan.counts.delete_right, 0);
                assert_eq!(plan.counts.conflicts, 1, "{preset:?}");
                let conflict = plan.rows.iter().find(|row| row.conflict).unwrap();
                assert_eq!(conflict.situation, Some(Situation::Different));
            } else {
                assert_eq!(plan.counts.conflicts, 0);
            }
        }
    }
}

#[test]
fn a6_26_mirror_directions_are_not_swapped() {
    // Mirror to Right makes the right what the left is; Mirror to Left the other way round.
    let vfs = fixture();
    run(
        &vfs,
        &options(
            SyncPreset::MirrorRight,
            SyncMatrix::default(),
            OrphanReading::Added,
        ),
    );
    let tree = snapshot(&vfs);
    let side = |base: &str| -> BTreeMap<String, (Vec<u8>, i64)> {
        tree.iter()
            .filter_map(|(path, it)| {
                path.strip_prefix(base)
                    .map(|rest| (rest.to_owned(), it.clone()))
            })
            .collect()
    };
    assert_eq!(side("/l/"), side("/r/"));
    assert!(tree.contains_key("/l/lo.txt") && !tree.contains_key("/r/ro.txt"));
    let vfs = fixture();
    run(
        &vfs,
        &options(
            SyncPreset::MirrorLeft,
            SyncMatrix::default(),
            OrphanReading::Added,
        ),
    );
    let tree = snapshot(&vfs);
    assert!(tree.contains_key("/r/ro.txt") && !tree.contains_key("/l/lo.txt"));
    assert_eq!(tree["/l/nl.txt"].0, b"right older");
}

#[test]
fn a6_26_parse_words() {
    let all_leave = SyncMatrix::default();
    let m = SyncMatrix::parse("newer-left=copy-right,added_right=delete_right", all_leave).unwrap();
    assert_eq!(m.newer_left, SyncAction::CopyRight);
    assert_eq!(m.added_right, SyncAction::DeleteRight);
    assert_eq!(m.same, SyncAction::Leave);
    // A preset first is the base.
    let m = SyncMatrix::parse("mirror-right,same=copy-right", all_leave).unwrap();
    let mut expected = SyncMatrix::of(SyncPreset::MirrorRight).unwrap();
    expected.same = SyncAction::CopyRight;
    assert_eq!(m, expected);
    let m = SyncMatrix::parse(" update_both , different = copy-left ", all_leave).unwrap();
    assert_eq!(m.different, SyncAction::CopyLeft);
    assert_eq!(m.newer_left, SyncAction::CopyRight);
    // The base is the session's matrix when no preset comes first.
    let base = SyncMatrix::of(SyncPreset::UpdateLeft).unwrap();
    let m = SyncMatrix::parse("same=leave", base).unwrap();
    assert_eq!(m, base);
    for bad in [
        "",
        "newer-left=copy-right,newer_left=leave",
        "newer-left=copy-right,mirror-right",
        "custom",
        "sideways=leave",
        "newer-left=move",
        "newer-left",
        "added-left=copy-left",
        "deleted-left=delete-left",
        "newer-left=copy-right,",
    ] {
        let error = SyncMatrix::parse(bad, all_leave).unwrap_err();
        assert_eq!(
            hikasa_core::Classify::kind(&error),
            hikasa_core::ErrorKind::InvalidInput,
            "{bad:?}"
        );
    }
    assert_eq!(
        SyncPreset::parse("Mirror-Left"),
        Some(SyncPreset::MirrorLeft)
    );
    assert_eq!(
        SyncPreset::parse("update_right"),
        Some(SyncPreset::UpdateRight)
    );
    assert_eq!(
        SyncAction::parse("DELETE-LEFT"),
        Some(SyncAction::DeleteLeft)
    );
    assert_eq!(
        OrphanReading::parse("deleted"),
        Some(OrphanReading::Deleted)
    );
    assert_eq!(OrphanReading::parse("gone"), None);
}

#[test]
fn a6_26_round_trip_through_toml() {
    let mut options = SyncOptions {
        preset: SyncPreset::Custom,
        orphans: OrphanReading::Deleted,
        create_empty: true,
        within_archives: true,
        recycle: true,
        recycle_fallback: true,
        ..SyncOptions::default()
    };
    options.matrix.newer_left = SyncAction::CopyRight;
    options.matrix.deleted_left = SyncAction::DeleteRight;
    options
        .overrides
        .insert("a/b.txt".to_owned(), SyncAction::Leave);
    let rules = SyncRules {
        folder: FolderRules::default(),
        sync: options.clone(),
    };
    let text = toml::to_string(&rules).unwrap();
    assert!(text.contains("[sync]"), "{text}");
    assert!(text.contains("[sync.matrix]"), "{text}");
    assert!(text.contains("\"a/b.txt\" = \"leave\""), "{text}");
    let back: SyncRules = toml::from_str(&text).unwrap();
    assert_eq!(back, rules);
    // Sparse: a matrix of two cells reads with the others `leave`; an empty table is the defaults.
    let sparse: SyncOptions =
        toml::from_str("preset = \"custom\"\n[matrix]\nnewer_left = \"copy_right\"\n").unwrap();
    assert_eq!(sparse.matrix.newer_left, SyncAction::CopyRight);
    assert_eq!(sparse.matrix.same, SyncAction::Leave);
    let empty: SyncRules = toml::from_str("").unwrap();
    assert_eq!(empty, SyncRules::default());
    assert_eq!(empty.sync.preset, SyncPreset::UpdateRight);
    // The folder rules of a folder-compare `[rules]` read as they did, with `sync` beside them.
    let folder_text = toml::to_string(&FolderRules::default()).unwrap();
    let as_sync: SyncRules = toml::from_str(&folder_text).unwrap();
    assert_eq!(as_sync.folder, FolderRules::default());
    let _ = mspec("/l");
}
