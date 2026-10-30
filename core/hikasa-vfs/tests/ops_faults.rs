//! M2 W3 acceptance A3.4 (D-95, D-103): a property test over generated trees (at most 50 items) × the position
//! of a fault × the faulting operation (`Open`, `Read`, `Write`, `Commit`, `Mkdir`, `SetTimes`, `SetAttrs`,
//! `Symlink`, `Remove`, `Rename`, and read or write errors after N bytes), for copies (another `MemoryVfs`, the
//! same instance, with and without existing targets) and moves (another instance: expanded; the same instance:
//! one `rename`). The failing item's target is exactly as before (its `dump()` entry), every other item reaches
//! its expected state, the summary counts match and the report is not ok (exit code 100). A second property
//! cancels at a random file item: the running item's target is untouched, later items are `not_run`, and the
//! report is `cancelled`.
//!
//! 64 cases per property unless `PROPTEST_CASES` says otherwise.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::Fx;
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, Spec};
use hikasa_vfs::ops::{
    self, Action, CopyOptions, ItemStatus, MoveOptions, OpReport, OverwritePolicy, Plan, PlanItem,
    SkipReason,
};
use hikasa_vfs::{
    AtomicWrite, AttrChange, Attrs, Capabilities, Entry, EntryKind, Fault, FileName, FollowLinks,
    ListOptions, Listing, MemDump, MemMeta, MemoryVfs, ReadSeek, SetTimes, UnixMeta, Vfs, VfsError,
    VfsOp, WriteOptions,
};
use proptest::prelude::*;
use proptest::test_runner::Config;

fn config() -> Config {
    let mut config = common::config("tests/proptest-regressions/ops_faults.txt");
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 64;
    }
    config
}

// ---- generated trees ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Gen {
    File(usize),
    Dir,
    Link(u8),
}

fn gen_strategy() -> impl Strategy<Value = Vec<(usize, Gen)>> {
    let kind = prop_oneof![
        4 => (0usize..160).prop_map(Gen::File),
        1 => Just(Gen::Dir),
        1 => any::<u8>().prop_map(Gen::Link),
    ];
    // The root folder plus at most 49 items.
    prop::collection::vec((any::<usize>(), kind), 0..=49)
}

/// One generated source item, relative to the source root `src/t`.
#[derive(Debug, Clone)]
struct Node {
    /// `""` for the root, else `n00/n03`.
    rel: String,
    kind: EntryKind,
    content: Vec<u8>,
    link: String,
    mtime: i64,
    mode: u32,
}

fn build(shape: &[(usize, Gen)]) -> Vec<Node> {
    let mut nodes = vec![Node {
        rel: String::new(),
        kind: EntryKind::Dir,
        content: Vec::new(),
        link: String::new(),
        mtime: 1_000_000,
        mode: 0o755,
    }];
    let mut folders = vec![String::new()];
    for (i, (sel, kind)) in shape.iter().enumerate() {
        let parent = &folders[sel % folders.len()];
        let name = format!("n{i:02}");
        let rel = if parent.is_empty() {
            name
        } else {
            format!("{parent}/{name}")
        };
        let mut node = Node {
            rel: rel.clone(),
            kind: EntryKind::File,
            content: Vec::new(),
            link: String::new(),
            mtime: 1_000_100 + i as i64,
            mode: 0o600 | (i as u32 % 8) << 3,
        };
        match kind {
            Gen::File(len) => node.content = common::XorShift(i as u64 + 1).bytes(*len),
            Gen::Dir => {
                node.kind = EntryKind::Dir;
                folders.push(rel);
            }
            Gen::Link(t) => {
                node.kind = EntryKind::Symlink;
                node.link = format!("../target{t}");
            }
        }
        nodes.push(node);
    }
    nodes
}

fn join(root: &str, rel: &str) -> String {
    if rel.is_empty() {
        root.to_owned()
    } else {
        format!("{root}/{rel}")
    }
}

fn unix(mode: u32) -> Attrs {
    Attrs::Unix {
        meta: UnixMeta {
            mode,
            uid: 0,
            gid: 0,
        },
    }
}

/// Inserts the tree below `root` (parents before children).
fn insert(fx: &Fx, root: &str, nodes: &[Node]) {
    let vfs = fx.memory();
    for node in nodes {
        let meta = MemMeta {
            mtime: Some(common::ft(node.mtime)),
            ctime: None,
            attrs: Some(unix(node.mode)),
            crc32: None,
        };
        let path = fx.text(&join(root, &node.rel));
        match node.kind {
            EntryKind::Dir => vfs.insert_dir(&path, meta).unwrap(),
            EntryKind::File => vfs.insert_file(&path, node.content.clone(), meta).unwrap(),
            _ => vfs.insert_symlink(&path, &node.link, meta).unwrap(),
        }
    }
}

/// Existing targets for a copy with `Always`, of every kind the policy replaces: most folders as folders (merged),
/// every other folder as a **file** (a folder replaces it), most files as older files, every third file as a
/// **link** (a file replaces it), and every link as a **file** (a link replaces it). Nothing is inserted below a
/// folder that became a file.
fn insert_existing(fx: &Fx, nodes: &[Node]) {
    let vfs = fx.memory();
    let mut as_files: Vec<String> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if as_files
            .iter()
            .any(|f| node.rel.starts_with(&format!("{f}/")))
        {
            continue;
        }
        let path = fx.text(&join("dst/t", &node.rel));
        let meta = MemMeta {
            mtime: Some(common::ft(5)),
            attrs: Some(unix(0o644)),
            ..MemMeta::default()
        };
        let old = format!("old {}", node.rel).into_bytes();
        match node.kind {
            EntryKind::Dir if i % 2 == 0 => vfs.insert_dir(&path, meta).unwrap(),
            EntryKind::Dir => {
                as_files.push(node.rel.clone());
                vfs.insert_file(&path, old, meta).unwrap();
            }
            EntryKind::File if i % 3 == 0 => {
                vfs.insert_symlink(&path, "../elsewhere", meta).unwrap()
            }
            EntryKind::File | EntryKind::Symlink => vfs.insert_file(&path, old, meta).unwrap(),
            _ => {}
        }
    }
}

fn dump_map(vfs: &MemoryVfs) -> BTreeMap<String, MemDump> {
    vfs.dump()
        .into_iter()
        .map(|d| (d.path.clone(), d))
        .collect()
}

// ---- operations and faults ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// Copy into another `MemoryVfs`.
    CopyOther,
    /// Copy within one `MemoryVfs`.
    CopySelf,
    /// Copy into another `MemoryVfs` over existing targets (`Always`).
    CopyOverExisting,
    /// Move into another `MemoryVfs` (expanded: copy + remove per item).
    MoveOther,
    /// Move within one `MemoryVfs` (one `rename` of the root).
    MoveSelf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FaultOp {
    Open,
    Read,
    Write,
    Commit,
    Mkdir,
    SetTimes,
    SetAttrs,
    Symlink,
    Remove,
    Rename,
    ReadAfter,
    WriteAfter,
}

const FAULT_OPS: [FaultOp; 12] = [
    FaultOp::Open,
    FaultOp::Read,
    FaultOp::Write,
    FaultOp::Commit,
    FaultOp::Mkdir,
    FaultOp::SetTimes,
    FaultOp::SetAttrs,
    FaultOp::Symlink,
    FaultOp::Remove,
    FaultOp::Rename,
    FaultOp::ReadAfter,
    FaultOp::WriteAfter,
];

impl FaultOp {
    /// A failure of metadata: the content stays (D-95 step 6).
    fn is_metadata(self) -> bool {
        matches!(self, FaultOp::SetTimes | FaultOp::SetAttrs)
    }
}

/// Where a fault goes: the item, the back-end side and the path.
struct Placed {
    index: usize,
    on_source: bool,
    path: String,
}

/// The items `op` can fail, with the side and the path of the fault.
fn candidates(
    plan: &Plan,
    by_spec: &BTreeMap<String, usize>,
    nodes: &[Node],
    op: FaultOp,
) -> Vec<(usize, bool, Spec)> {
    let node = |item: &PlanItem| -> Option<&Node> {
        let key = item.source.as_ref().unwrap_or(&item.target).canonical();
        by_spec.get(&key).map(|&i| &nodes[i])
    };
    let mut out = Vec::new();
    for (index, item) in plan.items.iter().enumerate() {
        let Some(n) = node(item) else { continue };
        let source = item.source.clone();
        let file = matches!(item.action, Action::Copy | Action::Move) && n.kind == EntryKind::File;
        let link =
            matches!(item.action, Action::Link | Action::Move) && n.kind == EntryKind::Symlink;
        let target = item.target.clone();
        let pick = match op {
            FaultOp::Open | FaultOp::Read if file => Some((true, source.unwrap())),
            FaultOp::ReadAfter if file && !n.content.is_empty() => Some((true, source.unwrap())),
            FaultOp::Write | FaultOp::Commit if file => Some((false, target)),
            FaultOp::WriteAfter if file && !n.content.is_empty() => Some((false, target)),
            FaultOp::Mkdir if item.action == Action::CreateDir => Some((false, target)),
            FaultOp::SetTimes | FaultOp::SetAttrs if file || item.action == Action::CreateDir => {
                Some((false, target))
            }
            FaultOp::Symlink if link => Some((false, target)),
            FaultOp::Remove if matches!(item.action, Action::Move) => Some((true, source.unwrap())),
            FaultOp::Remove if item.action == Action::RemoveDir => Some((true, target)),
            FaultOp::Rename if item.action == Action::Rename => Some((true, source.unwrap())),
            _ => None,
        };
        if let Some((on_source, spec)) = pick {
            out.push((index, on_source, spec));
        }
    }
    out
}

fn inject(vfs: &MemoryVfs, op: FaultOp, path: &str, after: u64) {
    let kind = ErrorKind::PermissionDenied;
    let vfs_op = match op {
        FaultOp::ReadAfter => return vfs.inject_read_error_after(path, after, ErrorKind::Io),
        FaultOp::WriteAfter => return vfs.inject_write_error_after(path, after, ErrorKind::Io),
        FaultOp::Open => VfsOp::Open,
        FaultOp::Read => VfsOp::Read,
        FaultOp::Write => VfsOp::Write,
        FaultOp::Commit => VfsOp::Commit,
        FaultOp::Mkdir => VfsOp::Mkdir,
        FaultOp::SetTimes => VfsOp::SetTimes,
        FaultOp::SetAttrs => VfsOp::SetAttrs,
        FaultOp::Symlink => VfsOp::Symlink,
        FaultOp::Remove => VfsOp::Remove,
        FaultOp::Rename => VfsOp::Rename,
    };
    vfs.inject(Fault {
        path: path.to_owned(),
        op: vfs_op,
        kind,
    });
}

/// True when `spec` is strictly below `dir`.
fn below(spec: &Spec, dir: &Spec) -> bool {
    let mut cur = spec.parent();
    while let Some(parent) = cur {
        if parent == *dir {
            return true;
        }
        cur = parent.parent();
    }
    false
}

/// The statuses the execution must reach with a fault at item `k` (D-103 rules).
fn model(plan: &Plan, k: usize, op: FaultOp) -> Vec<(ItemStatus, Option<SkipReason>)> {
    let items = &plan.items;
    let failed = BTreeSet::from([k]);
    // Below a failed folder item nothing runs: `not_run` (§1.3.12).
    let mut not_run = BTreeSet::new();
    if op == FaultOp::Mkdir {
        // The folder's subtree: the following items deeper than it (plans are pre-order).
        let depth = items[k].depth;
        for (i, item) in items.iter().enumerate().skip(k + 1) {
            if item.depth <= depth {
                break;
            }
            not_run.insert(i);
        }
    }
    // A folder's metadata failing after its subtree does not block anything.
    let blocks = |i: usize| !(op.is_metadata() && i == k && items[k].action == Action::CreateDir);
    let own = |item: &PlanItem| item.source.clone().unwrap_or_else(|| item.target.clone());
    let mut out: Vec<(ItemStatus, Option<SkipReason>)> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        if failed.contains(&i) {
            out.push((ItemStatus::Failed, None));
            continue;
        }
        if not_run.contains(&i) {
            out.push((ItemStatus::NotRun, None));
            continue;
        }
        if item.action == Action::RemoveDir {
            let dir = &item.target;
            let blocked = (0..i).any(|j| {
                let earlier = &items[j];
                let status = out[j].0;
                (status == ItemStatus::Failed && blocks(j)
                    || matches!(status, ItemStatus::Skipped | ItemStatus::NotRun))
                    && (below(&own(earlier), dir)
                        || (matches!(earlier.action, Action::CreateDir | Action::KeepDir)
                            && own(earlier) == *dir))
            });
            if blocked {
                out.push((ItemStatus::Skipped, Some(SkipReason::ChildrenFailed)));
                continue;
            }
        }
        out.push((ItemStatus::Done, None));
    }
    out
}

struct Case {
    op: Op,
    fault: FaultOp,
    pick: usize,
    after: u64,
}

fn run_case(shape: &[(usize, Gen)], case: &Case) -> Result<(), TestCaseError> {
    let nodes = build(shape);
    let src = Fx::mem(CaseSensitivity::Sensitive);
    let other = matches!(
        case.op,
        Op::CopyOther | Op::CopyOverExisting | Op::MoveOther
    )
    .then(|| Fx::mem(CaseSensitivity::Sensitive));
    let dst = other.as_ref().unwrap_or(&src);
    insert(&src, "src/t", &nodes);
    dst.dir("dst");
    if case.op == Op::CopyOverExisting {
        insert_existing(dst, &nodes);
    }
    let pair = [(src.spec("src/t"), dst.spec("dst/t"))];
    let plan = match case.op {
        Op::CopyOther | Op::CopySelf | Op::CopyOverExisting => {
            let opts = CopyOptions {
                recursive: true,
                overwrite: if case.op == Op::CopyOverExisting {
                    OverwritePolicy::Always
                } else {
                    OverwritePolicy::Fail
                },
                ..CopyOptions::default()
            };
            ops::plan_copy(src.vfs(), dst.vfs(), &pair, &opts, &Ctl::detached()).unwrap()
        }
        Op::MoveOther | Op::MoveSelf => ops::plan_move(
            src.vfs(),
            dst.vfs(),
            &pair,
            &MoveOptions::default(),
            &Ctl::detached(),
        )
        .unwrap(),
    };
    prop_assert!(
        plan.items.iter().all(|i| i.error.is_none()),
        "{:?}",
        plan.items
    );
    // Every source spec (and the moved folders' targets) → its node.
    let mut by_spec = BTreeMap::new();
    for (i, node) in nodes.iter().enumerate() {
        by_spec.insert(src.spec(&join("src/t", &node.rel)).canonical(), i);
    }
    let found = candidates(&plan, &by_spec, &nodes, case.fault);
    if found.is_empty() {
        return Ok(());
    }
    let (k, on_source, spec) = found[case.pick % found.len()].clone();
    let placed = Placed {
        index: k,
        on_source,
        path: match &spec {
            Spec::Local(p) => p.as_path().to_string_lossy().into_owned(),
            other => panic!("{other:?}"),
        },
    };
    let len = nodes[by_spec[&plan.items[k]
        .source
        .as_ref()
        .unwrap_or(&plan.items[k].target)
        .canonical()]]
        .content
        .len() as u64;
    let after = if len == 0 { 0 } else { case.after % len };
    let fault_vfs = if placed.on_source {
        src.memory()
    } else {
        dst.memory()
    };
    inject(fault_vfs, case.fault, &placed.path, after);

    let src_before = dump_map(src.memory());
    let dst_before = dump_map(dst.memory());
    let expected = model(&plan, k, case.fault);
    let report = ops::execute(src.vfs(), dst.vfs(), plan.clone(), &Ctl::detached());
    check(
        &report,
        &expected,
        &placed,
        case,
        &nodes,
        &by_spec,
        (&src_before, &dst_before),
        (src.memory(), dst.memory()),
    )
}

#[allow(clippy::too_many_arguments)]
fn check(
    report: &OpReport,
    expected: &[(ItemStatus, Option<SkipReason>)],
    placed: &Placed,
    case: &Case,
    nodes: &[Node],
    by_spec: &BTreeMap<String, usize>,
    before: (&BTreeMap<String, MemDump>, &BTreeMap<String, MemDump>),
    vfs: (&MemoryVfs, &MemoryVfs),
) -> Result<(), TestCaseError> {
    let label = format!("{:?} {:?} at item {}", case.op, case.fault, placed.index);
    let got: Vec<(ItemStatus, Option<SkipReason>)> =
        report.results.iter().map(|r| (r.status, r.skip)).collect();
    prop_assert_eq!(report.plan.items.len(), expected.len(), "{}", label);
    prop_assert_eq!(&got, &expected.to_vec(), "{}: {:#?}", label, report.results);
    let count = |s: ItemStatus| expected.iter().filter(|e| e.0 == s).count() as u64;
    prop_assert_eq!(report.summary.planned, expected.len() as u64);
    prop_assert_eq!(report.summary.done, count(ItemStatus::Done));
    prop_assert_eq!(report.summary.failed, count(ItemStatus::Failed));
    prop_assert_eq!(report.summary.skipped, count(ItemStatus::Skipped));
    prop_assert!(report.summary.failed >= 1);
    prop_assert!(!report.ok(), "exit code 100");
    prop_assert!(!report.cancelled);

    let (src_before, dst_before) = before;
    let src_after = dump_map(vfs.0);
    let dst_after = dump_map(vfs.1);
    for path in src_after.keys().chain(dst_after.keys()) {
        prop_assert!(
            !path.contains(".hikasa-tmp") && !path.contains(".hikasa-old"),
            "{}: temporary sibling {} left",
            label,
            path
        );
    }
    let path_of = |spec: &Spec| match spec {
        Spec::Local(p) => p.as_path().to_string_lossy().into_owned(),
        other => panic!("{other:?}"),
    };
    for (i, (item, (status, _))) in report.plan.items.iter().zip(expected).enumerate() {
        let target = path_of(&item.target);
        let key = item.source.as_ref().unwrap_or(&item.target).canonical();
        let Some(node) = by_spec.get(&key).map(|&n| &nodes[n]) else {
            return Err(TestCaseError::fail(format!(
                "{label}: unknown item {item:?}"
            )));
        };
        let source_path = item.source.as_ref().map(path_of);
        let src_entry = source_path.as_ref().and_then(|p| src_before.get(p));
        match (item.action, status) {
            (Action::Copy | Action::Move | Action::Link | Action::Rename, ItemStatus::Done)
                if node.kind != EntryKind::Dir =>
            {
                let (Some(s), Some(t)) = (src_entry, dst_after.get(&target)) else {
                    return Err(TestCaseError::fail(format!(
                        "{label}: item {i} done but {target} missing"
                    )));
                };
                prop_assert_eq!(
                    (s.kind, s.len, s.crc32, &s.link_target),
                    (t.kind, t.len, t.crc32, &t.link_target),
                    "{} {}",
                    label,
                    target
                );
                if node.kind == EntryKind::File {
                    prop_assert_eq!(t.mtime, s.mtime, "{} {}", label, target);
                    prop_assert_eq!(t.attrs, s.attrs, "{} {}", label, target);
                }
                if matches!(item.action, Action::Move) {
                    prop_assert!(
                        !src_after.contains_key(source_path.as_ref().unwrap()),
                        "{}: source left",
                        label
                    );
                }
            }
            (Action::CreateDir | Action::KeepDir | Action::Rename, ItemStatus::Done) => {
                let t = dst_after.get(&target);
                prop_assert_eq!(
                    t.map(|t| t.kind),
                    Some(EntryKind::Dir),
                    "{} {}",
                    label,
                    target
                );
                if item.action == Action::CreateDir {
                    prop_assert_eq!(
                        t.and_then(|t| t.mtime),
                        Some(common::ft(node.mtime)),
                        "{} {}",
                        label,
                        target
                    );
                }
            }
            (Action::RemoveDir, ItemStatus::Done) => {
                prop_assert!(
                    !src_after.contains_key(&target),
                    "{}: {} still there",
                    label,
                    target
                );
            }
            (Action::RemoveDir, _) => {
                prop_assert!(
                    src_after.contains_key(&target),
                    "{}: {} removed",
                    label,
                    target
                );
            }
            (_, ItemStatus::Failed) if i == placed.index && case.fault.is_metadata() => {
                // The content (or the folder) stays.
                let t = dst_after.get(&target);
                prop_assert_eq!(t.map(|t| t.kind), Some(node.kind), "{} {}", label, target);
                if node.kind == EntryKind::File {
                    prop_assert_eq!(
                        t.and_then(|t| t.crc32),
                        Some(crc32fast::hash(&node.content))
                    );
                }
            }
            (_, ItemStatus::Failed | ItemStatus::NotRun) => {
                // Exactly as before (absent, or the replaced target unchanged).
                prop_assert_eq!(
                    dst_after.get(&target),
                    dst_before.get(&target),
                    "{}: target {}",
                    label,
                    target
                );
                if let Some(path) = &source_path {
                    prop_assert_eq!(
                        src_after.get(path),
                        src_before.get(path),
                        "{}: source {}",
                        label,
                        path
                    );
                }
            }
            _ => {}
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn a3_4_faults_leave_the_failing_target_untouched(
        shape in gen_strategy(),
        op in prop::sample::select(vec![Op::CopyOther, Op::CopySelf, Op::CopyOverExisting, Op::MoveOther, Op::MoveSelf]),
        fault in prop::sample::select(FAULT_OPS.to_vec()),
        pick in any::<usize>(),
        after in any::<u64>(),
    ) {
        // `Rename` only fails the one-rename move; the others never plan a `rename`.
        let op = if fault == FaultOp::Rename { Op::MoveSelf } else { op };
        let op = if op == Op::MoveSelf && fault != FaultOp::Rename { Op::MoveOther } else { op };
        run_case(&shape, &Case { op, fault, pick, after })?;
    }
}

#[test]
fn a3_4_every_fault_op_fires_on_a_fixed_tree() {
    // A tree with every kind, so each fault op has at least one item to fail (guards the property against a
    // generator that never reaches an op).
    let shape = vec![
        (0, Gen::File(10)),
        (0, Gen::Dir),
        (1, Gen::File(20)),
        (1, Gen::Link(3)),
        (0, Gen::Link(4)),
        (2, Gen::File(0)),
        // Node 7: a folder whose existing target is a file (folder over file with `Always`).
        (0, Gen::Dir),
        (3, Gen::File(5)),
    ];
    for fault in FAULT_OPS {
        let ops: &[Op] = match fault {
            FaultOp::Rename => &[Op::MoveSelf],
            FaultOp::Remove => &[Op::MoveOther],
            FaultOp::Mkdir => &[Op::CopyOther, Op::CopySelf, Op::MoveOther],
            _ => &[
                Op::CopyOther,
                Op::CopySelf,
                Op::CopyOverExisting,
                Op::MoveOther,
            ],
        };
        for &op in ops {
            for pick in 0..4 {
                run_case(
                    &shape,
                    &Case {
                        op,
                        fault,
                        pick,
                        after: 7,
                    },
                )
                .unwrap_or_else(|e| panic!("{op:?} {fault:?} pick {pick}: {e}"));
            }
        }
    }
}

// ---- cancellation ------------------------------------------------------------------------------------------

/// A `MemoryVfs` wrapper that cancels `token` when `open_write` is called for the `at`-th time (counting from
/// 0): the copy of that file is the running item.
struct CancelAtWrite<'a> {
    inner: &'a MemoryVfs,
    token: CancelToken,
    at: usize,
    writes: AtomicUsize,
}

impl Vfs for CancelAtWrite<'_> {
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
        if self.writes.fetch_add(1, Ordering::SeqCst) == self.at {
            self.token.cancel();
        }
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        self.inner.local_path(spec)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.inner.create_dir(spec, parents)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.inner.remove(spec)
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.inner.rename(from, to, replace)
    }
    fn set_times(&self, spec: &Spec, times: &SetTimes) -> Result<(), VfsError> {
        self.inner.set_times(spec, times)
    }
    fn set_attrs(&self, spec: &Spec, change: &AttrChange) -> Result<(), VfsError> {
        self.inner.set_attrs(spec, change)
    }
    fn create_symlink(
        &self,
        spec: &Spec,
        target: &std::ffi::OsStr,
        is_dir: bool,
    ) -> Result<(), VfsError> {
        self.inner.create_symlink(spec, target, is_dir)
    }
    fn move_to_trash(&self, spec: &Spec) -> Result<(), VfsError> {
        self.inner.move_to_trash(spec)
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn a3_4_cancellation_at_a_random_item(shape in gen_strategy(), at in any::<usize>()) {
        let nodes = build(&shape);
        let files = nodes.iter().filter(|n| n.kind == EntryKind::File).count();
        prop_assume!(files > 0);
        let src = Fx::mem(CaseSensitivity::Sensitive);
        let dst = Fx::mem(CaseSensitivity::Sensitive);
        insert(&src, "src/t", &nodes);
        dst.dir("dst");
        let token = CancelToken::new();
        let wrapper = CancelAtWrite { inner: dst.memory(), token: token.clone(), at: at % files, writes: AtomicUsize::new(0) };
        let opts = CopyOptions { recursive: true, ..CopyOptions::default() };
        let plan = ops::plan_copy(src.vfs(), &wrapper, &[(src.spec("src/t"), dst.spec("dst/t"))], &opts, &Ctl::detached()).unwrap();
        // The running item: the (at % files)-th copy item.
        let running = plan.items.iter().enumerate().filter(|(_, i)| i.action == Action::Copy).nth(at % files).map(|(n, _)| n).unwrap();
        let running_target = plan.items[running].target.clone();
        let progress = hikasa_core::NullProgress;
        let report = ops::execute(src.vfs(), &wrapper, plan, &Ctl { cancel: &token, progress: &progress });
        prop_assert!(report.cancelled);
        prop_assert!(!report.ok());
        for (i, result) in report.results.iter().enumerate() {
            let want = match i.cmp(&running) {
                std::cmp::Ordering::Less => ItemStatus::Done,
                std::cmp::Ordering::Equal => ItemStatus::Failed,
                std::cmp::Ordering::Greater => ItemStatus::NotRun,
            };
            prop_assert_eq!(result.status, want, "item {} of {:?}", i, report.plan.items[i]);
        }
        prop_assert_eq!(report.results[running].error.as_ref().unwrap().kind, ErrorKind::Cancelled);
        prop_assert_eq!(report.summary.failed, 1);
        prop_assert_eq!(report.summary.done, running as u64);
        // The running item's target is untouched (absent: the target tree was empty), and nothing after it ran.
        let absent = |spec: &Spec| dst.vfs().stat(spec, FollowLinks::No).is_err();
        prop_assert!(absent(&running_target));
        for item in &report.plan.items[running + 1..] {
            prop_assert!(absent(&item.target), "{:?}", item.target);
        }
    }
}
