//! D-45 prefilter counting test (§6.2.6): entries excluded by name masks are never stat'ed (the back-end reports
//! them name-only), entries whose record states no kind are stat'ed, and the filtered side of an FC-054 pair is
//! stat'ed exactly once, after alignment.

mod common;

use common::*;
use hikasa_core::{CancelToken, CaseSensitivity, FileTime};
use hikasa_folder::{FolderRules, FolderRulesPatch, NodeFlags, NullObserver};
use hikasa_vfs::{MemMeta, MemoryVfs};

fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = MemMeta {
        mtime: Some(FileTime::parse(DEFAULT_MTIME).unwrap()),
        ..MemMeta::default()
    };
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(base), meta.clone()).unwrap();
        for f in ["keep.txt", "skip.log", "sub/x.log", "obj/inner.o"] {
            vfs.insert_file(&host(&format!("{base}/{f}")), b"x".to_vec(), meta.clone())
                .unwrap();
        }
    }
    vfs.insert_file(&host("/l/x.bak"), b"same".to_vec(), meta.clone())
        .unwrap();
    vfs.insert_file(&host("/r/x.txt"), b"same".to_vec(), meta.clone())
        .unwrap();
    vfs.insert_file(&host("/l/u.log"), b"u".to_vec(), meta)
        .unwrap();
    vfs.set_kind_unknown(&host("/l/u.log")).unwrap();
    vfs
}

#[test]
fn excluded_names_are_not_stated_and_fc054_sides_once() {
    let vfs = tree();
    let counting = CountingVfs::new(&vfs);
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some(r"-*.log;-obj\;-*.bak".into()),
            align_extensions: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let engines = engines(64);
    let now = default_now();
    let cancel = CancelToken::never();
    let cmp = run_on(
        &counting,
        &rules,
        None,
        &engines,
        &now,
        &NullObserver,
        &cancel,
        false,
    )
    .unwrap();
    cmp.check_invariants().unwrap();
    // `stat` calls: the two bases and the filtered side of the FC-054 pair (x.bak ↔ x.txt), once.
    assert_eq!(counting.stat_count("/l"), 1);
    assert_eq!(counting.stat_count("/r"), 1);
    assert_eq!(counting.stat_count("/l/x.bak"), 1);
    assert_eq!(
        counting.total_stats(),
        3,
        "{:?}",
        counting.stats.lock().unwrap()
    );
    // Every metadata read of the back-end: the 3 `stat` calls, plus the entries the listings described: left
    // root keep.txt, sub, u.log (kind unknown: always stat'ed); right root keep.txt, sub, x.txt. The declined
    // names (skip.log, obj, x.bak, sub/x.log on both sides) were never read.
    assert_eq!(vfs.stat_calls(), 3 + 3 + 3);
    // obj is excluded on both sides: not listed.
    assert_eq!(counting.lists.load(std::sync::atomic::Ordering::SeqCst), 4);
    let tree = cmp.to_canonical();
    let skip = find(&tree, "skip.log");
    assert!(skip.flags.contains(&"left_stat_skipped".to_owned()));
    assert!(skip.flags.contains(&"right_stat_skipped".to_owned()));
    let pair = find(&tree, "x.bak");
    assert_eq!(entry_name(&pair.right), Some("x.txt"));
    assert_eq!(pair.flags, ["left_filtered"]);
    assert_eq!(
        pair.left.as_ref().unwrap().size,
        4,
        "stat'ed after alignment"
    );
    assert_eq!(pair.state, hikasa_folder::CompareState::Same);
    let unknown = find(&tree, "u.log");
    assert_eq!(
        unknown.flags,
        ["left_filtered"],
        "stat'ed, then filtered by name"
    );
    let read = cmp.read();
    let x = read
        .children(read.root())
        .find(|&id| {
            read.node(id)
                .unwrap()
                .left
                .is_some_and(|e| e.name == "x.bak")
        })
        .unwrap();
    let flags = read.node(x).unwrap().flags;
    assert!(flags.contains(NodeFlags::LEFT_FILTERED));
    assert!(!flags.contains(NodeFlags::LEFT_STAT_SKIPPED));
}

#[test]
fn match_all_filters_pass_no_prefilter() {
    let vfs = tree();
    let engines = engines(64);
    let now = default_now();
    let cancel = CancelToken::never();
    let before = vfs.stat_calls();
    let cmp = run_on(
        &vfs,
        &FolderRules::default(),
        None,
        &engines,
        &now,
        &NullObserver,
        &cancel,
        false,
    )
    .unwrap();
    // Bases (2) + every entry of both trees (left: keep, skip, sub, sub/x.log, obj, obj/inner.o, x.bak, u.log;
    // right: keep, skip, sub, sub/x.log, obj, obj/inner.o, x.txt).
    assert_eq!(vfs.stat_calls() - before, 2 + 8 + 7);
    assert_eq!(cmp.stats(false).files.filtered, 0);
}
