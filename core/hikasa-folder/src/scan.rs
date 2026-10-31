//! The parallel, cancellable scanner (FC-016; §3.3, §5.1–§5.4): [`compare_into`], [`compare_folders`],
//! [`FolderEnv`], [`ContentEngines`].
//!
//! The scanner runs on the explicit pool of [`FolderEnv::pool`] (`None` = the current thread; the rayon global
//! pool is never touched, T7). Each folder pair is a task: both sides are listed (in parallel with `join`) with
//! the name-mask prefilter of D-45, aligned (§2.5), the filtered side of an FC-054 pair is stat'ed, the quick
//! tests run (§2.6.1), and the children are inserted as one contiguous block under the model's write lock,
//! which also updates the counters of every ancestor (§2.6.4). Content tests are tasks of their own that take
//! bytes from a shared [`ByteBudget`] (D-25) before they read. Events are sent after the lock is released.
//!
//! A batch keeps each listed entry once (the listing's own vector, with the scan facts in a parallel vector) and
//! small per-pair records, and every step of a batch polls the cancellation token at bounded intervals, so a
//! folder with a million entries neither multiplies its memory nor delays cancellation (§5.3, §5.7). A batch
//! cancelled before its insertion is dropped: the folder stays not scanned and the result incomplete.

use std::borrow::Cow;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use hikasa_core::{
    Cancelled, ContentComparator, ContentMethod, ContentSource, ErrorKind, ErrorReport,
    FolderVerdict, Pair, Phase, Progress, RelPath, Side, Spec, Unit, Warning, WarningCode,
};
use hikasa_vfs::{
    BinaryComparator, Capabilities, CrcComparator, Entry, EntryKind, FileId, FileName, FollowLinks,
    ListOptions, Listing, StreamOptions, Vfs, VfsError, VfsSource,
};
use parking_lot::Mutex;

use crate::align::{
    AlignInput, AlignOptions, NO, align_pairs, canonical_fold, exact_bytes, folded_exact, kind_rank,
};
use crate::budget::ByteBudget;
use crate::error::FolderError;
use crate::events::{ScanEvent, ScanObserver};
use crate::model::store::{DirInfo, NewNode, NewSide, internal, stage};
use crate::model::{
    BaseInfo, FolderComparison, FolderModel, NodeError, NodeFlags, NodeId, NodeKind, ScanStatus,
};
use crate::quick::{DiffFlags, quick_tests};
use crate::resolve::{ResolvedRules, resolve_case, side_of};
use crate::rules::{ArchiveHandling, Criteria, FolderRules};
use crate::state::{CompareState, base_state};
use crate::time_cmp::TimeRelation;

/// Depth limit of a scan that follows symbolic links (D-33).
pub const MAX_FOLLOW_DEPTH: u32 = 256;

/// Largest `NodesUpdated` batch.
const EVENT_BATCH: usize = 256;

/// Entries (or pairs) handled between two cancellation checks inside one batch (§5.3). A re-stat (I/O) checks
/// before every call.
const CHECK_EVERY: usize = 1024;

/// Content engines given to the scanner (hikasa-text's RulesComparator is passed as `rules`).
#[derive(Clone)]
pub struct ContentEngines {
    /// CRC method.
    pub crc: Arc<dyn ContentComparator>,
    /// Binary method.
    pub binary: Arc<dyn ContentComparator>,
    /// Rules method (None → `FolderError::RulesComparatorMissing` when requested).
    pub rules: Option<Arc<dyn ContentComparator>>,
}

impl ContentEngines {
    /// The hikasa-vfs CRC and binary comparators with `stream` options, plus an optional rules comparator.
    /// `stream.threads` is the caller's choice (the session passes `min(4, pool size)`, §5.1). Additive API.
    pub fn with_stream_options(
        stream: StreamOptions,
        rules: Option<Arc<dyn ContentComparator>>,
    ) -> ContentEngines {
        ContentEngines {
            crc: Arc::new(CrcComparator { opts: stream }),
            binary: Arc::new(BinaryComparator { opts: stream }),
            rules,
        }
    }

    /// The comparator of `method` (`None` for `ContentMethod::None` and for a missing rules comparator).
    pub fn get(&self, method: ContentMethod) -> Option<&Arc<dyn ContentComparator>> {
        match method {
            ContentMethod::None => None,
            ContentMethod::Crc => Some(&self.crc),
            ContentMethod::Binary => Some(&self.binary),
            ContentMethod::Rules => self.rules.as_ref(),
        }
    }
}

impl fmt::Debug for ContentEngines {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContentEngines")
            .field("crc", &self.crc.method())
            .field("binary", &self.binary.method())
            .field("rules", &self.rules.as_ref().map(|r| r.method()))
            .finish()
    }
}

/// Execution environment of a folder comparison.
pub struct FolderEnv<'a> {
    /// VFS (normally `VfsRouter`).
    pub vfs: &'a dyn Vfs,
    /// Content engines.
    pub content: &'a ContentEngines,
    /// Pool for listing and content tests (None = current thread).
    pub pool: Option<&'a rayon::ThreadPool>,
    /// Cancellation + progress.
    pub ctl: hikasa_core::Ctl<'a>,
    /// Incremental events.
    pub observer: &'a dyn ScanObserver,
    /// "Now" in the user's zone (date filters relative to midnight; D-24).
    pub now: &'a jiff::Zoned,
    /// Content-test memory budget in bytes (D-25). The session passes `content_memory_budget_mib` × 2^20.
    pub content_budget: u64,
    /// Entries listed inside archives after which a scan opens no further archive (D-151); normally
    /// [`SCAN_ARCHIVE_ENTRIES_MAX`].
    pub archive_entries_max: u64,
}

impl fmt::Debug for FolderEnv<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FolderEnv")
            .field("content", self.content)
            .field(
                "pool_threads",
                &self.pool.map(rayon::ThreadPool::current_num_threads),
            )
            .field("now", self.now)
            .field("content_budget", &self.content_budget)
            .finish_non_exhaustive()
    }
}

/// `FolderModel::new` + `compare_into` (headless callers).
pub fn compare_folders(
    left: &Spec,
    right: &Spec,
    rules: &FolderRules,
    env: &FolderEnv<'_>,
) -> Result<FolderComparison, FolderError> {
    let model = FolderModel::new(left.clone(), right.clone(), rules.clone())?;
    compare_into(&model, env)
}

/// Scans `model` (created by `FolderModel::new`, never scanned before: else `AlreadyScanned`). Loads the bases
/// (errors → `BaseNotLoadable` / `BaseNotFolder`), then fills the model (§2.5–§2.6, §3.3). Cancellation after the
/// bases are loaded returns `Ok` with `is_complete() == false`.
///
/// The bases are "loaded" when both are stat'ed and listed; a failure or a cancellation before that is an
/// error (no event is sent then). The model is marked finished on every return path.
pub fn compare_into(
    model: &Arc<FolderModel>,
    env: &FolderEnv<'_>,
) -> Result<FolderComparison, FolderError> {
    let rules = model.rules();
    let method = rules.criteria.content.method;
    if method != ContentMethod::None && env.content.get(method).is_none() {
        return Err(FolderError::RulesComparatorMissing);
    }
    if !model.claim() {
        return Err(FolderError::AlreadyScanned);
    }
    let mut guard = FinishGuard {
        model,
        ended: false,
    };
    let result = run(model, env);
    guard.ended = result.is_ok();
    drop(guard);
    result
}

/// Marks the model finished on every exit path of `compare_into`, unwinding included (§5.6): a panic in a task
/// propagates out of `compare_into`, and a GUI polling the shared model must not wait for a scan that ended. Unless
/// the scan returned normally (which records its completeness itself), the model is marked incomplete.
struct FinishGuard<'m> {
    model: &'m FolderModel,
    ended: bool,
}

impl Drop for FinishGuard<'_> {
    fn drop(&mut self) {
        if !self.ended {
            self.model.mutate(|s| s.finish(false));
        }
        self.model.set_finished();
    }
}

/// The scan after the checks of `compare_into`.
fn run(model: &Arc<FolderModel>, env: &FolderEnv<'_>) -> Result<FolderComparison, FolderError> {
    let rules = model.rules();
    env.ctl.check()?;
    let specs = model.specs();
    let left = load_base(env.vfs, Side::Left, &specs.left)?;
    let right = load_base(env.vfs, Side::Right, &specs.right)?;
    let caps = [left.capabilities, right.capabilities];
    let case = resolve_case(rules.alignment.case, caps);
    let resolved = ResolvedRules::new(rules, case, env.now)?;
    let follow = if rules.handling.follow_symlinks {
        FollowLinks::Yes
    } else {
        FollowLinks::No
    };
    let align = AlignOptions {
        case,
        align_extensions: rules.alignment.align_extensions,
        align_normalization: rules.alignment.align_normalization,
    };
    let open_archives = rules
        .open_archives
        .iter()
        .map(|path| path_key(&align, path).into_owned())
        .collect();
    let scan = Scan {
        env,
        model,
        rules,
        criteria: effective_criteria(&rules.criteria, caps),
        resolved,
        align,
        follow,
        budget: ByteBudget::new(env.content_budget),
        caps,
        wants_events: env.observer.wants_events(),
        pending: Mutex::new(Vec::new()),
        in_flight: AtomicUsize::new(0),
        folders_done: AtomicU64::new(0),
        content_done: AtomicU64::new(0),
        content_total: AtomicU64::new(0),
        mtime_warned: [AtomicBool::new(false), AtomicBool::new(false)],
        aborted: AtomicBool::new(false),
        flat_items: [Mutex::new(Items::default()), Mutex::new(Items::default())],
        flat_dirs: [Mutex::new(Vec::new()), Mutex::new(Vec::new())],
        flat_deferred: [Mutex::new(Vec::new()), Mutex::new(Vec::new())],
        open_archives,
        archive_entries: AtomicU64::new(0),
        archive_limit_warned: AtomicBool::new(false),
    };
    let root_sides = [
        Some(SideFolder::root(left.spec.clone(), &left.entry, follow)),
        Some(SideFolder::root(right.spec.clone(), &right.entry, follow)),
    ];
    // The root listing is part of loading the bases.
    let listings = scan.list_pair(&root_sides);
    let mut root_listings: [Option<Listing>; 2] = [None, None];
    for (index, listing) in listings.into_iter().enumerate() {
        let side = side_of(index);
        match listing {
            Some(Ok(listing)) => root_listings[index] = Some(listing),
            Some(Err(e)) if e.is_cancelled() => return Err(FolderError::Cancelled),
            Some(Err(source)) => {
                return Err(FolderError::BaseNotLoadable {
                    side,
                    spec: specs.get(side).map_or_else(String::new, Spec::canonical),
                    source,
                });
            }
            None => {}
        }
    }
    let warnings = scan.capability_warnings(&[&left, &right]);
    model.mutate(|s| {
        s.set_bases(Pair::new(left, right), case, scan.resolved.clone());
        for w in warnings {
            s.warn(w);
        }
    });
    scan.emit(ScanEvent::Started);
    if rules.handling.flatten {
        scan.run_flatten(root_sides, root_listings);
    } else {
        let mut initial = Vec::new();
        let [l, r] = root_listings;
        scan.process_folder(
            NodeId::ROOT,
            0,
            &root_sides,
            [l.map(Ok), r.map(Ok)],
            &mut |t| initial.push(t),
        );
        scan.execute(initial);
    }
    scan.flush();
    scan.final_progress();
    let complete = !scan.aborted.load(Ordering::Acquire);
    let back_end = env.vfs.take_warnings();
    model.mutate(|s| {
        for w in back_end {
            s.warn(w);
        }
        s.finish(complete);
    });
    let cmp = FolderComparison::new(Arc::clone(model));
    model.set_finished();
    scan.emit(ScanEvent::Finished { complete });
    debug_assert!(
        cmp.check_invariants().is_ok(),
        "{:?}",
        cmp.check_invariants()
    );
    Ok(cmp)
}

/// The alignment's case and normalisation options applied to a whole path.
fn path_key<'t>(align: &AlignOptions, text: &'t str) -> Cow<'t, str> {
    match align.case {
        hikasa_core::CaseSensitivity::Insensitive => canonical_fold(text),
        hikasa_core::CaseSensitivity::Sensitive if align.align_normalization => {
            hikasa_core::case::nfc(text)
        }
        hikasa_core::CaseSensitivity::Sensitive => Cow::Borrowed(text),
    }
}

/// `criteria` without the quick tests that a base's back-end cannot evaluate (§2.6.1).
fn effective_criteria(criteria: &Criteria, caps: [Capabilities; 2]) -> Criteria {
    let both = |cap: Capabilities| caps.iter().all(|c| c.contains(cap));
    let mut out = criteria.clone();
    if !both(Capabilities::MTIME) {
        out.timestamps.enabled = false;
    }
    if !both(Capabilities::DOS_ATTRS) {
        out.dos_attributes = hikasa_vfs::DosAttrs::empty();
    }
    if !both(Capabilities::UNIX_MODE) {
        out.unix.permissions = false;
    }
    if !both(Capabilities::UNIX_OWNER) {
        out.unix.owner = false;
        out.unix.group = false;
    }
    out
}

/// Stats a base (symlinks followed: a base named through a link is its target) and reads its capabilities.
fn load_base(vfs: &dyn Vfs, side: Side, spec: &Spec) -> Result<BaseInfo, FolderError> {
    let not_loadable = |source: VfsError| {
        if source.is_cancelled() {
            FolderError::Cancelled
        } else {
            FolderError::BaseNotLoadable {
                side,
                spec: spec.canonical(),
                source,
            }
        }
    };
    let entry = vfs.stat(spec, FollowLinks::Yes).map_err(not_loadable)?;
    if entry.kind != EntryKind::Dir {
        return Err(FolderError::BaseNotFolder {
            side,
            spec: spec.canonical(),
        });
    }
    let capabilities = vfs.capabilities(spec).map_err(not_loadable)?;
    Ok(BaseInfo {
        spec: spec.clone(),
        entry,
        capabilities,
    })
}

/// Entries listed inside archives after which a scan opens no further archive (D-151): it bounds a recursive
/// archive in `Always` mode, with `nesting_max` and the cache limits.
pub const SCAN_ARCHIVE_ENTRIES_MAX: u64 = 5_000_000;

/// One side of a folder that is listed.
#[derive(Debug, Clone)]
struct SideFolder {
    spec: Spec,
    /// That side's path relative to its base (its own names).
    rel: RelPath,
    /// The folder's verdict on that side (§2.3).
    verdict: FolderVerdict,
    /// File ids of the folder and its ancestors (only when following links, D-33).
    chain: Vec<FileId>,
}

impl SideFolder {
    fn root(spec: Spec, entry: &Entry, follow: FollowLinks) -> SideFolder {
        SideFolder {
            spec,
            rel: RelPath::root(),
            verdict: FolderVerdict::Base,
            chain: match (follow, entry.file_id) {
                (FollowLinks::Yes, Some(id)) => vec![id],
                _ => Vec::new(),
            },
        }
    }
}

/// What the scanner knows about one listed entry besides the entry itself (kept apart, so that a batch holds each
/// listed `Entry` once: in the listing's own vector).
#[derive(Debug)]
struct Facts {
    /// Hidden by a filter (name masks or other filters).
    filtered: bool,
    /// Name-only (D-45).
    stat_skipped: bool,
    /// The metadata could not be read: only the name is known.
    no_meta: bool,
    /// Flatten: a folder that could not be listed (or closed a loop), shown as a failed folder node.
    failed_folder: bool,
    /// The error is a symbolic-link loop (D-33): a followed link back to an ancestor, or a link cycle the back-end
    /// reported as `ELOOP`.
    looped: bool,
    /// Flatten: the entry lies at or below a folder whose listing failed on the other side. It is not aligned and
    /// its state is Unknown, never an orphan (§2.6.2: the failing side is unknown).
    partial: bool,
    /// Folder verdict of the name masks on this side (folders; `Excluded` for other kinds).
    verdict: FolderVerdict,
    /// Flatten folder index (`NO` outside flatten mode).
    dir: u32,
    /// A per-entry error (failed stat, broken or looping followed link).
    error: Option<Box<ErrorReport>>,
    /// An archive by its name (D-151): `NodeFlags::ARCHIVE`.
    archive: bool,
    /// An archive opened as a folder: its entry reads as a folder, its spec is the archive's root.
    opened: bool,
}

impl Facts {
    fn new(dir: u32) -> Facts {
        Facts {
            filtered: false,
            stat_skipped: false,
            no_meta: false,
            failed_folder: false,
            looped: false,
            partial: false,
            verdict: FolderVerdict::Excluded,
            dir,
            error: None,
            archive: false,
            opened: false,
        }
    }

    fn dir(&self) -> Option<u32> {
        (self.dir != NO).then_some(self.dir)
    }
}

/// One side's entries of a batch and their facts (parallel vectors, same length).
#[derive(Debug, Default)]
struct Items {
    entries: Vec<Entry>,
    facts: Vec<Facts>,
}

impl Items {
    fn len(&self) -> usize {
        self.entries.len().min(self.facts.len())
    }

    fn get(&self, i: u32) -> Option<(&Entry, &Facts)> {
        if i == NO {
            return None;
        }
        Some((self.entries.get(i as usize)?, self.facts.get(i as usize)?))
    }

    fn push(&mut self, entry: Entry, facts: Facts) {
        self.entries.push(entry);
        self.facts.push(facts);
    }

    fn append(&mut self, other: Items) {
        self.entries.extend(other.entries);
        self.facts.extend(other.facts);
    }

    fn index(i: usize) -> u32 {
        u32::try_from(i).unwrap_or(NO)
    }
}

/// A folder pair to list.
struct FolderTask {
    node: NodeId,
    depth: u32,
    sides: [Option<SideFolder>; 2],
}

/// A folder of one side to collect in flatten mode.
struct FlatTask {
    side: usize,
    folder: SideFolder,
    dir: u32,
    /// The collected folder that contains this one.
    parent_dir: u32,
    depth: u32,
    /// The folder's entry (shown as a failed folder node when the listing fails).
    entry: Entry,
}

/// Flatten: a folder hidden by the other filters only (exclude protected) on its side. It is collected once the
/// other side has a collected folder of the same path, so a folder protected on one side only is compared like any
/// FC-054 pair; a folder protected on every side stays hidden (§2.3, as in the folder tree).
struct Deferred {
    folder: SideFolder,
    parent_dir: u32,
    depth: u32,
    entry: Entry,
}

/// A content test.
struct ContentTask {
    node: NodeId,
    sides: Box<[(Spec, Entry); 2]>,
}

enum Task {
    Folder(FolderTask),
    Flat(FlatTask),
    Content(ContentTask),
}

/// Everything a scan shares between its tasks.
struct Scan<'a> {
    env: &'a FolderEnv<'a>,
    model: &'a FolderModel,
    rules: &'a FolderRules,
    /// The criteria the back-ends can evaluate (a criterion whose capability is missing on either side is
    /// switched off, with one `capability_missing` warning per side, §2.6.1).
    criteria: Criteria,
    resolved: ResolvedRules,
    align: AlignOptions,
    follow: FollowLinks,
    budget: ByteBudget,
    caps: [Capabilities; 2],
    wants_events: bool,
    pending: Mutex<Vec<NodeId>>,
    in_flight: AtomicUsize,
    folders_done: AtomicU64,
    content_done: AtomicU64,
    content_total: AtomicU64,
    mtime_warned: [AtomicBool; 2],
    /// Some work was skipped because of cancellation.
    aborted: AtomicBool,
    /// Flatten: collected entries per side.
    flat_items: [Mutex<Items>; 2],
    /// Flatten: collected folders per side.
    flat_dirs: [Mutex<Vec<DirInfo>>; 2],
    /// Flatten: protected folders waiting for their counterpart, per side.
    flat_deferred: [Mutex<Vec<Deferred>>; 2],
    /// `open_archives`, as [`Scan::path_key`] keys.
    open_archives: HashSet<String>,
    /// Entries listed inside archives (D-151).
    archive_entries: AtomicU64,
    /// The `archive_scan_limit` warning was added.
    archive_limit_warned: AtomicBool,
}

fn spawn_task<'s>(scope: &rayon::Scope<'s>, scan: &'s Scan<'_>, task: Task) {
    scope.spawn(move |scope| scan.run_task(task, &mut |t| spawn_task(scope, scan, t)));
}

/// Decrements the in-flight content counter (and flushes pending events at zero) on drop.
struct InFlight<'s, 'a>(&'s Scan<'a>);

impl Drop for InFlight<'_, '_> {
    fn drop(&mut self) {
        if self.0.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.flush();
        }
    }
}

/// A child being planned: its sides (indices into the side item lists, `NO` = absent) and its node data. The
/// rare parts (content specs, folders to list next) are boxed, so a planned child costs a few dozen bytes.
struct Planned {
    sides: [u32; 2],
    kind: NodeKind,
    state: CompareState,
    scan: ScanStatus,
    diffs: DiffFlags,
    time: Option<TimeRelation>,
    flags: NodeFlags,
    internal: u8,
    errors: Vec<NodeError>,
    /// A content test is scheduled (with both sides' specs).
    content: Option<Box<[Spec; 2]>>,
    /// Folder sides to list next (with their spec).
    descend: Option<Box<[Option<SideFolder>; 2]>>,
}

/// Entries in a batch from which it is freed on the pool ([`Scan::discard`]).
const DISCARD_ON_POOL: usize = 32_768;

/// The number of listed entries a value holds (what makes dropping it slow).
trait Bulk {
    fn bulk(&self) -> usize;
}

impl Bulk for Items {
    fn bulk(&self) -> usize {
        self.entries.len()
    }
}

impl Bulk for Listing {
    fn bulk(&self) -> usize {
        self.entries.len() + self.name_only.len() + self.errors.len()
    }
}

impl<T: Bulk, E> Bulk for Result<T, E> {
    fn bulk(&self) -> usize {
        self.as_ref().map_or(0, Bulk::bulk)
    }
}

impl<T: Bulk> Bulk for Option<T> {
    fn bulk(&self) -> usize {
        self.as_ref().map_or(0, Bulk::bulk)
    }
}

impl<T: Bulk, const N: usize> Bulk for [T; N] {
    fn bulk(&self) -> usize {
        self.iter().map(Bulk::bulk).sum()
    }
}

impl<A: Bulk, B: Bulk> Bulk for (A, B) {
    fn bulk(&self) -> usize {
        self.0.bulk() + self.1.bulk()
    }
}

impl<A: Bulk, B: Bulk, C: Bulk> Bulk for (A, B, C) {
    fn bulk(&self) -> usize {
        self.0.bulk() + self.1.bulk() + self.2.bulk()
    }
}

/// The task parts of a planned child, taken out before the insertion consumes the planned records: the child's
/// offset in the inserted block, its content test (specs and entries of both sides) and its folders to list.
struct Detached {
    offset: u32,
    content: Option<Box<[(Spec, Entry); 2]>>,
    descend: Option<Box<[Option<SideFolder>; 2]>>,
}

/// Builds the spec of an entry of one side: `(side, entry, flatten folder index)`.
type SpecOf<'f> = dyn Fn(usize, &Entry, Option<u32>) -> Result<Spec, VfsError> + 'f;

/// An error report that names no host path in its kind (loops, depth limit).
fn report(kind: ErrorKind, message: String, spec: Option<&Spec>) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: spec.map(Spec::canonical),
        causes: Vec::new(),
    }
}

/// The OS error code of a symbolic-link loop: `ELOOP` (Linux 40 on the common architectures, 62 on macOS and the
/// BSDs) or Windows' `ERROR_CANT_RESOLVE_FILENAME` (1921) — the codes LocalVfs passes on and MemoryVfs imitates
/// (§1.2.7 item 5).
#[cfg(all(
    any(target_os = "linux", target_os = "android"),
    not(any(
        target_arch = "mips",
        target_arch = "mips64",
        target_arch = "mips32r6",
        target_arch = "mips64r6",
        target_arch = "sparc",
        target_arch = "sparc64"
    ))
))]
const LOOP_OS_ERROR: Option<i32> = Some(40);
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
))]
const LOOP_OS_ERROR: Option<i32> = Some(62);
#[cfg(windows)]
const LOOP_OS_ERROR: Option<i32> = Some(1921);
#[cfg(not(any(
    all(
        any(target_os = "linux", target_os = "android"),
        not(any(
            target_arch = "mips",
            target_arch = "mips64",
            target_arch = "mips32r6",
            target_arch = "mips64r6",
            target_arch = "sparc",
            target_arch = "sparc64"
        ))
    ),
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly",
    windows
)))]
const LOOP_OS_ERROR: Option<i32> = None;

/// True when a per-entry error is the back-end's report of a symbolic-link cycle (D-33). An `ErrorReport` keeps the
/// rendered OS error only (`"… (os error 40)"`), so the code is matched in its message and causes; on a platform
/// whose code is not known, the generic text MemoryVfs uses there.
pub(crate) fn is_link_loop(error: &ErrorReport) -> bool {
    let texts = || std::iter::once(&error.message).chain(error.causes.iter());
    match LOOP_OS_ERROR {
        Some(code) => {
            let suffix = format!("(os error {code})");
            texts().any(|t| t.ends_with(&suffix))
        }
        None => texts().any(|t| t.contains("Too many levels of symbolic links")),
    }
}

/// The joined path of a flatten folder (`dir`) and a name.
fn join_text(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

impl<'a> Scan<'a> {
    fn cancelled(&self) -> bool {
        if self.env.ctl.cancel.is_cancelled() {
            self.aborted.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }

    /// `cancelled()` every `CHECK_EVERY` steps.
    fn cancelled_at(&self, step: usize) -> bool {
        step.is_multiple_of(CHECK_EVERY) && self.cancelled()
    }

    fn emit(&self, event: ScanEvent) {
        if self.wants_events {
            self.env.observer.on_event(event);
        }
    }

    fn note_updated(&self, id: NodeId) {
        if !self.wants_events {
            return;
        }
        let batch = {
            let mut pending = self.pending.lock();
            pending.push(id);
            if pending.len() >= EVENT_BATCH {
                Some(std::mem::take(&mut *pending))
            } else {
                None
            }
        };
        if let Some(mut nodes) = batch {
            nodes.sort_unstable();
            nodes.dedup();
            self.emit(ScanEvent::NodesUpdated { nodes });
        }
    }

    /// Unthrottled completion reports (`done == total`), so that progress displays reach their end (§5.4).
    fn final_progress(&self) {
        let folders = self.folders_done.load(Ordering::Relaxed);
        self.env.ctl.progress_now(Progress {
            phase: Phase::Scanning,
            done: folders,
            total: Some(folders),
            unit: Unit::Folders,
            current: None,
        });
        let total = self.content_total.load(Ordering::Relaxed);
        if total > 0 {
            self.env.ctl.progress_now(Progress {
                phase: Phase::ComparingContent,
                done: self.content_done.load(Ordering::Relaxed),
                total: Some(total),
                unit: Unit::Items,
                current: None,
            });
        }
    }

    fn flush(&self) {
        if !self.wants_events {
            return;
        }
        let mut nodes = std::mem::take(&mut *self.pending.lock());
        if !nodes.is_empty() {
            nodes.sort_unstable();
            nodes.dedup();
            self.emit(ScanEvent::NodesUpdated { nodes });
        }
    }

    /// `capability_missing` once per (criterion, side) for the criteria the back-ends cannot evaluate.
    fn capability_warnings(&self, bases: &[&BaseInfo; 2]) -> Vec<Warning> {
        let c = &self.rules.criteria;
        let mut out = Vec::new();
        let needs: [(bool, Capabilities, &str); 5] = [
            (
                c.timestamps.enabled,
                Capabilities::MTIME,
                "modification times (timestamps)",
            ),
            (
                !c.dos_attributes.is_empty(),
                Capabilities::DOS_ATTRS,
                "DOS attributes",
            ),
            (
                c.unix.permissions,
                Capabilities::UNIX_MODE,
                "Unix permissions",
            ),
            (c.unix.owner, Capabilities::UNIX_OWNER, "Unix owners"),
            (c.unix.group, Capabilities::UNIX_OWNER, "Unix groups"),
        ];
        for (index, base) in bases.iter().enumerate() {
            for (wanted, cap, what) in needs {
                if wanted && !self.caps[index].contains(cap) {
                    out.push(Warning {
                        code: WarningCode::CapabilityMissing,
                        message: format!(
                            "{} back-end provides no {what}; the criterion is not applied",
                            side_of(index)
                        ),
                        spec: Some(base.spec.canonical()),
                    });
                }
            }
        }
        out
    }

    /// The key under which flatten folder paths of the two sides are matched (the alignment's case and
    /// normalisation options applied to the whole path).
    fn path_key<'t>(&self, text: &'t str) -> Cow<'t, str> {
        path_key(&self.align, text)
    }

    // ------------------------------------------------------------------------------------------------------
    // Execution
    // ------------------------------------------------------------------------------------------------------

    fn execute(&self, initial: Vec<Task>) {
        match self.env.pool {
            Some(pool) => pool.scope(|scope| {
                for task in initial {
                    spawn_task(scope, self, task);
                }
            }),
            None => {
                let mut stack: Vec<Task> = initial.into_iter().rev().collect();
                while let Some(task) = stack.pop() {
                    let mut spawned = Vec::new();
                    self.run_task(task, &mut |t| spawned.push(t));
                    stack.extend(spawned.into_iter().rev());
                }
            }
        }
    }

    fn run_task(&self, task: Task, spawn: &mut dyn FnMut(Task)) {
        match task {
            Task::Folder(t) => self.folder_task(t, spawn),
            Task::Flat(t) => self.flat_task(t, spawn),
            Task::Content(t) => self.content_task(t),
        }
    }

    /// True when the mode says the file `name` in the folder `parent` opens as a folder (D-151, step 2). The name
    /// masks use this: it does not change during a scan, so the prefilter and the items agree.
    fn archive_mode_opens(&self, parent: &RelPath, name: &str) -> bool {
        let mode = self.rules.handling.archives;
        if mode == ArchiveHandling::AsFiles
            || (mode == ArchiveHandling::OnceOpened && self.open_archives.is_empty())
            || self.env.vfs.archive_format(name).is_none()
        {
            return false;
        }
        mode == ArchiveHandling::Always || {
            let path = join_text(parent.as_str(), name);
            self.open_archives.contains(self.path_key(&path).as_ref())
        }
    }

    /// [`Scan::archive_mode_opens`] within the budget (step 5): past it no further archive opens, and one warning
    /// says so.
    fn opens_archive(&self, parent: &RelPath, name: &str) -> bool {
        if !self.archive_mode_opens(parent, name) {
            return false;
        }
        let budget = self.env.archive_entries_max;
        if self.archive_entries.load(Ordering::Relaxed) <= budget {
            return true;
        }
        if !self.archive_limit_warned.swap(true, Ordering::Relaxed) {
            self.model.mutate(|s| {
                s.warn(Warning {
                    code: WarningCode::ArchiveScanLimit,
                    message: format!(
                        "over {budget} entries were listed inside archives: \
                         later archives stay files"
                    ),
                    spec: None,
                });
            });
        }
        false
    }

    fn list_side(&self, sf: &SideFolder) -> Result<Listing, VfsError> {
        let rules = &self.resolved;
        let prefilter = |name: &str, kind: EntryKind| {
            // An archive that opens passes the name masks as a folder (D-45, D-151 step 3).
            let kind = if kind == EntryKind::File && self.archive_mode_opens(&sf.rel, name) {
                EntryKind::Dir
            } else {
                kind
            };
            rules.names_accept(&sf.rel, sf.verdict, name, kind)
        };
        let opts = ListOptions {
            follow: self.follow,
            prefilter: if rules.has_name_filter() {
                Some(&prefilter)
            } else {
                None
            },
        };
        let listing = self.env.vfs.list(&sf.spec, &opts, self.env.ctl.cancel);
        if matches!(sf.spec, Spec::ArchiveMember { .. })
            && let Ok(listing) = &listing
        {
            self.archive_entries
                .fetch_add(listing.entries.len() as u64, Ordering::Relaxed);
        }
        listing
    }

    fn list_pair(&self, sides: &[Option<SideFolder>; 2]) -> [Option<Result<Listing, VfsError>>; 2] {
        match (&sides[0], &sides[1], self.env.pool) {
            (Some(l), Some(r), Some(pool)) => {
                let (a, b) = pool.join(|| self.list_side(l), || self.list_side(r));
                [Some(a), Some(b)]
            }
            (l, r, _) => [
                l.as_ref().map(|s| self.list_side(s)),
                r.as_ref().map(|s| self.list_side(s)),
            ],
        }
    }

    fn folder_task(&self, task: FolderTask, spawn: &mut dyn FnMut(Task)) {
        if self.cancelled() {
            return;
        }
        let listings = self.list_pair(&task.sides);
        self.process_folder(task.node, task.depth, &task.sides, listings, spawn);
    }

    // ------------------------------------------------------------------------------------------------------
    // Listing → items
    // ------------------------------------------------------------------------------------------------------

    /// Turns a listing into items: stat'ed entries with their filter verdicts, name-only entries (filtered,
    /// D-45), and errors — attached to the entry of the same name when there is one (a broken or looping
    /// followed link is in both lists, §1.2.7 item 2), else as an entry without metadata, which the name masks
    /// filter by its name (its kind is unknown: the file masks). When cancelled meanwhile, `Err` carries what was
    /// built (for [`Scan::discard`]).
    fn items(
        &self,
        listing: Listing,
        parent: &RelPath,
        verdict: FolderVerdict,
        dir: Option<u32>,
    ) -> Result<Items, Items> {
        let rules = &self.resolved;
        let Listing {
            mut entries,
            name_only,
            errors,
        } = listing;
        let dir = dir.unwrap_or(NO);
        let mut facts = Vec::with_capacity(entries.len() + name_only.len() + errors.len());
        for (step, entry) in entries.iter_mut().enumerate() {
            if self.cancelled_at(step) {
                return Err(Items { entries, facts });
            }
            let mut f = Facts::new(dir);
            if entry.kind == EntryKind::File
                && self.env.vfs.archive_format(entry.name.as_str()).is_some()
            {
                f.archive = true;
                if self.opens_archive(parent, entry.name.as_str()) {
                    f.opened = true;
                    entry.kind = EntryKind::Dir;
                    entry.size = 0;
                }
            }
            let kind = entry.kind;
            let name = entry.name.as_str();
            let names_ok = rules.names_accept(parent, verdict, name, kind);
            let other = rules.other_filtered(kind, entry.size, entry.mtime, &entry.attrs);
            f.filtered = !names_ok || other;
            if kind == EntryKind::Dir {
                f.verdict = rules.folder_verdict(parent, name, verdict);
            }
            facts.push(f);
        }
        if !name_only.is_empty() || !errors.is_empty() {
            entries.reserve_exact(name_only.len() + errors.len());
        }
        for (step, declined) in name_only.into_iter().enumerate() {
            if self.cancelled_at(step) {
                return Err(Items { entries, facts });
            }
            let mut f = Facts::new(dir);
            let mut kind = declined.kind;
            if kind == EntryKind::File {
                let name = declined.name.as_str();
                f.archive = self.env.vfs.archive_format(name).is_some();
                // The prefilter judged it as a folder: so it is one, also for the invariants.
                if f.archive && self.archive_mode_opens(parent, name) {
                    f.opened = true;
                    kind = EntryKind::Dir;
                }
            }
            if kind == EntryKind::Dir {
                f.verdict = rules.folder_verdict(parent, declined.name.as_str(), verdict);
            }
            f.filtered = true;
            f.stat_skipped = true;
            entries.push(Entry::new(declined.name, kind));
            facts.push(f);
        }
        if !errors.is_empty() {
            let mut by_name: HashMap<FileName, ErrorReport> = HashMap::with_capacity(errors.len());
            for failure in errors {
                by_name.entry(failure.name).or_insert(failure.error);
            }
            let mut stopped = false;
            for (step, (entry, f)) in entries.iter().zip(facts.iter_mut()).enumerate() {
                if by_name.is_empty() {
                    break;
                }
                if self.cancelled_at(step) {
                    stopped = true;
                    break;
                }
                if let Some(error) = by_name.remove(&entry.name) {
                    f.looped = is_link_loop(&error);
                    f.error = Some(Box::new(error));
                }
            }
            if stopped {
                return Err(Items { entries, facts });
            }
            let mut rest: Vec<(FileName, ErrorReport)> = by_name.into_iter().collect();
            rest.sort_by(|a, b| exact_bytes(&a.0).cmp(&exact_bytes(&b.0)));
            for (name, error) in rest {
                let mut f = Facts::new(dir);
                f.no_meta = true;
                f.filtered = !rules.names_accept(parent, verdict, name.as_str(), EntryKind::Other);
                f.looped = is_link_loop(&error);
                f.error = Some(Box::new(error));
                entries.push(Entry::new(name, EntryKind::Other));
                facts.push(f);
            }
        }
        Ok(Items { entries, facts })
    }

    // ------------------------------------------------------------------------------------------------------
    // One folder pair
    // ------------------------------------------------------------------------------------------------------

    fn process_folder(
        &self,
        node: NodeId,
        depth: u32,
        sides: &[Option<SideFolder>; 2],
        mut listings: [Option<Result<Listing, VfsError>>; 2],
        spawn: &mut dyn FnMut(Task),
    ) {
        // A listing that returned after cancellation is dropped: the folder stays not scanned.
        if self.cancelled() {
            self.discard(listings);
            return;
        }
        let mut items: [Items; 2] = [Items::default(), Items::default()];
        let mut failed = [false, false];
        let mut folder_flags = NodeFlags::empty();
        let mut folder_errors = Vec::new();
        let mut warnings = Vec::new();
        for index in 0..2 {
            let side = side_of(index);
            match (listings[index].take(), &sides[index]) {
                (Some(Ok(listing)), Some(sf)) => {
                    match self.items(listing, &sf.rel, sf.verdict, None) {
                        Ok(list) => items[index] = list,
                        Err(rest) => {
                            self.discard((rest, items, listings));
                            return;
                        }
                    }
                }
                (Some(Err(e)), _) if e.is_cancelled() => {
                    self.aborted.store(true, Ordering::Release);
                    self.discard((items, listings));
                    return;
                }
                (Some(Err(e)), sf) => {
                    let spec = sf.as_ref().map(|s| &s.spec);
                    let error = ErrorReport::capture(&e, spec);
                    warnings.push(Warning {
                        code: WarningCode::EntryError,
                        message: error.message.clone(),
                        spec: error.spec.clone(),
                    });
                    failed[index] = true;
                    folder_flags |= NodeFlags::error(side);
                    folder_errors.push(NodeError {
                        side: Some(side),
                        error,
                    });
                }
                (Some(Ok(_)), None) | (None, _) => {}
            }
        }
        let partial = failed.iter().any(|&f| f);
        let status = if partial {
            ScanStatus::Failed
        } else {
            ScanStatus::Scanned
        };
        let tasks = self.plan_and_insert(
            node,
            depth,
            sides,
            &mut items,
            partial,
            status,
            folder_flags,
            folder_errors,
            warnings,
        );
        self.discard(items);
        let Some(tasks) = tasks else {
            self.aborted.store(true, Ordering::Release);
            return;
        };
        self.folders_done.fetch_add(1, Ordering::Relaxed);
        self.env.ctl.progress(Phase::Scanning, || Progress {
            phase: Phase::Scanning,
            done: self.folders_done.load(Ordering::Relaxed),
            total: None,
            unit: Unit::Folders,
            current: sides
                .iter()
                .flatten()
                .next()
                .map(|s| s.rel.as_str().to_owned()),
        });
        self.emit(ScanEvent::FolderListed { folder: node });
        for task in tasks {
            spawn(task);
        }
    }

    /// Aligns, plans and inserts one folder batch; `None` when cancelled before the insertion (nothing of the
    /// batch is inserted then).
    #[allow(clippy::too_many_arguments)]
    fn plan_and_insert(
        &self,
        node: NodeId,
        depth: u32,
        sides: &[Option<SideFolder>; 2],
        items: &mut [Items; 2],
        partial: bool,
        status: ScanStatus,
        folder_flags: NodeFlags,
        folder_errors: Vec<NodeError>,
        mut warnings: Vec<Warning>,
    ) -> Option<Vec<Task>> {
        let pairs = self
            .pair_items(items, false, &[Vec::new(), Vec::new()])
            .ok()?;
        let spec_of = |side: usize, entry: &Entry, _dir: Option<u32>| -> Result<Spec, VfsError> {
            match &sides[side] {
                Some(sf) => self.env.vfs.child(&sf.spec, &entry.name),
                None => Err(VfsError::InvalidName {
                    name: entry.name.as_str().to_owned(),
                }),
            }
        };
        let planned = self
            .plan(
                items,
                pairs,
                depth,
                partial,
                Some(sides),
                &spec_of,
                &mut warnings,
            )
            .ok()?;
        self.insert(
            node,
            status,
            folder_flags,
            folder_errors,
            items,
            planned,
            warnings,
            depth,
            [&[], &[]],
            None,
        )
    }

    /// Frees a large batch (a listing, its items) on the pool instead of the calling thread: dropping a million
    /// listed entries — one name allocation each — takes tens of milliseconds, which must not delay the return
    /// after a cancellation (§5.3). Small batches, and every batch without a pool, are dropped here.
    fn discard<T: Bulk + Send + 'static>(&self, value: T) {
        match self.env.pool {
            Some(pool) if value.bulk() >= DISCARD_ON_POOL => pool.spawn(move || drop(value)),
            _ => drop(value),
        }
    }

    /// Aligns the items of both sides (§2.5) and pairs the entries without metadata by name. Flatten entries below
    /// a folder that failed on the other side are not aligned: each is a one-sided pair (state Unknown).
    fn pair_items(
        &self,
        items: &[Items; 2],
        flat: bool,
        dirs: &[Vec<DirInfo>; 2],
    ) -> Result<Vec<[u32; 2]>, Cancelled> {
        let dir_text = |side: usize, f: &Facts| -> &str {
            f.dir()
                .and_then(|d| dirs[side].get(d as usize))
                .map_or("", |d| d.text.as_str())
        };
        let mut alignable: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        let mut inputs: [Vec<AlignInput<'_>>; 2] = [Vec::new(), Vec::new()];
        for side in 0..2 {
            let list = &items[side];
            alignable[side].reserve(list.len());
            inputs[side].reserve(list.len());
            for (step, (entry, f)) in list.entries.iter().zip(&list.facts).enumerate() {
                if self.cancelled_at(step) {
                    return Err(Cancelled);
                }
                if f.no_meta || f.failed_folder || f.partial {
                    continue;
                }
                alignable[side].push(Items::index(step));
                inputs[side].push(AlignInput {
                    name: &entry.name,
                    kind: entry.kind,
                    dir: if flat { dir_text(side, f) } else { "" },
                });
            }
        }
        let [left, right] = inputs;
        let aligned = align_pairs(&left, &right, &self.align, flat, &|| self.cancelled())?;
        drop((left, right));
        let map = |side: usize, i: u32| -> u32 {
            if i == NO {
                NO
            } else {
                alignable[side].get(i as usize).copied().unwrap_or(NO)
            }
        };
        let mut pairs: Vec<[u32; 2]> = aligned
            .into_iter()
            .map(|[l, r]| [map(0, l), map(1, r)])
            .collect();
        drop(alignable);
        self.pair_special(items, &mut pairs, dirs)?;
        for (side, list) in items.iter().enumerate() {
            for (i, f) in list.facts.iter().enumerate() {
                if f.partial {
                    let mut pair = [NO, NO];
                    pair[side] = Items::index(i);
                    pairs.push(pair);
                }
            }
        }
        Ok(pairs)
    }

    /// Pairs entries without metadata (and, in flatten mode, failed folders) by name: first the same folder and
    /// exact name, then the exact name, then the case-folded exact name when case-insensitive. An entry without
    /// metadata pairs with another one or with an orphan of the other side; two failed folders pair with each
    /// other. Candidates are looked up in hash maps (O(n) per level) and taken in a fixed order — specials by
    /// `(exact name, folder)`, orphans by the same key — so the result does not depend on the listing order.
    fn pair_special(
        &self,
        items: &[Items; 2],
        pairs: &mut Vec<[u32; 2]>,
        dirs: &[Vec<DirInfo>; 2],
    ) -> Result<(), Cancelled> {
        let dir_of = |side: usize, i: u32| -> &str {
            items[side]
                .get(i)
                .and_then(|(_, f)| f.dir())
                .and_then(|d| dirs[side].get(d as usize))
                .map_or("", |d| d.text.as_str())
        };
        let name_of = |side: usize, i: u32| items[side].get(i).map(|(e, _)| &e.name);
        let failed_of =
            |side: usize, i: u32| items[side].get(i).is_some_and(|(_, f)| f.failed_folder);
        let order = |side: usize, a: u32, b: u32| -> CmpOrdering {
            let exact = |i: u32| name_of(side, i).map(exact_bytes);
            exact(a)
                .cmp(&exact(b))
                .then_with(|| dir_of(side, a).cmp(dir_of(side, b)))
        };
        let mut pending: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        for (side, list) in pending.iter_mut().enumerate() {
            for (i, f) in items[side].facts.iter().enumerate() {
                if (f.no_meta || f.failed_folder) && !f.partial {
                    list.push(Items::index(i));
                }
            }
            // A total order (flatten: one name can occur in several folders), independent of listing order.
            list.sort_by(|&a, &b| order(side, a, b));
        }
        if pending[0].is_empty() && pending[1].is_empty() {
            return Ok(());
        }
        let insensitive = self.align.case == hikasa_core::CaseSensitivity::Insensitive;
        type Key<'k> = (Option<&'k str>, Cow<'k, [u8]>);
        let key = |level: u8, side: usize, i: u32| -> Option<Key<'_>> {
            let name = name_of(side, i)?;
            match level {
                0 => Some((Some(dir_of(side, i)), exact_bytes(name))),
                1 => Some((None, exact_bytes(name))),
                _ => Some((None, folded_exact(name))),
            }
        };
        let levels: &[u8] = if insensitive { &[0, 1, 2] } else { &[0, 1] };
        for &level in levels {
            for side in 0..2usize {
                let other = 1 - side;
                if pending[side].is_empty() {
                    continue;
                }
                // The other side's specials by (key, failed folder), in their order.
                let mut partners: HashMap<(Key<'_>, bool), VecDeque<usize>> = HashMap::new();
                for (pos, &j) in pending[other].iter().enumerate() {
                    if self.cancelled_at(pos) {
                        return Err(Cancelled);
                    }
                    if let Some(k) = key(level, other, j) {
                        partners
                            .entry((k, failed_of(other, j)))
                            .or_default()
                            .push_back(pos);
                    }
                }
                // The other side's orphans by key, each list in (exact name, folder) order.
                let mut orphans: HashMap<Key<'_>, Vec<usize>> = HashMap::new();
                for (step, p) in pairs.iter().enumerate() {
                    if self.cancelled_at(step) {
                        return Err(Cancelled);
                    }
                    if p[side] == NO
                        && p[other] != NO
                        && let Some(k) = key(level, other, p[other])
                    {
                        orphans.entry(k).or_default().push(step);
                    }
                }
                let mut orphans: HashMap<Key<'_>, VecDeque<usize>> = orphans
                    .into_iter()
                    .map(|(k, mut list)| {
                        list.sort_by(|&a, &b| {
                            let at = |p: usize| pairs.get(p).map_or(NO, |q| q[other]);
                            order(other, at(a), at(b))
                        });
                        (k, VecDeque::from(list))
                    })
                    .collect();
                let mut taken = vec![false; pending[other].len()];
                let mut still = Vec::new();
                for (step, &i) in pending[side].iter().enumerate() {
                    if self.cancelled_at(step) {
                        return Err(Cancelled);
                    }
                    let Some(k) = key(level, side, i) else {
                        still.push(i);
                        continue;
                    };
                    let failed_folder = failed_of(side, i);
                    if let Some(pos) = partners
                        .get_mut(&(k.clone(), failed_folder))
                        .and_then(VecDeque::pop_front)
                    {
                        if let Some(flag) = taken.get_mut(pos) {
                            *flag = true;
                        }
                        let mut pair = [NO, NO];
                        pair[side] = i;
                        pair[other] = pending[other].get(pos).copied().unwrap_or(NO);
                        pairs.push(pair);
                        continue;
                    }
                    // An orphan of the other side (entries without metadata only).
                    if !failed_folder
                        && let Some(p) = orphans.get_mut(&k).and_then(VecDeque::pop_front)
                        && let Some(pair) = pairs.get_mut(p)
                    {
                        pair[side] = i;
                        continue;
                    }
                    still.push(i);
                }
                let mut keep = taken.iter();
                pending[other].retain(|_| !keep.next().copied().unwrap_or(false));
                pending[side] = still;
            }
        }
        for (side, list) in pending.iter().enumerate() {
            for &i in list {
                let mut pair = [NO, NO];
                pair[side] = i;
                pairs.push(pair);
            }
        }
        Ok(())
    }

    /// Plans every pair: re-stats the filtered side of FC-054 pairs, runs the quick tests, decides states,
    /// content tests and which folders to list next. `Err(Cancelled)` when cancelled meanwhile (checked every
    /// `CHECK_EVERY` pairs and before every re-stat).
    #[allow(clippy::too_many_arguments)]
    fn plan(
        &self,
        items: &mut [Items; 2],
        pairs: Vec<[u32; 2]>,
        depth: u32,
        partial: bool,
        parents: Option<&[Option<SideFolder>; 2]>,
        spec_of: &SpecOf<'_>,
        warnings: &mut Vec<Warning>,
    ) -> Result<Vec<Planned>, Cancelled> {
        let method = self.rules.criteria.content.method;
        let mut planned = Vec::with_capacity(pairs.len());
        for (step, pair) in pairs.into_iter().enumerate() {
            if self.cancelled_at(step) {
                return Err(Cancelled);
            }
            // FC-054: stat the filtered, name-only side when its counterpart is visible (D-45).
            let [l, r] = pair;
            if l != NO && r != NO {
                for (side, i, j) in [(0usize, l, r), (1usize, r, l)] {
                    let other = 1 - side;
                    let needs = items[side]
                        .get(i)
                        .is_some_and(|(_, f)| f.filtered && f.stat_skipped)
                        && items[other].get(j).is_some_and(|(_, f)| !f.filtered);
                    if needs {
                        if self.cancelled() {
                            return Err(Cancelled);
                        }
                        self.restat(items, side, i, spec_of)?;
                    }
                }
            }
            planned.push(self.plan_pair(
                items, pair, depth, partial, parents, spec_of, warnings, method,
            ));
        }
        Ok(planned)
    }

    /// Stats a name-only item (the filtered side of an FC-054 pair). An entry whose kind changed since it was
    /// listed gets an error (kinds never mix in a node, D-20).
    fn restat(
        &self,
        items: &mut [Items; 2],
        side: usize,
        i: u32,
        spec_of: &SpecOf<'_>,
    ) -> Result<(), Cancelled> {
        let Some((entry, f)) = items[side].get(i) else {
            return Ok(());
        };
        let (result, spec) = match spec_of(side, entry, f.dir()) {
            Ok(spec) => (self.env.vfs.stat(&spec, self.follow), Some(spec)),
            Err(e) => (Err(e), None),
        };
        let index = i as usize;
        let (Some(entry), Some(f)) = (
            items[side].entries.get_mut(index),
            items[side].facts.get_mut(index),
        ) else {
            return Ok(());
        };
        match result {
            Ok(mut stated) if stated.kind == entry.kind => {
                stated.name = entry.name.clone();
                *entry = stated;
                f.stat_skipped = false;
            }
            Ok(_) => {
                f.error = Some(Box::new(report(
                    ErrorKind::Io,
                    format!("{}: changed kind during the scan", entry.name.as_str()),
                    spec.as_ref(),
                )));
            }
            Err(e) if e.is_cancelled() => return Err(Cancelled),
            Err(e) => {
                f.error = Some(Box::new(ErrorReport::capture(&e, None)));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_pair(
        &self,
        items: &[Items; 2],
        pair: [u32; 2],
        depth: u32,
        partial: bool,
        parents: Option<&[Option<SideFolder>; 2]>,
        spec_of: &SpecOf<'_>,
        warnings: &mut Vec<Warning>,
        method: ContentMethod,
    ) -> Planned {
        let sides = [items[0].get(pair[0]), items[1].get(pair[1])];
        let kind = sides.iter().flatten().find(|(_, f)| !f.no_meta).map_or(
            NodeKind::Other,
            |(entry, f)| {
                if f.failed_folder {
                    NodeKind::Folder
                } else {
                    NodeKind::of(entry.kind)
                }
            },
        );
        let mut flags = NodeFlags::empty();
        let mut errors = Vec::new();
        for (index, item) in sides.iter().enumerate() {
            let Some((_, f)) = item else {
                continue;
            };
            let side = side_of(index);
            if f.filtered {
                flags |= NodeFlags::filtered(side);
            }
            if f.stat_skipped {
                flags |= NodeFlags::stat_skipped(side);
            }
            if f.archive {
                flags |= NodeFlags::ARCHIVE;
            }
            if f.opened {
                flags |= NodeFlags::archive_root(side);
            }
            if let Some(error) = &f.error {
                flags |= NodeFlags::error(side);
                if f.looped {
                    flags |= NodeFlags::SYMLINK_LOOP;
                }
                warnings.push(Warning {
                    code: if f.looped {
                        WarningCode::SymlinkLoop
                    } else {
                        WarningCode::EntryError
                    },
                    message: error.message.clone(),
                    spec: error.spec.clone(),
                });
                errors.push(NodeError {
                    side: Some(side),
                    error: ErrorReport::clone(error),
                });
            }
        }
        let present = sides.iter().filter(|s| s.is_some()).count();
        let all_filtered = sides.iter().flatten().all(|(_, f)| f.filtered);
        let has_error = sides.iter().flatten().any(|(_, f)| f.error.is_some());
        let partial = partial || sides.iter().flatten().any(|(_, f)| f.partial);
        let mut plan = Planned {
            sides: pair,
            kind,
            state: CompareState::Unknown,
            scan: ScanStatus::NotApplicable,
            diffs: DiffFlags::empty(),
            time: None,
            flags,
            internal: if partial {
                internal::PARTIAL_UNKNOWN
            } else {
                0
            },
            errors,
            content: None,
            descend: None,
        };
        if kind == NodeKind::Folder {
            // Folders are compared through their contents: no quick tests (docs/04 §3, §1.6.8).
            plan.scan = if has_error {
                ScanStatus::Failed
            } else {
                ScanStatus::NotScanned
            };
            if !has_error && !partial && !all_filtered && parents.is_some() {
                self.plan_descend(
                    &mut plan, &sides, depth, parents, spec_of, present, warnings,
                );
            }
            return plan;
        }
        // Two stat'ed entries (of the same kind: aligned entries never mix kinds).
        let stated = match sides {
            [Some((l, lf)), Some((r, rf))]
                if !lf.stat_skipped && !lf.no_meta && !rf.stat_skipped && !rf.no_meta =>
            {
                Some([(l, lf), (r, rf)])
            }
            _ => None,
        };
        plan.state = if all_filtered {
            // §2.3: a node whose present sides are all filtered is Filtered, also when an error is attached
            // (the error stays on the node: flag, `errors`, warning). Its quick tests run when both sides were
            // stat'ed; no content test runs.
            if let Some([(l, _), (r, _)]) = stated {
                let (diffs, time) = quick_tests(l, r, &self.criteria);
                plan.diffs = diffs;
                plan.time = time;
            }
            CompareState::Filtered
        } else if has_error {
            CompareState::Error
        } else if partial {
            CompareState::Unknown
        } else if present == 1 {
            if sides[0].is_some() {
                CompareState::LeftOrphan
            } else {
                CompareState::RightOrphan
            }
        } else if let Some([(l, lf), (r, rf)]) = stated {
            for (index, entry) in [l, r].into_iter().enumerate() {
                self.note_missing_mtime(index, entry, warnings);
            }
            let (diffs, time) = quick_tests(l, r, &self.criteria);
            plan.diffs = diffs;
            plan.time = time;
            let content_requested = method != ContentMethod::None
                && kind == NodeKind::File
                && !lf.filtered
                && !rf.filtered;
            if content_requested
                && self.rules.criteria.content.skip_if_quick_same
                && diffs.is_empty()
            {
                plan.flags |= NodeFlags::CONTENT_SKIPPED;
                base_state(diffs, time)
            } else if content_requested {
                match (spec_of(0, l, lf.dir()), spec_of(1, r, rf.dir())) {
                    (Ok(ls), Ok(rs)) => {
                        plan.content = Some(Box::new([ls, rs]));
                        plan.internal |= internal::CONTENT_PENDING;
                        CompareState::Unknown
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        plan.errors.push(NodeError {
                            side: None,
                            error: ErrorReport::capture(&e, None),
                        });
                        CompareState::Error
                    }
                }
            } else {
                base_state(diffs, time)
            }
        } else {
            CompareState::Unknown
        };
        plan
    }

    /// One `capability_missing` warning per side when a stat'ed entry has no modification time although the
    /// back-end reports the capability (§2.4).
    fn note_missing_mtime(&self, index: usize, entry: &Entry, warnings: &mut Vec<Warning>) {
        if !self.criteria.timestamps.enabled
            || entry.mtime.is_some()
            || !self.caps[index].contains(Capabilities::MTIME)
        {
            return;
        }
        if let Some(flag) = self.mtime_warned.get(index)
            && !flag.swap(true, Ordering::AcqRel)
        {
            warnings.push(Warning {
                code: WarningCode::CapabilityMissing,
                message: format!(
                    "{}: some entries have no modification time; timestamps are not compared for them",
                    side_of(index)
                ),
                spec: None,
            });
        }
    }

    /// Decides which sides of a folder pair are listed next (§2.3), with loop and depth checks (D-33). A side is
    /// listed unless the **name masks** exclude it: a side hidden by the other filters only (exclude protected) is
    /// listed when the pair is not filtered on every side (FC-054), so its contents are compared, not reported as
    /// orphans.
    #[allow(clippy::too_many_arguments)]
    fn plan_descend(
        &self,
        plan: &mut Planned,
        sides: &[Option<(&Entry, &Facts)>; 2],
        depth: u32,
        parents: Option<&[Option<SideFolder>; 2]>,
        spec_of: &SpecOf<'_>,
        present: usize,
        warnings: &mut Vec<Warning>,
    ) {
        let Some(parents) = parents else {
            return;
        };
        if present == 1 && !self.rules.handling.scan_orphan_folders {
            return;
        }
        let following = self.follow == FollowLinks::Yes;
        let mut descend: [Option<SideFolder>; 2] = [None, None];
        for (index, item) in sides.iter().enumerate() {
            let (Some((entry, f)), Some(parent)) = (item, &parents[index]) else {
                continue;
            };
            let side = side_of(index);
            if f.no_meta || f.verdict == FolderVerdict::Excluded {
                continue;
            }
            let spec = match spec_of(index, entry, f.dir()) {
                Ok(spec) if f.opened => Spec::archive_member(spec, ""),
                Ok(spec) => spec,
                Err(e) => {
                    plan.flags |= NodeFlags::error(side);
                    plan.errors.push(NodeError {
                        side: Some(side),
                        error: ErrorReport::capture(&e, None),
                    });
                    plan.scan = ScanStatus::Failed;
                    continue;
                }
            };
            let mut chain = Vec::new();
            if following {
                if let Some(id) = entry.file_id {
                    if parent.chain.contains(&id) {
                        plan.flags |= NodeFlags::SYMLINK_LOOP | NodeFlags::error(side);
                        let error = report(
                            ErrorKind::Io,
                            format!(
                                "{}: symbolic link loop (leads back to an ancestor folder)",
                                spec.canonical()
                            ),
                            Some(&spec),
                        );
                        warnings.push(Warning {
                            code: WarningCode::SymlinkLoop,
                            message: error.message.clone(),
                            spec: error.spec.clone(),
                        });
                        plan.errors.push(NodeError {
                            side: Some(side),
                            error,
                        });
                        plan.scan = ScanStatus::Failed;
                        continue;
                    }
                    chain.reserve(parent.chain.len() + 1);
                    chain.extend_from_slice(&parent.chain);
                    chain.push(id);
                }
                if depth + 1 > MAX_FOLLOW_DEPTH {
                    plan.flags |= NodeFlags::error(side);
                    let error = report(
                        ErrorKind::TooLarge,
                        format!(
                            "{}: folder depth limit of {MAX_FOLLOW_DEPTH} exceeded",
                            spec.canonical()
                        ),
                        Some(&spec),
                    );
                    warnings.push(Warning {
                        code: WarningCode::EntryError,
                        message: error.message.clone(),
                        spec: error.spec.clone(),
                    });
                    plan.errors.push(NodeError {
                        side: Some(side),
                        error,
                    });
                    plan.scan = ScanStatus::Failed;
                    continue;
                }
            }
            if let Some(slot) = descend.get_mut(index) {
                *slot = Some(SideFolder {
                    rel: parent
                        .rel
                        .join(entry.name.as_str())
                        .unwrap_or_else(|_| parent.rel.clone()),
                    spec,
                    verdict: f.verdict,
                    chain,
                });
            }
        }
        if plan.scan != ScanStatus::Failed && descend.iter().any(Option::is_some) {
            plan.descend = Some(Box::new(descend));
        }
    }

    /// Inserts the planned children of `node` in canonical order (§2.5.4) and returns the tasks to run next;
    /// `None` when cancelled before the write lock was taken (nothing is inserted then). `dirs` is installed as the
    /// flatten folder table in the same mutation (before any child can be read).
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &self,
        node: NodeId,
        status: ScanStatus,
        folder_flags: NodeFlags,
        folder_errors: Vec<NodeError>,
        items: &mut [Items; 2],
        mut planned: Vec<Planned>,
        mut warnings: Vec<Warning>,
        depth: u32,
        dirs: [&[DirInfo]; 2],
        table: Option<[Vec<DirInfo>; 2]>,
    ) -> Option<Vec<Task>> {
        let order = self.canonical_order(items, &planned, dirs)?;
        // Detach the task parts (rare; content tests take copies of both entries) before the planned records are
        // consumed by the insertion.
        let mut detached: Vec<Detached> = Vec::new();
        for (offset, &index) in order.iter().enumerate() {
            if let Some(p) = planned.get_mut(index as usize)
                && (p.content.is_some() || p.descend.is_some())
            {
                let entry = |side: usize| items[side].get(p.sides[side]).map(|(e, _)| e.clone());
                let content = match (p.content.take(), entry(0), entry(1)) {
                    (Some(specs), Some(le), Some(re)) => {
                        let [ls, rs] = *specs;
                        Some(Box::new([(ls, le), (rs, re)]))
                    }
                    _ => None,
                };
                detached.push(Detached {
                    offset: Items::index(offset),
                    content,
                    descend: p.descend.take(),
                });
            }
        }
        let children = order.iter().filter_map(|&index| {
            let p = planned.get_mut(index as usize)?;
            Some(NewNode {
                kind: p.kind,
                state: p.state,
                scan: p.scan,
                diffs: p.diffs,
                time: p.time,
                flags: p.flags,
                internal: p.internal,
                sides: [0usize, 1].map(|side| {
                    items[side].get(p.sides[side]).map(|(entry, f)| NewSide {
                        entry,
                        filtered: f.filtered,
                        stat_skipped: f.stat_skipped,
                        no_meta: f.no_meta,
                        dir: f.dir(),
                    })
                }),
                errors: std::mem::take(&mut p.errors),
            })
        });
        // Built outside the write lock (with cancellation checks); the lock only appends the prepared batch.
        let staged = stage(
            node.0,
            self.rules.criteria.content.override_quick,
            order.len(),
            children,
            &|| self.cancelled(),
        );
        // The listing and the plan are no longer needed: free them before the node chunks are prepared, so a huge
        // folder never holds its listing, its staged nodes and their destination at once.
        drop((order, planned));
        self.discard(std::mem::take(items));
        let batch = staged?.with_spares(&|| self.cancelled())?;
        if self.cancelled() {
            return None;
        }
        let (first, unused) = self.model.mutate(|s| {
            if let Some(table) = table {
                s.dirs = table;
            }
            for w in warnings.drain(..) {
                s.warn(w);
            }
            s.insert_staged(node, status, folder_flags, folder_errors, batch)
        });
        drop(unused);
        let mut tasks = Vec::with_capacity(detached.len());
        for Detached {
            offset,
            content,
            descend,
        } in detached
        {
            let id = NodeId(first.saturating_add(offset));
            if let Some(sides) = content {
                self.content_total.fetch_add(1, Ordering::Relaxed);
                self.in_flight.fetch_add(1, Ordering::AcqRel);
                tasks.push(Task::Content(ContentTask { node: id, sides }));
            }
            if let Some(sides) = descend {
                tasks.push(Task::Folder(FolderTask {
                    node: id,
                    depth: depth + 1,
                    sides: *sides,
                }));
            }
        }
        Some(tasks)
    }

    /// The planned children's indices in canonical order (§2.5.4, the order of `ChildKey`): kind rank, then
    /// `fold(nfc(representative name))` (computed once per child, borrowed when possible), then the exact names and
    /// (flatten) the folders. `None` when cancelled.
    fn canonical_order(
        &self,
        items: &[Items; 2],
        planned: &[Planned],
        dirs: [&[DirInfo]; 2],
    ) -> Option<Vec<u32>> {
        let dir_text = |side: usize, f: &Facts| -> &str {
            f.dir()
                .and_then(|d| dirs[side].get(d as usize))
                .map_or("", |d| d.text.as_str())
        };
        let repr_side = |p: &Planned| if p.sides[0] != NO { 0 } else { 1 };
        let mut keyed: Vec<(u8, Cow<'_, str>, u32)> = Vec::with_capacity(planned.len());
        for (step, p) in planned.iter().enumerate() {
            if self.cancelled_at(step) {
                return None;
            }
            let side = repr_side(p);
            let folded = items[side]
                .get(p.sides[side])
                .map_or(Cow::Borrowed(""), |(e, _)| canonical_fold(e.name.as_str()));
            keyed.push((kind_rank(p.kind.entry_kind()), folded, Items::index(step)));
        }
        let rest = |index: u32| {
            let p = planned.get(index as usize)?;
            let side = repr_side(p);
            let (repr, rf) = items[side].get(p.sides[side])?;
            let right = items[1].get(p.sides[1]);
            Some((
                exact_bytes(&repr.name),
                right.map(|(e, _)| exact_bytes(&e.name)),
                dir_text(side, rf),
                right.map(|(_, f)| dir_text(1, f)),
            ))
        };
        keyed.sort_by(|a, b| {
            (a.0, &a.1)
                .cmp(&(b.0, &b.1))
                .then_with(|| rest(a.2).cmp(&rest(b.2)))
        });
        if self.cancelled() {
            return None;
        }
        Some(keyed.into_iter().map(|(_, _, index)| index).collect())
    }

    // ------------------------------------------------------------------------------------------------------
    // Content tests
    // ------------------------------------------------------------------------------------------------------

    fn content_task(&self, task: ContentTask) {
        let _in_flight = InFlight(self);
        if self.cancelled() {
            return;
        }
        let method = self.rules.criteria.content.method;
        let Some(comparator) = self.env.content.get(method) else {
            return;
        };
        let [(ls, le), (rs, re)] = *task.sides;
        let bytes = le.size.saturating_add(re.size).min(self.budget.capacity());
        let Ok(guard) = self.budget.acquire(bytes, self.env.ctl.cancel) else {
            self.aborted.store(true, Ordering::Release);
            return;
        };
        let cancel = self.env.ctl.cancel;
        let left = VfsSource::new(self.env.vfs, ls, le).with_cancel(cancel);
        let right = VfsSource::new(self.env.vfs, rs, re).with_cancel(cancel);
        let outcome = comparator.compare(&left, &right, &self.env.ctl);
        drop(guard);
        let Ok(outcome) = outcome else {
            self.aborted.store(true, Ordering::Release);
            return;
        };
        if let hikasa_core::ContentResult::Error { error } = &outcome.result
            && error.kind == ErrorKind::Cancelled
        {
            self.aborted.store(true, Ordering::Release);
            return;
        }
        let warnings: Vec<Warning> = outcome.warnings.clone();
        self.model.mutate(|s| {
            for w in warnings {
                s.warn(w);
            }
            s.apply_content(task.node, outcome);
        });
        let done = self.content_done.fetch_add(1, Ordering::Relaxed) + 1;
        self.env.ctl.progress(Phase::ComparingContent, || Progress {
            phase: Phase::ComparingContent,
            done,
            total: Some(self.content_total.load(Ordering::Relaxed)),
            unit: Unit::Items,
            current: Some(left.display()),
        });
        self.note_updated(task.node);
    }

    // ------------------------------------------------------------------------------------------------------
    // Flatten (FC-014)
    // ------------------------------------------------------------------------------------------------------

    fn run_flatten(&self, roots: [Option<SideFolder>; 2], listings: [Option<Listing>; 2]) {
        let mut initial = Vec::new();
        for (index, (root, listing)) in roots.into_iter().zip(listings).enumerate() {
            let (Some(root), Some(listing)) = (root, listing) else {
                continue;
            };
            let dir = self.add_flat_dir(index, String::new(), Some(root.spec.clone()));
            self.collect_flat(index, &root, dir, 0, listing, &mut |t| initial.push(t));
        }
        self.execute(initial);
        // Protected folders whose counterpart was collected on the other side are collected next (rounds, since
        // a newly collected folder can be the counterpart of another deferred one).
        while !self.cancelled() && !self.aborted.load(Ordering::Acquire) {
            let tasks = self.activate_deferred();
            if tasks.is_empty() {
                break;
            }
            self.execute(tasks);
        }
        let mut items: [Items; 2] = [
            std::mem::take(&mut *self.flat_items[0].lock()),
            std::mem::take(&mut *self.flat_items[1].lock()),
        ];
        if self.cancelled() || self.aborted.load(Ordering::Acquire) {
            // Nothing is inserted: a partial collection would show missing files as orphans.
            self.discard(items);
            return;
        }
        let dirs: [Vec<DirInfo>; 2] = [
            std::mem::take(&mut *self.flat_dirs[0].lock()),
            std::mem::take(&mut *self.flat_dirs[1].lock()),
        ];
        let tasks = self.flatten_insert(&mut items, &dirs);
        self.discard(items);
        let Some(tasks) = tasks else {
            self.aborted.store(true, Ordering::Release);
            return;
        };
        self.folders_done.fetch_add(1, Ordering::Relaxed);
        self.emit(ScanEvent::FolderListed {
            folder: NodeId::ROOT,
        });
        self.execute(tasks);
    }

    /// Flatten: aligns, plans and inserts the collected files of both sides as the root's children; `None` when
    /// cancelled before the insertion.
    fn flatten_insert(
        &self,
        items: &mut [Items; 2],
        dirs: &[Vec<DirInfo>; 2],
    ) -> Option<Vec<Task>> {
        self.mark_below_failed(items, dirs);
        let pairs = self.pair_items(items, true, dirs).ok()?;
        let spec_of = |side: usize, entry: &Entry, dir: Option<u32>| -> Result<Spec, VfsError> {
            let parent = dir
                .and_then(|d| dirs[side].get(d as usize))
                .and_then(|d| d.spec.as_ref());
            match parent {
                Some(spec) => self.env.vfs.child(spec, &entry.name),
                None => Err(VfsError::InvalidName {
                    name: entry.name.as_str().to_owned(),
                }),
            }
        };
        let mut warnings = Vec::new();
        let planned = self
            .plan(items, pairs, 0, false, None, &spec_of, &mut warnings)
            .ok()?;
        // The folder table is installed in the same mutation as the children: no reader can see a child before
        // its folder exists.
        let table = dirs.clone();
        self.insert(
            NodeId::ROOT,
            ScanStatus::Scanned,
            NodeFlags::empty(),
            Vec::new(),
            items,
            planned,
            warnings,
            0,
            [&dirs[0], &dirs[1]],
            Some(table),
        )
    }

    /// Flatten (§2.6.2 in flatten mode): the entries of one side that lie at or below the path of a folder that
    /// failed (listing, loop or depth limit) on the other side are marked `partial` — Unknown rows that are not
    /// aligned, since what the failing side holds there is unknown. Paths are matched with the alignment's case
    /// and normalisation options.
    fn mark_below_failed(&self, items: &mut [Items; 2], dirs: &[Vec<DirInfo>; 2]) {
        for side in 0..2 {
            let other = 1 - side;
            let failed: HashSet<String> = items[side]
                .entries
                .iter()
                .zip(&items[side].facts)
                .filter(|(_, f)| f.failed_folder)
                .map(|(entry, f)| {
                    let parent = f
                        .dir()
                        .and_then(|d| dirs[side].get(d as usize))
                        .map_or("", |d| d.text.as_str());
                    self.path_key(&join_text(parent, entry.name.as_str()))
                        .into_owned()
                })
                .collect();
            if failed.is_empty() {
                continue;
            }
            let below: Vec<bool> = dirs[other]
                .iter()
                .map(|d| {
                    let text = d.text.as_str();
                    let ends = text
                        .match_indices('/')
                        .map(|(at, _)| at)
                        .chain(std::iter::once(text.len()));
                    ends.filter(|&end| end > 0).any(|end| {
                        text.get(..end)
                            .is_some_and(|prefix| failed.contains(self.path_key(prefix).as_ref()))
                    })
                })
                .collect();
            for f in &mut items[other].facts {
                if !f.failed_folder
                    && let Some(d) = f.dir()
                    && below.get(d as usize).copied().unwrap_or(false)
                {
                    f.partial = true;
                }
            }
        }
    }

    /// Flatten: moves the deferred protected folders whose path was collected on the other side into tasks.
    fn activate_deferred(&self) -> Vec<Task> {
        let keys: [HashSet<String>; 2] = [0, 1].map(|side| {
            self.flat_dirs[side]
                .lock()
                .iter()
                .map(|d| self.path_key(&d.text).into_owned())
                .collect()
        });
        let mut tasks = Vec::new();
        for side in 0..2 {
            let other = 1 - side;
            let ready: Vec<Deferred> = {
                let mut deferred = self.flat_deferred[side].lock();
                let (ready, wait): (Vec<Deferred>, Vec<Deferred>) =
                    std::mem::take(&mut *deferred).into_iter().partition(|d| {
                        keys[other].contains(self.path_key(d.folder.rel.as_str()).as_ref())
                    });
                *deferred = wait;
                ready
            };
            for d in ready {
                let dir = self.add_flat_dir(
                    side,
                    d.folder.rel.as_str().to_owned(),
                    Some(d.folder.spec.clone()),
                );
                tasks.push(Task::Flat(FlatTask {
                    side,
                    folder: d.folder,
                    dir,
                    parent_dir: d.parent_dir,
                    depth: d.depth,
                    entry: d.entry,
                }));
            }
        }
        tasks
    }

    fn add_flat_dir(&self, side: usize, text: String, spec: Option<Spec>) -> u32 {
        let mut dirs = self.flat_dirs[side].lock();
        dirs.push(DirInfo { text, spec });
        u32::try_from(dirs.len() - 1).unwrap_or(u32::MAX)
    }

    fn flat_task(&self, task: FlatTask, spawn: &mut dyn FnMut(Task)) {
        if self.cancelled() {
            return;
        }
        match self.list_side(&task.folder) {
            Ok(listing) => {
                self.collect_flat(
                    task.side,
                    &task.folder,
                    task.dir,
                    task.depth,
                    listing,
                    spawn,
                );
            }
            Err(e) if e.is_cancelled() => self.aborted.store(true, Ordering::Release),
            Err(e) => {
                let mut f = Facts::new(task.parent_dir);
                f.failed_folder = true;
                f.error = Some(Box::new(ErrorReport::capture(&e, Some(&task.folder.spec))));
                // The folder's own filter verdict (a protected folder stays filtered on its side).
                f.verdict = task.folder.verdict;
                f.filtered = self.resolved.other_filtered(
                    task.entry.kind,
                    task.entry.size,
                    task.entry.mtime,
                    &task.entry.attrs,
                );
                self.flat_items[task.side].lock().push(task.entry, f);
            }
        }
    }

    /// Flatten: files of a listed folder go to the side's collection; folders the name masks keep are listed
    /// next (a folder hidden by the other filters only waits for its counterpart, [`Deferred`]); a folder that
    /// cannot be listed (error, loop, depth limit) is collected as a failed folder row.
    fn collect_flat(
        &self,
        side: usize,
        folder: &SideFolder,
        dir: u32,
        depth: u32,
        listing: Listing,
        spawn: &mut dyn FnMut(Task),
    ) {
        let items = match self.items(listing, &folder.rel, folder.verdict, Some(dir)) {
            Ok(items) => items,
            Err(rest) => {
                self.discard(rest);
                return;
            }
        };
        let following = self.follow == FollowLinks::Yes;
        let mut files = Items::default();
        files.entries.reserve(items.len());
        files.facts.reserve(items.len());
        let mut deferred = Vec::new();
        // No cancellation check here: moving the entries is cheap, and a cancelled collection is discarded whole.
        for (entry, mut f) in items.entries.into_iter().zip(items.facts) {
            if entry.kind != EntryKind::Dir || f.no_meta {
                files.push(entry, f);
                continue;
            }
            if f.verdict == FolderVerdict::Excluded {
                // Excluded by the name masks: not listed, not shown (folders do not appear in flatten mode).
                continue;
            }
            if f.error.is_some() {
                f.failed_folder = true;
                files.push(entry, f);
                continue;
            }
            let spec = match self.env.vfs.child(&folder.spec, &entry.name) {
                Ok(spec) if f.opened => Spec::archive_member(spec, ""),
                Ok(spec) => spec,
                Err(e) => {
                    f.failed_folder = true;
                    f.error = Some(Box::new(ErrorReport::capture(&e, None)));
                    files.push(entry, f);
                    continue;
                }
            };
            let mut chain = Vec::new();
            if following {
                if let Some(id) = entry.file_id {
                    if folder.chain.contains(&id) {
                        f.failed_folder = true;
                        f.looped = true;
                        f.error = Some(Box::new(report(
                            ErrorKind::Io,
                            format!(
                                "{}: symbolic link loop (leads back to an ancestor folder)",
                                spec.canonical()
                            ),
                            Some(&spec),
                        )));
                        files.push(entry, f);
                        continue;
                    }
                    chain.extend_from_slice(&folder.chain);
                    chain.push(id);
                }
                if depth + 1 > MAX_FOLLOW_DEPTH {
                    f.failed_folder = true;
                    f.error = Some(Box::new(report(
                        ErrorKind::TooLarge,
                        format!(
                            "{}: folder depth limit of {MAX_FOLLOW_DEPTH} exceeded",
                            spec.canonical()
                        ),
                        Some(&spec),
                    )));
                    files.push(entry, f);
                    continue;
                }
            }
            let rel = folder
                .rel
                .join(entry.name.as_str())
                .unwrap_or_else(|_| folder.rel.clone());
            let child = SideFolder {
                spec,
                rel,
                verdict: f.verdict,
                chain,
            };
            if f.filtered {
                deferred.push(Deferred {
                    folder: child,
                    parent_dir: dir,
                    depth: depth + 1,
                    entry,
                });
                continue;
            }
            let child_dir = self.add_flat_dir(
                side,
                child.rel.as_str().to_owned(),
                Some(child.spec.clone()),
            );
            spawn(Task::Flat(FlatTask {
                side,
                folder: child,
                dir: child_dir,
                parent_dir: dir,
                depth: depth + 1,
                entry,
            }));
        }
        self.flat_items[side].lock().append(files);
        if !deferred.is_empty() {
            self.flat_deferred[side].lock().extend(deferred);
        }
        self.folders_done.fetch_add(1, Ordering::Relaxed);
        self.env.ctl.progress(Phase::Scanning, || Progress {
            phase: Phase::Scanning,
            done: self.folders_done.load(Ordering::Relaxed),
            total: None,
            unit: Unit::Folders,
            current: Some(folder.rel.as_str().to_owned()),
        });
    }
}
