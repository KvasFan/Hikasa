//! M2 W6a acceptance A6.1 (batch plans, D-162), A6.2 (stamps, D-163), A6.4 (exchange, D-165), A6.5 (the fence, D-166), and
//! the bits of durability a back-end records (A6.3, D-164). Core logic runs on `MemoryVfs`; contract M2 §1.6.2–§1.6.4.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};

use common::archives::{OnMemory, TreeData, bytes_of, on_disk, on_memory};
use common::json::{self, J};
use common::{ctl, ft, host, mspec, read_to_vec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, Phase, Progress, ProgressSink, Spec,
};
use hikasa_vfs::ops::{
    self, Action, BatchOptions, CopyOptions, ExecOptions, Expect, Fence, ItemStamp, ItemStatus,
    OpKind, OpReport, Plan, PlanItem, PlanOptions, Preserve, SkipReason, within,
};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, AtomicWrite, Attrs, Capabilities, Durability, Entry, EntryKind,
    Fault, FileName, Finish, FollowLinks, ListOptions, Listing, LocalVfs, MemDump, MemMeta,
    MemoryVfs, ReadSeek, SetTimes, UnixMeta, Vfs, VfsError, VfsOp, WriteOptions,
};

// ---- helpers -----------------------------------------------------------------------------------------------

const CHANGED: &str = "changed since the plan was made";
const OUTSIDE: &str = "is outside the base folders";

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: Some(ft(secs)),
        ..MemMeta::default()
    }
}

fn memory() -> MemoryVfs {
    MemoryVfs::new(CaseSensitivity::Sensitive)
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content, meta(secs)).unwrap();
}

fn batch(items: Vec<PlanItem>) -> Plan {
    batch_with(items, BatchOptions::default())
}

fn batch_with(items: Vec<PlanItem>, options: BatchOptions) -> Plan {
    Plan {
        op: OpKind::Batch,
        options: PlanOptions::Batch(options),
        items,
        warnings: Vec::new(),
    }
}

fn run(vfs: &dyn Vfs, plan: Plan) -> OpReport {
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn run_with(vfs: &dyn Vfs, plan: Plan, options: &ExecOptions) -> OpReport {
    ops::execute_with(vfs, vfs, plan, options, &Ctl::detached())
}

fn item(
    action: Action,
    kind: EntryKind,
    source: Option<&str>,
    target: &str,
    depth: u32,
) -> PlanItem {
    PlanItem::planned(action, kind, source.map(mspec), mspec(target), depth)
}

fn copy_item(source: &str, target: &str, depth: u32) -> PlanItem {
    item(Action::Copy, EntryKind::File, Some(source), target, depth)
}

fn statuses(report: &OpReport) -> Vec<ItemStatus> {
    report.results.iter().map(|r| r.status).collect()
}

fn dump_of(vfs: &MemoryVfs, path: &str) -> MemDump {
    let path = host(path);
    vfs.dump()
        .into_iter()
        .find(|d| d.path == path)
        .unwrap_or_else(|| panic!("no {path}"))
}

fn content(vfs: &MemoryVfs, path: &str) -> Vec<u8> {
    vfs.read_back(&host(path)).unwrap().to_vec()
}

/// `(status, message)` of a failed item.
fn failure(report: &OpReport, index: usize) -> String {
    report.results[index]
        .error
        .as_ref()
        .map(|e| e.message.clone())
        .unwrap_or_default()
}

// ---- A6.1 batches ------------------------------------------------------------------------------------------

/// A hand-built batch mixing every action runs in plan order, one journal.
#[test]
fn a6_1_a_batch_runs_in_plan_order() {
    let vfs = memory();
    file(&vfs, "/l/a", b"a", 100);
    file(&vfs, "/l/b", b"b", 100);
    file(&vfs, "/r/gone", b"x", 100);
    file(&vfs, "/r/t", b"t", 100);
    vfs.insert_dir(&host("/r/old"), MemMeta::default()).unwrap();
    vfs.insert_symlink(&host("/l/link"), "a", MemMeta::default())
        .unwrap();
    vfs.set_journal(true);
    let mut touch = item(Action::Touch, EntryKind::File, None, "/r/t", 0);
    touch.times = Some(SetTimes {
        modified: Some(ft(777)),
        created: None,
    });
    let plan = batch(vec![
        item(Action::CreateDir, EntryKind::Dir, None, "/r/new", 0),
        copy_item("/l/a", "/r/new/a", 1),
        item(
            Action::Link,
            EntryKind::Symlink,
            Some("/l/link"),
            "/r/link",
            0,
        ),
        item(Action::Move, EntryKind::File, Some("/l/b"), "/r/b", 0),
        item(Action::Remove, EntryKind::File, None, "/r/gone", 0),
        item(Action::RemoveDir, EntryKind::Dir, None, "/r/old", 0),
        touch,
    ]);
    let report = run(&vfs, plan);
    assert!(report.ok(), "{:?}", report.results);
    let journal: Vec<(VfsOp, String)> = vfs.journal();
    let order: Vec<VfsOp> = journal.iter().map(|(op, _)| *op).collect();
    assert_eq!(
        order,
        [
            VfsOp::Mkdir,
            VfsOp::Commit, // a
            VfsOp::SetTimes,
            VfsOp::Symlink,
            VfsOp::Commit, // b
            VfsOp::SetTimes,
            VfsOp::Remove, // the source of the move
            VfsOp::Remove,
            VfsOp::Remove,
            VfsOp::SetTimes, // the touch
        ],
        "{journal:?}"
    );
    assert_eq!(dump_of(&vfs, "/r/t").mtime, Some(ft(777)));
    assert_eq!(content(&vfs, "/r/new/a"), b"a");
    assert_eq!(report.plan.op.as_str(), "batch");
}

/// The bytes of a batch: both sizes of an `exchange` count.
#[test]
fn a6_1_bytes_count_both_sides_of_an_exchange() {
    let vfs = memory();
    file(&vfs, "/l/x", b"12345", 1);
    file(&vfs, "/r/x", b"123", 2);
    let mut swap = item(Action::Exchange, EntryKind::File, Some("/l/x"), "/r/x", 0);
    swap.size = Some(5);
    swap.replaces = true;
    swap.expect = Some(Expect {
        source: stamp(&vfs, "/l/x"),
        target: stamp(&vfs, "/r/x"),
    });
    let plan = batch(vec![swap]);
    assert_eq!(plan.bytes(), 8);
}

/// `times` of an item win, and a `create` of a batch takes them too.
#[test]
fn a6_1_times_per_item() {
    let vfs = memory();
    file(&vfs, "/a", b"a", 1);
    file(&vfs, "/b", b"b", 1);
    let at = |secs| {
        Some(SetTimes {
            modified: Some(ft(secs)),
            created: None,
        })
    };
    let mut first = item(Action::Touch, EntryKind::File, None, "/a", 0);
    first.times = at(10);
    let mut second = item(Action::Touch, EntryKind::File, None, "/b", 0);
    second.times = at(20);
    let mut third = item(Action::Create, EntryKind::File, None, "/c", 0);
    third.times = at(30);
    assert!(run(&vfs, batch(vec![first, second, third])).ok());
    assert_eq!(dump_of(&vfs, "/a").mtime, Some(ft(10)));
    assert_eq!(dump_of(&vfs, "/b").mtime, Some(ft(20)));
    assert_eq!(dump_of(&vfs, "/c").mtime, Some(ft(30)));
}

/// `Preserve` of `BatchOptions`: copies follow it, moves keep the default.
#[test]
fn a6_1_preserve_of_the_options() {
    let vfs = memory();
    let unix = Attrs::Unix {
        meta: UnixMeta {
            mode: 0o640,
            uid: 0,
            gid: 0,
        },
    };
    let meta = MemMeta {
        mtime: Some(ft(100)),
        ctime: Some(ft(50)),
        attrs: Some(unix),
        ..MemMeta::default()
    };
    vfs.insert_file(&host("/a"), &b"a"[..], meta.clone())
        .unwrap();
    vfs.insert_file(&host("/m"), &b"m"[..], meta.clone())
        .unwrap();
    let options = BatchOptions {
        preserve: Preserve::NONE,
        ..BatchOptions::default()
    };
    let plan = batch_with(
        vec![
            copy_item("/a", "/plain", 0),
            item(Action::Move, EntryKind::File, Some("/m"), "/moved", 0),
        ],
        options,
    );
    assert!(run(&vfs, plan).ok());
    assert_eq!(dump_of(&vfs, "/plain").mtime, None);
    assert_eq!(
        dump_of(&vfs, "/moved").mtime,
        Some(ft(100)),
        "moves keep it"
    );
    let created = BatchOptions {
        preserve: Preserve {
            created: true,
            ..Preserve::default()
        },
        ..BatchOptions::default()
    };
    file(&vfs, "/e", b"e", 1);
    let plan = batch_with(vec![copy_item("/a", "/kept", 0)], created);
    assert!(run(&vfs, plan).ok());
    assert_eq!(dump_of(&vfs, "/kept").ctime, Some(ft(50)));
    assert_eq!(dump_of(&vfs, "/kept").attrs, unix);
}

/// A failed folder cuts its subtree and no sibling root.
#[test]
fn a6_1_a_failed_folder_cuts_its_subtree_only() {
    let vfs = memory();
    file(&vfs, "/l/a", b"a", 1);
    file(&vfs, "/l/b", b"b", 1);
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
    vfs.inject(Fault {
        path: host("/r/d"),
        op: VfsOp::Mkdir,
        kind: ErrorKind::PermissionDenied,
    });
    let report = run(
        &vfs,
        batch(vec![
            item(Action::CreateDir, EntryKind::Dir, None, "/r/d", 0),
            copy_item("/l/a", "/r/d/a", 1),
            copy_item("/l/b", "/r/b", 0),
        ]),
    );
    assert_eq!(
        statuses(&report),
        [ItemStatus::Failed, ItemStatus::NotRun, ItemStatus::Done]
    );
    assert_eq!(content(&vfs, "/r/b"), b"b");
}

/// A declined `skip` blocks the `remove_dir` of its parent.
#[test]
fn a6_1_a_declined_skip_blocks_its_parent() {
    let vfs = memory();
    file(&vfs, "/r/d/keep", b"k", 1);
    file(&vfs, "/r/d/go", b"g", 1);
    let mut declined = item(Action::Skip, EntryKind::File, None, "/r/d/keep", 1);
    declined.skip = Some(SkipReason::Declined);
    let report = run(
        &vfs,
        batch(vec![
            declined,
            item(Action::Remove, EntryKind::File, None, "/r/d/go", 1),
            item(Action::RemoveDir, EntryKind::Dir, None, "/r/d", 0),
        ]),
    );
    assert_eq!(
        statuses(&report),
        [ItemStatus::Skipped, ItemStatus::Done, ItemStatus::Skipped]
    );
    assert_eq!(report.results[0].skip, Some(SkipReason::Declined));
    assert_eq!(report.results[2].skip, Some(SkipReason::ChildrenFailed));
    assert_eq!(content(&vfs, "/r/d/keep"), b"k");
    assert!(report.ok());
}

/// `moves` gives two-phase behaviour into an archive: an empty source folder waits for the rebuild as a W3 move's does.
#[test]
fn a6_1_moves_are_two_phase_into_an_archive() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![("keep.txt".into(), b"k".to_vec())];
    let archive = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    memory
        .mem
        .insert_file(&host("/from/x"), &b"x"[..], meta(5))
        .unwrap();
    let vfs: &dyn Vfs = &*memory.vfs;
    let plan = |memory: &OnMemory| {
        let target = Spec::archive_member(archive.clone(), "x");
        let mut folder = PlanItem::planned(
            Action::KeepDir,
            EntryKind::Dir,
            Some(mspec("/from")),
            Spec::archive_member(archive.clone(), ""),
            0,
        );
        folder.expect = None;
        let moved = PlanItem::planned(
            Action::Move,
            EntryKind::File,
            Some(mspec("/from/x")),
            target,
            1,
        );
        let gone = PlanItem::planned(Action::RemoveDir, EntryKind::Dir, None, mspec("/from"), 0);
        let _ = memory;
        batch_with(
            vec![folder, moved, gone],
            BatchOptions {
                moves: true,
                ..BatchOptions::default()
            },
        )
    };
    // A rebuild that fails leaves the sources: nothing was removed before it.
    memory.mem.inject(Fault {
        path: host("/a.zip"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    let report = run(vfs, plan(&memory));
    assert!(!report.ok());
    assert_eq!(
        memory.mem.read_back(&host("/from/x")).unwrap().as_ref(),
        b"x"
    );
    memory.mem.clear_faults();
    let report = run(vfs, plan(&memory));
    assert!(report.ok(), "{:?}", report.results);
    assert!(memory.mem.read_back(&host("/from/x")).is_none());
    assert!(
        vfs.stat(&mspec("/from"), FollowLinks::No).is_err(),
        "the folder went after the rebuild"
    );
    let member = Spec::archive_member(archive.clone(), "x");
    assert_eq!(read_to_vec(vfs.open_read(&member).unwrap()), b"x");
}

/// `execute` is `execute_with` with default options.
#[test]
fn a6_1_execute_equals_execute_with_defaults() {
    let build = || {
        let vfs = memory();
        file(&vfs, "/a", b"a", 1);
        vfs
    };
    let options = CopyOptions::default();
    let pairs = [(mspec("/a"), mspec("/b"))];
    let (one, two) = (build(), build());
    let plan = ops::plan_copy(&one, &one, &pairs, &options, &Ctl::detached()).unwrap();
    let plain = ops::execute(&one, &one, plan.clone(), &Ctl::detached());
    let with = run_with(&two, plan, &ExecOptions::default());
    assert_eq!(plain, with);
    assert_eq!(one.dump(), two.dump());
}

/// `PlanItem` round-trips through JSON with and without the new fields; a W3 plan text reads.
#[test]
fn a6_1_plan_items_round_trip() {
    let mut full = copy_item("/a", "/b", 2);
    full.expect = Some(Expect {
        source: Some(ItemStamp {
            kind: EntryKind::File,
            size: Some(3),
            mtime: Some(ft(9)),
        }),
        target: None,
    });
    full.backup = Some(mspec("/b.bak"));
    full.times = Some(SetTimes {
        modified: Some(ft(1)),
        created: None,
    });
    full.follow_source = true;
    full.clear_read_only = true;
    full.trash_fallback = true;
    let plain = copy_item("/c", "/d", 0);
    for one in [&full, &plain] {
        let back: PlanItem = json::from_value(json::to_value(one)).unwrap();
        assert_eq!(&back, one);
    }
    let text = json::to_pretty(&plain);
    for key in [
        "expect",
        "backup",
        "times",
        "follow_source",
        "clear_read_only",
        "trash_fallback",
    ] {
        assert!(!text.contains(key), "{key} in {text}");
    }
    let old = J::Obj(vec![
        ("action".into(), J::Str("copy".into())),
        ("kind".into(), J::Str("file".into())),
        ("source".into(), J::Str(mspec("/c").canonical())),
        ("target".into(), J::Str(mspec("/d").canonical())),
        ("size".into(), J::U(1)),
        ("replaces".into(), J::Bool(false)),
        ("skip".into(), J::Null),
        ("error".into(), J::Null),
        ("depth".into(), J::U(0)),
    ]);
    let read: PlanItem = json::from_value(old).unwrap();
    assert_eq!(read.expect, None);
    assert_eq!(read.size, Some(1));
    assert!(!read.follow_source);
    assert_eq!(OpKind::Batch.as_str(), "batch");
    assert_eq!(Action::Exchange.as_str(), "exchange");
    assert!(!Action::Exchange.needs_absent_target());
    let words = [
        (SkipReason::Declined, "declined"),
        (SkipReason::Filtered, "filtered"),
        (SkipReason::CompareError, "compare_error"),
        (SkipReason::FollowedLink, "followed_link"),
        (SkipReason::ReadOnlySide, "read_only_side"),
        (SkipReason::Capability, "capability"),
        (SkipReason::DuplicateTarget, "duplicate_target"),
        (SkipReason::NoMatch, "no_match"),
        (SkipReason::SameName, "same_name"),
    ];
    for (reason, word) in words {
        assert_eq!(reason.as_str(), word);
        assert_eq!(json::to_value(&reason), J::Str(word.into()));
    }
}

/// A `Vfs` that panics in `remove`: a panic inside `execute_with` aborts the batches.
struct PanicOnRemove<'a>(&'a dyn Vfs);

impl Vfs for PanicOnRemove<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.0.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.0.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.0.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.0.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.0.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.0.child(dir, name)
    }
    fn remove(&self, _spec: &Spec) -> Result<(), VfsError> {
        panic!("a panic inside execute_with")
    }
    fn begin(&self, at: &Spec) -> Result<(), VfsError> {
        self.0.begin(at)
    }
    fn finish(&self, at: &Spec, how: Finish) -> Result<(), VfsError> {
        self.0.finish(at, how)
    }
    fn finish_with(&self, at: &Spec, how: Finish, ctl: &Ctl<'_>) -> Result<(), VfsError> {
        self.0.finish_with(at, how, ctl)
    }
}

#[test]
fn a6_1_a_panic_aborts_the_batches() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec()), ("b".into(), b"b".to_vec())];
    let bytes = bytes_of(ArchiveFormat::Zip, &tree);
    let archive = memory.put("a.zip", bytes.clone());
    let wrapper = PanicOnRemove(&*memory.vfs);
    let plan = batch(vec![PlanItem::planned(
        Action::Remove,
        EntryKind::File,
        None,
        Spec::archive_member(archive.clone(), "a"),
        0,
    )]);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        ops::execute_with(
            &wrapper,
            &wrapper,
            plan,
            &ExecOptions::default(),
            &Ctl::detached(),
        )
    }));
    assert!(outcome.is_err());
    assert_eq!(
        memory.mem.read_back(&host("/a.zip")).unwrap().as_ref(),
        bytes.as_slice()
    );
    // No batch is left behind: the next one opens, and sees every member.
    let vfs: &dyn Vfs = &*memory.vfs;
    vfs.begin(&archive).unwrap();
    assert!(
        vfs.stat(&Spec::archive_member(archive.clone(), "a"), FollowLinks::No)
            .is_ok()
    );
    vfs.finish(&archive, Finish::Abort).unwrap();
}

// ---- A6.2 stamps ---------------------------------------------------------------------------------------------

/// The stamp of an item as a planner takes it.
fn stamp(vfs: &dyn Vfs, path: &str) -> Option<ItemStamp> {
    let entry = vfs.stat(&mspec(path), FollowLinks::No).unwrap();
    Some(ItemStamp {
        kind: entry.kind,
        size: (entry.kind != EntryKind::Dir).then_some(entry.size),
        mtime: entry.mtime,
    })
}

fn stamped(vfs: &dyn Vfs, source: &str, target: &str) -> PlanItem {
    let mut one = copy_item(source, target, 0);
    one.replaces = true;
    one.expect = Some(Expect {
        source: stamp(vfs, source),
        target: stamp(vfs, target),
    });
    one
}

/// Every way a stamp stops holding, on either side: the item fails, its target byte for byte, other items run.
#[test]
fn a6_2_a_changed_item_fails_and_others_run() {
    type Change = (&'static str, fn(&MemoryVfs, &str));
    let changes: [Change; 4] = [
        ("size", |vfs, at| file(vfs, at, b"longer content", 100)),
        ("time", |vfs, at| file(vfs, at, b"same!", 101)),
        ("kind", |vfs, at| {
            vfs.remove(&mspec(at)).unwrap();
            vfs.insert_symlink(&host(at), "elsewhere", meta(100))
                .unwrap();
        }),
        ("gone", |vfs, at| vfs.remove(&mspec(at)).unwrap()),
    ];
    for side in ["/s", "/t"] {
        for (what, change) in changes {
            let vfs = memory();
            file(&vfs, "/s", b"sssss", 100);
            file(&vfs, "/t", b"ttttt", 100);
            file(&vfs, "/other", b"o", 100);
            let plan = batch(vec![
                stamped(&vfs, "/s", "/t"),
                copy_item("/other", "/copy", 0),
            ]);
            change(&vfs, side);
            let before = vfs.dump();
            let report = run(&vfs, plan);
            assert_eq!(
                statuses(&report),
                [ItemStatus::Failed, ItemStatus::Done],
                "{what} of {side}"
            );
            let error = report.results[0].error.as_ref().unwrap();
            assert!(error.message.contains(CHANGED), "{}", error.message);
            assert_eq!(
                error.spec.as_deref(),
                Some(mspec(side).canonical().as_str()),
                "{what} of {side}"
            );
            let after: Vec<_> = vfs
                .dump()
                .into_iter()
                .filter(|d| d.path != host("/copy"))
                .collect();
            assert_eq!(after, before, "{what} of {side}");
            assert_eq!(content(&vfs, "/copy"), b"o");
        }
    }
}

/// A new item where none was stamped: D-103's absent-target check stays.
#[test]
fn a6_2_a_new_item_where_none_was_stamped() {
    let vfs = memory();
    file(&vfs, "/s", b"s", 100);
    let mut one = copy_item("/s", "/t", 0);
    one.expect = Some(Expect {
        source: stamp(&vfs, "/s"),
        target: None,
    });
    let plan = batch(vec![one]);
    file(&vfs, "/t", b"new", 200);
    let report = run(&vfs, plan);
    assert_eq!(statuses(&report), [ItemStatus::Failed]);
    assert!(failure(&report, 0).contains(CHANGED));
    assert_eq!(content(&vfs, "/t"), b"new");
    // The stamped target is gone: the same error.
    let vfs = memory();
    file(&vfs, "/s", b"s", 100);
    let mut one = copy_item("/s", "/t", 0);
    one.replaces = true;
    one.expect = Some(Expect {
        source: None,
        target: Some(ItemStamp {
            kind: EntryKind::File,
            size: None,
            mtime: None,
        }),
    });
    let report = run(&vfs, batch(vec![one]));
    assert_eq!(statuses(&report), [ItemStatus::Failed]);
    assert!(failure(&report, 0).contains(CHANGED));
}

/// A stamp's `None`s are not compared, and a folder's stamp is its kind.
#[test]
fn a6_2_a_folder_stamp_ignores_time() {
    let vfs = memory();
    vfs.insert_dir(&host("/d"), meta(100)).unwrap();
    let mut one = item(Action::RemoveDir, EntryKind::Dir, None, "/d", 0);
    one.expect = Some(Expect {
        source: None,
        target: Some(ItemStamp {
            kind: EntryKind::Dir,
            size: Some(99),
            mtime: Some(ft(1)),
        }),
    });
    assert!(run(&vfs, batch(vec![one])).ok());
    let vfs = memory();
    file(&vfs, "/d", b"x", 100);
    let mut one = item(Action::Remove, EntryKind::Other, None, "/d", 0);
    one.expect = Some(Expect {
        source: None,
        target: Some(ItemStamp {
            kind: EntryKind::Dir,
            size: None,
            mtime: None,
        }),
    });
    let report = run(&vfs, batch(vec![one]));
    assert!(failure(&report, 0).contains(CHANGED));
}

/// A remove is stamped on its target too: a file modified after the comparison is never deleted on an old row.
#[test]
fn a6_2_a_remove_of_a_changed_file_is_refused() {
    let vfs = memory();
    file(&vfs, "/x", b"old", 100);
    let mut one = item(Action::Remove, EntryKind::File, None, "/x", 0);
    one.expect = Some(Expect {
        source: None,
        target: stamp(&vfs, "/x"),
    });
    let plan = batch(vec![one]);
    file(&vfs, "/x", b"new", 200);
    let report = run(&vfs, plan);
    assert!(failure(&report, 0).contains(CHANGED));
    assert_eq!(content(&vfs, "/x"), b"new");
}

/// `follow_source` stamps through the link.
#[test]
fn a6_2_follow_source_stamps_through_the_link() {
    let vfs = memory();
    file(&vfs, "/real", b"content", 100);
    vfs.insert_symlink(&host("/link"), "real", meta(5)).unwrap();
    let mut one = copy_item("/link", "/t", 1);
    one.follow_source = true;
    one.expect = Some(Expect {
        source: stamp(&vfs, "/real"),
        target: None,
    });
    assert!(run(&vfs, batch(vec![one.clone()])).ok(), "through the link");
    assert_eq!(content(&vfs, "/t"), b"content");
    assert_eq!(dump_of(&vfs, "/t").mtime, Some(ft(100)));
    vfs.remove(&mspec("/t")).unwrap();
    one.follow_source = false;
    let report = run(&vfs, batch(vec![one]));
    assert!(failure(&report, 0).contains(CHANGED), "the link is no file");
}

/// One `lstat` per side per item, not two.
#[test]
fn a6_2_one_lstat_per_side_per_item() {
    let vfs = memory();
    file(&vfs, "/s", b"s", 100);
    file(&vfs, "/t", b"t", 100);
    file(&vfs, "/new", b"n", 100);
    let replacing = stamped(&vfs, "/s", "/t");
    let mut fresh = copy_item("/new", "/fresh", 0);
    fresh.expect = Some(Expect {
        source: stamp(&vfs, "/new"),
        target: None,
    });
    let mut removed = item(Action::Remove, EntryKind::File, None, "/s", 0);
    removed.expect = Some(Expect {
        source: None,
        target: stamp(&vfs, "/s"),
    });
    for (one, expected) in [(replacing, 2), (fresh, 2), (removed, 1)] {
        let before = vfs.stat_calls();
        assert!(run(&vfs, batch(vec![one])).ok());
        assert_eq!(vfs.stat_calls() - before, expected);
        file(&vfs, "/s", b"s", 100);
    }
}

/// Member stamps; and a member the plan does not touch, changed by another operation since planning, survives the
/// rebuild.
#[test]
fn a6_2_member_stamps_and_other_members_survive() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![
        ("a".into(), b"a".to_vec()),
        ("b".into(), b"b".to_vec()),
        ("c".into(), b"c".to_vec()),
    ];
    let archive = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let vfs: &dyn Vfs = &*memory.vfs;
    let spec = |name: &str| Spec::archive_member(archive.clone(), name);
    let member_stamp = |name: &str| {
        let entry = vfs.stat(&spec(name), FollowLinks::No).unwrap();
        Some(ItemStamp {
            kind: entry.kind,
            size: Some(entry.size),
            mtime: entry.mtime,
        })
    };
    memory
        .mem
        .insert_file(&host("/new"), &b"new a"[..], meta(77))
        .unwrap();
    let mut one = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(mspec("/new")),
        spec("a"),
        0,
    );
    one.replaces = true;
    one.expect = Some(Expect {
        source: None,
        target: member_stamp("a"),
    });
    let mut stale = PlanItem::planned(Action::Remove, EntryKind::File, None, spec("c"), 0);
    stale.expect = Some(Expect {
        source: None,
        target: member_stamp("c"),
    });
    let plan = batch(vec![one, stale]);
    // Another operation changes `b` and `c` after the plan was made.
    let other = |name: &str, bytes: &[u8]| {
        let options = WriteOptions {
            overwrite: true,
            ..WriteOptions::default()
        };
        let mut writer = vfs.open_write(&spec(name), &options).unwrap();
        std::io::Write::write_all(&mut writer, bytes).unwrap();
        writer.commit()
    };
    vfs.begin(&archive).unwrap();
    other("b", b"changed b").unwrap();
    other("c", b"changed c").unwrap();
    vfs.finish(&archive, Finish::Commit).unwrap();
    let report = run(vfs, plan);
    assert_eq!(statuses(&report), [ItemStatus::Done, ItemStatus::Failed]);
    assert!(failure(&report, 1).contains(CHANGED));
    let read = |name: &str| read_to_vec(vfs.open_read(&spec(name)).unwrap());
    assert_eq!(read("a"), b"new a");
    assert_eq!(read("b"), b"changed b", "kept by the rebuild");
    assert_eq!(read("c"), b"changed c", "never removed on a stale stamp");
}

/// A file changed through a held handle: either a pass or the error, never a lost change.
#[test]
fn a6_2_a_file_changed_through_a_held_handle() {
    use std::io::Write as _;
    let disk = tempfile::tempdir().unwrap();
    let root = disk.path().canonicalize().unwrap();
    let (held, source) = (root.join("held"), root.join("source"));
    std::fs::write(&held, b"old").unwrap();
    std::fs::write(&source, b"new").unwrap();
    let vfs = LocalVfs::default();
    let spec = |p: &std::path::Path| common::lspec(p);
    let entry = vfs.stat(&spec(&held), FollowLinks::No).unwrap();
    let mut one = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(spec(&source)),
        spec(&held),
        0,
    );
    one.replaces = true;
    one.expect = Some(Expect {
        source: None,
        target: Some(ItemStamp {
            kind: entry.kind,
            size: Some(entry.size),
            mtime: entry.mtime,
        }),
    });
    let mut handle = std::fs::OpenOptions::new().write(true).open(&held).unwrap();
    handle.write_all(b"a change of another length").unwrap();
    handle.flush().unwrap();
    let report = run(&vfs, batch(vec![one]));
    drop(handle);
    let now = std::fs::read(&held).unwrap();
    match report.results[0].status {
        ItemStatus::Done => assert_eq!(now, b"new"),
        ItemStatus::Failed => {
            assert!(failure(&report, 0).contains(CHANGED));
            assert_eq!(now, b"a change of another length");
        }
        other => panic!("{other:?}"),
    }
}

// ---- A6.3: what a back-end records --------------------------------------------------------------------------

#[test]
fn a6_3_the_case_decides_the_durability() {
    let vfs = memory();
    file(&vfs, "/s", b"s", 1);
    file(&vfs, "/m", b"m", 1);
    file(&vfs, "/t", b"t", 1);
    let mut replacing = copy_item("/s", "/t", 0);
    replacing.replaces = true;
    let report = run(
        &vfs,
        batch(vec![
            copy_item("/s", "/new", 0),
            replacing,
            item(Action::Move, EntryKind::File, Some("/m"), "/moved", 0),
        ]),
    );
    assert!(report.ok());
    let spec = |path: &str| mspec(path).canonical();
    assert_eq!(
        vfs.durabilities(),
        [
            (spec("/new"), Durability::None),
            (spec("/t"), Durability::FileOnly),
            (spec("/moved"), Durability::Full),
        ]
    );
}

// ---- A6.4 exchange -------------------------------------------------------------------------------------------

/// An `exchange` item with both stamps.
fn swap_item(vfs: &dyn Vfs, left: &str, right: &str) -> PlanItem {
    let entry = vfs.stat(&mspec(left), FollowLinks::No).unwrap();
    let mut one = PlanItem::planned(
        Action::Exchange,
        entry.kind,
        Some(mspec(left)),
        mspec(right),
        0,
    );
    one.size = Some(entry.size);
    one.replaces = true;
    one.expect = Some(Expect {
        source: stamp(vfs, left),
        target: stamp(vfs, right),
    });
    one
}

fn pair() -> MemoryVfs {
    let vfs = memory();
    let (left, right) = (
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o640,
                uid: 0,
                gid: 0,
            },
        },
        Attrs::Unix {
            meta: UnixMeta {
                mode: 0o604,
                uid: 0,
                gid: 0,
            },
        },
    );
    let with = |secs: i64, attrs: Attrs| MemMeta {
        mtime: Some(ft(secs)),
        attrs: Some(attrs),
        ..MemMeta::default()
    };
    vfs.insert_file(&host("/l/f"), &b"left side"[..], with(100, left))
        .unwrap();
    vfs.insert_file(&host("/r/f"), &b"right"[..], with(200, right))
        .unwrap();
    vfs
}

/// Two files of different sizes swap, with their metadata, and no sibling stays.
#[test]
fn a6_4_files_swap_with_their_metadata() {
    let vfs = pair();
    let (left, right) = (dump_of(&vfs, "/l/f"), dump_of(&vfs, "/r/f"));
    let report = run(&vfs, batch(vec![swap_item(&vfs, "/l/f", "/r/f")]));
    assert!(report.ok(), "{:?}", report.results);
    let (now_left, now_right) = (dump_of(&vfs, "/l/f"), dump_of(&vfs, "/r/f"));
    assert_eq!((now_left.crc32, now_left.mtime), (right.crc32, right.mtime));
    assert_eq!(now_left.attrs, right.attrs);
    assert_eq!((now_right.crc32, now_right.mtime), (left.crc32, left.mtime));
    assert_eq!(now_right.attrs, left.attrs);
    assert_eq!(
        vfs.dump().len(),
        5,
        "/, two folders, two files: {:?}",
        vfs.dump()
    );
    assert_eq!(report.summary.bytes, 14);
    assert_eq!(report.plan.bytes(), 14);
}

/// Across devices: renames stay on one device, because each copy is a sibling.
#[test]
fn a6_4_across_devices() {
    let vfs = pair();
    vfs.set_device(&host("/r"), 7);
    let report = run(&vfs, batch(vec![swap_item(&vfs, "/l/f", "/r/f")]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(content(&vfs, "/l/f"), b"right");
    assert_eq!(content(&vfs, "/r/f"), b"left side");
}

/// Links travel as links, and a file swaps with a link.
#[test]
fn a6_4_links() {
    let vfs = memory();
    vfs.insert_symlink(&host("/l/x"), "one", meta(1)).unwrap();
    vfs.insert_symlink(&host("/r/x"), "two", meta(2)).unwrap();
    let report = run(&vfs, batch(vec![swap_item(&vfs, "/l/x", "/r/x")]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(dump_of(&vfs, "/l/x").link_target.as_deref(), Some("two"));
    assert_eq!(dump_of(&vfs, "/r/x").link_target.as_deref(), Some("one"));

    let vfs = memory();
    file(&vfs, "/l/x", b"file", 1);
    vfs.insert_symlink(&host("/r/x"), "two", meta(2)).unwrap();
    let report = run(&vfs, batch(vec![swap_item(&vfs, "/l/x", "/r/x")]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(dump_of(&vfs, "/l/x").link_target.as_deref(), Some("two"));
    assert_eq!(content(&vfs, "/r/x"), b"file");
}

/// Real links on disk, where the platform allows them.
#[test]
fn a6_4_links_on_disk() {
    let disk = tempfile::tempdir().unwrap();
    let root = disk.path().canonicalize().unwrap();
    let vfs = LocalVfs::default();
    let (left, right) = (root.join("l"), root.join("r"));
    let made = vfs.create_symlink(&common::lspec_new(&left), "one".as_ref(), false);
    if matches!(made, Err(VfsError::PermissionDenied { .. })) {
        return;
    }
    made.unwrap();
    std::fs::write(&right, b"file").unwrap();
    let spec = |p: &std::path::Path| Spec::Local(hikasa_core::LocalPath::new(p.into()).unwrap());
    let mut one = PlanItem::planned(
        Action::Exchange,
        EntryKind::Symlink,
        Some(spec(&left)),
        spec(&right),
        0,
    );
    one.replaces = true;
    let report = run(&vfs, batch(vec![one]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(std::fs::read(&left).unwrap(), b"file");
    assert_eq!(
        std::fs::read_link(&right).unwrap(),
        std::path::Path::new("one")
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}

/// A fault at each of the four steps.
#[test]
fn a6_4_a_fault_at_each_step() {
    let t1 = host("/r/.f.hikasa-xchg");
    let t2 = host("/l/.f.hikasa-xchg");
    // Rows 1–3: both sides byte-identical, no hidden sibling.
    let faults = [
        Fault {
            path: host("/l/f"),
            op: VfsOp::Read,
            kind: ErrorKind::Io,
        },
        Fault {
            path: t1.clone(),
            op: VfsOp::Write,
            kind: ErrorKind::Io,
        },
        Fault {
            path: host("/r/f"),
            op: VfsOp::Read,
            kind: ErrorKind::Io,
        },
        Fault {
            path: t2.clone(),
            op: VfsOp::Commit,
            kind: ErrorKind::Io,
        },
        Fault {
            path: t1.clone(),
            op: VfsOp::Rename,
            kind: ErrorKind::Io,
        },
    ];
    for fault in faults {
        let vfs = pair();
        let before = vfs.dump();
        let plan = batch(vec![swap_item(&vfs, "/l/f", "/r/f")]);
        vfs.inject(fault.clone());
        let report = run(&vfs, plan);
        assert_eq!(statuses(&report), [ItemStatus::Failed], "{fault:?}");
        assert_eq!(vfs.dump(), before, "{fault:?}");
    }
    // Row 4: the target holds the new content, `T2` the other side's old one, and the message names it.
    let vfs = pair();
    let plan = batch(vec![swap_item(&vfs, "/l/f", "/r/f")]);
    vfs.inject(Fault {
        path: t2.clone(),
        op: VfsOp::Rename,
        kind: ErrorKind::Io,
    });
    let report = run(&vfs, plan);
    assert_eq!(statuses(&report), [ItemStatus::Failed]);
    let error = report.results[0].error.as_ref().unwrap();
    assert!(
        error
            .message
            .contains("changed during the move; the copy was kept"),
        "{}",
        error.message
    );
    assert_eq!(
        error.spec.as_deref(),
        Some(mspec("/l/.f.hikasa-xchg").canonical().as_str())
    );
    assert_eq!(content(&vfs, "/r/f"), b"left side");
    assert_eq!(content(&vfs, "/l/f"), b"left side");
    assert_eq!(vfs.read_back(&t2).unwrap().as_ref(), b"right");
    assert!(vfs.read_back(&t1).is_none());
}

/// A free sibling is looked for: a leftover `T2` of an earlier run stays as it is.
#[test]
fn a6_4_a_hidden_name_in_use_moves_on() {
    let vfs = pair();
    file(&vfs, "/r/.f.hikasa-xchg", b"mine", 9);
    let report = run(&vfs, batch(vec![swap_item(&vfs, "/l/f", "/r/f")]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(content(&vfs, "/r/.f.hikasa-xchg"), b"mine");
    assert_eq!(content(&vfs, "/r/f"), b"left side");
}

/// A progress sink that cancels at the n-th `Phase::Writing` report.
struct CancelAt {
    token: CancelToken,
    left: std::sync::atomic::AtomicI32,
}

impl ProgressSink for CancelAt {
    fn report(&self, event: Progress) {
        if event.phase == Phase::Writing
            && self.left.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) == 0
        {
            self.token.cancel();
        }
    }
}

/// Cancellation at each stage: before step 1, then at the report of each copy, which on a disk is the moment before the
/// next step. Both sides stay, no sibling.
#[test]
fn a6_4_cancel_at_each_step() {
    for stop in 0..3 {
        let disk = tempfile::tempdir().unwrap();
        let root = disk.path().canonicalize().unwrap();
        let (left, right) = (root.join("l"), root.join("r"));
        std::fs::write(&left, b"left side").unwrap();
        std::fs::write(&right, b"right").unwrap();
        let vfs = LocalVfs::default();
        let cancel = CancelToken::new();
        if stop == 0 {
            cancel.cancel();
        }
        let sink = CancelAt {
            token: cancel.clone(),
            left: std::sync::atomic::AtomicI32::new(stop - 1),
        };
        let ctl = Ctl {
            cancel: &cancel,
            progress: &sink,
        };
        let plan = batch(vec![swap_specs(
            &common::lspec(&left),
            &common::lspec(&right),
        )]);
        let report = ops::execute(&vfs, &vfs, plan, &ctl);
        assert_ne!(statuses(&report), [ItemStatus::Done], "stop {stop}");
        assert!(report.cancelled, "stop {stop}");
        assert_eq!(std::fs::read(&left).unwrap(), b"left side", "stop {stop}");
        assert_eq!(std::fs::read(&right).unwrap(), b"right", "stop {stop}");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2, "stop {stop}");
    }
    for stop in 0..3 {
        let vfs = pair();
        let before = vfs.dump();
        let plan = batch(vec![swap_item(&vfs, "/l/f", "/r/f")]);
        let cancel = CancelToken::new();
        if stop == 0 {
            cancel.cancel();
        }
        let sink = CancelAt {
            token: cancel.clone(),
            left: std::sync::atomic::AtomicI32::new(stop - 1),
        };
        let ctl = Ctl {
            cancel: &cancel,
            progress: &sink,
        };
        let report = ops::execute(&vfs, &vfs, plan, &ctl);
        assert_ne!(statuses(&report), [ItemStatus::Done], "stop {stop}");
        assert!(report.cancelled, "stop {stop}");
        assert_eq!(vfs.dump(), before, "stop {stop}");
    }
}

/// Both sides in one archive: one batch, one rebuild.
#[test]
fn a6_4_both_sides_in_one_archive() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![
        ("l/f".into(), b"left".to_vec()),
        ("r/f".into(), b"right side".to_vec()),
    ];
    let archive = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let vfs: &dyn Vfs = &*memory.vfs;
    let (left, right) = (
        Spec::archive_member(archive.clone(), "l/f"),
        Spec::archive_member(archive.clone(), "r/f"),
    );
    let mut one = PlanItem::planned(
        Action::Exchange,
        EntryKind::File,
        Some(left.clone()),
        right.clone(),
        0,
    );
    one.replaces = true;
    let report = run(vfs, batch(vec![one]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(memory.vfs.counters().rebuilds, 1);
    assert_eq!(read_to_vec(vfs.open_read(&left).unwrap()), b"right side");
    assert_eq!(read_to_vec(vfs.open_read(&right).unwrap()), b"left");
    let listing = vfs
        .list(
            &Spec::archive_member(archive.clone(), "r"),
            &ListOptions::default(),
            &CancelToken::new(),
        )
        .unwrap();
    assert_eq!(listing.entries.len(), 1, "no hidden sibling");
}

/// The two ways a plain file meets a member.
fn plain_and_member() -> (OnMemory, Spec, Spec) {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![("m".into(), b"member".to_vec())];
    let archive = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    memory
        .mem
        .insert_file(&host("/d/plain"), &b"plain file"[..], meta(5))
        .unwrap();
    let member = Spec::archive_member(archive, "m");
    (memory, mspec("/d/plain"), member)
}

fn swap_specs(left: &Spec, right: &Spec) -> PlanItem {
    let mut one = PlanItem::planned(
        Action::Exchange,
        EntryKind::File,
        Some(left.clone()),
        right.clone(),
        0,
    );
    one.replaces = true;
    one
}

#[test]
fn a6_4_a_plain_file_against_a_member() {
    for member_on_the_left in [false, true] {
        let (memory, plain, member) = plain_and_member();
        let vfs: &dyn Vfs = &*memory.vfs;
        let (left, right) = if member_on_the_left {
            (member.clone(), plain.clone())
        } else {
            (plain.clone(), member.clone())
        };
        let report = run(vfs, batch(vec![swap_specs(&left, &right)]));
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(read_to_vec(vfs.open_read(&plain).unwrap()), b"member");
        assert_eq!(read_to_vec(vfs.open_read(&member).unwrap()), b"plain file");
        let names = memory.mem.dump().len();
        assert_eq!(names, 4, "{:?}", memory.mem.dump());
    }
}

/// A failing rebuild keeps both sides, no sibling left: on disk, the rename is what waits for the rebuild.
#[test]
fn a6_4_a_failing_rebuild_keeps_both_sides() {
    for member_on_the_left in [false, true] {
        let (memory, plain, member) = plain_and_member();
        let vfs: &dyn Vfs = &*memory.vfs;
        let (left, right) = if member_on_the_left {
            (member.clone(), plain.clone())
        } else {
            (plain.clone(), member.clone())
        };
        memory.mem.inject(Fault {
            path: host("/a.zip"),
            op: VfsOp::Commit,
            kind: ErrorKind::Io,
        });
        let before = memory.mem.dump();
        let report = run(vfs, batch(vec![swap_specs(&left, &right)]));
        assert_eq!(statuses(&report), [ItemStatus::Failed]);
        assert_eq!(memory.mem.dump(), before);
        assert_eq!(read_to_vec(vfs.open_read(&member).unwrap()), b"member");
    }
}

/// Members of two archives: an execution refuses it too (the builder's item error is W6b).
#[test]
fn a6_4_two_archives_are_refused() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![("m".into(), b"member".to_vec())];
    let one = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let two = memory.put("b.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let vfs: &dyn Vfs = &*memory.vfs;
    let before = memory.mem.dump();
    let report = run(
        vfs,
        batch(vec![swap_specs(
            &Spec::archive_member(one, "m"),
            &Spec::archive_member(two, "m"),
        )]),
    );
    assert_eq!(statuses(&report), [ItemStatus::Failed]);
    assert_eq!(
        report.results[0].error.as_ref().unwrap().kind,
        ErrorKind::Unsupported
    );
    assert_eq!(memory.mem.dump(), before);
}

// ---- A6.5 fence ----------------------------------------------------------------------------------------------

#[test]
fn a6_5_within_table() {
    let vfs = memory();
    vfs.insert_dir(&host("/a"), MemMeta::default()).unwrap();
    let zip = Spec::archive_member(mspec("/a/b.zip"), "");
    let member = Spec::archive_member(mspec("/a/b.zip"), "m/x");
    let nested = Spec::archive_member(Spec::archive_member(mspec("/a/b.zip"), "c.tar"), "y");
    let elsewhere = Spec::archive_member(mspec("/z.zip"), "x");
    let rows = [
        (mspec("/a"), mspec("/a/b"), true),
        (mspec("/a"), mspec("/a/b/c/d"), true),
        (mspec("/a"), mspec("/a"), false),
        (mspec("/a/b"), mspec("/a/bc"), false),
        (mspec("/a/b"), mspec("/a"), false),
        (mspec("/a"), mspec("/b"), false),
        // A member and a nested member, of an archive opened inside a base.
        (mspec("/a"), member.clone(), true),
        (mspec("/a"), nested.clone(), true),
        (mspec("/b"), member.clone(), false),
        // A base inside an archive, and an archive as a base, by its root or as the file.
        (
            Spec::archive_member(mspec("/a/b.zip"), "m"),
            member.clone(),
            true,
        ),
        (
            Spec::archive_member(mspec("/a/b.zip"), "n"),
            member.clone(),
            false,
        ),
        (zip.clone(), member.clone(), true),
        (zip.clone(), nested.clone(), true),
        (mspec("/a/b.zip"), member.clone(), true),
        (mspec("/a/b.zip"), zip.clone(), false),
        (zip.clone(), zip.clone(), false),
        (zip.clone(), elsewhere.clone(), false),
        (
            Spec::archive_member(mspec("/a/b.zip"), "c.tar"),
            nested.clone(),
            true,
        ),
    ];
    for (base, spec, expected) in rows {
        assert_eq!(
            within(&vfs, &base, &spec),
            expected,
            "{} in {}",
            spec.canonical(),
            base.canonical()
        );
    }
}

#[test]
fn a6_5_case_goes_by_the_back_end() {
    let insensitive = MemoryVfs::new(CaseSensitivity::Insensitive);
    insensitive
        .insert_dir(&host("/a"), MemMeta::default())
        .unwrap();
    assert!(within(&insensitive, &mspec("/A"), &mspec("/a/b")));
    assert!(!within(&memory(), &mspec("/A"), &mspec("/a/b")));
}

#[cfg(windows)]
#[test]
fn a6_5_drive_letters() {
    let vfs = LocalVfs::default();
    let path = |text: &str| {
        Spec::Local(hikasa_core::LocalPath::new(std::path::PathBuf::from(text)).unwrap())
    };
    assert!(within(&vfs, &path(r"C:\a"), &path(r"C:\a\b")));
    assert!(within(&vfs, &path(r"c:\A"), &path(r"C:\a\b")));
    assert!(!within(&vfs, &path(r"D:\a"), &path(r"C:\a\b")));
}

fn fence(writes: bool) -> ExecOptions {
    ExecOptions {
        fence: Some(Fence {
            bases: vec![mspec("/l"), mspec("/r")],
            writes,
        }),
        ..ExecOptions::default()
    }
}

/// A `remove` outside the bases fails, nothing removed; the base itself is never removed.
#[test]
fn a6_5_execution_refuses_what_lies_outside() {
    let vfs = memory();
    file(&vfs, "/l/in", b"i", 1);
    file(&vfs, "/out/x", b"x", 1);
    let before = vfs.dump();
    let plan = batch(vec![
        item(Action::Remove, EntryKind::File, None, "/out/x", 0),
        item(Action::RemoveDir, EntryKind::Dir, None, "/l", 0),
        item(Action::Trash, EntryKind::File, None, "/out/x", 0),
        item(Action::Move, EntryKind::File, Some("/out/x"), "/l/moved", 0),
        item(
            Action::Rename,
            EntryKind::File,
            Some("/out/x"),
            "/l/named",
            0,
        ),
        item(
            Action::Exchange,
            EntryKind::File,
            Some("/out/x"),
            "/l/in",
            0,
        ),
    ]);
    let report = run_with(&vfs, plan, &fence(false));
    assert_eq!(statuses(&report), [ItemStatus::Failed; 6]);
    for index in 0..6 {
        assert!(failure(&report, index).contains(OUTSIDE), "{index}");
    }
    assert_eq!(vfs.dump(), before);
    assert!(vfs.trashed().is_empty());
}

/// `writes`: every target must lie below a base; without it a copy may leave them, by request.
#[test]
fn a6_5_writes_on_and_off() {
    for writes in [true, false] {
        let vfs = memory();
        file(&vfs, "/l/in", b"i", 1);
        vfs.insert_dir(&host("/out"), MemMeta::default()).unwrap();
        let plan = batch(vec![
            copy_item("/l/in", "/out/copy", 0),
            copy_item("/l/in", "/l/copy", 0),
        ]);
        let report = run_with(&vfs, plan, &fence(writes));
        let expected = if writes {
            [ItemStatus::Failed, ItemStatus::Done]
        } else {
            [ItemStatus::Done, ItemStatus::Done]
        };
        assert_eq!(statuses(&report), expected);
        assert_eq!(vfs.read_back(&host("/out/copy")).is_some(), !writes);
    }
}

/// A refused item opens no archive: a fence that refuses leaves no batch and no rebuild.
#[test]
fn a6_5_a_refused_item_opens_no_archive() {
    let memory = on_memory(ArchiveLimits::default());
    let tree: TreeData = vec![("m".into(), b"member".to_vec())];
    let archive = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let vfs: &dyn Vfs = &*memory.vfs;
    let plan = batch(vec![PlanItem::planned(
        Action::Remove,
        EntryKind::File,
        None,
        Spec::archive_member(archive.clone(), "m"),
        0,
    )]);
    let options = ExecOptions {
        fence: Some(Fence {
            bases: vec![mspec("/l")],
            writes: true,
        }),
        ..ExecOptions::default()
    };
    let before = memory.mem.dump();
    let report = run_with(vfs, plan, &options);
    assert!(failure(&report, 0).contains(OUTSIDE));
    assert_eq!(memory.mem.dump(), before);
    assert_eq!(memory.vfs.counters().rebuilds, 0);
}

/// Copies of a local plan on disk go through the handle path, and keep `vfs cp`'s result.
#[test]
fn a6_3_copy_on_disk_keeps_the_result() {
    let disk = on_disk(ArchiveLimits::default());
    let source = disk.put("s", b"bytes");
    let vfs: &dyn Vfs = &*disk.vfs;
    vfs.set_times(
        &source,
        &SetTimes {
            modified: Some(ft(1_000)),
            created: None,
        },
    )
    .unwrap();
    let target = common::lspec_new(&disk.dir.join("t"));
    let pairs = [(source, target.clone())];
    let options = CopyOptions::default();
    let plan = ops::plan_copy(vfs, vfs, &pairs, &options, &ctl(&CancelToken::new())).unwrap();
    let report = run(vfs, plan);
    assert!(report.ok(), "{:?}", report.results);
    let copied: FileTime = vfs.stat(&target, FollowLinks::No).unwrap().mtime.unwrap();
    assert_eq!(copied, ft(1_000));
    assert_eq!(read_to_vec(vfs.open_read(&target).unwrap()), b"bytes");
}
