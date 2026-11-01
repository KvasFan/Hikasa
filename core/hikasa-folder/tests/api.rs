//! Behaviour of the public API: whole-comparison errors, the live model read during a scan, the colour table of
//! §2.6.4, verdict cases (§2.6.5, D-02), and the per-node queries.

mod common;

use std::sync::Arc;

use common::*;
use hikasa_core::{CancelToken, CompareVerdict, ContentMethod, Ctl, ExitCode, NullProgress, Side};
use hikasa_folder::{
    ChannelObserver, ColourClass, CompareState, ContentEngines, DisplayOptions, FolderComparison,
    FolderDisplayFilter, FolderEnv, FolderError, FolderMode, FolderModel, FolderRules,
    FolderRulesPatch, NodeId, NullObserver, ScanEvent, check_tree_invariants, compare_into,
    visible_tree,
};
use hikasa_vfs::{MemMeta, MemoryVfs, StreamOptions};

fn tree(text: &str) -> (TreeFile, MemoryVfs) {
    let tree: TreeFile = toml::from_str(text).unwrap();
    let vfs = materialize(&tree);
    (tree, vfs)
}

fn id_of(cmp: &FolderComparison, path: &str) -> NodeId {
    let read = cmp.read();
    let mut id = read.root();
    for name in path.split('/') {
        id = read
            .children(id)
            .find(|&c| {
                read.node(c)
                    .unwrap()
                    .representative()
                    .is_some_and(|e| e.name == name)
            })
            .unwrap_or_else(|| panic!("no {path}"));
    }
    id
}

#[test]
fn whole_comparison_errors() {
    let vfs = MemoryVfs::default();
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_file(&host("/file"), b"x".to_vec(), MemMeta::default())
        .unwrap();
    let engines = engines(16);
    let now = default_now();
    let cancel = CancelToken::never();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs: &vfs,
        content: &engines,
        pool: None,
        ctl: Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 20,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let rules = FolderRules::default();
    // A missing base → BaseNotLoadable (exit 107 at the CLI).
    let model = FolderModel::new(mspec("/l"), mspec("/missing"), rules.clone()).unwrap();
    match compare_into(&model, &env) {
        Err(FolderError::BaseNotLoadable { side, .. }) => assert_eq!(side, Side::Right),
        other => panic!("{other:?}"),
    }
    assert!(model.is_finished());
    assert!(!model.read().is_complete());
    // Scanning a model twice → AlreadyScanned.
    assert!(matches!(
        compare_into(&model, &env),
        Err(FolderError::AlreadyScanned)
    ));
    // A file base → BaseNotFolder.
    let model = FolderModel::new(mspec("/file"), mspec("/l"), rules.clone()).unwrap();
    assert!(matches!(
        compare_into(&model, &env),
        Err(FolderError::BaseNotFolder {
            side: Side::Left,
            ..
        })
    ));
    // Cancelled before the bases are loaded → Err(Cancelled), no event.
    let cancelled = CancelToken::new();
    cancelled.cancel();
    let (observer, rx) = ChannelObserver::bounded(16);
    let env_cancelled = FolderEnv {
        ctl: Ctl {
            cancel: &cancelled,
            progress: &progress,
        },
        observer: &observer,
        ..env
    };
    let model = FolderModel::new(mspec("/l"), mspec("/l"), rules.clone()).unwrap();
    assert!(matches!(
        compare_into(&model, &env_cancelled),
        Err(FolderError::Cancelled)
    ));
    assert!(rx.try_recv().is_err());
    // Rules requested without a rules comparator: a configuration error; the model is not consumed.
    let mut with_rules = rules.clone();
    with_rules.criteria.content.method = ContentMethod::Rules;
    let model = FolderModel::new(mspec("/l"), mspec("/l"), with_rules).unwrap();
    let no_rules = ContentEngines::with_stream_options(StreamOptions::default(), None);
    let env_no_rules = FolderEnv {
        content: &no_rules,
        ..env
    };
    assert!(matches!(
        compare_into(&model, &env_no_rules),
        Err(FolderError::RulesComparatorMissing)
    ));
    assert!(compare_into(&model, &env).is_ok());
    // Invalid rules are rejected by FolderModel::new.
    let mut bad = rules;
    bad.filters.names.include_files = vec!["[a".into()];
    assert!(matches!(
        FolderModel::new(mspec("/l"), mspec("/l"), bad),
        Err(FolderError::Filter(_))
    ));
}

#[test]
fn live_model_is_readable_during_the_scan() {
    // A wide and deep tree, scanned on a pool while the main thread reads the model at every event.
    let vfs = MemoryVfs::default();
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(base), MemMeta::default()).unwrap();
        for d in 0..12 {
            for f in 0..15 {
                let content = if base == "/r" && f == 3 {
                    b"changed".to_vec()
                } else {
                    b"x".to_vec()
                };
                vfs.insert_file(
                    &host(&format!("{base}/d{d}/s{}/f{f}.txt", d % 3)),
                    content,
                    MemMeta::default(),
                )
                .unwrap();
            }
        }
    }
    let model = FolderModel::new(mspec("/l"), mspec("/r"), FolderRules::default()).unwrap();
    let (observer, rx) = ChannelObserver::bounded(4);
    let pool = pool(4).unwrap();
    let scanner = {
        let model = Arc::clone(&model);
        std::thread::spawn(move || {
            let engines = engines(16);
            let now = default_now();
            let cancel = CancelToken::never();
            let progress = NullProgress;
            let env = FolderEnv {
                vfs: &vfs,
                content: &engines,
                pool: Some(&pool),
                ctl: Ctl {
                    cancel: &cancel,
                    progress: &progress,
                },
                observer: &observer,
                now: &now,
                content_budget: 1 << 20,
                archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
            };
            compare_into(&model, &env).map(|c| c.verdict(false))
        })
    };
    let mut events = Vec::new();
    let mut last_generation = 0;
    while let Ok(event) = rx.recv() {
        let read = model.read();
        check_tree_invariants(&read).unwrap();
        // Partial trees have O(1) statistics and verdicts.
        let _ = read.stats(false);
        let _ = read.verdict(true);
        let generation = model.generation();
        assert!(generation >= last_generation);
        last_generation = generation;
        if let ScanEvent::FolderListed { folder } = &event {
            assert_eq!(
                read.node(*folder).unwrap().scan,
                hikasa_folder::ScanStatus::Scanned
            );
        }
        drop(read);
        let finished = matches!(event, ScanEvent::Finished { .. });
        events.push(event);
        if finished {
            break;
        }
    }
    let verdict = scanner.join().unwrap().unwrap();
    assert_eq!(verdict, CompareVerdict::Differences);
    assert_eq!(events.first(), Some(&ScanEvent::Started));
    assert_eq!(events.last(), Some(&ScanEvent::Finished { complete: true }));
    let listed = events
        .iter()
        .filter(|e| matches!(e, ScanEvent::FolderListed { .. }))
        .count();
    // The root, 12 folders d0..d11 and the one s-folder inside each.
    assert_eq!(listed, 1 + 12 + 12);
    assert!(model.is_finished());
    assert!(model.read().is_complete());
}

const COLOURS: &str = r#"
[[left]]
path = "same.txt"
content = "s"
[[right]]
path = "same.txt"
content = "s"
[[left]]
path = "diff.txt"
content = "a"
[[right]]
path = "diff.txt"
content = "bb"
[[left]]
path = "lnewer.txt"
content = "n"
mtime = "2026-05-01T00:00:00Z"
[[right]]
path = "lnewer.txt"
content = "n"
[[left]]
path = "rnewer.txt"
content = "n"
[[right]]
path = "rnewer.txt"
content = "n"
mtime = "2026-05-01T00:00:00Z"
[[left]]
path = "lonly.txt"
content = "o"
[[left]]
path = "err.txt"
content = "e"
[[right]]
path = "err.txt"
content = "e"
[[left]]
path = "x.bak"
content = "b"
[[right]]
path = "x.bak"
content = "b"
[[left]]
path = "fsame/a.txt"
content = "a"
[[right]]
path = "fsame/a.txt"
content = "a"
[[left]]
path = "fdiff/a.txt"
content = "a"
[[right]]
path = "fdiff/a.txt"
content = "aa"
[[left]]
path = "forph/a.txt"
content = "a"
[[right]]
path = "forph/b.txt"
content = "b"
[[left]]
path = "frnewer/a.txt"
content = "a"
[[right]]
path = "frnewer/a.txt"
content = "a"
mtime = "2026-05-01T00:00:00Z"
[[left]]
path = "fonly/a.txt"
content = "a"
[[left]]
path = "ffail/a.txt"
content = "a"
[[right]]
path = "ffail/a.txt"
content = "a"
[[right]]
path = "ffail/sub/b.txt"
content = "b"
[[left]]
path = "ferr/e.txt"
content = "e"
[[right]]
path = "ferr/e.txt"
content = "e"
[[left]]
path = "ffilt/only.bak"
content = "x"
[[right]]
path = "ffilt/only.bak"
content = "x"
[[faults]]
side = "left"
path = "err.txt"
op = "stat"
kind = "permission_denied"
[[faults]]
side = "left"
path = "ffail"
op = "list"
kind = "permission_denied"
[[faults]]
side = "right"
path = "ferr/e.txt"
op = "stat"
kind = "io"
"#;

/// §2.6.4 colour table, every row.
#[test]
fn colour_table() {
    let (tree, vfs) = tree(COLOURS);
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some("-*.bak".into()),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let cmp = run_tree(&tree, &vfs, &rules, None).unwrap();
    cmp.check_invariants().unwrap();
    let read = cmp.read();
    let colours = |path: &str| {
        let id = id_of(&cmp, path);
        (
            read.colour(id, Side::Left, false),
            read.colour(id, Side::Right, false),
        )
    };
    use ColourClass::*;
    assert_eq!(colours("same.txt"), (Same, Same));
    assert_eq!(colours("diff.txt"), (NewerOrDifferent, NewerOrDifferent));
    assert_eq!(colours("lnewer.txt"), (NewerOrDifferent, OlderOrUnknown));
    assert_eq!(colours("rnewer.txt"), (OlderOrUnknown, NewerOrDifferent));
    assert_eq!(colours("lonly.txt"), (Orphan, Absent));
    assert_eq!(colours("err.txt"), (Error, Error));
    assert_eq!(colours("x.bak"), (Filtered, Filtered));
    assert_eq!(colours("fsame"), (Same, Same));
    assert_eq!(colours("fdiff"), (NewerOrDifferent, NewerOrDifferent));
    assert_eq!(colours("forph"), (Orphan, Orphan));
    assert_eq!(colours("frnewer"), (OlderOrUnknown, NewerOrDifferent));
    assert_eq!(colours("fonly"), (Orphan, Absent));
    assert_eq!(colours("ffail"), (Error, Error));
    assert_eq!(
        colours("ffail/sub"),
        (Absent, OlderOrUnknown),
        "below a failed listing: unknown"
    );
    assert_eq!(colours("ffail/a.txt"), (Absent, OlderOrUnknown));
    assert_eq!(colours("ferr"), (Error, Error));
    assert_eq!(colours("ffilt"), (Filtered, Filtered), "only filtered");
    assert_eq!(read.colour(read.root(), Side::Center, false), Absent);
    // A folder still being scanned (cancelled before its listing) is "older or unknown".
    drop(read);
    let mut cancelled = TreeFile {
        options: TreeOptions {
            cancel_after_listed: Some(1),
            ..TreeOptions::default()
        },
        ..toml::from_str(COLOURS).unwrap()
    };
    cancelled.description.clear();
    let partial = run_tree(&cancelled, &vfs, &rules, None).unwrap();
    let read = partial.read();
    let id = id_of(&partial, "fsame");
    assert_eq!(
        read.node(id).unwrap().scan,
        hikasa_folder::ScanStatus::NotScanned
    );
    assert_eq!(read.colour(id, Side::Left, false), OlderOrUnknown);
    assert_eq!(read.colour(id, Side::Right, false), OlderOrUnknown);
}

#[test]
fn verdict_cases() {
    let verdict = |text: &str, method: ContentMethod| {
        let (tree, vfs) = tree(text);
        let mut rules = FolderRules::default();
        rules.criteria.content.method = method;
        let cmp = run_tree(&tree, &vfs, &rules, None).unwrap();
        (cmp.verdict(false), cmp.verdict(true))
    };
    let same = r#"
[[left]]
path = "a.txt"
content = "a"
[[right]]
path = "a.txt"
content = "a"
"#;
    assert_eq!(
        verdict(same, ContentMethod::None),
        (CompareVerdict::NoDifferences, CompareVerdict::NoDifferences)
    );
    let unimportant = r#"
[[left]]
path = "a.txt"
content = "a b"
[[right]]
path = "a.txt"
content = "ab"
"#;
    let v = verdict(unimportant, ContentMethod::Rules);
    assert_eq!(
        v,
        (
            CompareVerdict::UnimportantOnly,
            CompareVerdict::NoDifferences
        )
    );
    assert_eq!(ExitCode::for_verdict(v.0).code(), 12);
    assert_eq!(ExitCode::for_verdict(v.1).code(), 2);
    let important = r#"
[[left]]
path = "a.txt"
content = "one"
[[right]]
path = "a.txt"
content = "two"
"#;
    assert_eq!(
        verdict(important, ContentMethod::Rules),
        (CompareVerdict::Differences, CompareVerdict::Differences)
    );
    let error = r#"
[[left]]
path = "a.txt"
content = "!error"
[[right]]
path = "a.txt"
content = "x y"
[[left]]
path = "b.txt"
content = "a b"
[[right]]
path = "b.txt"
content = "ab"
"#;
    // An error makes the result incomplete; unimportant differences do not outweigh it (D-02).
    assert_eq!(
        verdict(error, ContentMethod::Rules),
        (CompareVerdict::Incomplete, CompareVerdict::Incomplete)
    );
    assert_eq!(
        ExitCode::for_verdict(CompareVerdict::Incomplete).code(),
        100
    );
}

#[test]
fn node_queries() {
    let (tree, vfs) = tree(
        r#"
[options]
rules_limit = 4
[[left]]
path = "dir/sub/big.txt"
content = "0123456789"
[[right]]
path = "dir/sub/big.txt"
content = "0123456789"
[[left]]
path = "dir/broken.txt"
content = "!error"
[[right]]
path = "dir/broken.txt"
content = "?"
"#,
    );
    let mut rules = FolderRules::default();
    rules.criteria.content.method = ContentMethod::Rules;
    let cmp = run_tree(&tree, &vfs, &rules, None).unwrap();
    let read = cmp.read();
    let big = id_of(&cmp, "dir/sub/big.txt");
    assert_eq!(read.path(big).as_str(), "dir/sub/big.txt");
    assert_eq!(
        read.spec(big, Side::Left),
        Some(mspec("/l/dir/sub/big.txt"))
    );
    assert_eq!(
        read.spec(big, Side::Right),
        Some(mspec("/r/dir/sub/big.txt"))
    );
    assert_eq!(read.spec(read.root(), Side::Right), Some(mspec("/r")));
    let outcome = read.content_outcome(big).unwrap();
    assert_eq!(outcome.method, ContentMethod::Rules);
    assert_eq!(outcome.warnings.len(), 1, "rules_size_limit is kept");
    assert!(
        read.node(big)
            .unwrap()
            .flags
            .contains(hikasa_folder::NodeFlags::RULES_SKIPPED_SIZE)
    );
    let broken = id_of(&cmp, "dir/broken.txt");
    assert_eq!(read.node(broken).unwrap().state, CompareState::Error);
    assert!(matches!(
        read.content_outcome(broken).unwrap().result,
        hikasa_core::ContentResult::Error { .. }
    ));
    assert_eq!(read.errors(broken).len(), 1);
    assert_eq!(read.totals(big).left.bytes, 10);
    assert_eq!(read.aggregates(big), hikasa_folder::Aggregates::default());
    let dir = id_of(&cmp, "dir");
    assert_eq!(read.totals(dir).left.files, 2);
    assert_eq!(read.totals(dir).left.folders, 1);
    assert_eq!(
        read.ancestors(big),
        vec![id_of(&cmp, "dir/sub"), dir, read.root()]
    );
    let pre: Vec<(String, u32)> = read
        .preorder()
        .map(|(id, depth)| (read.path(id).as_str().to_owned(), depth))
        .collect();
    assert_eq!(
        pre,
        [
            ("dir".to_owned(), 0),
            ("dir/sub".to_owned(), 1),
            ("dir/sub/big.txt".to_owned(), 2),
            ("dir/broken.txt".to_owned(), 1)
        ]
    );
    assert!(read.node(NodeId(9_999)).is_none());
    assert_eq!(read.children(NodeId(9_999)).len(), 0);
    assert_eq!(read.case(), Some(hikasa_core::CaseSensitivity::Sensitive));
    assert!(read.bases().left.is_some());
    let opts = DisplayOptions {
        filter: FolderDisplayFilter::Differences,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    drop(read);
    let visible = visible_tree(&cmp, &opts, &|_| true);
    let paths: Vec<String> = visible
        .rows()
        .iter()
        .map(|r| cmp.read().path(r.node).as_str().to_owned())
        .collect();
    assert_eq!(
        paths,
        ["dir", "dir/broken.txt"],
        "Error counts as a difference (D-19)"
    );
    assert!(!visible.is_empty() && visible.len() == 2);
    let collapsed = visible_tree(&cmp, &opts, &|id| id != dir);
    assert_eq!(collapsed.len(), 1);
    let debug = format!("{cmp:?}");
    assert!(debug.contains("FolderComparison") && debug.contains("complete"));
    assert_eq!(
        serde_json::to_string(&opts).unwrap(),
        r#"{"filter":"differences","folders":"compare_structure","ignore_unimportant":false,"suppress_filters":false}"#
    );
}

#[test]
fn engines_and_env_debug() {
    let engines = ContentEngines::with_stream_options(
        StreamOptions {
            threads: 4,
            ..StreamOptions::default()
        },
        None,
    );
    assert!(engines.get(ContentMethod::Crc).is_some());
    assert!(engines.get(ContentMethod::Rules).is_none());
    assert!(engines.get(ContentMethod::None).is_none());
    assert!(format!("{engines:?}").contains("Crc"));
    let model = FolderModel::new(mspec("/l"), mspec("/r"), FolderRules::default()).unwrap();
    assert!(format!("{model:?}").contains("FolderModel"));
    let read = model.read();
    assert_eq!(read.len(), 1);
    assert!(!read.is_empty());
    assert_eq!(read.node(read.root()).unwrap().state, CompareState::Unknown);
    assert_eq!(read.verdict(false), CompareVerdict::Incomplete);
    assert!(read.bases().left.is_none() && read.case().is_none());
}
