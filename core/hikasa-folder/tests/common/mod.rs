//! Shared helpers of the hikasa-folder integration tests: the golden-file helper (§6.1), the proptest
//! configuration, host-style `MemoryVfs` paths, the `tree.toml` fixture format and its materialisation, a
//! scripted fake rules comparator, a call-counting `Vfs` wrapper, the side mirror of canonical trees, and random
//! tree pairs for the properties.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use hikasa_core::case::{fold, nfc};
use hikasa_core::{
    CancelToken, CaseSensitivity, ContentComparator, ContentMethod, ContentOutcome, ContentResult,
    ContentSource, Ctl, ErrorKind, ErrorReport, FileTime, LocalPath, NullProgress, Side, Spec,
    Warning, WarningCode,
};
use hikasa_folder::{
    CanonicalEntry, CanonicalError, CanonicalNode, CanonicalTree, CompareState, ContentEngines,
    FolderComparison, FolderEnv, FolderError, FolderModel, FolderRules, NodeKind, NullObserver,
    ScanEvent, ScanObserver, TimeRelation,
};
use hikasa_vfs::{
    AtomicWrite, Attrs, Capabilities, DosAttrs, Entry, Fault, FileName, FollowLinks, ListOptions,
    Listing, MemMeta, MemoryVfs, ReadSeek, StreamOptions, UnixMeta, Vfs, VfsError, VfsOp,
    WriteOptions,
};
use proptest::test_runner::{Config, FileFailurePersistence};
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------------------------------------------
// Golden files, fixtures, proptest
// ----------------------------------------------------------------------------------------------------------

/// Compares `actual` with the golden file `expected`; with `HIKASA_BLESS=1` the golden file is (re)written
/// instead. Text files are shown as a `pretty_assertions` diff.
pub fn assert_golden(expected: &Path, actual: &[u8]) {
    let bless = std::env::var_os("HIKASA_BLESS").is_some_and(|v| v == "1");
    if bless {
        if let Some(dir) = expected.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let golden = std::fs::read(expected).unwrap_or_else(|e| {
        panic!(
            "missing golden file {} ({e}); run with HIKASA_BLESS=1",
            expected.display()
        )
    });
    if golden != actual {
        match (std::str::from_utf8(&golden), std::str::from_utf8(actual)) {
            (Ok(want), Ok(got)) => {
                pretty_assertions::assert_eq!(want, got, "golden {}", expected.display())
            }
            _ => panic!("golden {} differs (binary)", expected.display()),
        }
    }
}

/// `tests/fixtures/<suite>`.
pub fn fixtures(suite: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(suite)
}

/// Proptest configuration: 256 cases unless `PROPTEST_CASES` says otherwise (§6.1); failing seeds are persisted
/// in `regressions` (relative to the package).
pub fn config(regressions: &'static str) -> Config {
    config_with(regressions, 256)
}

/// Like [`config`] with another default case count (expensive properties).
pub fn config_with(regressions: &'static str, default_cases: u32) -> Config {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default_cases);
    Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(regressions))),
        ..Config::default()
    }
}

/// A host-style absolute path for `MemoryVfs` from a `/`-separated one: `/a/b` on Unix, `C:\a\b` on Windows.
pub fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// `Spec::Local` of [`host`]`(path)`.
pub fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(PathBuf::from(host(path))).unwrap())
}

/// A rayon pool with `n` threads (`None` for 0).
pub fn pool(n: usize) -> Option<rayon::ThreadPool> {
    (n > 0).then(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("test-pool-{i}"))
            .build()
            .unwrap()
    })
}

/// The default "now" of the tests: 2026-10-20T12:00:00Z in UTC.
pub fn default_now() -> jiff::Zoned {
    "2026-10-20T12:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::UTC)
}

/// The default modification time of fixture entries.
pub const DEFAULT_MTIME: &str = "2026-01-01T00:00:00Z";

// ----------------------------------------------------------------------------------------------------------
// tree.toml
// ----------------------------------------------------------------------------------------------------------

/// A `tree.toml` fixture: both sides of a comparison.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeFile {
    /// What the case shows.
    pub description: String,
    /// Options.
    pub options: TreeOptions,
    /// Entries below the left base (`/l`).
    pub left: Vec<EntrySpec>,
    /// Entries below the right base (`/r`).
    pub right: Vec<EntrySpec>,
    /// Injected faults.
    pub faults: Vec<FaultSpec>,
}

/// Options of a fixture.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TreeOptions {
    /// `MemoryVfs` name case: `sensitive` (default) or `insensitive`.
    pub fs_case: String,
    /// Replaces the back-end capabilities (snake_case names).
    pub capabilities: Option<Vec<String>>,
    /// Cancel after this many `FolderListed` events (the scan then runs on the current thread).
    pub cancel_after_listed: Option<usize>,
    /// "Now" (RFC 3339) for date filters.
    pub now: Option<String>,
    /// Size limit of the fake rules comparator (bytes).
    pub rules_limit: u64,
    /// Content budget in bytes.
    pub content_budget: u64,
}

impl Default for TreeOptions {
    fn default() -> TreeOptions {
        TreeOptions {
            fs_case: "sensitive".into(),
            capabilities: None,
            cancel_after_listed: None,
            now: None,
            rules_limit: 64,
            content_budget: 1 << 30,
        }
    }
}

/// One entry of a fixture side.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EntrySpec {
    /// Path relative to the base (`/`-separated).
    pub path: String,
    /// `file` (default), `dir` or `symlink`.
    pub kind: Option<String>,
    /// Inline content.
    pub content: Option<String>,
    /// Generated content of this length (`'x'` repeated) when `content` is absent.
    pub size: Option<u64>,
    /// Modification time (RFC 3339); default [`DEFAULT_MTIME`]; `"none"` for no time.
    pub mtime: Option<String>,
    /// DOS attribute letters.
    pub dos: Option<String>,
    /// Unix mode (sets Unix attributes; uid/gid default 1000).
    pub mode: Option<u32>,
    /// Unix uid.
    pub uid: Option<u32>,
    /// Unix gid.
    pub gid: Option<u32>,
    /// Symlink target (verbatim; an absolute `/…` target is made host-style).
    pub target: Option<String>,
    /// Back-end CRC.
    pub crc32: Option<u32>,
    /// The directory record states no kind (`MemoryVfs::set_kind_unknown`).
    pub kind_unknown: bool,
}

/// An injected fault.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    /// `left` or `right`.
    pub side: String,
    /// Path relative to that side's base.
    pub path: String,
    /// `stat`, `list`, `open`, `read`.
    pub op: VfsOp,
    /// Error kind.
    pub kind: ErrorKind,
}

fn base(side: &str) -> &'static str {
    match side {
        "left" => "/l",
        "right" => "/r",
        other => panic!("unknown side {other}"),
    }
}

fn join(base: &str, path: &str) -> String {
    if path.is_empty() {
        base.to_owned()
    } else {
        format!("{base}/{path}")
    }
}

/// Builds the `MemoryVfs` of a fixture (bases `/l` and `/r`).
pub fn materialize(tree: &TreeFile) -> MemoryVfs {
    let case = match tree.options.fs_case.as_str() {
        "insensitive" => CaseSensitivity::Insensitive,
        _ => CaseSensitivity::Sensitive,
    };
    let vfs = MemoryVfs::new(case);
    let meta = MemMeta {
        mtime: Some(FileTime::parse(DEFAULT_MTIME).unwrap()),
        ..MemMeta::default()
    };
    vfs.insert_dir(&host("/l"), meta.clone()).unwrap();
    vfs.insert_dir(&host("/r"), meta).unwrap();
    for (side, entries) in [("left", &tree.left), ("right", &tree.right)] {
        for e in entries {
            insert_entry(&vfs, base(side), e);
        }
    }
    for f in &tree.faults {
        vfs.inject(Fault {
            path: host(&join(base(&f.side), &f.path)),
            op: f.op,
            kind: f.kind,
        });
    }
    if let Some(names) = &tree.options.capabilities {
        let caps = names.iter().fold(Capabilities::empty(), |acc, n| {
            acc | Capabilities::parse_name(n).unwrap_or_else(|| panic!("capability {n}"))
        });
        vfs.set_capabilities(caps);
    }
    vfs
}

fn insert_entry(vfs: &MemoryVfs, base: &str, e: &EntrySpec) {
    let path = host(&join(base, &e.path));
    let mtime = match e.mtime.as_deref() {
        Some("none") => None,
        Some(t) => Some(FileTime::parse(t).unwrap()),
        None => Some(FileTime::parse(DEFAULT_MTIME).unwrap()),
    };
    let attrs = if let Some(letters) = &e.dos {
        Some(Attrs::Dos {
            attrs: DosAttrs::parse_letters(letters).unwrap(),
        })
    } else {
        e.mode.map(|mode| Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: e.uid.unwrap_or(1000),
                gid: e.gid.unwrap_or(1000),
            },
        })
    };
    let meta = MemMeta {
        mtime,
        ctime: None,
        attrs,
        crc32: e.crc32,
    };
    match e.kind.as_deref().unwrap_or("file") {
        "file" => {
            let content: Vec<u8> = match (&e.content, e.size) {
                (Some(c), _) => c.as_bytes().to_vec(),
                (None, Some(n)) => vec![b'x'; n as usize],
                (None, None) => Vec::new(),
            };
            vfs.insert_file(&path, content, meta).unwrap();
        }
        "dir" => vfs.insert_dir(&path, meta).unwrap(),
        "symlink" => {
            let target = e.target.clone().unwrap_or_default();
            let target = if target.starts_with('/') {
                host(&target)
            } else if cfg!(windows) {
                target.replace('/', "\\")
            } else {
                target
            };
            vfs.insert_symlink(&path, &target, meta).unwrap();
        }
        other => panic!("unknown kind {other}"),
    }
    if e.kind_unknown {
        vfs.set_kind_unknown(&path).unwrap();
    }
}

// ----------------------------------------------------------------------------------------------------------
// Fake rules comparator
// ----------------------------------------------------------------------------------------------------------

/// A scripted rules comparator: content starting with `!error` fails; content above `limit` is compared by bytes
/// with warning `rules_size_limit`; equal bytes → `BinarySame`; equal after CRLF → LF → `RulesSame`; equal after
/// removing ASCII whitespace → `UnimportantOnly`; else `ImportantDifferences`.
#[derive(Debug, Clone, Copy)]
pub struct FakeRules {
    /// Largest file loaded completely.
    pub limit: u64,
}

impl ContentComparator for FakeRules {
    fn method(&self) -> ContentMethod {
        ContentMethod::Rules
    }

    fn size_limit(&self) -> Option<u64> {
        Some(self.limit)
    }

    fn compare(
        &self,
        left: &dyn ContentSource,
        right: &dyn ContentSource,
        ctl: &Ctl<'_>,
    ) -> Result<ContentOutcome, hikasa_core::Cancelled> {
        ctl.check()?;
        let read = |s: &dyn ContentSource| s.read_all(u64::MAX, ctl.cancel);
        let (l, r) = match (read(left), read(right)) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(hikasa_core::ReadError::Cancelled), _)
            | (_, Err(hikasa_core::ReadError::Cancelled)) => {
                return Err(hikasa_core::Cancelled);
            }
            (Err(e), _) | (_, Err(e)) => {
                return Ok(ContentOutcome {
                    method: ContentMethod::Rules,
                    result: ContentResult::Error {
                        error: ErrorReport::capture(&e, None),
                    },
                    warnings: Vec::new(),
                });
            }
        };
        let mut warnings = Vec::new();
        let result = if l.starts_with(b"!error") || r.starts_with(b"!error") {
            ContentResult::Error {
                error: ErrorReport {
                    kind: ErrorKind::InvalidData,
                    message: "scripted rules failure".into(),
                    spec: None,
                    causes: Vec::new(),
                },
            }
        } else if l.len() as u64 > self.limit || r.len() as u64 > self.limit {
            warnings.push(Warning {
                code: WarningCode::RulesSizeLimit,
                message: "compared by bytes".into(),
                spec: None,
            });
            if l == r {
                ContentResult::BinarySame
            } else {
                ContentResult::BinaryDifferent
            }
        } else if l == r {
            ContentResult::BinarySame
        } else {
            let crlf = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
            let squeeze = |b: &[u8]| {
                b.iter()
                    .copied()
                    .filter(|c| !c.is_ascii_whitespace())
                    .collect::<Vec<u8>>()
            };
            if crlf(&l) == crlf(&r) {
                ContentResult::RulesSame
            } else if squeeze(&l) == squeeze(&r) {
                ContentResult::UnimportantOnly
            } else {
                ContentResult::ImportantDifferences
            }
        };
        Ok(ContentOutcome {
            method: ContentMethod::Rules,
            result,
            warnings,
        })
    }
}

/// CRC and binary comparators of hikasa-vfs plus [`FakeRules`] with `limit`.
pub fn engines(limit: u64) -> ContentEngines {
    ContentEngines::with_stream_options(
        StreamOptions::default(),
        Some(Arc::new(FakeRules { limit })),
    )
}

// ----------------------------------------------------------------------------------------------------------
// Running a comparison
// ----------------------------------------------------------------------------------------------------------

/// Cancels a token after `after` `FolderListed` events; forwards every event to `inner`.
pub struct CancelAfter<'a> {
    pub after: usize,
    pub seen: AtomicUsize,
    pub cancel: &'a CancelToken,
}

impl ScanObserver for CancelAfter<'_> {
    fn on_event(&self, event: ScanEvent) {
        if let ScanEvent::FolderListed { .. } = event
            && self.seen.fetch_add(1, Ordering::SeqCst) + 1 >= self.after
        {
            self.cancel.cancel();
        }
    }
}

/// Runs a comparison of `/l` and `/r` (or swapped) on `vfs`.
#[allow(clippy::too_many_arguments)]
pub fn run_on(
    vfs: &dyn Vfs,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
    engines: &ContentEngines,
    now: &jiff::Zoned,
    observer: &dyn ScanObserver,
    cancel: &CancelToken,
    swapped: bool,
) -> Result<FolderComparison, FolderError> {
    let progress = NullProgress;
    let env = FolderEnv {
        vfs,
        content: engines,
        pool,
        ctl: Ctl {
            cancel,
            progress: &progress,
        },
        observer,
        now,
        content_budget: 1 << 30,
        archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
    };
    let (l, r) = if swapped { ("/r", "/l") } else { ("/l", "/r") };
    let model = FolderModel::new(mspec(l), mspec(r), rules.clone())?;
    hikasa_folder::compare_into(&model, &env)
}

/// Runs a fixture: its options decide cancellation; `pool` is ignored when the case cancels part-way.
pub fn run_tree(
    tree: &TreeFile,
    vfs: &MemoryVfs,
    rules: &FolderRules,
    pool: Option<&rayon::ThreadPool>,
) -> Result<FolderComparison, FolderError> {
    let engines = engines(tree.options.rules_limit);
    let now = match &tree.options.now {
        Some(t) => t
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::UTC),
        None => default_now(),
    };
    let cancel = CancelToken::new();
    match tree.options.cancel_after_listed {
        Some(after) => {
            let observer = CancelAfter {
                after,
                seen: AtomicUsize::new(0),
                cancel: &cancel,
            };
            run_on(vfs, rules, None, &engines, &now, &observer, &cancel, false)
        }
        None => run_on(
            vfs,
            rules,
            pool,
            &engines,
            &now,
            &NullObserver,
            &cancel,
            false,
        ),
    }
}

/// The JSON document of a golden case.
#[derive(Debug, Serialize)]
pub struct Expected<'a> {
    pub description: &'a str,
    pub verdict: hikasa_core::CompareVerdict,
    pub verdict_ignoring_unimportant: hikasa_core::CompareVerdict,
    pub stats: hikasa_folder::FolderStats,
    pub stats_ignoring_unimportant: hikasa_folder::FolderStats,
    pub warnings: Vec<WarningCode>,
    pub tree: CanonicalTree,
}

/// The golden document of a comparison.
pub fn expected_json(description: &str, cmp: &FolderComparison) -> String {
    let read = cmp.read();
    let doc = Expected {
        description,
        verdict: cmp.verdict(false),
        verdict_ignoring_unimportant: cmp.verdict(true),
        stats: cmp.stats(false),
        stats_ignoring_unimportant: cmp.stats(true),
        warnings: read.warnings().iter().map(|w| w.code).collect(),
        tree: cmp.to_canonical(),
    };
    drop(read);
    let mut text = serde_json::to_string_pretty(&doc).unwrap();
    text.push('\n');
    text
}

// ----------------------------------------------------------------------------------------------------------
// Mirror (P-FOLDER-3)
// ----------------------------------------------------------------------------------------------------------

fn mirror_flag(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("left_") {
        format!("right_{rest}")
    } else if let Some(rest) = name.strip_prefix("right_") {
        format!("left_{rest}")
    } else {
        name.to_owned()
    }
}

fn flag_order(name: &str) -> usize {
    [
        "left_filtered",
        "right_filtered",
        "left_stat_skipped",
        "right_stat_skipped",
        "left_error",
        "right_error",
        "symlink_loop",
        "rules_skipped_size",
        "content_skipped",
    ]
    .iter()
    .position(|n| *n == name)
    .unwrap_or(usize::MAX)
}

fn kind_rank(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Folder => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Other => 3,
    }
}

/// Canonical order key of a node: kind rank, folded name, name bytes, right name bytes, folders.
type OrderKey = (u8, String, Vec<u8>, Option<Vec<u8>>, String, Option<String>);

/// Canonical order key of a node (§2.5.4, with the flatten folder tie-breakers).
fn order_key(n: &CanonicalNode) -> OrderKey {
    let repr = n.left.as_ref().or(n.right.as_ref());
    (
        kind_rank(n.kind),
        fold(&nfc(&n.name)).into_owned(),
        n.name.as_bytes().to_vec(),
        n.right.as_ref().map(|r| r.name.as_bytes().to_vec()),
        repr.and_then(|r| r.dir.clone()).unwrap_or_default(),
        n.right.as_ref().map(|r| r.dir.clone().unwrap_or_default()),
    )
}

/// The canonical tree of the same comparison with the sides exchanged.
pub fn mirror(tree: &CanonicalTree) -> CanonicalTree {
    fn node(n: &CanonicalNode, root: bool) -> CanonicalNode {
        let left = n.right.clone();
        let right = n.left.clone();
        let name = if root {
            String::new()
        } else {
            left.as_ref()
                .or(right.as_ref())
                .map(|e| e.name.clone())
                .unwrap_or_default()
        };
        let mut flags: Vec<String> = n.flags.iter().map(|f| mirror_flag(f)).collect();
        flags.sort_by_key(|f| flag_order(f));
        let mut errors: Vec<CanonicalError> = n
            .errors
            .iter()
            .map(|e| CanonicalError {
                side: e.side.map(Side::opposite),
                kind: e.kind,
            })
            .collect();
        errors.sort_by_key(|e| (e.side, e.kind.as_str()));
        let mut children: Vec<CanonicalNode> = n.children.iter().map(|c| node(c, false)).collect();
        children.sort_by_key(order_key);
        CanonicalNode {
            name,
            kind: n.kind,
            state: n.state.mirrored(),
            scan: n.scan,
            diffs: n.diffs.clone(),
            time: n.time.map(TimeRelation::mirrored),
            content: n.content,
            flags,
            errors,
            left,
            right,
            children,
        }
    }
    CanonicalTree {
        complete: tree.complete,
        root: node(&tree.root, true),
    }
}

/// Sorts the errors of every node by (side, kind) (their insertion order is not canonical across sides).
pub fn normalize_errors(tree: &mut CanonicalTree) {
    fn walk(n: &mut CanonicalNode) {
        n.errors.sort_by_key(|e| (e.side, e.kind.as_str()));
        for c in &mut n.children {
            walk(c);
        }
    }
    walk(&mut tree.root);
}

// ----------------------------------------------------------------------------------------------------------
// Counting VFS
// ----------------------------------------------------------------------------------------------------------

/// A `Vfs` wrapper that counts `stat` calls per spec and `list` calls.
pub struct CountingVfs<'a> {
    pub inner: &'a dyn Vfs,
    pub stats: Mutex<BTreeMap<String, usize>>,
    pub lists: AtomicUsize,
    /// Thread names seen in `list` (global-pool trap).
    pub threads: Mutex<Vec<String>>,
}

impl<'a> CountingVfs<'a> {
    pub fn new(inner: &'a dyn Vfs) -> CountingVfs<'a> {
        CountingVfs {
            inner,
            stats: Mutex::new(BTreeMap::new()),
            lists: AtomicUsize::new(0),
            threads: Mutex::new(Vec::new()),
        }
    }

    pub fn stat_count(&self, path: &str) -> usize {
        self.stats
            .lock()
            .unwrap()
            .get(&mspec(path).canonical())
            .copied()
            .unwrap_or(0)
    }

    pub fn total_stats(&self) -> usize {
        self.stats.lock().unwrap().values().sum()
    }
}

impl Vfs for CountingVfs<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        *self
            .stats
            .lock()
            .unwrap()
            .entry(spec.canonical())
            .or_default() += 1;
        self.inner.stat(spec, follow)
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        let name = std::thread::current().name().unwrap_or("").to_owned();
        self.threads.lock().unwrap().push(name);
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

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        self.inner.local_path(spec)
    }
}

// ----------------------------------------------------------------------------------------------------------
// Random tree pairs
// ----------------------------------------------------------------------------------------------------------

/// Names used by random trees (case variants, NFC/NFD, extensions).
pub const NAMES: [&str; 10] = [
    "a",
    "A",
    "b.txt",
    "B.TXT",
    "b.csv",
    "caf\u{e9}",
    "cafe\u{301}",
    "d",
    "e.bak",
    "f.txt",
];

/// One node of a random tree pair.
#[derive(Debug, Clone)]
pub struct GenNode {
    pub name: usize,
    /// 0 file, 1 folder, 2 symlink.
    pub left_kind: Option<u8>,
    pub right_kind: Option<u8>,
    pub left_content: u8,
    pub right_content: u8,
    pub left_mtime: u8,
    pub right_mtime: u8,
    pub left_attrs: u8,
    pub right_attrs: u8,
    pub children: Vec<GenNode>,
}

/// Contents of random files (some differ only in line endings or whitespace; one is long; one fails).
pub const CONTENTS: [&str; 6] = [
    "alpha\n",
    "alpha\r\n",
    "al pha\n",
    "beta\n",
    "a much longer content line that exceeds the fake limit of the rules comparator easily\n",
    "!error\n",
];

/// Symlink targets of random trees (relative; `.` loops when followed, `missing` is broken).
pub const TARGETS: [&str; 4] = ["a", "../d", ".", "missing"];

pub fn gen_tree() -> impl proptest::strategy::Strategy<Value = Vec<GenNode>> {
    use proptest::prelude::*;
    let leaf = (
        0..NAMES.len(),
        proptest::option::weighted(0.85, 0u8..3),
        proptest::option::weighted(0.85, 0u8..3),
        0u8..6,
        0u8..6,
        0u8..3,
        0u8..3,
        0u8..3,
        0u8..3,
    )
        .prop_map(|(name, lk, rk, lc, rc, lm, rm, la, ra)| GenNode {
            name,
            left_kind: lk,
            right_kind: rk,
            left_content: lc,
            right_content: if rc % 2 == 0 { lc } else { rc },
            left_mtime: lm,
            right_mtime: if rm == 0 { lm } else { rm },
            left_attrs: la,
            right_attrs: if ra == 0 { la } else { ra },
            children: Vec::new(),
        });
    let node = leaf.prop_recursive(3, 40, 5, |inner| {
        (
            0..NAMES.len(),
            proptest::bool::weighted(0.9),
            proptest::bool::weighted(0.9),
            proptest::collection::vec(inner, 0..5),
        )
            .prop_map(|(name, l, r, children)| GenNode {
                name,
                left_kind: l.then_some(1),
                right_kind: r.then_some(1),
                left_content: 0,
                right_content: 0,
                left_mtime: 0,
                right_mtime: 0,
                left_attrs: 0,
                right_attrs: 0,
                children,
            })
    });
    proptest::collection::vec(node, 0..7)
}

fn mtime_of(k: u8) -> FileTime {
    FileTime::from_unix(1_790_000_000 + i64::from(k) * 7_200 + i64::from(k % 2), 0).unwrap()
}

fn insert_gen(vfs: &MemoryVfs, base: &str, node: &GenNode, left: bool, unix: bool) {
    let path = format!("{base}/{}", NAMES[node.name]);
    let kind = if left {
        node.left_kind
    } else {
        node.right_kind
    };
    let Some(kind) = kind else {
        return;
    };
    let (content, mtime, attrs) = if left {
        (node.left_content, node.left_mtime, node.left_attrs)
    } else {
        (node.right_content, node.right_mtime, node.right_attrs)
    };
    let attrs = if unix {
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o640 + u32::from(attrs),
                uid: 1000 + u32::from(attrs),
                gid: 1000,
            },
        }
    } else {
        Attrs::Dos {
            attrs: match attrs {
                0 => DosAttrs::ARCHIVE,
                1 => DosAttrs::HIDDEN | DosAttrs::SYSTEM,
                _ => DosAttrs::READONLY,
            },
        }
    };
    let meta = MemMeta {
        mtime: Some(mtime_of(mtime)),
        ctime: None,
        attrs: Some(attrs),
        crc32: None,
    };
    let host_path = host(&path);
    let ok = match kind {
        0 => vfs
            .insert_file(
                &host_path,
                CONTENTS[usize::from(content)].as_bytes().to_vec(),
                meta,
            )
            .is_ok(),
        1 => vfs.insert_dir(&host_path, meta).is_ok(),
        _ => {
            let target = TARGETS[usize::from(content) % TARGETS.len()];
            let target = if cfg!(windows) {
                target.replace('/', "\\")
            } else {
                target.to_owned()
            };
            vfs.insert_symlink(&host_path, &target, meta).is_ok()
        }
    };
    if ok && kind == 1 {
        for child in &node.children {
            insert_gen(vfs, &path, child, left, unix);
        }
    }
}

/// Builds a `MemoryVfs` with the random pair below `/l` and `/r` (a name that collides with an earlier sibling
/// of another kind is skipped).
pub fn materialize_gen(nodes: &[GenNode], case: CaseSensitivity, unix: bool) -> MemoryVfs {
    let vfs = MemoryVfs::new(case);
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
    for node in nodes {
        insert_gen(&vfs, "/l", node, true, unix);
        insert_gen(&vfs, "/r", node, false, unix);
    }
    vfs
}

/// Random rules for the properties.
#[derive(Debug, Clone)]
pub struct GenRules {
    pub case: u8,
    pub align_extensions: bool,
    pub align_normalization: bool,
    pub filter: u8,
    pub method: u8,
    pub skip: bool,
    pub override_quick: bool,
    pub follow: bool,
    pub flatten: bool,
    pub size_filter: bool,
    pub timestamps: bool,
    pub orphans: bool,
    pub unix: bool,
    pub dos: bool,
}

pub const FILTERS: [&str; 7] = [
    "",
    "*.txt",
    r"-*.csv",
    r"-a\",
    r"A\",
    r"*.txt;-d\",
    r"-e.*;-caf*\",
];

pub fn gen_rules() -> impl proptest::strategy::Strategy<Value = GenRules> {
    use proptest::prelude::*;
    (
        (
            0u8..3,
            any::<bool>(),
            any::<bool>(),
            0u8..7,
            0u8..4,
            any::<bool>(),
            any::<bool>(),
        ),
        (
            proptest::bool::weighted(0.3),
            proptest::bool::weighted(0.2),
            proptest::bool::weighted(0.2),
            proptest::bool::weighted(0.8),
            proptest::bool::weighted(0.9),
            any::<bool>(),
            any::<bool>(),
        ),
    )
        .prop_map(
            |(
                (case, align_extensions, align_normalization, filter, method, skip, override_quick),
                (follow, flatten, size_filter, timestamps, orphans, unix, dos),
            )| GenRules {
                case,
                align_extensions,
                align_normalization,
                filter,
                method,
                skip,
                override_quick,
                follow,
                flatten,
                size_filter,
                timestamps,
                orphans,
                unix,
                dos,
            },
        )
}

impl GenRules {
    /// The folder rules (`allow_rules`: the fake rules method may be chosen).
    pub fn rules(&self, allow_rules: bool) -> FolderRules {
        use hikasa_folder::{CaseMode, Cmp, FolderRulesPatch, SizeFilter, UnixCriteria};
        let mut rules = FolderRules::default();
        let method = match self.method {
            0 => ContentMethod::None,
            1 => ContentMethod::Crc,
            2 => ContentMethod::Binary,
            _ if allow_rules => ContentMethod::Rules,
            _ => ContentMethod::None,
        };
        rules
            .apply(&FolderRulesPatch {
                case: Some(match self.case {
                    0 => CaseMode::Auto,
                    1 => CaseMode::Sensitive,
                    _ => CaseMode::Insensitive,
                }),
                align_extensions: Some(self.align_extensions),
                align_normalization: Some(self.align_normalization),
                name_filter: Some(FILTERS[usize::from(self.filter)].to_owned()),
                content_method: Some(method),
                skip_if_quick_same: Some(self.skip),
                override_quick: Some(self.override_quick),
                follow_symlinks: Some(self.follow),
                flatten: Some(self.flatten),
                size_filter: Some(if self.size_filter {
                    hikasa_folder::Change::Set(SizeFilter {
                        cmp: Cmp::Less,
                        bytes: 10,
                    })
                } else {
                    hikasa_folder::Change::Clear
                }),
                timestamps_enabled: Some(self.timestamps),
                scan_orphan_folders: Some(self.orphans),
                unix: Some(UnixCriteria {
                    permissions: self.unix,
                    owner: self.unix,
                    group: false,
                }),
                dos_attributes: Some(if self.dos {
                    DosAttrs::HIDDEN | DosAttrs::READONLY
                } else {
                    DosAttrs::empty()
                }),
                ..FolderRulesPatch::default()
            })
            .unwrap();
        rules
    }
}

/// Paths of every node of a canonical tree (pre-order, `/`-joined representative names).
pub fn paths(tree: &CanonicalTree) -> Vec<String> {
    fn walk(n: &CanonicalNode, prefix: &str, out: &mut Vec<String>) {
        for c in &n.children {
            let p = if prefix.is_empty() {
                c.name.clone()
            } else {
                format!("{prefix}/{}", c.name)
            };
            out.push(p.clone());
            walk(c, &p, out);
        }
    }
    let mut out = Vec::new();
    walk(&tree.root, "", &mut out);
    out
}

/// Finds a node by path of representative names.
pub fn find<'t>(tree: &'t CanonicalTree, path: &str) -> &'t CanonicalNode {
    let mut node = &tree.root;
    for name in path.split('/').filter(|n| !n.is_empty()) {
        node = node
            .children
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no node {path} (at {name})"));
    }
    node
}

/// The state of the node at `path`.
pub fn state_at(tree: &CanonicalTree, path: &str) -> CompareState {
    find(tree, path).state
}

/// Entry of a canonical node side (for assertions).
pub fn entry_name(e: &Option<CanonicalEntry>) -> Option<&str> {
    e.as_ref().map(|e| e.name.as_str())
}
