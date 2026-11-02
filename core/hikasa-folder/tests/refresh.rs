//! M2 W6a acceptance A6.11 (D-173): subtree refresh. Every row of the table of §1.6.7, ids, counters, side tables,
//! archives, failures, cancellation, and the property P-REFRESH: after any script of changes the refreshed model equals a
//! fresh comparison.

mod common;

use std::sync::{Arc, Mutex};

use common::{
    compared, config, engines, gen_rules, gen_tree, host, materialize_gen, mspec, with_env,
};
use hikasa_core::{
    CancelToken, CaseSensitivity, CompareVerdict, ContentMethod, ErrorKind, FileTime, RelPath,
    Side, Spec,
};
use hikasa_folder::{
    CompareState, FolderComparison, FolderEnv, FolderError, FolderModel, FolderRules, NodeId,
    RefreshReport, ScanStatus, check_tree_invariants, compare_into, refresh_into,
};
use hikasa_vfs::{
    AtomicWrite, Attrs, Capabilities, DosAttrs, Entry, Fault, FileName, FollowLinks, ListOptions,
    Listing, MemMeta, MemoryVfs, ReadSeek, Vfs, VfsError, VfsOp, WriteOptions,
};
use proptest::prelude::*;

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

/// What a test asks of a back-end: every `list` and `open_read`.
struct Spy<'a> {
    inner: &'a dyn Vfs,
    lists: Mutex<Vec<String>>,
    opens: Mutex<Vec<String>>,
}

impl<'a> Spy<'a> {
    fn new(inner: &'a dyn Vfs) -> Spy<'a> {
        Spy {
            inner,
            lists: Mutex::new(Vec::new()),
            opens: Mutex::new(Vec::new()),
        }
    }

    fn take_lists(&self) -> Vec<String> {
        let mut lists = std::mem::take(&mut *self.lists.lock().unwrap());
        lists.sort();
        lists
    }

    fn take_opens(&self) -> Vec<String> {
        std::mem::take(&mut *self.opens.lock().unwrap())
    }
}

impl Vfs for Spy<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.lists.lock().unwrap().push(dir.canonical());
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.opens.lock().unwrap().push(spec.canonical());
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn archive_format(&self, name: &str) -> Option<hikasa_vfs::ArchiveFormat> {
        self.inner.archive_format(name)
    }
}

fn refresh(cmp: &FolderComparison, vfs: &dyn Vfs, nodes: &[NodeId]) -> RefreshReport {
    let engines = engines(1 << 30);
    with_env(vfs, &engines, &CancelToken::new(), |env| {
        refresh_into(cmp.model(), nodes, env).unwrap()
    })
}

fn id(cmp: &FolderComparison, path: &str) -> NodeId {
    let rel = RelPath::parse(path).unwrap();
    cmp.read()
        .node_at(&rel)
        .unwrap_or_else(|| panic!("no {path}"))
}

fn crc_rules() -> FolderRules {
    let mut rules = FolderRules::default();
    rules.criteria.content.method = ContentMethod::Crc;
    rules.criteria.content.skip_if_quick_same = false;
    rules
}

/// What `to_canonical` leaves out, row by row: a file's time and CRC, and each side's attributes. A kept folder keeps its
/// old time (§1.6.25 note 21), and a folder's time says nothing (§3), so that one stays out here too.
fn facts(cmp: &FolderComparison) -> Vec<String> {
    let read = cmp.read();
    let mut out = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(at) = stack.pop() {
        stack.extend(read.children(at));
        let Some(view) = read.node(at) else { continue };
        let folder = view.is_folder();
        for (side, entry) in [("l", view.left), ("r", view.right)] {
            let Some(entry) = entry else { continue };
            let time = if folder { None } else { entry.mtime };
            out.push(format!(
                "{} {side} {time:?} {:?} {:?}",
                read.path(at).as_str(),
                entry.attrs,
                entry.crc32
            ));
        }
    }
    out.sort();
    out
}

fn assert_same_facts(cmp: &FolderComparison, fresh: &FolderComparison) {
    assert_eq!(facts(cmp), facts(fresh));
}

/// Equal to a fresh comparison, and every counter equal to a recount.
fn assert_fresh(cmp: &FolderComparison, vfs: &dyn Vfs, rules: &FolderRules) {
    check_tree_invariants(&cmp.read()).unwrap();
    let fresh = compared(vfs, rules);
    assert_eq!(cmp.to_canonical(), fresh.to_canonical());
    assert_eq!(cmp.stats(false), fresh.stats(false));
    assert_eq!(cmp.verdict(false), fresh.verdict(false));
}

fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for side in ["/l", "/r"] {
        file(&vfs, &format!("{side}/a/keep"), b"k", 10);
        file(&vfs, &format!("{side}/a/change"), b"c", 10);
        file(&vfs, &format!("{side}/a/sub/deep"), b"d", 10);
        file(&vfs, &format!("{side}/b/other"), b"o", 10);
        file(&vfs, &format!("{side}/top"), b"t", 10);
    }
    vfs
}

/// One `list` per side per refreshed folder; a kept subtree unread; a kept pair not read for content.
#[test]
fn a6_11_one_list_per_side_and_a_kept_pair_is_not_read() {
    let vfs = tree();
    let rules = crc_rules();
    let cmp = compared(&vfs, &rules);
    file(&vfs, "/r/a/change", b"changed", 20);
    let spy = Spy::new(&vfs);
    let report = refresh(&cmp, &spy, &[id(&cmp, "a")]);
    assert_eq!(report.folders, 1);
    assert_eq!(report.rows_changed, 1);
    assert_eq!(report.content_tests, 1);
    assert!(report.complete);
    assert!(report.renumbered.is_empty(), "the same rows keep their ids");
    assert_eq!(
        spy.take_lists(),
        [mspec("/l/a").canonical(), mspec("/r/a").canonical()]
    );
    let mut opens = spy.take_opens();
    opens.sort();
    assert_eq!(
        opens,
        [
            mspec("/l/a/change").canonical(),
            mspec("/r/a/change").canonical()
        ]
    );
    assert_fresh(&cmp, &vfs, &rules);
}

/// A file node counts as its parent, and duplicates are refreshed once.
#[test]
fn a6_11_file_nodes_count_as_their_parent() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let spy = Spy::new(&vfs);
    let report = refresh(
        &cmp,
        &spy,
        &[id(&cmp, "a/keep"), id(&cmp, "a"), id(&cmp, "a/change")],
    );
    assert_eq!(report.folders, 1);
    assert_eq!(spy.take_lists().len(), 2);
    assert_eq!(report.rows_changed, 0);
}

/// A folder of the same presence is kept, its children unlisted, though they changed.
#[test]
fn a6_11_a_folder_with_the_same_presence_is_kept() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    file(&vfs, "/l/a/sub/deep", b"changed below", 20);
    let spy = Spy::new(&vfs);
    let report = refresh(&cmp, &spy, &[id(&cmp, "a")]);
    assert_eq!(report.rows_changed, 0);
    assert_eq!(spy.take_lists().len(), 2);
    // What changed below is the refresh of its own folder.
    let report = refresh(&cmp, &spy, &[id(&cmp, "a/sub")]);
    assert_eq!(report.rows_changed, 1);
    assert_fresh(&cmp, &vfs, &FolderRules::default());
}

/// Rows that come and go: `renumbered` is right, and every read through a mapped id works.
#[test]
fn a6_11_rows_come_and_go() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    let (kept_file, kept_folder, deep) = (
        id(&cmp, "a/keep"),
        id(&cmp, "a/sub"),
        id(&cmp, "a/sub/deep"),
    );
    file(&vfs, "/l/a/new", b"n", 10);
    vfs.remove(&mspec("/r/a/change")).unwrap();
    let report = refresh(&cmp, &vfs, &[id(&cmp, "a")]);
    assert!(report.complete);
    assert!(!report.renumbered.is_empty());
    let map = |old: NodeId| {
        report
            .renumbered
            .iter()
            .find(|(from, _)| *from == old)
            .map_or(old, |(_, to)| *to)
    };
    assert_ne!(map(kept_file), kept_file);
    let read = cmp.read();
    assert_eq!(read.path(map(kept_file)).as_str(), "a/keep");
    assert_eq!(read.path(map(kept_folder)).as_str(), "a/sub");
    assert!(read.node(kept_file).is_none(), "the old id is gone");
    assert_eq!(read.path(deep).as_str(), "a/sub/deep", "ids below stay");
    assert_eq!(read.ancestors(deep)[0], map(kept_folder));
    drop(read);
    assert_fresh(&cmp, &vfs, &rules);
}

/// A folder appears or goes on a side: a new record, its subtree scanned anew; an old row is dropped with its subtree.
#[test]
fn a6_11_a_folder_side_appears_or_goes() {
    let vfs = tree();
    let rules = crc_rules();
    let cmp = compared(&vfs, &rules);
    let gone = id(&cmp, "b/other");
    file(&vfs, "/l/fresh/sub/x", b"x", 10);
    vfs.remove(&mspec("/l/b/other")).unwrap();
    vfs.remove(&mspec("/l/b")).unwrap();
    let report = refresh(&cmp, &vfs, &[NodeId::ROOT]);
    assert!(report.complete);
    assert_fresh(&cmp, &vfs, &rules);
    let read = cmp.read();
    assert_eq!(
        read.node(id(&cmp, "b")).unwrap().state,
        CompareState::RightOrphan
    );
    assert_eq!(
        read.node(id(&cmp, "fresh/sub")).unwrap().scan,
        ScanStatus::Scanned
    );
    assert!(read.node(gone).is_none());
}

/// A side table holds nothing for dropped ids, and `content_counts` follow.
#[test]
fn a6_11_side_tables_and_content_counts_follow() {
    let vfs = tree();
    let rules = crc_rules();
    let cmp = compared(&vfs, &rules);
    let before = cmp.stats(false).content;
    assert_eq!(before.binary_same, 5);
    let gone = id(&cmp, "b/other");
    vfs.remove(&mspec("/l/b/other")).unwrap();
    vfs.remove(&mspec("/r/b/other")).unwrap();
    vfs.remove(&mspec("/l/a/sub/deep")).unwrap();
    vfs.remove(&mspec("/r/a/sub/deep")).unwrap();
    vfs.remove(&mspec("/l/a/sub")).unwrap();
    vfs.remove(&mspec("/r/a/sub")).unwrap();
    refresh(&cmp, &vfs, &[id(&cmp, "b"), id(&cmp, "a")]);
    assert_eq!(cmp.stats(false).content.binary_same, 3);
    assert!(cmp.read().content_outcome(gone).is_none());
    assert!(cmp.read().errors(gone).is_empty());
    assert_fresh(&cmp, &vfs, &rules);
}

/// A kept pair keeps its CRC; a changed one loses it.
#[test]
fn a6_11_a_changed_row_loses_its_crc() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    let engines = engines(1 << 30);
    let (keep, change) = (id(&cmp, "a/keep"), id(&cmp, "a/change"));
    with_env(&vfs, &engines, &CancelToken::new(), |env| {
        hikasa_folder::compute_crcs(
            cmp.model(),
            &[(keep, Side::Left), (change, Side::Left)],
            env,
        )
        .unwrap()
    });
    file(&vfs, "/l/a/change", b"changed", 20);
    let report = refresh(&cmp, &vfs, &[id(&cmp, "a")]);
    assert!(report.renumbered.is_empty());
    let read = cmp.read();
    assert!(read.node(keep).unwrap().left.unwrap().crc32.is_some());
    assert_eq!(read.node(change).unwrap().left.unwrap().crc32, None);
}

/// A failing listing takes the scan's error path, and the next refresh clears it.
#[test]
fn a6_11_a_failing_listing() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    vfs.inject(Fault {
        path: host("/l/a"),
        op: VfsOp::List,
        kind: ErrorKind::PermissionDenied,
    });
    let a = id(&cmp, "a");
    let report = refresh(&cmp, &vfs, &[a]);
    assert!(report.complete);
    check_tree_invariants(&cmp.read()).unwrap();
    {
        let read = cmp.read();
        assert_eq!(read.node(a).unwrap().scan, ScanStatus::Failed);
        assert_eq!(read.errors(a).len(), 1);
        let row = read.children(a).next().unwrap();
        assert_eq!(read.node(row).unwrap().state, CompareState::Unknown);
    }
    vfs.clear_faults();
    refresh(&cmp, &vfs, &[a]);
    assert_fresh(&cmp, &vfs, &rules);
    assert!(cmp.read().errors(a).is_empty());
}

/// Cancel at the n-th folder: `complete: false`, verdict incomplete.
#[test]
fn a6_11_cancel_leaves_it_incomplete() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let engines = engines(1 << 30);
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = with_env(&vfs, &engines, &cancel, |env: &FolderEnv<'_>| {
        refresh_into(cmp.model(), &[id(&cmp, "a"), id(&cmp, "b")], env).unwrap()
    });
    assert!(!report.complete);
    assert_eq!(report.folders, 0);
    assert!(!cmp.read().is_complete());
    assert_eq!(cmp.verdict(false), hikasa_core::CompareVerdict::Incomplete);
}

/// A back-end that cancels at the n-th `list`.
struct CancelAtList<'a> {
    inner: &'a dyn Vfs,
    token: &'a CancelToken,
    left: Mutex<u32>,
}

impl Vfs for CancelAtList<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        let listing = self.inner.list(dir, opts, cancel);
        let mut left = self.left.lock().unwrap();
        *left = left.saturating_sub(1);
        if *left == 0 {
            self.token.cancel();
        }
        listing
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
}

/// Cancel at the second folder: the first holds what it found, the rest what it had.
#[test]
fn a6_11_cancel_at_the_nth_folder() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let (a, b) = (id(&cmp, "a"), id(&cmp, "b"));
    let cancel = CancelToken::new();
    let wrapper = CancelAtList {
        inner: &vfs,
        token: &cancel,
        left: Mutex::new(3),
    };
    let engines = engines(1 << 30);
    file(&vfs, "/l/a/new", b"n", 10);
    file(&vfs, "/l/b/new", b"n", 10);
    let report = with_env(&wrapper, &engines, &cancel, |env| {
        refresh_into(cmp.model(), &[a, b], env).unwrap()
    });
    assert!(!report.complete);
    assert_eq!(report.folders, 1);
    check_tree_invariants(&cmp.read()).unwrap();
    assert!(!cmp.read().is_complete());
    let read = cmp.read();
    let found = |path: &str| read.node_at(&RelPath::parse(path).unwrap()).is_some();
    assert_eq!(
        found("a/new"),
        !found("b/new"),
        "one of the two folders ran"
    );
}

fn refresh_cancelled(cmp: &FolderComparison, vfs: &dyn Vfs, nodes: &[NodeId]) -> RefreshReport {
    let engines = engines(1 << 30);
    let cancel = CancelToken::new();
    cancel.cancel();
    with_env(vfs, &engines, &cancel, |env| {
        refresh_into(cmp.model(), nodes, env).unwrap()
    })
}

/// §1.6.7 step 8 says what a stop leaves; a later refresh that covers it restores the verdict, which a stop does not take
/// for good.
#[test]
fn a6_11_cancel_then_a_refresh_recovers() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);
    assert!(!refresh_cancelled(&cmp, &vfs, &[NodeId::ROOT]).complete);
    assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
    assert!(refresh(&cmp, &vfs, &[NodeId::ROOT]).complete);
    assert!(cmp.read().is_complete(), "complete after a whole refresh");
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);
    assert_fresh(&cmp, &vfs, &rules);
}

/// A refresh of another subtree claims nothing: what was asked and not reached stays owed, by its own refresh or one that
/// covers it.
#[test]
fn a6_11_cancel_and_a_refresh_of_another_subtree() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    let (a, b) = (id(&cmp, "a"), id(&cmp, "b"));
    refresh_cancelled(&cmp, &vfs, &[a]);
    assert!(refresh(&cmp, &vfs, &[b]).complete, "this refresh did its part");
    assert!(!cmp.read().is_complete());
    assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
    refresh(&cmp, &vfs, &[a]);
    assert!(cmp.read().is_complete());
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);

    // The same, covered from above: `a` is below the root, and a refresh of the root goes to it.
    refresh_cancelled(&cmp, &vfs, &[a]);
    refresh(&cmp, &vfs, &[NodeId::ROOT]);
    assert!(cmp.read().is_complete());
    assert_fresh(&cmp, &vfs, &rules);
}

/// A scan stopped deep in the tree leaves folders it never listed, below rows a refresh keeps: it goes to them.
#[test]
fn a6_11_a_cancelled_scan_is_repaired() {
    let vfs = tree();
    file(&vfs, "/l/a/sub/more/deepest", b"x", 10);
    file(&vfs, "/r/a/sub/more/deepest", b"x", 10);
    let rules = FolderRules::default();
    for lists in 1..8 {
        let cancel = CancelToken::new();
        let wrapper = CancelAtList {
            inner: &vfs,
            token: &cancel,
            left: Mutex::new(lists),
        };
        let engines = engines(1 << 30);
        let scanned = with_env(&wrapper, &engines, &cancel, |env| {
            let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
            compare_into(&model, env)
        });
        // A stop before the bases are read has no comparison to go on with.
        let Ok(cmp) = scanned else {
            assert!(lists <= 2, "{lists}");
            continue;
        };
        assert!(!cmp.read().is_complete(), "{lists}");
        assert!(refresh(&cmp, &vfs, &[NodeId::ROOT]).complete, "{lists}");
        assert!(cmp.read().is_complete(), "{lists}");
        assert_fresh(&cmp, &vfs, &rules);
    }
}

/// A refresh stopped inside a new subtree: the second one goes down to what the first left.
#[test]
fn a6_11_a_cancelled_refresh_of_a_new_subtree() {
    let vfs = tree();
    let rules = FolderRules::default();
    let cmp = compared(&vfs, &rules);
    file(&vfs, "/l/n/x/y/z", b"z", 10);
    file(&vfs, "/r/n/x/y/z", b"z", 10);
    let cancel = CancelToken::new();
    let wrapper = CancelAtList {
        inner: &vfs,
        token: &cancel,
        left: Mutex::new(4),
    };
    let engines = engines(1 << 30);
    let report = with_env(&wrapper, &engines, &cancel, |env| {
        refresh_into(cmp.model(), &[NodeId::ROOT], env).unwrap()
    });
    assert!(!report.complete);
    assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
    assert!(refresh(&cmp, &vfs, &[NodeId::ROOT]).complete);
    assert!(cmp.read().is_complete());
    assert_fresh(&cmp, &vfs, &rules);
}

/// A back-end that cancels at the n-th `open_read`: a content test stopped halfway.
struct CancelAtOpen<'a> {
    spy: Spy<'a>,
    token: &'a CancelToken,
    left: Mutex<u32>,
}

impl Vfs for CancelAtOpen<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.spy.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.spy.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.spy.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let mut left = self.left.lock().unwrap();
        *left = left.saturating_sub(1);
        if *left == 0 {
            self.token.cancel();
        }
        self.spy.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.spy.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.spy.child(dir, name)
    }
}

/// A stop in a content test: the row owes its result, in a folder a refresh would keep.
#[test]
fn a6_11_a_cancelled_content_test_is_repaired() {
    let vfs = tree();
    let rules = crc_rules();
    let cmp = compared(&vfs, &rules);
    file(&vfs, "/l/a/sub/deep", b"e", 11);
    file(&vfs, "/l/b/other", b"e", 11);
    let cancel = CancelToken::new();
    let wrapper = CancelAtOpen {
        spy: Spy::new(&vfs),
        token: &cancel,
        left: Mutex::new(1),
    };
    let engines = engines(1 << 30);
    let nodes = [id(&cmp, "a/sub"), id(&cmp, "b")];
    let report = with_env(&wrapper, &engines, &cancel, |env| {
        refresh_into(cmp.model(), &nodes, env).unwrap()
    });
    assert!(!report.complete);
    assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
    assert!(refresh(&cmp, &vfs, &[NodeId::ROOT]).complete);
    assert!(cmp.read().is_complete());
    assert_fresh(&cmp, &vfs, &rules);
}

/// A refresh on a pool, with content tests: a row at a time, or several at once, the same result.
#[test]
fn a6_11_on_a_pool() {
    let vfs = tree();
    let rules = crc_rules();
    let cmp = compared(&vfs, &rules);
    file(&vfs, "/l/a/change", b"cc", 11);
    file(&vfs, "/r/a/sub/deep", b"dd", 12);
    file(&vfs, "/l/n/x/y", b"new", 13);
    file(&vfs, "/r/n/x/y", b"new", 13);
    file(&vfs, "/r/b/other2", b"o2", 14);
    let contents = engines(1 << 30);
    let cancel = CancelToken::new();
    let progress = hikasa_core::NullProgress;
    let now = common::default_now();
    let pool = common::pool(4);
    let env = FolderEnv {
        vfs: &vfs,
        content: &contents,
        pool: pool.as_ref(),
        ctl: hikasa_core::Ctl {
            cancel: &cancel,
            progress: &progress,
        },
        observer: &hikasa_folder::NullObserver,
        now: &now,
        content_budget: 1 << 30,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let nodes = [NodeId::ROOT, id(&cmp, "a"), id(&cmp, "a/sub"), id(&cmp, "b")];
    let report = refresh_into(cmp.model(), &nodes, &env).unwrap();
    assert!(report.complete);
    assert_fresh(&cmp, &vfs, &rules);
    assert_same_facts(&cmp, &compared(&vfs, &rules));
}

/// `RefreshFlat`, and `RefreshBusy` for a model still scanning.
#[test]
fn a6_11_flat_and_busy() {
    let vfs = tree();
    let mut rules = FolderRules::default();
    rules.handling.flatten = true;
    let cmp = compared(&vfs, &rules);
    let engines = engines(1 << 30);
    with_env(&vfs, &engines, &CancelToken::new(), |env| {
        assert!(matches!(
            refresh_into(cmp.model(), &[NodeId::ROOT], env),
            Err(FolderError::RefreshFlat)
        ));
        let waiting = FolderModel::new(mspec("/l"), mspec("/r"), FolderRules::default()).unwrap();
        assert!(matches!(
            refresh_into(&waiting, &[NodeId::ROOT], env),
            Err(FolderError::RefreshBusy)
        ));
    });
}

/// A `Vfs` that starts a second refresh from inside the first.
struct Reenter<'a> {
    inner: &'a dyn Vfs,
    model: Arc<FolderModel>,
    engines: hikasa_folder::ContentEngines,
    seen: Mutex<Option<bool>>,
}

impl Vfs for Reenter<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        let mut seen = self.seen.lock().unwrap();
        if seen.is_none() {
            let busy = with_env(self.inner, &self.engines, cancel, |env| {
                matches!(
                    refresh_into(&self.model, &[NodeId::ROOT], env),
                    Err(FolderError::RefreshBusy)
                )
            });
            *seen = Some(busy);
        }
        drop(seen);
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
}

#[test]
fn a6_11_a_second_refresh_is_busy() {
    let vfs = tree();
    let cmp = compared(&vfs, &FolderRules::default());
    let reenter = Reenter {
        inner: &vfs,
        model: Arc::clone(cmp.model()),
        engines: engines(1 << 30),
        seen: Mutex::new(None),
    };
    refresh(&cmp, &reenter, &[NodeId::ROOT]);
    assert_eq!(*reenter.seen.lock().unwrap(), Some(true));
    // The claim ended with the call.
    assert!(refresh(&cmp, &vfs, &[NodeId::ROOT]).complete);
}

/// A folder not scanned (auto-scan off) fills: every row is new.
#[test]
fn a6_8_a_refresh_of_a_not_scanned_folder_fills_it() {
    let vfs = tree();
    let mut rules = FolderRules::default();
    rules.handling.auto_scan_subfolders = false;
    let cmp = compared(&vfs, &rules);
    let a = id(&cmp, "a");
    assert_eq!(cmp.read().node(a).unwrap().scan, ScanStatus::NotScanned);
    let report = refresh(&cmp, &vfs, &[a]);
    assert_eq!(report.rows_changed, 3);
    check_tree_invariants(&cmp.read()).unwrap();
    let read = cmp.read();
    assert_eq!(read.node(a).unwrap().scan, ScanStatus::Scanned);
    assert_eq!(read.children(a).len(), 3);
    assert_eq!(
        read.node(id(&cmp, "a/sub")).unwrap().scan,
        ScanStatus::NotScanned
    );
    assert_eq!(
        read.node(id(&cmp, "a/keep")).unwrap().state,
        CompareState::Same
    );
}

// ---- disk ----------------------------------------------------------------------------------------------------

/// One fixed script on a tempdir, all platforms.
#[test]
fn a6_11_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (left, right) = (root.join("l"), root.join("r"));
    for side in [&left, &right] {
        std::fs::create_dir_all(side.join("a/sub")).unwrap();
        std::fs::write(side.join("a/keep"), b"k").unwrap();
        std::fs::write(side.join("a/sub/deep"), b"d").unwrap();
    }
    let vfs = hikasa_vfs::LocalVfs::default();
    let spec =
        |path: &std::path::Path| Spec::Local(hikasa_core::LocalPath::new(path.into()).unwrap());
    let mut rules = crc_rules();
    rules.criteria.timestamps.enabled = false;
    let engines = engines(1 << 30);
    let run = || {
        with_env(&vfs, &engines, &CancelToken::new(), |env| {
            let model = FolderModel::new(spec(&left), spec(&right), rules.clone()).unwrap();
            compare_into(&model, env).unwrap()
        })
    };
    let cmp = run();
    std::fs::write(left.join("a/new"), b"n").unwrap();
    std::fs::write(right.join("a/keep"), b"changed").unwrap();
    std::fs::remove_file(left.join("a/sub/deep")).unwrap();
    std::fs::create_dir(right.join("fresh")).unwrap();
    let report = with_env(&vfs, &engines, &CancelToken::new(), |env| {
        let nodes = [id(&cmp, "a"), id(&cmp, "a/sub"), NodeId::ROOT];
        refresh_into(cmp.model(), &nodes, env).unwrap()
    });
    assert!(report.complete);
    check_tree_invariants(&cmp.read()).unwrap();
    assert_eq!(cmp.to_canonical(), run().to_canonical());
}

// ---- P-REFRESH -----------------------------------------------------------------------------------------------

/// One change of a script: which item, on which side, and what happens to it.
#[derive(Debug, Clone)]
struct Change {
    at: usize,
    what: u8,
    right: bool,
}

fn change() -> impl Strategy<Value = Change> {
    (any::<proptest::sample::Index>(), 0u8..8, any::<bool>()).prop_map(|(at, what, right)| Change {
        at: at.index(1 << 20),
        what,
        right,
    })
}

/// The files and folders of one side, relative to it.
fn items_of(vfs: &MemoryVfs, side: &str) -> Vec<(String, EntryIs)> {
    let base = host(side);
    let mut out = Vec::new();
    for item in vfs.dump() {
        if let Some(rest) = item.path.strip_prefix(&base)
            && let Some(rest) = rest.strip_prefix(std::path::MAIN_SEPARATOR)
        {
            let kind = match item.kind {
                hikasa_vfs::EntryKind::Dir => EntryIs::Folder,
                _ => EntryIs::File,
            };
            out.push((rest.replace(std::path::MAIN_SEPARATOR, "/"), kind));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryIs {
    File,
    Folder,
}

/// Removes a folder with what is in it.
fn remove_tree(vfs: &MemoryVfs, path: &str) {
    let prefix = format!("{path}/");
    let mut below: Vec<String> = vfs
        .dump()
        .into_iter()
        .map(|item| item.path.replace(std::path::MAIN_SEPARATOR, "/"))
        .filter(|item| {
            let plain = host(&prefix).replace(std::path::MAIN_SEPARATOR, "/");
            item.starts_with(&plain)
        })
        .collect();
    below.sort_by_key(|item| std::cmp::Reverse(item.len()));
    for item in below {
        let spec = hikasa_core::LocalPath::new(std::path::PathBuf::from(
            item.replace('/', std::path::MAIN_SEPARATOR_STR),
        ))
        .map(Spec::Local)
        .unwrap();
        vfs.remove(&spec).unwrap();
    }
    vfs.remove(&mspec(path)).unwrap();
}

/// Applies `change`; the rel path of the folder whose listing it changed.
fn apply(vfs: &MemoryVfs, change: &Change, tick: i64) -> String {
    let side = if change.right { "/r" } else { "/l" };
    let items = items_of(vfs, side);
    let folder_of = |rel: &str| rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
    if items.is_empty() {
        file(vfs, &format!("{side}/seed{tick}"), b"seed", tick);
        return String::new();
    }
    let (rel, kind) = items[change.at % items.len()].clone();
    let path = format!("{side}/{rel}");
    match (change.what, kind) {
        (0, EntryIs::File) => file(vfs, &path, b"changed content", 1_500_000_000 + tick),
        (1, EntryIs::File) => vfs.remove(&mspec(&path)).unwrap(),
        (2 | 3, EntryIs::Folder) => {
            file(vfs, &format!("{path}/new{tick}"), b"new", tick);
            return rel;
        }
        (4, EntryIs::Folder) => remove_tree(vfs, &path),
        (5, EntryIs::Folder) => {
            file(vfs, &format!("{path}/made{tick}/in/it"), b"deep", tick);
            return rel;
        }
        // Another attribute, the rest as it was.
        (6, EntryIs::File) if vfs.read_back(&host(&path)).is_some() => {
            let found = vfs.dump().into_iter().find(|item| item.path == host(&path));
            let found = found.unwrap();
            let attrs = match found.attrs {
                Attrs::Dos { attrs } => Attrs::Dos {
                    attrs: attrs ^ DosAttrs::READONLY,
                },
                Attrs::Unix { meta } => Attrs::Unix {
                    meta: hikasa_vfs::UnixMeta {
                        mode: meta.mode ^ 0o111,
                        ..meta
                    },
                },
                Attrs::None => Attrs::Dos {
                    attrs: DosAttrs::READONLY,
                },
            };
            let meta = MemMeta {
                mtime: found.mtime,
                attrs: Some(attrs),
                ..MemMeta::default()
            };
            let content = vfs.read_back(&host(&path)).unwrap();
            vfs.insert_file(&host(&path), content, meta).unwrap();
        }
        // Another kind, under the same name.
        (7, EntryIs::File) => {
            vfs.remove(&mspec(&path)).unwrap();
            file(vfs, &format!("{path}/inner"), b"inner", tick);
        }
        (7, EntryIs::Folder) => {
            remove_tree(vfs, &path);
            file(vfs, &path, b"a file now", tick);
        }
        _ => {
            file(vfs, &format!("{side}/top{tick}"), b"top", tick);
            return String::new();
        }
    }
    folder_of(&rel)
}

/// The node to refresh for a folder that changed: its row, or the nearest row above it. A folder no scan went into stays
/// so: a refresh of its own is how it is opened, which a fresh comparison does not do.
fn nearest(cmp: &FolderComparison, rel: &str) -> NodeId {
    let read = cmp.read();
    let mut at = rel.to_owned();
    loop {
        let found = RelPath::parse(&at)
            .ok()
            .and_then(|path| read.node_at(&path))
            .filter(|&id| {
                let scanned = read
                    .node(id)
                    .is_some_and(|view| view.is_folder() && view.scan != ScanStatus::NotScanned);
                scanned || id == NodeId::ROOT
            });
        if let Some(found) = found {
            return found;
        }
        at = at.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
    }
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/refresh.txt"))]

    /// P-REFRESH: after any script of changes, kinds and attributes among them, the refreshed model equals a fresh
    /// comparison, also with content tests, and also in what `to_canonical` leaves out.
    #[test]
    fn p_refresh_equals_a_fresh_comparison(
        nodes in gen_tree(),
        generated in gen_rules(),
        crc in any::<bool>(),
        script in proptest::collection::vec(change(), 1..8),
    ) {
        let mut rules = generated.rules(false);
        rules.handling.flatten = false;
        // A folder followed through a link is the same folder at two paths: a refresh lists the one it is asked for.
        rules.handling.follow_symlinks = false;
        if crc {
            rules.criteria.content.method = ContentMethod::Crc;
        }
        let vfs = materialize_gen(&nodes, CaseSensitivity::Sensitive, generated.unix);
        let cmp = compared(&vfs, &rules);
        for (tick, step) in script.iter().enumerate() {
            let rel = apply(&vfs, step, tick as i64 + 1);
            let node = nearest(&cmp, &rel);
            let report = refresh(&cmp, &vfs, &[node]);
            prop_assert!(report.complete);
            prop_assert!(check_tree_invariants(&cmp.read()).is_ok());
        }
        let fresh = compared(&vfs, &rules);
        prop_assert_eq!(cmp.to_canonical(), fresh.to_canonical());
        prop_assert_eq!(cmp.stats(false), fresh.stats(false));
        prop_assert_eq!(facts(&cmp), facts(&fresh));
    }
}
