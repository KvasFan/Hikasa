//! M2 W6a acceptance A6.9 (FC-033, D-171): Ignored marks. A flag changes what a category reads as, and nothing else.

mod common;

use common::{compared, host, mspec};
use hikasa_core::{CaseSensitivity, CompareVerdict, ContentMethod, ErrorKind, FileTime, Side};
use hikasa_folder::{
    CaseMode, Category, CompareState, DisplayOptions, FolderComparison, FolderDisplayFilter,
    FolderMode, FolderRules, NodeFlags, NodeId, check_tree_invariants,
};
use hikasa_vfs::{Fault, MemMeta, MemoryVfs, VfsOp};

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

/// A tree with a difference of each kind.
fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    file(&vfs, "/l/same", b"s", 10);
    file(&vfs, "/r/same", b"s", 10);
    file(&vfs, "/l/newer", b"n", 20);
    file(&vfs, "/r/newer", b"n", 10);
    file(&vfs, "/l/a/x", b"xx", 10);
    file(&vfs, "/r/a/x", b"yyy", 10);
    file(&vfs, "/l/a/keep", b"k", 10);
    file(&vfs, "/r/a/keep", b"kk", 10);
    file(&vfs, "/l/only", b"o", 10);
    vfs
}

fn marked(list: &[&str]) -> FolderRules {
    FolderRules {
        ignored: list.iter().map(|&path| path.to_owned()).collect(),
        ..FolderRules::default()
    }
}

fn id(cmp: &FolderComparison, path: &str) -> NodeId {
    let rel = hikasa_core::RelPath::parse(path).unwrap();
    cmp.read().node_at(&rel).unwrap()
}

fn category(cmp: &FolderComparison, path: &str) -> Category {
    cmp.read().category(id(cmp, path), false)
}

fn flagged(cmp: &FolderComparison, path: &str) -> bool {
    let read = cmp.read();
    read.node(id(cmp, path))
        .unwrap()
        .flags
        .contains(NodeFlags::IGNORED)
}

#[test]
fn a6_9_paths_folders_exceptions_and_the_longest_path() {
    let vfs = tree();
    let cmp = compared(&vfs, &marked(&["newer", "a", "!a/keep"]));
    cmp.check_invariants().unwrap();
    assert!(flagged(&cmp, "newer"));
    assert!(flagged(&cmp, "a"));
    assert!(flagged(&cmp, "a/x"), "inherited from the folder");
    assert!(!flagged(&cmp, "a/keep"), "an exception");
    assert!(!flagged(&cmp, "only"));
    assert_eq!(category(&cmp, "newer"), Category::Same);
    assert_eq!(category(&cmp, "a/x"), Category::Same);
    assert_eq!(category(&cmp, "a/keep"), Category::Different);
    assert_eq!(category(&cmp, "only"), Category::LeftOrphan);
    // The exception's difference is still under the folder.
    assert_eq!(
        cmp.read()
            .aggregates(id(&cmp, "a"))
            .get(false)
            .files
            .different,
        1
    );
    // A longer mark wins over an exception above it.
    let cmp = compared(&vfs, &marked(&["!a", "a/x"]));
    assert!(flagged(&cmp, "a/x"));
    assert!(!flagged(&cmp, "a/keep"));
}

#[test]
fn a6_9_case_follows_the_alignment() {
    let vfs = tree();
    let mut rules = marked(&["NEWER"]);
    let cmp = compared(&vfs, &rules);
    assert!(!flagged(&cmp, "newer"), "case-sensitive");
    rules.alignment.case = CaseMode::Insensitive;
    let cmp = compared(&vfs, &rules);
    assert!(flagged(&cmp, "newer"), "the alignment folds");
}

/// `state`, `diffs`, `content` stay; `category` is `same`: verdict, exit code, display filter, statistics.
#[test]
fn a6_9_only_the_category_changes() {
    let vfs = tree();
    let mut rules = FolderRules::default();
    rules.criteria.content.method = ContentMethod::Crc;
    let plain = compared(&vfs, &rules);
    rules.ignored = vec!["newer".into(), "a".into(), "only".into()];
    let cmp = compared(&vfs, &rules);
    cmp.check_invariants().unwrap();
    let (read, base) = (cmp.read(), plain.read());
    for path in ["newer", "a/x", "a/keep", "only"] {
        let (now, was) = (
            read.node(id(&cmp, path)).unwrap(),
            base.node(id(&plain, path)).unwrap(),
        );
        assert_eq!(
            (now.state, now.diffs, now.time),
            (was.state, was.diffs, was.time),
            "{path}"
        );
        assert_eq!(now.content, was.content, "{path}");
    }
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);
    assert_eq!(plain.verdict(false), CompareVerdict::Differences);
    let stats = cmp.stats(false);
    assert_eq!(stats.files.differences(), 0);
    assert_eq!(stats.ignored, 3, "`newer` is same by its content");
    assert_eq!(plain.stats(false).ignored, 0);
    let options = DisplayOptions {
        filter: FolderDisplayFilter::Differences,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let mut rows = Vec::new();
    read.visible_rows(&options, &|_| true, &mut rows);
    assert!(rows.is_empty(), "{rows:?}");
}

/// Error and Unknown stay: a failure is never hidden (D-19).
#[test]
fn a6_9_a_failure_is_never_hidden() {
    let vfs = tree();
    file(&vfs, "/l/bad", b"b", 10);
    file(&vfs, "/r/bad", b"bb", 10);
    vfs.inject(Fault {
        path: host("/l/bad"),
        op: VfsOp::Stat,
        kind: ErrorKind::PermissionDenied,
    });
    let cmp = compared(&vfs, &marked(&["bad"]));
    cmp.check_invariants().unwrap();
    assert_eq!(
        cmp.read().node(id(&cmp, "bad")).unwrap().state,
        CompareState::Error
    );
    assert_eq!(category(&cmp, "bad"), Category::Error);
    assert_eq!(cmp.verdict(false), CompareVerdict::Differences);
}

/// `set_ignored` keeps the counters equal to a recount, and its list round-trips through a new scan to equal states.
#[test]
fn a6_9_set_ignored_in_place() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    let model = cmp.model();
    let list = model.set_ignored(&[id(&cmp, "a"), id(&cmp, "newer")], true);
    check_tree_invariants(&cmp.read()).unwrap();
    assert_eq!(list, ["a", "newer"]);
    assert!(flagged(&cmp, "a/x"));
    assert_eq!(category(&cmp, "a/x"), Category::Same);
    let list = model.set_ignored(&[id(&cmp, "a/keep")], false);
    check_tree_invariants(&cmp.read()).unwrap();
    assert_eq!(list, ["a", "!a/keep", "newer"]);
    assert!(!flagged(&cmp, "a/keep"));
    let again = compared(&vfs, &marked(&["a", "!a/keep", "newer"]));
    assert_eq!(again.to_canonical(), cmp.to_canonical());
    assert_eq!(again.verdict(false), cmp.verdict(false));
    // The list stays minimal: a path under an equal ancestor is dropped, `!` only where needed.
    let list = model.set_ignored(&[id(&cmp, "a/keep")], true);
    assert_eq!(list, ["a", "newer"]);
    let list = model.set_ignored(&[id(&cmp, "a")], false);
    assert_eq!(list, ["newer"]);
    assert!(!flagged(&cmp, "a/x"));
    check_tree_invariants(&cmp.read()).unwrap();
    let list = model.set_ignored(&[id(&cmp, "a/x")], true);
    assert_eq!(list, ["a/x", "newer"]);
    let list = model.set_ignored(&[id(&cmp, "a")], true);
    assert_eq!(list, ["a", "newer"], "what lies under `a` goes into it");
    check_tree_invariants(&cmp.read()).unwrap();
}

/// JSON flag, `FLAG_NAMES`, TOML.
#[test]
fn a6_9_names() {
    assert_eq!(NodeFlags::IGNORED.bits(), 4096);
    assert_eq!(NodeFlags::IGNORED.names(), ["ignored"]);
    let rules = marked(&["a"]);
    let text = rules.to_toml();
    assert!(text.contains("ignored = [\"a\"]"), "{text}");
    assert_eq!(FolderRules::from_toml(&text).unwrap().0, rules);
    let vfs = tree();
    let cmp = compared(&vfs, &rules);
    assert!(
        common::find(&cmp.to_canonical(), "a")
            .flags
            .contains(&"ignored".to_owned())
    );
}

/// A selected ignored row can still be read: the mark does not hide its sides.
#[test]
fn a6_9_sides_stay() {
    let vfs = tree();
    let cmp = compared(&vfs, &marked(&["only"]));
    assert_eq!(
        cmp.read().spec(id(&cmp, "only"), Side::Left),
        Some(mspec("/l/only"))
    );
}
