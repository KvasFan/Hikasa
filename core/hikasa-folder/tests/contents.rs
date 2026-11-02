//! M2 W6a acceptance A6.12 (FC-029 b and c, FC-040; D-174): contents on demand update the model in place.

mod common;

use common::{compared, engines, host, with_env};
use hikasa_core::{
    CancelToken, CaseSensitivity, ContentKind, ContentMethod, ContentOutcome, ContentResult,
    ErrorKind, FileTime, Pair, RelPath, Side,
};
use hikasa_folder::{
    CompareState, ContentReport, FolderComparison, FolderError, FolderRules, NodeId,
    check_tree_invariants, compare_contents, compute_crcs,
};
use hikasa_vfs::ops::ItemStamp;
use hikasa_vfs::{EntryKind, Fault, MemMeta, MemoryVfs, VfsOp};

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    let meta = MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    };
    vfs.insert_file(&host(path), content, meta).unwrap();
}

/// Pairs equal by size and time, so no quick test tells them apart.
fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    file(&vfs, "/l/d/same", b"aaa", 10);
    file(&vfs, "/r/d/same", b"aaa", 10);
    file(&vfs, "/l/d/differ", b"bbb", 10);
    file(&vfs, "/r/d/differ", b"ccc", 10);
    file(&vfs, "/l/top", b"ttt", 10);
    file(&vfs, "/r/top", b"ttt", 10);
    file(&vfs, "/l/only", b"o", 10);
    vfs
}

fn id(cmp: &FolderComparison, path: &str) -> NodeId {
    cmp.read().node_at(&RelPath::parse(path).unwrap()).unwrap()
}

fn run(
    cmp: &FolderComparison,
    vfs: &MemoryVfs,
    nodes: &[NodeId],
    method: ContentMethod,
) -> ContentReport {
    let engines = engines(1 << 30);
    with_env(vfs, &engines, &CancelToken::new(), |env| {
        compare_contents(cmp.model(), nodes, method, env).unwrap()
    })
}

#[test]
fn a6_12_a_pair_and_a_folder_with_each_method() {
    for method in [
        ContentMethod::Crc,
        ContentMethod::Binary,
        ContentMethod::Rules,
    ] {
        let vfs = tree();
        let cmp = compared(&vfs, &FolderRules::default());
        assert_eq!(
            cmp.read().node(id(&cmp, "d/differ")).unwrap().state,
            CompareState::Same
        );
        let pair = run(&cmp, &vfs, &[id(&cmp, "top")], method);
        assert_eq!(pair.items.len(), 1, "{method:?}");
        assert_eq!(pair.items[0].path, "top");
        assert!(pair.complete);
        let folder = run(&cmp, &vfs, &[id(&cmp, "d")], method);
        assert_eq!(folder.items.len(), 2, "{method:?}");
        assert_eq!(
            cmp.read().node(id(&cmp, "d/differ")).unwrap().state,
            CompareState::Different,
            "{method:?}"
        );
        check_tree_invariants(&cmp.read()).unwrap();
    }
}

/// A pair already tested counts once, whatever method came first, and the root's statistics equal a recount.
#[test]
fn a6_12_a_repeated_test_counts_once() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let d = id(&cmp, "d");
    run(&cmp, &vfs, &[d], ContentMethod::Crc);
    let first = cmp.stats(false).content;
    run(&cmp, &vfs, &[d], ContentMethod::Crc);
    run(&cmp, &vfs, &[d], ContentMethod::Binary);
    assert_eq!(cmp.stats(false).content, first);
    assert_eq!(first.binary_same + first.binary_different, 2);
    check_tree_invariants(&cmp.read()).unwrap();
    // A row that changes its mind does not leave its old result behind.
    let again = run(&cmp, &vfs, &[id(&cmp, "d/differ")], ContentMethod::Crc);
    assert_eq!(again.counts.binary_different, 1);
    check_tree_invariants(&cmp.read()).unwrap();
}

/// A failing read is an error row, and a later test clears it.
#[test]
fn a6_12_a_failing_read_is_an_error_row() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let top = id(&cmp, "top");
    vfs.inject(Fault {
        path: host("/l/top"),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    let report = run(&cmp, &vfs, &[top], ContentMethod::Binary);
    assert_eq!(report.counts.error, 1);
    assert_eq!(cmp.read().node(top).unwrap().state, CompareState::Error);
    assert!(!cmp.read().errors(top).is_empty());
    check_tree_invariants(&cmp.read()).unwrap();
    vfs.clear_faults();
    run(&cmp, &vfs, &[top], ContentMethod::Binary);
    assert_eq!(cmp.read().node(top).unwrap().state, CompareState::Same);
    assert!(cmp.read().errors(top).is_empty());
    assert_eq!(cmp.stats(false).content.error, 0);
    check_tree_invariants(&cmp.read()).unwrap();
}

/// Cancellation restores the old state, no pending rows.
#[test]
fn a6_12_cancellation_restores_the_old_state() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let d = id(&cmp, "d");
    let before = cmp.to_canonical();
    let engines = engines(1 << 30);
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = with_env(&vfs, &engines, &cancel, |env| {
        compare_contents(cmp.model(), &[d], ContentMethod::Binary, env).unwrap()
    });
    assert!(!report.complete);
    assert!(report.items.is_empty());
    assert_eq!(cmp.to_canonical(), before);
    check_tree_invariants(&cmp.read()).unwrap();
}

/// The budget is the one of `env`: a pair larger than it still runs, alone, with the comparators of a scan.
#[test]
fn a6_12_the_budget_is_shared() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let contents = engines(1 << 30);
    let progress = hikasa_core::NullProgress;
    let now = common::default_now();
    let cancel = CancelToken::new();
    let pool = common::pool(4);
    let env = hikasa_folder::FolderEnv {
        vfs: &vfs,
        content: &contents,
        pool: pool.as_ref(),
        ctl: hikasa_core::Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &hikasa_folder::NullObserver,
        now: &now,
        content_budget: 1,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let report = compare_contents(
        cmp.model(),
        &[id(&cmp, "d"), id(&cmp, "top")],
        ContentMethod::Binary,
        &env,
    );
    let report = report.unwrap();
    assert_eq!(report.items.len(), 3);
    assert!(report.complete);
}

#[test]
fn a6_12_a_method_is_needed() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let engines = engines(1 << 30);
    with_env(&vfs, &engines, &CancelToken::new(), |env| {
        let result = compare_contents(cmp.model(), &[NodeId::ROOT], ContentMethod::None, env);
        assert!(matches!(result, Err(FolderError::InvalidRules(_))));
    });
}

/// `compute_crcs` reads a local file, and keeps the result.
#[test]
fn a6_12_compute_crcs_reads_and_keeps() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let (top, only) = (id(&cmp, "top"), id(&cmp, "only"));
    let engines = engines(1 << 30);
    let results = with_env(&vfs, &engines, &CancelToken::new(), |env| {
        compute_crcs(
            cmp.model(),
            &[(top, Side::Left), (only, Side::Right), (only, Side::Left)],
            env,
        )
        .unwrap()
    });
    assert_eq!(
        results[0].2.as_ref().copied().unwrap(),
        crc32fast_of(b"ttt")
    );
    assert!(results[1].2.is_err(), "no such side");
    assert_eq!(results[2].2.as_ref().copied().unwrap(), crc32fast_of(b"o"));
    let read = cmp.read();
    assert_eq!(
        read.node(top).unwrap().left.unwrap().crc32,
        Some(crc32fast_of(b"ttt"))
    );
    assert_eq!(read.node(top).unwrap().right.unwrap().crc32, None);
    assert_eq!(
        read.node(only).unwrap().left.unwrap().crc32,
        Some(crc32fast_of(b"o"))
    );
}

fn crc32fast_of(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn stamp(size: u64, secs: i64) -> ItemStamp {
    ItemStamp {
        kind: EntryKind::File,
        size: Some(size),
        mtime: FileTime::from_unix(secs, 0),
    }
}

fn binary(result: ContentResult) -> ContentOutcome {
    ContentOutcome {
        method: ContentMethod::Rules,
        result,
        warnings: Vec::new(),
    }
}

/// `apply_child_result`, with equal and changed stamps, replacing the row's result.
#[test]
fn a6_12_a_child_result() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let row = id(&cmp, "d/differ");
    let model = cmp.model();
    let stale = Pair::new(stamp(3, 11), stamp(3, 10));
    assert!(!model.apply_child_result(row, stale, binary(ContentResult::ImportantDifferences)));
    assert_eq!(cmp.read().node(row).unwrap().content, None);
    let fresh = Pair::new(stamp(3, 10), stamp(3, 10));
    assert!(model.apply_child_result(row, fresh, binary(ContentResult::ImportantDifferences)));
    let view = cmp
        .read()
        .node(row)
        .map(|v| (v.state, v.content.map(|c| c.kind)));
    assert_eq!(
        view,
        Some((
            CompareState::Different,
            Some(ContentKind::ImportantDifferences)
        ))
    );
    assert!(model.apply_child_result(row, fresh, binary(ContentResult::RulesSame)));
    assert_eq!(cmp.stats(false).content.important_differences, 0);
    assert_eq!(cmp.stats(false).content.rules_same, 1);
    assert_eq!(cmp.read().node(row).unwrap().state, CompareState::Same);
    check_tree_invariants(&cmp.read()).unwrap();
}
