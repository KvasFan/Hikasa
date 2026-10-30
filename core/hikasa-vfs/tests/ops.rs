//! M2 W3 acceptance A3.3 (generic copy: `MemoryVfs` → another `MemoryVfs`, `MemoryVfs` → itself, `LocalVfs` →
//! `LocalVfs`; metadata order, missing capabilities, failing metadata, owner, the local fast and streamed paths),
//! A3.5 (the overwrite policy table, the execution re-check, special files), A3.7 (move: rename, expansion,
//! merge, `children_failed`, `CrossDevice` at execution, rollback), A3.8 (the `plan_remove` table), the
//! `resolve_targets` table, the nesting checks, `dry_run` and cancellation. Contract M2 §1.3.4, §1.3.5.

mod common;

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use common::{Fx, ctl, ft, mspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, Phase, Progress, ProgressSink, Spec,
    WarningCode,
};
use hikasa_vfs::ops::{
    self, Action, CopyOptions, ItemStatus, MoveOptions, OpReport, OverwritePolicy, Plan, Preserve,
    RemoveOptions, SkipReason,
};
use hikasa_vfs::{
    AttrChange, Attrs, Blob, Capabilities, EntryKind, FAST_COPY_MAX, Fault, FollowLinks, LocalVfs,
    MemMeta, MemoryVfs, SetTimes, UnixMeta, Vfs, VfsError, VfsOp, VfsRouter, WriteOptions,
};

// ---- helpers -----------------------------------------------------------------------------------------------

fn copy_opts() -> CopyOptions {
    CopyOptions {
        recursive: true,
        ..CopyOptions::default()
    }
}

fn copy(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &CopyOptions) -> OpReport {
    let plan = ops::plan_copy(src, dst, pairs, opts, &Ctl::detached()).unwrap();
    ops::execute(src, dst, plan, &Ctl::detached())
}

fn mv(src: &dyn Vfs, dst: &dyn Vfs, pairs: &[(Spec, Spec)], opts: &MoveOptions) -> OpReport {
    let plan = ops::plan_move(src, dst, pairs, opts, &Ctl::detached()).unwrap();
    ops::execute(src, dst, plan, &Ctl::detached())
}

fn rm(vfs: &dyn Vfs, specs: &[Spec], opts: &RemoveOptions) -> OpReport {
    let plan = ops::plan_remove(vfs, specs, opts, &Ctl::detached()).unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

/// `(action, status)` of every item.
fn statuses(report: &OpReport) -> Vec<(Action, ItemStatus)> {
    report
        .plan
        .items
        .iter()
        .zip(&report.results)
        .map(|(item, result)| (item.action, result.status))
        .collect()
}

fn all_done(report: &OpReport) -> bool {
    report.results.iter().all(|r| r.status == ItemStatus::Done)
}

fn assert_all_done(report: &OpReport, label: &str) {
    assert!(
        all_done(report) && report.ok(),
        "{label}: {:#?}",
        statuses(report)
            .into_iter()
            .zip(&report.results)
            .collect::<Vec<_>>()
    );
}

fn mode_of(fx: &Fx, rel: &str) -> Option<u32> {
    match fx.lstat(rel)?.attrs {
        Attrs::Unix { meta } => Some(meta.mode),
        _ => None,
    }
}

/// True when modes can be set on `fx` (memory, or a Unix local back-end).
fn has_modes(fx: &Fx) -> bool {
    fx.is_mem() || cfg!(unix)
}

/// Sets the modification time (and on memory the creation time) and the mode of `rel`.
fn meta(fx: &Fx, rel: &str, mtime: i64, mode: u32) {
    let created = fx.is_mem().then(|| ft(mtime - 1000));
    fx.vfs()
        .set_times(
            &fx.spec(rel),
            &SetTimes {
                modified: Some(ft(mtime)),
                created,
            },
        )
        .unwrap();
    if has_modes(fx) {
        fx.vfs()
            .set_attrs(&fx.spec(rel), &AttrChange::Mode(mode))
            .unwrap();
    }
}

/// The source tree below `src/`: `f.txt`, `empty.bin`, `tree/{a, sub/b}`, with times and modes (folders last).
fn src_tree(fx: &Fx) {
    fx.file("src/f.txt", b"hello");
    fx.file("src/empty.bin", b"");
    fx.file("src/tree/a", b"aaa");
    fx.file("src/tree/sub/b", b"bbbb");
    fx.dir("dst");
    meta(fx, "src/f.txt", 1_600_000_100, 0o640);
    meta(fx, "src/empty.bin", 1_600_000_200, 0o600);
    meta(fx, "src/tree/a", 1_600_000_300, 0o644);
    meta(fx, "src/tree/sub/b", 1_600_000_400, 0o604);
    meta(fx, "src/tree/sub", 1_600_000_500, 0o750);
    meta(fx, "src/tree", 1_600_000_600, 0o755);
}

/// The three copy set-ups of A3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setup {
    /// `MemoryVfs` → another `MemoryVfs` instance (another back-end).
    MemOther,
    /// `MemoryVfs` → itself.
    MemSelf,
    /// `LocalVfs` → `LocalVfs` (the same instance: fast path).
    Local,
}

struct Pair {
    src: Fx,
    other: Option<Fx>,
}

impl Pair {
    fn new(setup: Setup) -> Pair {
        let (src, other) = match setup {
            Setup::MemOther => (
                Fx::mem(CaseSensitivity::Sensitive),
                Some(Fx::mem(CaseSensitivity::Sensitive)),
            ),
            Setup::MemSelf => (Fx::mem(CaseSensitivity::Sensitive), None),
            Setup::Local => (Fx::local(), None),
        };
        src_tree(&src);
        if let Some(other) = &other {
            other.dir("dst");
        }
        Pair { src, other }
    }

    fn dst(&self) -> &Fx {
        self.other.as_ref().unwrap_or(&self.src)
    }

    fn copy(&self, from: &str, to: &str, opts: &CopyOptions) -> OpReport {
        copy(
            self.src.vfs(),
            self.dst().vfs(),
            &[(self.src.spec(from), self.dst().spec(to))],
            opts,
        )
    }
}

const SETUPS: [Setup; 3] = [Setup::MemOther, Setup::MemSelf, Setup::Local];

/// Content, mtime and mode of `to` on `dst` equal those of `from` on `src`.
fn assert_same_file(src: &Fx, from: &str, dst: &Fx, to: &str, label: &str) {
    assert_eq!(dst.read(to), src.read(from), "{label}: content of {to}");
    let (s, d) = (src.lstat(from).unwrap(), dst.lstat(to).unwrap());
    assert_eq!(d.mtime, s.mtime, "{label}: mtime of {to}");
    if has_modes(src) && has_modes(dst) {
        assert_eq!(
            mode_of(dst, to),
            mode_of(src, from),
            "{label}: mode of {to}"
        );
    }
}

// ---- A3.3 copy ---------------------------------------------------------------------------------------------

#[test]
fn a3_3_copy_file_and_empty_file() {
    for setup in SETUPS {
        let pair = Pair::new(setup);
        let label = format!("{setup:?}");
        let report = pair.copy("src/f.txt", "dst/f.txt", &copy_opts());
        assert_all_done(&report, &label);
        assert_eq!(statuses(&report), [(Action::Copy, ItemStatus::Done)]);
        assert_eq!(report.summary.bytes, 5, "{label}");
        assert_same_file(&pair.src, "src/f.txt", pair.dst(), "dst/f.txt", &label);
        let report = pair.copy("src/empty.bin", "dst/empty.bin", &copy_opts());
        assert_all_done(&report, &label);
        assert_eq!(report.summary.bytes, 0);
        assert_same_file(
            &pair.src,
            "src/empty.bin",
            pair.dst(),
            "dst/empty.bin",
            &label,
        );
        assert_eq!(pair.dst().lstat("dst/empty.bin").unwrap().size, 0);
        // The source is unchanged.
        assert_eq!(pair.src.read("src/f.txt"), b"hello");
    }
}

#[test]
fn a3_3_copy_folder_tree() {
    for setup in SETUPS {
        let pair = Pair::new(setup);
        let label = format!("{setup:?}");
        let report = pair.copy("src/tree", "dst/tree", &copy_opts());
        assert_all_done(&report, &label);
        let targets: Vec<(Action, Spec, u32)> = report
            .plan
            .items
            .iter()
            .map(|i| (i.action, i.target.clone(), i.depth))
            .collect();
        let d = pair.dst();
        assert_eq!(
            targets,
            [
                (Action::CreateDir, d.spec("dst/tree"), 0),
                (Action::Copy, d.spec("dst/tree/a"), 1),
                (Action::CreateDir, d.spec("dst/tree/sub"), 1),
                (Action::Copy, d.spec("dst/tree/sub/b"), 2),
            ],
            "{label}"
        );
        assert_eq!(report.summary.planned, 4);
        assert_eq!(report.summary.done, 4);
        assert_eq!(report.summary.bytes, 7);
        for (from, to) in [
            ("src/tree/a", "dst/tree/a"),
            ("src/tree/sub/b", "dst/tree/sub/b"),
            ("src/tree/sub", "dst/tree/sub"),
            ("src/tree", "dst/tree"),
        ] {
            let s = pair.src.lstat(from).unwrap();
            let t = d.lstat(to).unwrap();
            assert_eq!(t.kind, s.kind, "{label} {to}");
            assert_eq!(
                t.mtime, s.mtime,
                "{label}: mtime of {to} (folders after their subtree)"
            );
            if has_modes(&pair.src) {
                assert_eq!(mode_of(d, to), mode_of(&pair.src, from), "{label} {to}");
            }
        }
        assert_eq!(d.read("dst/tree/sub/b"), b"bbbb");
        // Without `recursive` a folder is an item error.
        let report = pair.copy("src/tree", "dst/tree2", &CopyOptions::default());
        assert_eq!(statuses(&report), [(Action::CreateDir, ItemStatus::Failed)]);
        assert_eq!(
            report.results[0].error.as_ref().unwrap().kind,
            ErrorKind::IsADirectory
        );
        assert_eq!(d.kind("dst/tree2"), None);
    }
}

#[test]
fn a3_3_copy_without_preserve() {
    for setup in SETUPS {
        let pair = Pair::new(setup);
        let label = format!("{setup:?}");
        let opts = CopyOptions {
            preserve: Preserve::NONE,
            ..copy_opts()
        };
        let report = pair.copy("src/tree", "dst/tree", &opts);
        assert_all_done(&report, &label);
        let d = pair.dst();
        for (from, to) in [("src/tree/a", "dst/tree/a"), ("src/tree", "dst/tree")] {
            assert_ne!(
                d.lstat(to).unwrap().mtime,
                pair.src.lstat(from).unwrap().mtime,
                "{label} {to}"
            );
        }
        if d.is_mem() {
            assert_eq!(d.lstat("dst/tree/a").unwrap().attrs, Attrs::None, "{label}");
        }
    }
}

#[test]
fn a3_3_copy_created_time() {
    let opts = CopyOptions {
        preserve: Preserve {
            times: true,
            attrs: true,
            created: true,
        },
        ..copy_opts()
    };
    for setup in SETUPS {
        let pair = Pair::new(setup);
        let label = format!("{setup:?}");
        let report = pair.copy("src/f.txt", "dst/f.txt", &opts);
        assert_all_done(&report, &label);
        let (s, t) = (
            pair.src.lstat("src/f.txt").unwrap(),
            pair.dst().lstat("dst/f.txt").unwrap(),
        );
        assert_eq!(t.mtime, s.mtime, "{label}");
        let caps = pair
            .dst()
            .vfs()
            .capabilities(&pair.dst().spec("dst"))
            .unwrap();
        if caps.contains(Capabilities::SET_CREATED) {
            assert_eq!(t.ctime, s.ctime, "{label}");
            assert!(report.warnings.is_empty(), "{label}: {:?}", report.warnings);
        } else {
            // Linux: one capability_missing warning, the item done.
            assert_eq!(report.warnings.len(), 1, "{label}: {:?}", report.warnings);
            assert_eq!(report.warnings[0].code, WarningCode::CapabilityMissing);
            assert!(report.warnings[0].message.contains("set_created"));
        }
        // Without `created`, the creation time is not copied (memory).
        if pair.dst().is_mem() {
            let report = pair.copy("src/empty.bin", "dst/e2", &copy_opts());
            assert_all_done(&report, &label);
            assert_eq!(pair.dst().lstat("dst/e2").unwrap().ctime, None, "{label}");
        }
    }
}

#[test]
fn a3_3_journal_orders_content_times_attributes_and_folders_after_children() {
    for setup in [Setup::MemOther, Setup::MemSelf] {
        let pair = Pair::new(setup);
        let d = pair.dst();
        d.memory().set_journal(true);
        let report = pair.copy("src/tree", "dst/tree", &copy_opts());
        assert_all_done(&report, &format!("{setup:?}"));
        let journal = d.memory().journal();
        let s = |rel: &str| d.spec(rel).canonical();
        assert_eq!(
            journal,
            [
                (VfsOp::Mkdir, s("dst/tree")),
                (VfsOp::Commit, s("dst/tree/a")),
                (VfsOp::SetTimes, s("dst/tree/a")),
                (VfsOp::SetAttrs, s("dst/tree/a")),
                (VfsOp::Mkdir, s("dst/tree/sub")),
                (VfsOp::Commit, s("dst/tree/sub/b")),
                (VfsOp::SetTimes, s("dst/tree/sub/b")),
                (VfsOp::SetAttrs, s("dst/tree/sub/b")),
                (VfsOp::SetTimes, s("dst/tree/sub")),
                (VfsOp::SetAttrs, s("dst/tree/sub")),
                (VfsOp::SetTimes, s("dst/tree")),
                (VfsOp::SetAttrs, s("dst/tree")),
            ],
            "{setup:?}"
        );
    }
}

#[test]
fn a3_3_missing_target_capability_gives_one_warning() {
    for (removed, name) in [
        (Capabilities::SET_TIMES, "set_times"),
        (Capabilities::SET_ATTRS, "set_attrs"),
    ] {
        for setup in [Setup::MemOther, Setup::MemSelf] {
            let pair = Pair::new(setup);
            let d = pair.dst();
            d.memory()
                .set_capabilities(Capabilities::all().difference(removed));
            let report = pair.copy("src/tree", "dst/tree", &copy_opts());
            let label = format!("{setup:?} without {name}");
            assert_all_done(&report, &label);
            let missing: Vec<_> = report
                .warnings
                .iter()
                .chain(&report.plan.warnings)
                .filter(|w| w.code == WarningCode::CapabilityMissing)
                .collect();
            assert_eq!(missing.len(), 1, "{label}: {missing:?}");
            assert!(missing[0].message.contains(name), "{label}: {missing:?}");
            assert_eq!(d.read("dst/tree/sub/b"), b"bbbb");
            let t = d.lstat("dst/tree/a").unwrap();
            if removed == Capabilities::SET_TIMES {
                assert_eq!(t.mtime, None, "{label}");
                assert_eq!(mode_of(d, "dst/tree/a"), Some(0o644), "{label}");
            } else {
                assert_eq!(t.mtime, Some(ft(1_600_000_300)), "{label}");
                assert_eq!(t.attrs, Attrs::None, "{label}");
            }
        }
    }
}

#[test]
fn a3_3_failing_set_attrs_fails_the_item_and_keeps_the_content() {
    for setup in [Setup::MemOther, Setup::MemSelf] {
        let pair = Pair::new(setup);
        let d = pair.dst();
        d.memory().inject(Fault {
            path: d.text("dst/tree/a"),
            op: VfsOp::SetAttrs,
            kind: ErrorKind::PermissionDenied,
        });
        let report = pair.copy("src/tree", "dst/tree", &copy_opts());
        let label = format!("{setup:?}");
        assert_eq!(
            statuses(&report),
            [
                (Action::CreateDir, ItemStatus::Done),
                (Action::Copy, ItemStatus::Failed),
                (Action::CreateDir, ItemStatus::Done),
                (Action::Copy, ItemStatus::Done),
            ],
            "{label}"
        );
        assert_eq!(
            report.results[1].error.as_ref().unwrap().kind,
            ErrorKind::PermissionDenied
        );
        assert!(!report.ok());
        assert_eq!(report.summary.failed, 1);
        // The content is complete and the times (applied before the attributes) are set.
        assert_eq!(d.read("dst/tree/a"), b"aaa", "{label}");
        assert_eq!(
            d.lstat("dst/tree/a").unwrap().mtime,
            Some(ft(1_600_000_300))
        );
    }
}

#[test]
fn a3_3_owner_never_changes() {
    let src = MemoryVfs::default();
    let dst = MemoryVfs::default();
    let owned = |uid, gid| MemMeta {
        mtime: Some(ft(1_600_000_000)),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: 0o640,
                uid,
                gid,
            },
        }),
        ..MemMeta::default()
    };
    src.insert_file(&common::host("/s/f"), b"x".to_vec(), owned(1000, 1000))
        .unwrap();
    // A replaced target keeps its owner; a new one gets none copied.
    dst.insert_file(&common::host("/d/f"), b"old".to_vec(), owned(42, 43))
        .unwrap();
    let opts = CopyOptions {
        overwrite: OverwritePolicy::Always,
        ..copy_opts()
    };
    let report = copy(
        &src,
        &dst,
        &[
            (mspec("/s/f"), mspec("/d/f")),
            (mspec("/s/f"), mspec("/d/new")),
        ],
        &opts,
    );
    assert_all_done(&report, "owner");
    let attrs = |p| dst.stat(&mspec(p), FollowLinks::No).unwrap().attrs;
    assert_eq!(
        attrs("/d/f"),
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o640,
                uid: 42,
                gid: 43
            }
        }
    );
    match attrs("/d/new") {
        Attrs::Unix { meta } => {
            assert_eq!(meta.mode, 0o640);
            assert_ne!(meta.uid, 1000);
        }
        other => panic!("{other:?}"),
    }
    // Local: the owner is the current user's, whatever the source's.
    #[cfg(unix)]
    {
        let fx = Fx::local();
        fx.file("src/f", b"x");
        fx.dir("dst");
        let report = copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src/f"), fx.spec("dst/f"))],
            &copy_opts(),
        );
        assert_all_done(&report, "local owner");
        let owner = |rel: &str| match fx.lstat(rel).unwrap().attrs {
            Attrs::Unix { meta } => (meta.uid, meta.gid),
            other => panic!("{other:?}"),
        };
        assert_eq!(owner("dst/f"), owner("src/f"));
    }
}

/// Counts progress reports.
#[derive(Default)]
struct Counter(AtomicU64);

impl ProgressSink for Counter {
    fn report(&self, progress: Progress) {
        assert_eq!(progress.phase, Phase::Writing);
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn a3_3_local_fast_and_streamed_paths_agree() {
    let fx = Fx::local();
    let mut rng = common::XorShift(7);
    // Larger than one stream buffer (1 MiB), smaller than FAST_COPY_MAX.
    fx.file("src/f", &rng.bytes(3_000_000));
    meta(&fx, "src/f", 1_600_000_000, 0o640);
    fx.dir("dst");
    let fast_vfs = fx.vfs();
    // A second instance: not the same back-end instance, so the copy streams.
    let other = LocalVfs::default();
    let run = |dst: &dyn Vfs, to: &str| -> u64 {
        let counter = Counter::default();
        let cancel = CancelToken::never();
        let ctl = Ctl {
            cancel: &cancel,
            progress: &counter,
        };
        let plan = ops::plan_copy(
            fx.vfs(),
            dst,
            &[(fx.spec("src/f"), fx.spec(to))],
            &copy_opts(),
            &Ctl::detached(),
        )
        .unwrap();
        let report = ops::execute(fx.vfs(), dst, plan, &ctl);
        assert_all_done(&report, to);
        counter.0.load(Ordering::Relaxed)
    };
    let fast_reports = run(fast_vfs, "dst/fast");
    let streamed_reports = run(&other, "dst/streamed");
    assert_eq!(fast_reports, 1, "the fast path reports once");
    assert!(streamed_reports > 1, "the stream reports per buffer");
    for to in ["dst/fast", "dst/streamed"] {
        assert_same_file(&fx, "src/f", &fx, to, to);
    }
    let names = fx.names("dst");
    assert_eq!(names, ["fast", "streamed"], "no temporary sibling left");
}

#[test]
fn a3_3_file_above_fast_copy_max_is_streamed() {
    let fx = Fx::local();
    fx.dir("src");
    fx.dir("dst");
    let path = fx.path("src/big");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"head").unwrap();
        file.set_len(FAST_COPY_MAX + 1).unwrap();
        file.flush().unwrap();
    }
    meta(&fx, "src/big", 1_600_000_000, 0o600);
    let cancel = CancelToken::never();
    assert_eq!(
        fx.vfs()
            .copy_file_fast(
                &fx.spec("src/big"),
                &fx.spec("dst/x"),
                &WriteOptions::default(),
                &cancel
            )
            .unwrap(),
        None,
        "no fast path above FAST_COPY_MAX"
    );
    let counter = Counter::default();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &counter,
    };
    let plan = ops::plan_copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/big"), fx.spec("dst/big"))],
        &copy_opts(),
        &Ctl::detached(),
    )
    .unwrap();
    let report = ops::execute(fx.vfs(), fx.vfs(), plan, &ctl);
    assert_all_done(&report, "big");
    assert!(counter.0.load(Ordering::Relaxed) > 1, "streamed");
    assert_eq!(report.summary.bytes, FAST_COPY_MAX + 1);
    let t = fx.lstat("dst/big").unwrap();
    assert_eq!(t.size, FAST_COPY_MAX + 1);
    assert_eq!(t.mtime, Some(ft(1_600_000_000)));
    if cfg!(unix) {
        assert_eq!(mode_of(&fx, "dst/big"), Some(0o600));
    }
    let mut head = [0u8; 4];
    std::io::Read::read_exact(
        &mut fx.vfs().open_read(&fx.spec("dst/big")).unwrap(),
        &mut head,
    )
    .unwrap();
    assert_eq!(&head, b"head");
}

// ---- A3.5 overwrite ----------------------------------------------------------------------------------------

/// The expected result of one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Exp {
    /// Done; the target has the source's content.
    Done,
    /// Skipped for the reason; the target is unchanged.
    Skip(SkipReason),
    /// Failed with the kind; the target is unchanged.
    Fail(ErrorKind),
}

/// Target states of the overwrite table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Absent,
    Older,
    Newer,
    Equal,
    Unknown,
    Folder,
    Link,
}

const SOURCE_MTIME: i64 = 2_000_000;

fn expected(policy: OverwritePolicy, target: Target) -> Exp {
    use OverwritePolicy::*;
    match (target, policy) {
        (Target::Absent, _) => Exp::Done,
        (Target::Folder, _) => Exp::Fail(ErrorKind::IsADirectory),
        (_, Fail) => Exp::Fail(ErrorKind::AlreadyExists),
        (_, Skip) => Exp::Skip(SkipReason::Exists),
        (_, Always) => Exp::Done,
        (Target::Older | Target::Link, Newer) => Exp::Done,
        (Target::Newer | Target::Equal | Target::Unknown, Newer) => Exp::Skip(SkipReason::NotNewer),
    }
}

/// Builds the target state at `dst/t` (and `dst/pointed` for links).
fn make_target(fx: &Fx, target: Target) {
    let file = |mtime: Option<i64>| {
        fx.file("dst/t", b"old target");
        if let Some(m) = mtime {
            fx.vfs()
                .set_times(
                    &fx.spec("dst/t"),
                    &SetTimes {
                        modified: Some(ft(m)),
                        created: None,
                    },
                )
                .unwrap();
        }
    };
    match target {
        Target::Absent => {}
        Target::Older => file(Some(SOURCE_MTIME - 10)),
        Target::Newer => file(Some(SOURCE_MTIME + 10)),
        Target::Equal => file(Some(SOURCE_MTIME)),
        Target::Unknown => file(None),
        Target::Folder => fx.file("dst/t/inside", b"inside"),
        Target::Link => {
            fx.file("dst/pointed", b"pointed");
            assert!(fx.link("dst/t", "pointed", false));
            fx.vfs()
                .set_times(
                    &fx.spec("dst/t"),
                    &SetTimes {
                        modified: Some(ft(SOURCE_MTIME - 10)),
                        created: None,
                    },
                )
                .unwrap();
        }
    }
}

fn check_overwrite(fx: &Fx, policy: OverwritePolicy, target: Target, exp: Exp) {
    let label = format!("{} {policy:?} × {target:?}", fx.label());
    let report = report_with(fx, policy);
    let result = &report.results[0];
    match exp {
        Exp::Done => {
            assert_eq!(result.status, ItemStatus::Done, "{label}: {result:?}");
            assert_eq!(fx.lstat("dst/t").unwrap().kind, EntryKind::File, "{label}");
            assert_eq!(fx.read("dst/t"), b"source", "{label}");
            assert_eq!(
                report.plan.items[0].replaces,
                target != Target::Absent,
                "{label}"
            );
        }
        Exp::Skip(reason) => {
            assert_eq!(result.status, ItemStatus::Skipped, "{label}: {result:?}");
            assert_eq!(result.skip, Some(reason), "{label}");
            assert!(report.ok(), "{label}");
        }
        Exp::Fail(kind) => {
            assert_eq!(result.status, ItemStatus::Failed, "{label}: {result:?}");
            assert_eq!(result.error.as_ref().unwrap().kind, kind, "{label}");
            assert!(!report.ok(), "{label}");
        }
    }
    if exp != Exp::Done {
        let kind = fx.kind("dst/t");
        match target {
            Target::Folder => assert_eq!(fx.read("dst/t/inside"), b"inside", "{label}"),
            Target::Link => assert_eq!(kind, Some(EntryKind::Symlink), "{label}"),
            Target::Absent => {}
            _ => assert_eq!(fx.read("dst/t"), b"old target", "{label}"),
        }
    }
    if target == Target::Link {
        // The file the link pointed to is never written through (D-94 rule 4).
        assert_eq!(fx.read("dst/pointed"), b"pointed", "{label}");
    }
}

fn report_with(fx: &Fx, policy: OverwritePolicy) -> OpReport {
    let opts = CopyOptions {
        overwrite: policy,
        ..copy_opts()
    };
    copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/s"), fx.spec("dst/t"))],
        &opts,
    )
}

fn overwrite_fixture(fx: &Fx, target: Target) {
    fx.file("src/s", b"source");
    fx.vfs()
        .set_times(
            &fx.spec("src/s"),
            &SetTimes {
                modified: Some(ft(SOURCE_MTIME)),
                created: None,
            },
        )
        .unwrap();
    fx.dir("dst");
    make_target(fx, target);
}

#[test]
fn a3_5_overwrite_policy_table() {
    let targets = [
        Target::Absent,
        Target::Older,
        Target::Newer,
        Target::Equal,
        Target::Unknown,
        Target::Folder,
        Target::Link,
    ];
    for policy in OverwritePolicy::ALL {
        for target in targets {
            let mut fixtures = vec![Fx::mem(CaseSensitivity::Sensitive), Fx::local()];
            for fx in fixtures.drain(..) {
                // A local file always has an mtime; links need the privilege on Windows.
                if !fx.is_mem()
                    && (target == Target::Unknown || (cfg!(windows) && target == Target::Link))
                {
                    continue;
                }
                overwrite_fixture(&fx, target);
                check_overwrite(&fx, policy, target, expected(policy, target));
            }
        }
    }
}

#[test]
fn a3_5_newer_with_unknown_source_mtime_is_not_newer() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("src/s", b"source");
    fx.dir("dst");
    make_target(&fx, Target::Older);
    let report = report_with(&fx, OverwritePolicy::Newer);
    assert_eq!(report.results[0].skip, Some(SkipReason::NotNewer));
    assert_eq!(fx.read("dst/t"), b"old target");
}

#[cfg(unix)]
fn mkfifo(path: &std::path::Path) -> bool {
    match std::process::Command::new("mkfifo").arg(path).status() {
        Ok(status) if status.success() => true,
        other => {
            eprintln!("skipping: mkfifo unavailable ({other:?})");
            false
        }
    }
}

/// Runs `f` on a helper thread and fails when it does not finish in 20 s (a FIFO opened for reading would block
/// forever).
#[cfg(unix)]
fn with_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(20))
        .expect("timed out: a special file was opened")
}

#[cfg(unix)]
#[test]
fn a3_5_special_file_target_table() {
    for policy in OverwritePolicy::ALL {
        let fx = Fx::local();
        overwrite_fixture(&fx, Target::Absent);
        if !mkfifo(&fx.path("dst/t")) {
            return;
        }
        let exp = match policy {
            OverwritePolicy::Fail => Exp::Fail(ErrorKind::AlreadyExists),
            OverwritePolicy::Skip => Exp::Skip(SkipReason::Exists),
            OverwritePolicy::Always => Exp::Done,
            // The FIFO was made now: newer than the source.
            OverwritePolicy::Newer => Exp::Skip(SkipReason::NotNewer),
        };
        let report = with_timeout(move || {
            let report = report_with(&fx, policy);
            (fx, report)
        });
        let (fx, report) = report;
        let result = &report.results[0];
        let label = format!("{policy:?} × special file");
        match exp {
            Exp::Done => {
                assert_eq!(result.status, ItemStatus::Done, "{label}: {result:?}");
                assert_eq!(
                    fx.kind("dst/t"),
                    Some(EntryKind::File),
                    "{label}: the FIFO replaced"
                );
                assert_eq!(fx.read("dst/t"), b"source");
            }
            Exp::Skip(reason) => {
                assert_eq!(result.skip, Some(reason), "{label}: {result:?}");
                assert_eq!(fx.kind("dst/t"), Some(EntryKind::Other), "{label}");
            }
            Exp::Fail(kind) => {
                assert_eq!(
                    result.error.as_ref().map(|e| e.kind),
                    Some(kind),
                    "{label}: {result:?}"
                );
                assert_eq!(fx.kind("dst/t"), Some(EntryKind::Other), "{label}");
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn a3_5_source_fifo_is_skipped_and_never_opened() {
    let fx = Fx::local();
    fx.file("src/tree/a", b"a");
    fx.dir("dst");
    if !mkfifo(&fx.path("src/tree/fifo")) {
        return;
    }
    let (fx, root, tree) = with_timeout(move || {
        // The FIFO as a root and inside a tree.
        let root = copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src/tree/fifo"), fx.spec("dst/fifo"))],
            &copy_opts(),
        );
        let tree = copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src/tree"), fx.spec("dst/tree"))],
            &copy_opts(),
        );
        (fx, root, tree)
    });
    assert_eq!(statuses(&root), [(Action::Skip, ItemStatus::Skipped)]);
    assert_eq!(root.results[0].skip, Some(SkipReason::SpecialFile));
    assert_eq!(root.plan.items[0].kind, EntryKind::Other);
    assert!(root.ok());
    assert_eq!(
        statuses(&tree),
        [
            (Action::CreateDir, ItemStatus::Done),
            (Action::Copy, ItemStatus::Done),
            (Action::Skip, ItemStatus::Skipped),
        ]
    );
    assert_eq!(tree.results[2].skip, Some(SkipReason::SpecialFile));
    assert_eq!(fx.kind("dst/fifo"), None);
    assert_eq!(fx.names("dst/tree"), ["a"]);
}

#[test]
fn a3_5_source_folder_onto_file_link_and_folder() {
    use OverwritePolicy::*;
    for policy in OverwritePolicy::ALL {
        for target in [Target::Older, Target::Link, Target::Folder] {
            for fx in [Fx::mem(CaseSensitivity::Sensitive), Fx::local()] {
                if !fx.is_mem() && cfg!(windows) && target == Target::Link {
                    continue;
                }
                let label = format!("{} folder × {policy:?} × {target:?}", fx.label());
                fx.file("src/d/child", b"child");
                fx.vfs()
                    .set_times(
                        &fx.spec("src/d"),
                        &SetTimes {
                            modified: Some(ft(SOURCE_MTIME)),
                            created: None,
                        },
                    )
                    .unwrap();
                fx.dir("dst");
                make_target(&fx, target);
                let opts = CopyOptions {
                    overwrite: policy,
                    ..copy_opts()
                };
                let report = copy(
                    fx.vfs(),
                    fx.vfs(),
                    &[(fx.spec("src/d"), fx.spec("dst/t"))],
                    &opts,
                );
                let first = (report.plan.items[0].action, report.results[0].status);
                match (target, policy) {
                    (Target::Folder, _) => {
                        // Merged: the folder is kept, the child copied, the existing content kept.
                        assert_eq!(first, (Action::KeepDir, ItemStatus::Done), "{label}");
                        assert_eq!(fx.read("dst/t/child"), b"child", "{label}");
                        assert_eq!(fx.read("dst/t/inside"), b"inside", "{label}");
                    }
                    (_, Always) | (_, Newer) => {
                        assert_eq!(first, (Action::CreateDir, ItemStatus::Done), "{label}");
                        assert!(report.plan.items[0].replaces, "{label}");
                        assert_eq!(fx.kind("dst/t"), Some(EntryKind::Dir), "{label}");
                        assert_eq!(fx.read("dst/t/child"), b"child", "{label}");
                    }
                    (_, Fail) => {
                        assert_eq!(first, (Action::CreateDir, ItemStatus::Failed), "{label}");
                        assert_eq!(report.plan.items.len(), 1, "{label}: subtree not planned");
                        assert_ne!(fx.kind("dst/t"), Some(EntryKind::Dir), "{label}");
                    }
                    (_, Skip) => {
                        assert_eq!(first, (Action::Skip, ItemStatus::Skipped), "{label}");
                        assert_eq!(report.results[0].skip, Some(SkipReason::Exists));
                        assert_eq!(report.plan.items.len(), 1, "{label}");
                    }
                }
                if target == Target::Link {
                    assert_eq!(fx.read("dst/pointed"), b"pointed", "{label}");
                }
            }
        }
    }
}

#[test]
fn a3_5_merge_applies_the_policy_per_item() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("src/d/a", b"new a");
    fx.file("src/d/b", b"new b");
    fx.file("dst/d/a", b"old a");
    for (policy, a_status, a_content) in [
        (OverwritePolicy::Fail, ItemStatus::Failed, &b"old a"[..]),
        (OverwritePolicy::Skip, ItemStatus::Skipped, b"old a"),
        (OverwritePolicy::Always, ItemStatus::Done, b"new a"),
    ] {
        let opts = CopyOptions {
            overwrite: policy,
            ..copy_opts()
        };
        let report = copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("src/d"), fx.spec("dst/d"))],
            &opts,
        );
        assert_eq!(report.results[0].status, ItemStatus::Done, "{policy:?}");
        assert_eq!(report.plan.items[0].action, Action::KeepDir);
        assert_eq!(report.results[1].status, a_status, "{policy:?}");
        assert_eq!(fx.read("dst/d/a"), a_content, "{policy:?}");
        assert_eq!(fx.read("dst/d/b"), b"new b", "{policy:?}");
        fx.memory().remove(&fx.spec("dst/d/b")).unwrap();
    }
}

#[test]
fn a3_5_execution_recheck_refuses_a_new_target() {
    for fx in [Fx::mem(CaseSensitivity::Sensitive), Fx::local()] {
        fx.file("src/s", b"source");
        fx.file("src/d/x", b"x");
        fx.dir("dst");
        let plan = ops::plan_copy(
            fx.vfs(),
            fx.vfs(),
            &[
                (fx.spec("src/s"), fx.spec("dst/s")),
                (fx.spec("src/d"), fx.spec("dst/d")),
            ],
            &copy_opts(),
            &Ctl::detached(),
        )
        .unwrap();
        // Created between planning and execution.
        fx.file("dst/s", b"intruder");
        fx.file("dst/d", b"intruder folder");
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        let label = fx.label();
        assert_eq!(
            statuses(&report),
            [
                (Action::Copy, ItemStatus::Failed),
                (Action::CreateDir, ItemStatus::Failed),
                // Below a failed folder item nothing runs (§1.3.12).
                (Action::Copy, ItemStatus::NotRun),
            ],
            "{label}"
        );
        for result in &report.results[..2] {
            let error = result.error.as_ref().unwrap();
            assert_eq!(error.kind, ErrorKind::InvalidInput, "{label}");
            assert!(
                error.message.contains("changed since the plan was made"),
                "{label}: {}",
                error.message
            );
        }
        assert_eq!(fx.read("dst/s"), b"intruder", "{label}: nothing replaced");
        assert_eq!(fx.read("dst/d"), b"intruder folder", "{label}");
        assert!(!report.ok());
    }
}

// ---- A3.7 move ---------------------------------------------------------------------------------------------

fn move_tree(fx: &Fx) {
    fx.file("src/tree/a", b"aaa");
    fx.file("src/tree/sub/b", b"bbbb");
    fx.file("src/f", b"file");
    fx.dir("dst");
}

#[test]
fn a3_7_same_device_is_one_rename_and_keeps_the_folder_id() {
    for fx in [
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Insensitive),
        Fx::local(),
    ] {
        move_tree(&fx);
        let id = fx.lstat("src/tree").unwrap().file_id;
        assert!(id.is_some());
        let report = mv(
            fx.vfs(),
            fx.vfs(),
            &[
                (fx.spec("src/tree"), fx.spec("dst/tree")),
                (fx.spec("src/f"), fx.spec("dst/f")),
            ],
            &MoveOptions::default(),
        );
        let label = fx.label();
        assert_eq!(
            statuses(&report),
            [
                (Action::Rename, ItemStatus::Done),
                (Action::Rename, ItemStatus::Done)
            ],
            "{label}"
        );
        assert_eq!(fx.lstat("dst/tree").unwrap().file_id, id, "{label}");
        assert_eq!(fx.read("dst/tree/sub/b"), b"bbbb");
        assert_eq!(fx.kind("src/tree"), None);
        assert_eq!(fx.kind("src/f"), None);
    }
}

fn expanded_items(fx_src: &Fx, fx_dst: &Fx) -> Vec<(Action, Spec)> {
    vec![
        (Action::CreateDir, fx_dst.spec("dst/tree")),
        (Action::Move, fx_dst.spec("dst/tree/a")),
        (Action::CreateDir, fx_dst.spec("dst/tree/sub")),
        (Action::Move, fx_dst.spec("dst/tree/sub/b")),
        (Action::RemoveDir, fx_src.spec("src/tree/sub")),
        (Action::RemoveDir, fx_src.spec("src/tree")),
    ]
}

fn items(report: &OpReport) -> Vec<(Action, Spec)> {
    report
        .plan
        .items
        .iter()
        .map(|i| (i.action, i.target.clone()))
        .collect()
}

#[test]
fn a3_7_other_device_or_back_end_copies_and_removes_per_item() {
    // Another device of the same instance.
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    move_tree(&fx);
    fx.memory().set_device(&fx.text("dst"), 2);
    let report = mv(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/tree"), fx.spec("dst/tree"))],
        &MoveOptions::default(),
    );
    assert_eq!(items(&report), expanded_items(&fx, &fx));
    assert_all_done(&report, "device");
    assert_eq!(fx.read("dst/tree/sub/b"), b"bbbb");
    assert_eq!(fx.kind("src/tree"), None);
    assert_eq!(report.summary.bytes, 7);

    // Two instances.
    let (src, dst) = (
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Sensitive),
    );
    move_tree(&src);
    dst.dir("dst");
    let report = mv(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("src/tree"), dst.spec("dst/tree"))],
        &MoveOptions::default(),
    );
    assert_eq!(items(&report), expanded_items(&src, &dst));
    assert_all_done(&report, "instances");
    assert_eq!(dst.read("dst/tree/a"), b"aaa");
    assert_eq!(src.kind("src/tree"), None);
    assert_eq!(src.names("src"), ["f"]);
}

#[test]
fn a3_7_merge_into_an_existing_folder() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    move_tree(&fx);
    fx.file("dst/tree/existing", b"e");
    let report = mv(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/tree"), fx.spec("dst/tree"))],
        &MoveOptions::default(),
    );
    assert_eq!(report.plan.items[0].action, Action::KeepDir);
    assert!(!items(&report).iter().any(|(a, _)| *a == Action::Rename));
    assert_all_done(&report, "merge");
    assert_eq!(fx.names("dst/tree"), ["a", "existing", "sub"]);
    assert_eq!(fx.kind("src/tree"), None);
}

#[test]
fn a3_7_source_folder_kept_when_a_child_failed() {
    let (src, dst) = (
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Sensitive),
    );
    move_tree(&src);
    dst.dir("dst");
    src.memory().inject(Fault {
        path: src.text("src/tree/sub/b"),
        op: VfsOp::Open,
        kind: ErrorKind::PermissionDenied,
    });
    let report = mv(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("src/tree"), dst.spec("dst/tree"))],
        &MoveOptions::default(),
    );
    assert_eq!(
        statuses(&report),
        [
            (Action::CreateDir, ItemStatus::Done),
            (Action::Move, ItemStatus::Done),
            (Action::CreateDir, ItemStatus::Done),
            (Action::Move, ItemStatus::Failed),
            (Action::RemoveDir, ItemStatus::Skipped),
            (Action::RemoveDir, ItemStatus::Skipped),
        ]
    );
    assert_eq!(report.results[4].skip, Some(SkipReason::ChildrenFailed));
    assert_eq!(report.results[5].skip, Some(SkipReason::ChildrenFailed));
    src.memory().clear_faults();
    assert_eq!(src.read("src/tree/sub/b"), b"bbbb");
    assert_eq!(src.kind("src/tree/a"), None, "the moved child left");
    assert_eq!(dst.kind("dst/tree/sub/b"), None);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.summary.skipped, 2);
    assert!(!report.ok());
}

#[test]
fn a3_7_cross_device_at_execution_is_expanded_in_the_report() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    move_tree(&fx);
    let plan = ops::plan_move(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/tree"), fx.spec("dst/tree"))],
        &MoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert_eq!(plan.items[0].action, Action::Rename);
    let dry = ops::dry_run(plan.clone());
    assert_eq!(statuses(&dry), [(Action::Rename, ItemStatus::Planned)]);
    // The target becomes another device after planning.
    fx.memory().set_device(&fx.text("dst"), 9);
    let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
    assert_eq!(items(&report), expanded_items(&fx, &fx));
    assert_eq!(report.results.len(), 6);
    assert_all_done(&report, "cross device");
    assert_eq!(report.summary.planned, 6);
    assert_eq!(fx.read("dst/tree/sub/b"), b"bbbb");
    assert_eq!(fx.kind("src/tree"), None);
}

#[test]
fn a3_7_source_removal_fault_rolls_the_copy_back() {
    let (src, dst) = (
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Sensitive),
    );
    move_tree(&src);
    dst.dir("dst");
    src.memory().inject(Fault {
        path: src.text("src/f"),
        op: VfsOp::Remove,
        kind: ErrorKind::PermissionDenied,
    });
    // Without a target: the copy is removed again.
    let before = dst.memory().dump();
    let report = mv(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("src/f"), dst.spec("dst/f"))],
        &MoveOptions::default(),
    );
    assert_eq!(statuses(&report), [(Action::Move, ItemStatus::Failed)]);
    assert_eq!(
        report.results[0].error.as_ref().unwrap().kind,
        ErrorKind::PermissionDenied
    );
    assert_eq!(dst.memory().dump(), before, "rolled back");
    assert_eq!(src.read("src/f"), b"file");
    // Replacing a target: the copy stays.
    dst.file("dst/f", b"old");
    let opts = MoveOptions {
        overwrite: OverwritePolicy::Always,
        ..MoveOptions::default()
    };
    let report = mv(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("src/f"), dst.spec("dst/f"))],
        &opts,
    );
    assert_eq!(statuses(&report), [(Action::Move, ItemStatus::Failed)]);
    assert!(report.plan.items[0].replaces);
    assert_eq!(dst.read("dst/f"), b"file", "the copy is kept");
    assert_eq!(src.read("src/f"), b"file");
}

#[test]
fn a3_7_move_refusals() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    move_tree(&fx);
    fx.file("dst/f", b"exists");
    let report = mv(
        fx.vfs(),
        fx.vfs(),
        &[
            (mspec("/"), fx.spec("dst/root")),
            (fx.spec("src/tree"), fx.spec("src/tree/sub/x")),
            (fx.spec("src/f"), fx.spec("dst/f")),
        ],
        &MoveOptions::default(),
    );
    let messages: Vec<String> = report
        .results
        .iter()
        .map(|r| r.error.as_ref().unwrap().message.clone())
        .collect();
    assert!(messages[0].contains("is a root folder"), "{messages:?}");
    assert!(
        messages[1].contains("is inside the source folder"),
        "{messages:?}"
    );
    assert_eq!(
        report.results[2].error.as_ref().unwrap().kind,
        ErrorKind::AlreadyExists
    );
    assert_eq!(fx.read("dst/f"), b"exists");
    assert_eq!(fx.read("src/f"), b"file");
}

// ---- A3.8 remove -------------------------------------------------------------------------------------------

#[test]
fn a3_8_remove_table() {
    for fx in [
        Fx::mem(CaseSensitivity::Sensitive),
        Fx::mem(CaseSensitivity::Insensitive),
        Fx::local(),
    ] {
        let label = fx.label();
        fx.file("f", b"f");
        fx.file("target", b"target");
        let links = fx.link("l", "target", false);
        fx.dir("empty");
        fx.file("full/a", b"a");
        fx.file("full/sub/b", b"b");
        let none = RemoveOptions::default();
        let recursive = RemoveOptions {
            recursive: true,
            trash: false,
        };
        // File.
        let report = rm(fx.vfs(), &[fx.spec("f")], &none);
        assert_eq!(
            statuses(&report),
            [(Action::Remove, ItemStatus::Done)],
            "{label}"
        );
        assert_eq!(fx.kind("f"), None);
        // Link: removed as a link, its target stays.
        if links {
            let report = rm(fx.vfs(), &[fx.spec("l")], &none);
            assert_eq!(
                statuses(&report),
                [(Action::Remove, ItemStatus::Done)],
                "{label}"
            );
            assert_eq!(report.plan.items[0].kind, EntryKind::Symlink);
            assert_eq!(fx.kind("l"), None);
            assert_eq!(fx.read("target"), b"target");
        }
        // Empty folder.
        let report = rm(fx.vfs(), &[fx.spec("empty")], &none);
        assert_eq!(
            statuses(&report),
            [(Action::RemoveDir, ItemStatus::Done)],
            "{label}"
        );
        // Non-empty without recursive: an item error, nothing removed.
        let report = rm(fx.vfs(), &[fx.spec("full")], &none);
        assert_eq!(
            statuses(&report),
            [(Action::RemoveDir, ItemStatus::Failed)],
            "{label}"
        );
        assert_eq!(
            report.results[0].error.as_ref().unwrap().kind,
            ErrorKind::NotEmpty
        );
        assert_eq!(fx.read("full/sub/b"), b"b");
        // Dry run of the recursive removal: post-order, nothing removed.
        let plan =
            ops::plan_remove(fx.vfs(), &[fx.spec("full")], &recursive, &Ctl::detached()).unwrap();
        let order: Vec<(Action, Spec, u32)> = plan
            .items
            .iter()
            .map(|i| (i.action, i.target.clone(), i.depth))
            .collect();
        assert_eq!(
            order,
            [
                (Action::Remove, fx.spec("full/a"), 1),
                (Action::Remove, fx.spec("full/sub/b"), 2),
                (Action::RemoveDir, fx.spec("full/sub"), 1),
                (Action::RemoveDir, fx.spec("full"), 0),
            ],
            "{label}"
        );
        let dry = ops::dry_run(plan.clone());
        assert!(dry.results.iter().all(|r| r.status == ItemStatus::Planned));
        assert_eq!(fx.read("full/sub/b"), b"b");
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert_all_done(&report, label);
        assert_eq!(fx.kind("full"), None);
        // A missing item is an item error.
        let report = rm(fx.vfs(), &[fx.spec("missing")], &none);
        assert_eq!(
            report.results[0].error.as_ref().unwrap().kind,
            ErrorKind::NotFound
        );
    }
}

#[test]
fn a3_8_remove_refuses_a_root() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("x", b"x");
    let root = mspec("/");
    for opts in [
        RemoveOptions::default(),
        RemoveOptions {
            recursive: true,
            trash: false,
        },
        RemoveOptions {
            recursive: true,
            trash: true,
        },
    ] {
        let before = fx.memory().dump();
        let report = rm(fx.vfs(), std::slice::from_ref(&root), &opts);
        assert_eq!(report.results.len(), 1);
        let error = report.results[0].error.as_ref().unwrap();
        assert!(error.message.contains("is a root folder"), "{error:?}");
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert_eq!(fx.memory().dump(), before);
    }
    // Local: refused at planning, nothing is listed or removed.
    let local = LocalVfs::default();
    let root = Spec::Local(
        hikasa_core::LocalPath::new(std::path::PathBuf::from(common::host("/"))).unwrap(),
    );
    let plan = ops::plan_remove(
        &local,
        &[root],
        &RemoveOptions {
            recursive: true,
            trash: false,
        },
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(plan.items.len(), 1);
    assert!(
        plan.items[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("is a root folder")
    );
}

#[test]
fn a3_8_remove_keeps_a_folder_whose_child_failed() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("d/a", b"a");
    fx.file("d/b", b"b");
    fx.memory().inject(Fault {
        path: fx.text("d/a"),
        op: VfsOp::Remove,
        kind: ErrorKind::PermissionDenied,
    });
    let report = rm(
        fx.vfs(),
        &[fx.spec("d")],
        &RemoveOptions {
            recursive: true,
            trash: false,
        },
    );
    assert_eq!(
        statuses(&report),
        [
            (Action::Remove, ItemStatus::Failed),
            (Action::Remove, ItemStatus::Done),
            (Action::RemoveDir, ItemStatus::Skipped),
        ]
    );
    assert_eq!(report.results[2].skip, Some(SkipReason::ChildrenFailed));
    assert_eq!(fx.names("d"), ["a"]);
}

// ---- resolve_targets and nesting ---------------------------------------------------------------------------

fn blob() -> Blob {
    Blob {
        bytes: Arc::from(&b"clip"[..]),
        mtime: FileTime::from_unix(1_700_000_000, 0).unwrap(),
        name: None,
    }
}

#[test]
fn resolve_targets_table() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("src/a", b"a");
    fx.file("src/b", b"b");
    fx.dir("dst");
    fx.file("file", b"f");
    fx.link("ldst", "dst", true);
    let s = |rel: &str| fx.spec(rel);
    let both = [s("src/a"), s("src/b")];
    let one = [s("src/a")];
    let resolve = |sources: &[Spec], target: &str, into: bool, parents: bool| {
        ops::resolve_targets(fx.vfs(), sources, &s(target), into, parents)
    };
    // Existing folder.
    assert_eq!(
        resolve(&both, "dst", false, false).unwrap(),
        [(s("src/a"), s("dst/a")), (s("src/b"), s("dst/b"))]
    );
    assert_eq!(
        resolve(&both, "dst", true, false).unwrap(),
        [(s("src/a"), s("dst/a")), (s("src/b"), s("dst/b"))]
    );
    // A link to a folder is followed (the user named it).
    assert_eq!(
        resolve(&one, "ldst", false, false).unwrap(),
        [(s("src/a"), s("ldst/a"))]
    );
    // A new name; an existing file (the overwrite policy decides later).
    assert_eq!(
        resolve(&one, "dst/new", false, false).unwrap(),
        [(s("src/a"), s("dst/new"))]
    );
    assert_eq!(
        resolve(&one, "file", false, false).unwrap(),
        [(s("src/a"), s("file"))]
    );
    // A trailing separator: into a folder that is created with create_parents, else NotFound.
    assert_eq!(
        resolve(&both, "dst/newdir", true, true).unwrap(),
        [
            (s("src/a"), s("dst/newdir/a")),
            (s("src/b"), s("dst/newdir/b"))
        ]
    );
    assert!(matches!(
        resolve(&one, "dst/newdir", true, false),
        Err(VfsError::NotFound { .. })
    ));
    assert!(matches!(
        resolve(&one, "file", true, true),
        Err(VfsError::NotADirectory { .. })
    ));
    // Several sources into a non-folder.
    for target in ["dst/new", "file"] {
        let err = resolve(&both, target, false, true).unwrap_err();
        assert!(
            matches!(
                err,
                VfsError::InvalidTarget {
                    reason: "is not an existing folder",
                    ..
                }
            ),
            "{target}: {err:?}"
        );
    }
    assert_eq!(fx.kind("dst/newdir"), None, "resolving never writes");

    // A blob source into a folder needs a file name; as a new name it is fine.
    let dir = tempfile::tempdir().unwrap();
    let router = VfsRouter::new(LocalVfs::default()).with_clipboard(blob());
    let folder = common::lspec(dir.path());
    let err = ops::resolve_targets(&router, &[Spec::Clipboard], &folder, false, false).unwrap_err();
    assert!(
        matches!(
            err,
            VfsError::InvalidTarget {
                reason: "needs a file name",
                ..
            }
        ),
        "{err:?}"
    );
    let new = common::lspec_new(&dir.path().join("clip.txt"));
    assert_eq!(
        ops::resolve_targets(&router, &[Spec::Clipboard], &new, false, false).unwrap(),
        [(Spec::Clipboard, new.clone())]
    );
    // And the copy itself works across the router.
    let report = copy(&router, &router, &[(Spec::Clipboard, new)], &copy_opts());
    assert_all_done(&report, "clipboard copy");
    assert_eq!(std::fs::read(dir.path().join("clip.txt")).unwrap(), b"clip");
}

#[test]
fn nesting_checks_are_plan_item_errors() {
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let fx = Fx::mem(case);
        fx.file("src/tree/sub/x", b"x");
        fx.file("src/f", b"f");
        let before = fx.memory().dump();
        let mut pairs = vec![
            (fx.spec("src/tree"), fx.spec("src/tree/sub/copy")),
            (fx.spec("src/tree"), fx.spec("src/tree")),
            (fx.spec("src/f"), fx.spec("src/f")),
        ];
        if case == CaseSensitivity::Insensitive {
            pairs.push((fx.spec("src/tree"), fx.spec("SRC/Tree/new")));
            pairs.push((fx.spec("src/f"), fx.spec("SRC/F")));
        }
        let plan =
            ops::plan_copy(fx.vfs(), fx.vfs(), &pairs, &copy_opts(), &Ctl::detached()).unwrap();
        let reasons: Vec<String> = plan
            .items
            .iter()
            .map(|i| i.error.as_ref().unwrap().message.clone())
            .collect();
        let mut expected = vec![
            "is inside the source folder",
            "is the source itself",
            "is the source itself",
        ];
        if case == CaseSensitivity::Insensitive {
            expected.extend(["is inside the source folder", "is the source itself"]);
        }
        assert_eq!(reasons.len(), expected.len(), "{case:?}: {reasons:?}");
        for (reason, want) in reasons.iter().zip(expected) {
            assert!(reason.contains(want), "{case:?}: {reason} vs {want}");
        }
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert_eq!(report.summary.failed, report.summary.planned);
        assert_eq!(fx.memory().dump(), before, "{case:?}: nothing written");
    }
    // A case-sensitive back-end: `SRC/F` is another item.
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("src/f", b"f");
    fx.dir("SRC");
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/f"), fx.spec("SRC/F"))],
        &copy_opts(),
    );
    assert_all_done(&report, "sensitive");
}

// ---- dry run and cancellation ------------------------------------------------------------------------------

#[test]
fn dry_run_reports_the_plan_and_writes_nothing() {
    let pair = Pair::new(Setup::MemOther);
    let (s, d) = (&pair.src, pair.dst());
    let before = (s.memory().dump(), d.memory().dump());
    let plan = ops::plan_copy(
        s.vfs(),
        d.vfs(),
        &[
            (s.spec("src/tree"), d.spec("dst/tree")),
            (s.spec("src/missing"), d.spec("dst/missing")),
            (s.spec("src/f.txt"), d.spec("dst/f.txt")),
        ],
        &copy_opts(),
        &Ctl::detached(),
    )
    .unwrap();
    let report = ops::dry_run(plan.clone());
    assert_eq!(report.plan, plan);
    let got: Vec<ItemStatus> = report.results.iter().map(|r| r.status).collect();
    assert_eq!(
        got,
        [
            ItemStatus::Planned,
            ItemStatus::Planned,
            ItemStatus::Planned,
            ItemStatus::Planned,
            ItemStatus::Failed,
            ItemStatus::Planned,
        ]
    );
    assert_eq!(
        report.results[4].error.as_ref().unwrap().kind,
        ErrorKind::NotFound
    );
    assert_eq!(report.summary.planned, 6);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(report.summary.done, 0);
    assert_eq!(plan.bytes(), 12);
    assert!(!report.ok());
    assert!(!report.cancelled);
    assert_eq!((s.memory().dump(), d.memory().dump()), before);
}

/// Cancels its token at the first progress report.
struct CancelOnProgress(CancelToken);

impl ProgressSink for CancelOnProgress {
    fn report(&self, _progress: Progress) {
        self.0.cancel();
    }
}

#[test]
fn cancellation_fails_the_running_item_and_leaves_the_rest_not_run() {
    // Cancelled before execution: nothing runs.
    let pair = Pair::new(Setup::MemOther);
    let (s, d) = (&pair.src, pair.dst());
    let plan = || -> Plan {
        ops::plan_copy(
            s.vfs(),
            d.vfs(),
            &[(s.spec("src/tree"), d.spec("dst/tree"))],
            &copy_opts(),
            &Ctl::detached(),
        )
        .unwrap()
    };
    let before = d.memory().dump();
    let token = CancelToken::new();
    token.cancel();
    let report = ops::execute(s.vfs(), d.vfs(), plan(), &ctl(&token));
    assert!(report.cancelled);
    assert!(!report.ok());
    assert!(
        report
            .results
            .iter()
            .all(|r| r.status == ItemStatus::NotRun)
    );
    assert_eq!(d.memory().dump(), before);
    // Planning with a cancelled token is Err(Cancelled).
    assert!(matches!(
        ops::plan_copy(
            s.vfs(),
            d.vfs(),
            &[(s.spec("src/tree"), d.spec("dst/tree"))],
            &copy_opts(),
            &ctl(&token)
        ),
        Err(VfsError::Cancelled)
    ));

    // Cancelled while the first file streams: that item fails `cancelled`, its target untouched.
    let sink = CancelOnProgress(CancelToken::new());
    let token = sink.0.clone();
    let report = ops::execute(
        s.vfs(),
        d.vfs(),
        plan(),
        &Ctl {
            cancel: &token,
            progress: &sink,
        },
    );
    assert_eq!(
        statuses(&report),
        [
            (Action::CreateDir, ItemStatus::Done),
            (Action::Copy, ItemStatus::Failed),
            (Action::CreateDir, ItemStatus::NotRun),
            (Action::Copy, ItemStatus::NotRun),
        ]
    );
    assert_eq!(
        report.results[1].error.as_ref().unwrap().kind,
        ErrorKind::Cancelled
    );
    assert!(report.cancelled);
    assert!(!report.ok());
    assert_eq!(
        d.kind("dst/tree/a"),
        None,
        "the running item's target is untouched"
    );
    assert_eq!(d.names("dst/tree"), Vec::<String>::new());
}

// ---- W3 review fixes (§1.3.12) -----------------------------------------------------------------------------

/// The error message of item `i`.
fn message(report: &OpReport, i: usize) -> String {
    report.results[i]
        .error
        .as_ref()
        .map(|e| e.message.clone())
        .unwrap_or_default()
}

#[test]
fn fix_dot_names_and_roots_are_never_operated_on() {
    for fx in Fx::all() {
        let label = fx.label();
        fx.file("d/x", b"x");
        fx.file("f", b"f");
        let before = fx.names("d");
        // `rm`, `rm -r`, `rm --trash` of `d/..`, `d/x/..` and `d/.`-like specs: refused at planning.
        for opts in [
            RemoveOptions::default(),
            RemoveOptions {
                recursive: true,
                trash: false,
            },
            RemoveOptions {
                recursive: true,
                trash: true,
            },
        ] {
            let report = rm(fx.vfs(), &[fx.spec("d/.."), fx.spec("d/x/..")], &opts);
            assert_eq!(report.results.len(), 2, "{label} {opts:?}");
            for i in 0..2 {
                assert_eq!(report.results[i].status, ItemStatus::Failed, "{label}");
                assert!(
                    message(&report, i).ends_with("needs a file name"),
                    "{label}: {}",
                    message(&report, i)
                );
            }
        }
        assert_eq!(fx.names("d"), before, "{label}");
        assert!(fx.lstat("").is_some(), "{label}");
        // `mv d/.. e` and `cp f e/..`.
        let report = mv(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("d/.."), fx.spec("e"))],
            &MoveOptions::default(),
        );
        assert!(
            message(&report, 0).ends_with("needs a file name"),
            "{label}"
        );
        let report = mv(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("f"), fx.spec("d/.."))],
            &MoveOptions::default(),
        );
        assert!(
            message(&report, 0).ends_with("needs a file name"),
            "{label}"
        );
        let report = copy(
            fx.vfs(),
            fx.vfs(),
            &[(fx.spec("f"), fx.spec("d/.."))],
            &copy_opts(),
        );
        assert!(
            message(&report, 0).ends_with("needs a file name"),
            "{label}"
        );
        assert_eq!(fx.read("f"), b"f", "{label}");
        assert!(fx.lstat("e").is_none(), "{label}");
    }
}

#[cfg(unix)]
#[test]
fn fix_local_root_through_dot_dot_is_refused_at_planning() {
    // `/var/..` names `/`: planned only (never executed), refused for every remove form and as a move source.
    let local = LocalVfs::default();
    let spec = mspec("/var/..");
    for opts in [
        RemoveOptions::default(),
        RemoveOptions {
            recursive: true,
            trash: false,
        },
        RemoveOptions {
            recursive: false,
            trash: true,
        },
    ] {
        let plan =
            ops::plan_remove(&local, std::slice::from_ref(&spec), &opts, &Ctl::detached()).unwrap();
        assert_eq!(plan.items.len(), 1, "{opts:?}: {:?}", plan.items);
        let error = plan.items[0].error.as_ref().expect("refused");
        assert_eq!(error.kind, ErrorKind::InvalidInput);
        assert!(
            error.message.ends_with("needs a file name"),
            "{}",
            error.message
        );
    }
    let plan = ops::plan_remove(
        &local,
        &[mspec("/")],
        &RemoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert!(
        plan.items[0]
            .error
            .as_ref()
            .unwrap()
            .message
            .ends_with("is a root folder")
    );
    let plan = ops::plan_move(
        &local,
        &local,
        &[(spec, mspec("/var/hikasa-never"))],
        &MoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert!(plan.items[0].error.is_some(), "{:?}", plan.items);
}

#[cfg(unix)]
#[test]
fn fix_local_copy_of_a_read_only_source() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::local();
    fx.file("src/ro", b"read only");
    fx.file("src/tree/ro", b"inner");
    fx.dir("dst");
    for rel in ["src/ro", "src/tree/ro"] {
        std::fs::set_permissions(fx.path(rel), std::fs::Permissions::from_mode(0o444)).unwrap();
    }
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[
            (fx.spec("src/ro"), fx.spec("dst/ro")),
            (fx.spec("src/tree"), fx.spec("dst/tree")),
        ],
        &copy_opts(),
    );
    assert_all_done(&report, "read-only sources");
    assert_eq!(fx.read("dst/ro"), b"read only");
    assert_eq!(fx.read("dst/tree/ro"), b"inner");
    assert_eq!(mode_of(&fx, "dst/ro"), Some(0o444));
    assert_eq!(fx.names("dst"), ["ro", "tree"], "no temporary sibling left");
    // Replacing an existing target with a read-only source, and without preserving the mode.
    fx.file("dst/plain", b"old");
    let opts = CopyOptions {
        overwrite: OverwritePolicy::Always,
        preserve: Preserve::NONE,
        ..copy_opts()
    };
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src/ro"), fx.spec("dst/plain"))],
        &opts,
    );
    assert_all_done(&report, "replace");
    assert_eq!(fx.read("dst/plain"), b"read only");
}

#[test]
fn fix_failed_folder_replacement_keeps_the_target_file() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("src/a", b"a");
    fx.file("dst", b"precious");
    fx.memory().inject(Fault {
        path: fx.text("dst"),
        op: VfsOp::Mkdir,
        kind: ErrorKind::PermissionDenied,
    });
    let before = fx.memory().dump();
    let opts = CopyOptions {
        overwrite: OverwritePolicy::Always,
        ..copy_opts()
    };
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src"), fx.spec("dst"))],
        &opts,
    );
    assert_eq!(
        statuses(&report),
        [
            (Action::CreateDir, ItemStatus::Failed),
            (Action::Copy, ItemStatus::NotRun),
        ]
    );
    assert_eq!(fx.read("dst"), b"precious");
    assert_eq!(
        fx.memory().dump(),
        before,
        "the target and the tree are as before"
    );
    // Without the fault the folder replaces the file.
    fx.memory().clear_faults();
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("src"), fx.spec("dst"))],
        &opts,
    );
    assert_all_done(&report, "replace");
    assert_eq!(fx.read("dst/a"), b"a");
    assert_eq!(fx.names(""), ["dst", "src"], "no sibling left");
}

#[test]
fn fix_nothing_runs_below_a_failed_folder_item() {
    // Copy and move into another back-end: the folder `sub` cannot be created, so nothing below it runs, the
    // later siblings do, and a move keeps every source below it.
    for moving in [false, true] {
        let src = Fx::mem(CaseSensitivity::Sensitive);
        let dst = Fx::mem(CaseSensitivity::Sensitive);
        src.file("t/sub/deep/x", b"x");
        src.file("t/sub/y", b"y");
        src.file("t/z", b"z");
        dst.dir("out");
        dst.memory().inject(Fault {
            path: dst.text("out/t/sub"),
            op: VfsOp::Mkdir,
            kind: ErrorKind::PermissionDenied,
        });
        let pairs = [(src.spec("t"), dst.spec("out/t"))];
        let report = if moving {
            mv(src.vfs(), dst.vfs(), &pairs, &MoveOptions::default())
        } else {
            copy(src.vfs(), dst.vfs(), &pairs, &copy_opts())
        };
        let got: Vec<(String, ItemStatus)> = report
            .plan
            .items
            .iter()
            .zip(&report.results)
            .map(|(i, r)| {
                let own = i.source.as_ref().unwrap_or(&i.target).canonical();
                (own.rsplit('/').next().unwrap().to_owned(), r.status)
            })
            .filter(|(_, s)| *s != ItemStatus::Done)
            .collect();
        let mut want = vec![
            ("sub".to_owned(), ItemStatus::Failed),
            ("deep".to_owned(), ItemStatus::NotRun),
            ("x".to_owned(), ItemStatus::NotRun),
        ];
        if moving {
            want.push(("deep".to_owned(), ItemStatus::NotRun));
        }
        want.push(("y".to_owned(), ItemStatus::NotRun));
        if moving {
            // `remove_dir` of `sub` (its creation failed) and of `t` (a child was not moved).
            want.push(("sub".to_owned(), ItemStatus::Skipped));
            want.push(("t".to_owned(), ItemStatus::Skipped));
        }
        assert_eq!(got, want, "moving {moving}");
        assert_eq!(dst.read("out/t/z"), b"z");
        assert!(dst.lstat("out/t/sub").is_none());
        if moving {
            assert_eq!(src.read("t/sub/deep/x"), b"x", "sources below kept");
            assert_eq!(src.read("t/sub/y"), b"y");
        }
        assert!(!report.ok());
    }
}

#[test]
fn fix_case_only_rename_through_mv() {
    let fx = Fx::mem(CaseSensitivity::Insensitive);
    fx.file("File.txt", b"x");
    fx.file("Dir/inner", b"i");
    for (from, to) in [("File.txt", "file.txt"), ("Dir", "dir")] {
        let pairs =
            ops::resolve_targets(fx.vfs(), &[fx.spec(from)], &fx.spec(to), false, false).unwrap();
        assert_eq!(pairs, [(fx.spec(from), fx.spec(to))], "{from}");
        let report = mv(fx.vfs(), fx.vfs(), &pairs, &MoveOptions::default());
        assert_eq!(
            statuses(&report),
            [(Action::Rename, ItemStatus::Done)],
            "{from}"
        );
    }
    assert_eq!(fx.names(""), ["dir", "file.txt"]);
    assert_eq!(fx.read("dir/inner"), b"i");
    // A copy onto itself spelt differently is still "the source itself".
    let report = copy(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("file.txt"), fx.spec("FILE.txt"))],
        &copy_opts(),
    );
    assert!(
        message(&report, 0).ends_with("is the source itself"),
        "{}",
        message(&report, 0)
    );
    // Case-sensitive: two different items, the usual rules.
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    fx.file("File.txt", b"x");
    let report = mv(
        fx.vfs(),
        fx.vfs(),
        &[(fx.spec("File.txt"), fx.spec("file.txt"))],
        &MoveOptions::default(),
    );
    assert!(report.ok());
    assert_eq!(fx.names(""), ["file.txt"]);
}

/// A `MemoryVfs` whose `remove` removes the item and then reports a failure (the source disappears while the
/// move fails to remove it, as when another process removed it first).
struct RemoveThenFail<'a>(&'a MemoryVfs);

impl Vfs for RemoveThenFail<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.0.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<hikasa_vfs::Entry, VfsError> {
        self.0.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &hikasa_vfs::ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<hikasa_vfs::Listing, VfsError> {
        self.0.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn hikasa_vfs::ReadSeek>, VfsError> {
        self.0.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, VfsError> {
        self.0.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &hikasa_vfs::FileName) -> Result<Spec, VfsError> {
        self.0.child(dir, name)
    }
    fn local_path(&self, spec: &Spec) -> Option<std::path::PathBuf> {
        self.0.local_path(spec)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.0.remove(spec)?;
        Err(VfsError::from_io(
            spec.canonical(),
            VfsOp::Remove,
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ))
    }
}

#[test]
fn fix_move_keeps_the_copy_when_the_source_is_already_gone() {
    let src = Fx::mem(CaseSensitivity::Sensitive);
    let dst = Fx::mem(CaseSensitivity::Sensitive);
    src.file("f", b"only copy");
    dst.dir("out");
    let wrapper = RemoveThenFail(src.memory());
    let report = mv(
        &wrapper,
        dst.vfs(),
        &[(src.spec("f"), dst.spec("out/f"))],
        &MoveOptions::default(),
    );
    assert_eq!(statuses(&report), [(Action::Move, ItemStatus::Failed)]);
    assert!(
        message(&report, 0).ends_with("changed during the move; the copy was kept"),
        "{}",
        message(&report, 0)
    );
    assert!(src.lstat("f").is_none());
    assert_eq!(dst.read("out/f"), b"only copy", "the only copy is kept");
}

#[test]
fn fix_newer_is_decided_again_at_execution() {
    for fx in [Fx::mem(CaseSensitivity::Sensitive), Fx::local()] {
        let label = fx.label();
        fx.file("src/f", b"new");
        fx.file("dst/f", b"old");
        meta(&fx, "src/f", 1_600_000_100, 0o644);
        meta(&fx, "dst/f", 1_600_000_000, 0o644);
        let opts = CopyOptions {
            overwrite: OverwritePolicy::Newer,
            ..copy_opts()
        };
        let pairs = [(fx.spec("src/f"), fx.spec("dst/f"))];
        let plan = ops::plan_copy(fx.vfs(), fx.vfs(), &pairs, &opts, &Ctl::detached()).unwrap();
        assert!(plan.items[0].replaces, "{label}");
        // The target became newer between planning and execution.
        meta(&fx, "dst/f", 1_600_000_200, 0o644);
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert_eq!(report.results[0].status, ItemStatus::Skipped, "{label}");
        assert_eq!(
            report.results[0].skip,
            Some(SkipReason::NotNewer),
            "{label}"
        );
        assert_eq!(fx.read("dst/f"), b"old", "{label}");
        // The same for a move.
        let plan = ops::plan_move(
            fx.vfs(),
            fx.vfs(),
            &pairs,
            &MoveOptions {
                overwrite: OverwritePolicy::Newer,
                create_parents: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        assert_eq!(plan.items[0].skip, Some(SkipReason::NotNewer), "{label}");
        meta(&fx, "dst/f", 1_600_000_000, 0o644);
        let plan = ops::plan_move(
            fx.vfs(),
            fx.vfs(),
            &pairs,
            &MoveOptions {
                overwrite: OverwritePolicy::Newer,
                create_parents: false,
            },
            &Ctl::detached(),
        )
        .unwrap();
        assert!(plan.items[0].replaces, "{label}");
        meta(&fx, "dst/f", 1_600_000_300, 0o644);
        let report = ops::execute(fx.vfs(), fx.vfs(), plan, &Ctl::detached());
        assert_eq!(
            report.results[0].skip,
            Some(SkipReason::NotNewer),
            "{label}"
        );
        assert_eq!(fx.read("src/f"), b"new", "{label}");
        assert_eq!(fx.read("dst/f"), b"old", "{label}");
    }
}

#[test]
fn fix_source_without_mtime_gives_one_capability_warning() {
    let src = Fx::mem(CaseSensitivity::Sensitive);
    let dst = Fx::mem(CaseSensitivity::Sensitive);
    src.file("a", b"a");
    src.file("b", b"b");
    dst.dir("out");
    let report = copy(
        src.vfs(),
        dst.vfs(),
        &[
            (src.spec("a"), dst.spec("out/a")),
            (src.spec("b"), dst.spec("out/b")),
        ],
        &copy_opts(),
    );
    assert_all_done(&report, "no mtime");
    let warnings: Vec<_> = report
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::CapabilityMissing)
        .collect();
    assert_eq!(warnings.len(), 1, "{:?}", report.warnings);
    assert!(
        warnings[0].message.contains("`mtime`"),
        "{}",
        warnings[0].message
    );
    // With times not preserved: no warning.
    let opts = CopyOptions {
        preserve: Preserve::NONE,
        overwrite: OverwritePolicy::Always,
        ..copy_opts()
    };
    let report = copy(
        src.vfs(),
        dst.vfs(),
        &[(src.spec("a"), dst.spec("out/a"))],
        &opts,
    );
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn fix_blob_targets_are_refused_before_anything_is_read() {
    let router = VfsRouter::new(LocalVfs::default());
    for target in [Spec::Stdin, Spec::Clipboard] {
        for into in [false, true] {
            let err = ops::resolve_targets(&router, &[mspec("/nowhere/a")], &target, into, false)
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    VfsError::OperationUnsupported {
                        op: VfsOp::Write,
                        ..
                    }
                ),
                "{target:?}: {err:?}"
            );
            assert!(!err.to_string().contains("in this version"), "{err}");
        }
    }
    // Removing, creating or touching a blob: an item error `unsupported` (never "not supported in this version").
    let report = rm(
        &router,
        &[Spec::Clipboard, Spec::Stdin],
        &RemoveOptions::default(),
    );
    for i in 0..2 {
        assert_eq!(
            report.results[i].error.as_ref().unwrap().kind,
            ErrorKind::Unsupported
        );
        assert!(!message(&report, i).contains("in this version"));
    }
    let plan = ops::plan_mkdir(&router, &[Spec::Clipboard], true);
    assert_eq!(
        plan.items[0].error.as_ref().unwrap().kind,
        ErrorKind::Unsupported
    );
}

/// Counts `Phase::Scanning` reports; cancels `token` at the `at`-th one.
struct ScanSink {
    count: AtomicU64,
    at: u64,
    token: CancelToken,
}

impl ProgressSink for ScanSink {
    fn report(&self, progress: Progress) {
        assert_eq!(progress.phase, Phase::Scanning);
        assert_eq!(progress.unit, hikasa_core::Unit::Items);
        assert!(progress.current.is_some());
        if self.count.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.token.cancel();
        }
    }
}

#[test]
fn fix_planning_reports_scanning_progress_and_is_cancellable() {
    let fx = Fx::mem(CaseSensitivity::Sensitive);
    for i in 0..40 {
        fx.file(&format!("src/d{}/f{i}", i % 4), b"x");
    }
    fx.dir("dst");
    let run = |at: u64, which: u8| {
        let sink = ScanSink {
            count: AtomicU64::new(0),
            at,
            token: CancelToken::new(),
        };
        let ctl = Ctl {
            cancel: &sink.token,
            progress: &sink,
        };
        let result = match which {
            0 => ops::plan_copy(
                fx.vfs(),
                fx.vfs(),
                &[(fx.spec("src"), fx.spec("dst/src"))],
                &copy_opts(),
                &ctl,
            ),
            1 => ops::plan_move(
                fx.vfs(),
                fx.vfs(),
                &[(fx.spec("src"), fx.spec("dst"))],
                &MoveOptions::default(),
                &ctl,
            ),
            _ => ops::plan_remove(
                fx.vfs(),
                &[fx.spec("src")],
                &RemoveOptions {
                    recursive: true,
                    trash: false,
                },
                &ctl,
            ),
        };
        (result, sink.count.load(Ordering::Relaxed))
    };
    for which in [0, 2] {
        let (plan, reports) = run(u64::MAX, which);
        assert!(plan.is_ok());
        assert!(reports >= 40, "{which}: {reports} scanning reports");
        // Cancelled during the scan of the tree: `Err(Cancelled)`, nothing written.
        let (plan, _) = run(10, which);
        assert!(
            matches!(plan, Err(VfsError::Cancelled)),
            "{which}: {plan:?}"
        );
    }
    let (plan, reports) = run(u64::MAX, 1);
    assert!(plan.is_ok() && reports >= 1);
    assert_eq!(fx.names("dst"), Vec::<String>::new());
}
