//! Regression tests of the wave-3 review (§1.6.8): the model is finished when a scan panics; cancellation inside
//! one huge folder batch (listing → alignment → re-stats → insertion) drops the batch and never reports a complete
//! result; the re-stat storm of FC-054 stops at the cancellation; entries whose stat failed are paired in linear
//! time; quick tests on filtered pairs; the one-sided protected folder in both scan modes.

mod common;

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use common::*;
use hikasa_core::{
    CancelToken, CaseSensitivity, CompareVerdict, Ctl, ErrorKind, ErrorReport, FileTime,
    NullProgress, Spec,
};
use hikasa_folder::{
    CompareState, DiffFlags, FolderComparison, FolderEnv, FolderModel, FolderRules,
    FolderRulesPatch, NodeId, NullObserver, ScanStatus, compare_into,
};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, EntryError, FileName, FollowLinks, ListOptions, Listing,
    MemMeta, MemoryVfs, ReadSeek, Vfs, VfsError, WriteOptions,
};

/// What a [`Hooked`] back-end does besides forwarding.
#[derive(Default)]
struct Hooks {
    /// Panic when this folder is listed.
    panic_on_list: Option<String>,
    /// Cancel `token` right after this folder was listed on both sides (the second listing returns).
    cancel_after_listing: Option<String>,
    /// Cancel `token` at this (1-based) `stat` call.
    cancel_at_stat: Option<usize>,
    /// Cancel `token` at this (1-based) `child` call (the scanner builds content-test specs while planning).
    cancel_at_child: Option<usize>,
    /// Move every listed entry of folders below this prefix into `Listing::errors` (names known, stat failed).
    stat_fails_below: Option<String>,
}

/// A `Vfs` wrapper with test hooks; counts `stat` calls (also those after the cancellation).
struct Hooked<'a> {
    inner: &'a MemoryVfs,
    hooks: Hooks,
    token: &'a CancelToken,
    listed: AtomicUsize,
    stats: AtomicUsize,
    stats_after_cancel: AtomicUsize,
    children: AtomicUsize,
    children_after_cancel: AtomicUsize,
    cancelled_at: std::sync::Mutex<Option<Instant>>,
}

impl<'a> Hooked<'a> {
    fn new(inner: &'a MemoryVfs, hooks: Hooks, token: &'a CancelToken) -> Hooked<'a> {
        Hooked {
            inner,
            hooks,
            token,
            listed: AtomicUsize::new(0),
            stats: AtomicUsize::new(0),
            stats_after_cancel: AtomicUsize::new(0),
            children: AtomicUsize::new(0),
            children_after_cancel: AtomicUsize::new(0),
            cancelled_at: std::sync::Mutex::new(None),
        }
    }

    fn cancel(&self) {
        *self.cancelled_at.lock().unwrap() = Some(Instant::now());
        self.token.cancel();
    }
}

impl Vfs for Hooked<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        if self.token.is_cancelled() {
            self.stats_after_cancel.fetch_add(1, Ordering::SeqCst);
        }
        let n = self.stats.fetch_add(1, Ordering::SeqCst) + 1;
        if self.hooks.cancel_at_stat == Some(n) {
            self.cancel();
        }
        self.inner.stat(spec, follow)
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        let path = dir.canonical();
        if self
            .hooks
            .panic_on_list
            .as_ref()
            .is_some_and(|p| path.ends_with(p.as_str()))
        {
            panic!("scripted panic while listing {path}");
        }
        let mut listing = self.inner.list(dir, opts, cancel)?;
        if let Some(prefix) = &self.hooks.stat_fails_below
            && path.starts_with(&host(prefix))
        {
            for entry in std::mem::take(&mut listing.entries) {
                listing.errors.push(EntryError {
                    name: entry.name,
                    error: ErrorReport {
                        kind: ErrorKind::PermissionDenied,
                        message: "permission denied".into(),
                        spec: None,
                        causes: Vec::new(),
                    },
                });
            }
        }
        if self
            .hooks
            .cancel_after_listing
            .as_ref()
            .is_some_and(|p| path.ends_with(p.as_str()))
            && self.listed.fetch_add(1, Ordering::SeqCst) == 1
        {
            self.cancel();
        }
        Ok(listing)
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
        if self.token.is_cancelled() {
            self.children_after_cancel.fetch_add(1, Ordering::SeqCst);
        }
        let n = self.children.fetch_add(1, Ordering::SeqCst) + 1;
        if self.hooks.cancel_at_child == Some(n) {
            self.cancel();
        }
        self.inner.child(dir, name)
    }

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        self.inner.local_path(spec)
    }
}

fn meta() -> MemMeta {
    MemMeta {
        mtime: Some(FileTime::from_unix(1_790_000_000, 0).unwrap()),
        ..MemMeta::default()
    }
}

/// Runs `/l` against `/r` on `vfs`; returns the model too (it exists also when `compare_into` unwinds).
fn scan(
    vfs: &dyn Vfs,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
    cancel: &CancelToken,
) -> (
    std::sync::Arc<FolderModel>,
    std::thread::Result<Result<FolderComparison, hikasa_folder::FolderError>>,
) {
    let engines = engines(64);
    let now = default_now();
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: &engines,
        pool,
        ctl: Ctl {
            cancel,
            progress: &progress,
        },
        observer: &NullObserver,
        now: &now,
        content_budget: 1 << 30,
    };
    let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| compare_into(&model, &env)));
    (model, result)
}

/// One folder `/l/big`, `/r/big` with `n` identical files on each side.
fn big_folder(n: usize) -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let content: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"content"[..]);
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(&format!("{base}/big")), meta())
            .unwrap();
        vfs.insert_file(&host(&format!("{base}/top.txt")), b"t".to_vec(), meta())
            .unwrap();
        for f in 0..n {
            vfs.insert_file(
                &host(&format!("{base}/big/file{f:07}.dat")),
                std::sync::Arc::clone(&content),
                meta(),
            )
            .unwrap();
        }
    }
    vfs
}

/// §5.6, §1.6.4: a panic inside the scan propagates out of `compare_into`, and the model is still marked finished
/// (and incomplete), so a GUI polling the shared model does not wait forever.
#[test]
fn panic_in_scan_marks_the_model_finished_and_incomplete() {
    let vfs = big_folder(3);
    for threads in [0, 2] {
        let pool = pool(threads);
        let token = CancelToken::new();
        let hooked = Hooked::new(
            &vfs,
            Hooks {
                panic_on_list: Some("/big".into()),
                ..Hooks::default()
            },
            &token,
        );
        let (model, result) = scan(&hooked, &FolderRules::default(), pool.as_ref(), &token);
        assert!(result.is_err(), "the panic propagates (threads {threads})");
        assert!(model.is_finished(), "threads {threads}");
        assert!(!model.read().is_complete(), "threads {threads}");
    }
}

/// §5.3, G4: a cancellation that arrives right after a huge folder was listed drops the whole batch (nothing of
/// it is inserted, no other back-end call follows) and the result is incomplete — never `complete == true`.
#[test]
fn cancellation_inside_a_huge_folder_batch_drops_it() {
    let n = 200_000;
    let vfs = big_folder(n);
    for threads in [0, 4] {
        let pool = pool(threads);
        let token = CancelToken::new();
        let hooked = Hooked::new(
            &vfs,
            Hooks {
                cancel_after_listing: Some("/big".into()),
                ..Hooks::default()
            },
            &token,
        );
        let (_, result) = scan(&hooked, &FolderRules::default(), pool.as_ref(), &token);
        let returned = Instant::now();
        let cmp = result.unwrap().unwrap();
        let latency =
            returned.saturating_duration_since(hooked.cancelled_at.lock().unwrap().unwrap());
        assert!(!cmp.is_complete(), "threads {threads}");
        assert_eq!(cmp.verdict(false), CompareVerdict::Incomplete);
        let read = cmp.read();
        let big = read
            .children(NodeId::ROOT)
            .find(|&id| read.path(id).as_str() == "big")
            .unwrap();
        assert_eq!(
            read.children(big).len(),
            0,
            "the cancelled batch is dropped"
        );
        assert_eq!(read.node(big).unwrap().scan, ScanStatus::NotScanned);
        assert_eq!(read.node(big).unwrap().state, CompareState::Unknown);
        drop(read);
        cmp.check_invariants().unwrap();
        assert_eq!(hooked.stats_after_cancel.load(Ordering::SeqCst), 0);
        // The release gate (≤ 100 ms) is measured by the `cancel_latency` bench; unoptimised code gets slack.
        let bound = if cfg!(debug_assertions) {
            Duration::from_secs(10)
        } else {
            Duration::from_millis(100)
        };
        assert!(latency <= bound, "threads {threads}: latency {latency:?}");
    }
}

/// A cancellation while a huge batch is planned (here: while the content-test specs are built) stops the
/// planning within one check interval (1024 pairs) and drops the batch.
#[test]
fn cancellation_while_planning_a_huge_batch_drops_it() {
    let n = 50_000;
    let vfs = big_folder(n);
    let mut rules = FolderRules::default();
    rules.criteria.content.method = hikasa_core::ContentMethod::Crc;
    let token = CancelToken::new();
    let hooked = Hooked::new(
        &vfs,
        Hooks {
            cancel_at_child: Some(20_000),
            ..Hooks::default()
        },
        &token,
    );
    let (_, result) = scan(&hooked, &rules, None, &token);
    let cmp = result.unwrap().unwrap();
    assert!(!cmp.is_complete());
    let after = hooked.children_after_cancel.load(Ordering::SeqCst);
    assert!(
        after <= 2 * 1024,
        "{after} specs built after the cancellation"
    );
    let read = cmp.read();
    let big = read
        .children(NodeId::ROOT)
        .find(|&id| read.path(id).as_str() == "big")
        .unwrap();
    assert_eq!(read.children(big).len(), 0);
    drop(read);
    cmp.check_invariants().unwrap();
}

/// Whenever a cancellation lands during a huge batch (listing, alignment, planning, insertion), the batch is
/// either dropped or inserted whole, the invariants hold, and the result is incomplete.
#[test]
fn cancellation_at_any_time_keeps_batches_whole() {
    let n = 100_000;
    let vfs = big_folder(n);
    for delay_ms in [0u64, 20, 80, 200, 400] {
        let token = CancelToken::new();
        let canceller = token.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(delay_ms));
            canceller.cancel();
        });
        let (model, result) = scan(&vfs, &FolderRules::default(), None, &token);
        handle.join().unwrap();
        let cmp = match result.unwrap() {
            Ok(cmp) => cmp,
            // Cancelled before the bases were loaded: an error, and the model is finished.
            Err(hikasa_folder::FolderError::Cancelled) => {
                assert!(model.is_finished() && !model.read().is_complete());
                continue;
            }
            Err(e) => panic!("delay {delay_ms}: {e}"),
        };
        cmp.check_invariants().unwrap();
        let read = cmp.read();
        if let Some(big) = read
            .children(NodeId::ROOT)
            .find(|&id| read.path(id).as_str() == "big")
        {
            let children = read.children(big).len();
            assert!(
                children == 0 || children == n,
                "delay {delay_ms}: {children} children"
            );
            if children == n {
                // The batch was complete before the cancellation took effect; the scan may still be complete.
                continue;
            }
        }
        assert!(!read.is_complete(), "delay {delay_ms}");
    }
}

/// FC-054 re-stat storm (§2.3): every left `x{i}.bak` (excluded, name-only) is aligned with a visible right
/// `x{i}.txt`, so it is stat'ed after alignment. A cancellation in the middle stops the re-stats at once.
#[test]
fn restat_storm_stops_at_the_cancellation() {
    let n = 20_000;
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for f in 0..n {
        vfs.insert_file(&host(&format!("/l/d/x{f:06}.bak")), b"x".to_vec(), meta())
            .unwrap();
        vfs.insert_file(&host(&format!("/r/d/x{f:06}.txt")), b"x".to_vec(), meta())
            .unwrap();
    }
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some("-*.bak".into()),
            align_extensions: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    // Without cancellation: one re-stat per pair (plus the two bases).
    let token = CancelToken::new();
    let hooked = Hooked::new(&vfs, Hooks::default(), &token);
    let (_, result) = scan(&hooked, &rules, None, &token);
    let cmp = result.unwrap().unwrap();
    assert!(cmp.is_complete());
    assert_eq!(hooked.stats.load(Ordering::SeqCst), n + 2);
    // Cancelled at the 1000th stat: no further stat, nothing of the batch inserted.
    let token = CancelToken::new();
    let hooked = Hooked::new(
        &vfs,
        Hooks {
            cancel_at_stat: Some(1000),
            ..Hooks::default()
        },
        &token,
    );
    let (_, result) = scan(&hooked, &rules, None, &token);
    let cmp = result.unwrap().unwrap();
    assert!(!cmp.is_complete());
    assert_eq!(hooked.stats.load(Ordering::SeqCst), 1000);
    let read = cmp.read();
    let d = read.children(NodeId::ROOT).next().unwrap();
    assert_eq!(read.children(d).len(), 0);
}

/// Entries whose stat failed (names only) are paired by name through hash maps: 20,000 of them against 20,000
/// other names (no match) and against the same names. The former quadratic pairing took minutes here.
#[test]
fn stat_failed_entries_pair_in_linear_time() {
    let n = 20_000;
    for same in [false, true] {
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        for f in 0..n {
            vfs.insert_file(&host(&format!("/l/d/a{f:06}")), b"x".to_vec(), meta())
                .unwrap();
            let right = if same {
                format!("/r/d/a{f:06}")
            } else {
                format!("/r/d/b{f:06}")
            };
            vfs.insert_file(&host(&right), b"x".to_vec(), meta())
                .unwrap();
        }
        let token = CancelToken::new();
        let hooked = Hooked::new(
            &vfs,
            Hooks {
                stat_fails_below: Some("/l/d".into()),
                ..Hooks::default()
            },
            &token,
        );
        let start = Instant::now();
        let (_, result) = scan(&hooked, &FolderRules::default(), None, &token);
        let elapsed = start.elapsed();
        let cmp = result.unwrap().unwrap();
        let stats = cmp.stats(false);
        assert_eq!(stats.files.error as usize, n, "same = {same}");
        let expected_nodes = if same { n } else { 2 * n };
        let read = cmp.read();
        let d = read.children(NodeId::ROOT).next().unwrap();
        assert_eq!(read.children(d).len(), expected_nodes, "same = {same}");
        drop(read);
        cmp.check_invariants().unwrap();
        assert!(
            elapsed < Duration::from_secs(60),
            "same = {same}: {elapsed:?}"
        );
    }
}

/// §2.3: quick tests run on a pair whose sides are both filtered by size or date filters (both stat'ed); the
/// state stays Filtered and no content test runs.
#[test]
fn filtered_pairs_of_stated_entries_get_quick_tests() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_file(
        &host("/l/big"),
        vec![b'x'; 100],
        MemMeta {
            mtime: Some(FileTime::from_unix(1_000_000, 0).unwrap()),
            ..MemMeta::default()
        },
    )
    .unwrap();
    vfs.insert_file(
        &host("/r/big"),
        vec![b'x'; 200],
        MemMeta {
            mtime: Some(FileTime::from_unix(2_000_000, 0).unwrap()),
            ..MemMeta::default()
        },
    )
    .unwrap();
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            size_filter: Some(hikasa_folder::Change::Set(
                hikasa_folder::SizeFilter::parse("<10").unwrap(),
            )),
            content_method: Some(hikasa_core::ContentMethod::Binary),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    let token = CancelToken::new();
    let (_, result) = scan(&vfs, &rules, None, &token);
    let cmp = result.unwrap().unwrap();
    let read = cmp.read();
    let node = read
        .node(read.children(NodeId::ROOT).next().unwrap())
        .unwrap();
    assert_eq!(node.state, CompareState::Filtered);
    assert_eq!(node.diffs, DiffFlags::SIZE | DiffFlags::TIME);
    assert_eq!(node.time, Some(hikasa_folder::TimeRelation::RightNewer));
    assert_eq!(node.content, None);
    drop(read);
    assert_eq!(cmp.verdict(false), CompareVerdict::NoDifferences);
}

/// FC-053/FC-054 with the default `exclude_protected`: a folder that is SYSTEM+HIDDEN on one side only does not
/// turn identical contents into orphans, in the folder tree and in flatten mode (exit 2, not 13).
#[test]
fn one_sided_protected_folder_in_both_modes() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(
        &host("/l/sys"),
        MemMeta {
            attrs: Some(hikasa_vfs::Attrs::Dos {
                attrs: hikasa_vfs::DosAttrs::SYSTEM | hikasa_vfs::DosAttrs::HIDDEN,
            }),
            ..meta()
        },
    )
    .unwrap();
    vfs.insert_dir(&host("/r/sys"), meta()).unwrap();
    for side in ["/l", "/r"] {
        vfs.insert_file(&host(&format!("{side}/sys/a.txt")), b"a".to_vec(), meta())
            .unwrap();
    }
    for flatten in [false, true] {
        let mut rules = FolderRules::default();
        rules.handling.flatten = flatten;
        for threads in [0, 4] {
            let pool = pool(threads);
            let token = CancelToken::new();
            let (_, result) = scan(&vfs, &rules, pool.as_ref(), &token);
            let cmp = result.unwrap().unwrap();
            cmp.check_invariants().unwrap();
            assert_eq!(
                cmp.verdict(false),
                CompareVerdict::NoDifferences,
                "flatten {flatten}"
            );
            assert_eq!(cmp.stats(false).files.same, 1, "flatten {flatten}");
        }
    }
}
