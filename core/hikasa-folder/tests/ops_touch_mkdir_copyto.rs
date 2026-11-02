//! M2 W6b acceptance A6.19 (FC-072, FC-076, FC-077): rules 14–16 of §1.6.10.

mod common;

use std::sync::Arc;

use common::{compared, default_now, engines, host, mspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, NullProgress, RelPath, Side, Spec,
};
use hikasa_folder::ops::{
    FolderOp, OpPlan, OpScope, OpSettings, PathStructure, ReadOnlySides, TouchFrom, plan_op,
};
use hikasa_folder::select::{Include, SelectMask, SelectScope, Selection, SideSet, select};
use hikasa_folder::{
    ArchiveHandling, DisplayOptions, FolderComparison, FolderDisplayFilter, FolderEnv, FolderError,
    FolderMode, FolderModel, FolderRules, NodeId, NullObserver, compare_into,
};
use hikasa_vfs::ops::{self, Action, ItemStatus, OpReport, OverwritePolicy, SkipReason};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, EntryKind, Fault,
    FollowLinks, MemMeta, MemoryVfs, Vfs, VfsOp,
};

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

fn memory() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), meta(1)).unwrap();
    vfs.insert_dir(&host("/r"), meta(1)).unwrap();
    vfs
}

fn file(vfs: &MemoryVfs, path: &str, content: &[u8], secs: i64) {
    vfs.insert_file(&host(path), content.to_vec(), meta(secs))
        .unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn row(cmp: &FolderComparison, path: &str) -> NodeId {
    cmp.read().node_at(&RelPath::parse(path).unwrap()).unwrap()
}

fn plan_of(
    vfs: &dyn Vfs,
    cmp: &FolderComparison,
    rows: &[(&str, SideSet)],
    op: &FolderOp,
) -> Result<OpPlan, FolderError> {
    let read = cmp.read();
    let mut selection = Selection::default();
    for (path, sides) in rows {
        selection.insert(
            read.node_at(&RelPath::parse(path).unwrap()).unwrap(),
            *sides,
        );
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    plan_op(
        &read,
        vfs,
        &scope,
        op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

fn short(spec: &Spec) -> String {
    let text = spec.canonical().replace('\\', "/");
    text.strip_prefix("C:").map(str::to_owned).unwrap_or(text)
}

fn lines(plan: &OpPlan) -> Vec<String> {
    plan.plan
        .items
        .iter()
        .map(|item| {
            let mut line = format!(
                "{} {} @{}",
                item.action.as_str(),
                short(&item.target),
                item.depth
            );
            if let Some(reason) = item.skip {
                line.push_str(&format!(" !{}", reason.as_str()));
            }
            if let Some(error) = &item.error {
                line.push_str(&format!(" !{}", error.kind.as_str()));
            }
            line
        })
        .collect()
}

const L: SideSet = SideSet::LEFT;
const R: SideSet = SideSet::RIGHT;

#[test]
fn a6_19_touch_at_every_row_folders_too() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let at = FileTime::from_unix(1234, 0).unwrap();
    let op = FolderOp::Touch {
        sides: L,
        from: TouchFrom::At(at),
    };
    let out = plan_of(&vfs, &cmp, &[("d", L)], &op).unwrap();
    assert_eq!(lines(&out), ["touch /l/d @0", "touch /l/d/x @0"]);
    assert!(
        out.plan
            .items
            .iter()
            .all(|item| item.times.unwrap().modified == Some(at))
    );
    assert!(
        out.plan
            .items
            .iter()
            .all(|item| item.times.unwrap().created.is_none())
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    for path in ["/l/d", "/l/d/x"] {
        assert_eq!(
            vfs.stat(&mspec(path), FollowLinks::No).unwrap().mtime,
            Some(at)
        );
    }
}

#[test]
fn a6_19_touch_other_side_on_pairs_and_folders() {
    let vfs = memory();
    vfs.insert_dir(&host("/l/d"), meta(5)).unwrap();
    vfs.insert_dir(&host("/r/d"), meta(6)).unwrap();
    file(&vfs, "/l/d/x", b"1", 10);
    file(&vfs, "/r/d/x", b"1", 20);
    file(&vfs, "/l/d/only", b"1", 30);
    let cmp = compared(&vfs, &FolderRules::default());
    let op = FolderOp::Touch {
        sides: R,
        from: TouchFrom::OtherSide { to: Side::Right },
    };
    let out = plan_of(&vfs, &cmp, &[("d", SideSet::all())], &op).unwrap();
    // A row with one side gives no item.
    assert_eq!(lines(&out), ["touch /r/d @0", "touch /r/d/x @0"]);
    assert_eq!(
        out.plan.items[1].times.unwrap().modified,
        FileTime::from_unix(10, 0)
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(
        vfs.stat(&mspec("/r/d/x"), FollowLinks::No).unwrap().mtime,
        FileTime::from_unix(10, 0)
    );
    assert_eq!(
        vfs.stat(&mspec("/r/d"), FollowLinks::No).unwrap().mtime,
        FileTime::from_unix(5, 0)
    );
}

#[test]
fn a6_19_a_folder_whose_set_times_fails_is_that_items_failure() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let at = FileTime::from_unix(1234, 0).unwrap();
    let op = FolderOp::Touch {
        sides: L,
        from: TouchFrom::At(at),
    };
    let out = plan_of(&vfs, &cmp, &[("d", L)], &op).unwrap();
    vfs.inject(Fault {
        path: host("/l/d"),
        op: VfsOp::SetTimes,
        kind: ErrorKind::PermissionDenied,
    });
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(report.results[1].status, ItemStatus::Done);
}

#[test]
fn a6_19_mkdir_on_both_sides() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", 10);
    file(&vfs, "/r/d/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let op = FolderOp::Mkdir {
        sides: SideSet::all(),
        parent: row(&cmp, "d"),
        name: "new".to_owned(),
        folder: false,
    };
    let out = plan_of(&vfs, &cmp, &[], &op).unwrap();
    assert_eq!(
        lines(&out),
        ["create_dir /l/d/new @0", "create_dir /r/d/new @0"]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(
        vfs.stat(&mspec("/r/d/new"), FollowLinks::No).unwrap().kind,
        EntryKind::Dir
    );
}

#[test]
fn a6_19_mkdir_with_a_parent_missing_on_a_side_gets_its_chain() {
    let vfs = memory();
    file(&vfs, "/l/a/b/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let op = FolderOp::Mkdir {
        sides: R,
        parent: row(&cmp, "a/b"),
        name: "new".to_owned(),
        folder: false,
    };
    let out = plan_of(&vfs, &cmp, &[], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /r/a @0",
            "create_dir /r/a/b @1",
            "create_dir /r/a/b/new @2"
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
}

#[test]
fn a6_19_mkdir_names_that_exist_or_are_bad() {
    let vfs = memory();
    file(&vfs, "/l/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let mk = |name: &str| FolderOp::Mkdir {
        sides: L,
        parent: NodeId::ROOT,
        name: name.to_owned(),
        folder: false,
    };
    assert_eq!(
        lines(&plan_of(&vfs, &cmp, &[], &mk("x")).unwrap()),
        ["create_dir /l/x @0 !already_exists"]
    );
    for bad in ["", "..", "a/b"] {
        let out = plan_of(&vfs, &cmp, &[], &mk(bad)).unwrap();
        assert_eq!(
            out.plan.items[0].error.as_ref().unwrap().kind,
            ErrorKind::InvalidInput
        );
    }
}

struct World {
    mem: Arc<MemoryVfs>,
    vfs: ArchiveVfs,
    _dir: tempfile::TempDir,
}

fn world() -> World {
    let mem = Arc::new(memory());
    let dir = tempfile::tempdir().unwrap();
    let masks = ArchiveMasks::new([
        (ArchiveFormat::Zip, "*.zip".to_owned()),
        (ArchiveFormat::Gz, "*.gz".to_owned()),
    ])
    .unwrap();
    let config = ArchiveConfig {
        masks,
        limits: ArchiveLimits::default(),
        clock: Arc::new(default_now),
        cache_root: dir.path().to_path_buf(),
    };
    let vfs = ArchiveVfs::new(mem.clone() as Arc<dyn Vfs>, config);
    World {
        mem,
        vfs,
        _dir: dir,
    }
}

impl World {
    fn compared(&self, rules: &FolderRules) -> FolderComparison {
        let contents = engines(1 << 20);
        let now = default_now();
        let cancel = CancelToken::never();
        let progress = NullProgress;
        let env = FolderEnv {
            vfs: &self.vfs,
            content: &contents,
            pool: None,
            ctl: Ctl {
                cancel: &cancel,
                progress: &progress,
            },
            observer: &NullObserver,
            now: &now,
            content_budget: 1 << 30,
            archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
        };
        let model = FolderModel::new(mspec("/l"), mspec("/r"), rules.clone()).unwrap();
        compare_into(&model, &env).unwrap()
    }
}

#[test]
fn a6_19_mkdir_of_archive_names() {
    let world = world();
    let cmp = world.compared(&FolderRules::default());
    let mk = |name: &str, folder| FolderOp::Mkdir {
        sides: L,
        parent: NodeId::ROOT,
        name: name.to_owned(),
        folder,
    };
    // A writable mask is an archive, a trailing separator a folder, a single-stream mask a folder.
    let out = plan_of(&world.vfs, &cmp, &[], &mk("x.zip", false)).unwrap();
    assert_eq!(lines(&out), ["create_archive /l/x.zip @0"]);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(
        world
            .mem
            .stat(&mspec("/l/x.zip"), FollowLinks::No)
            .unwrap()
            .kind,
        EntryKind::File
    );
    let out = plan_of(&world.vfs, &cmp, &[], &mk("y.zip", true)).unwrap();
    assert_eq!(lines(&out), ["create_dir /l/y.zip @0"]);
    let out = plan_of(&world.vfs, &cmp, &[], &mk("x.gz", false)).unwrap();
    assert_eq!(lines(&out), ["create_dir /l/x.gz @0"]);
}

fn copy_to(sides: SideSet, target: &str, structure: PathStructure) -> FolderOp {
    FolderOp::CopyTo {
        sides,
        target: mspec(target),
        folder: false,
        structure,
        overwrite: OverwritePolicy::Fail,
    }
}

fn two_folders() -> (MemoryVfs, FolderComparison) {
    let vfs = memory();
    file(&vfs, "/l/a/b/x.txt", b"1", 10);
    file(&vfs, "/l/a/c/y.txt", b"2", 10);
    vfs.insert_dir(&host("/out"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    (vfs, cmp)
}

#[test]
fn a6_19_copyto_base() {
    let (vfs, cmp) = two_folders();
    let op = copy_to(L, "/out", PathStructure::Base);
    let out = plan_of(&vfs, &cmp, &[("a/b/x.txt", L), ("a/c/y.txt", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /out/a @0",
            "create_dir /out/a/b @1",
            "copy /out/a/b/x.txt @2",
            "create_dir /out/a/c @1",
            "copy /out/a/c/y.txt @2",
        ]
    );
    assert_eq!(out.meta[2].side, None);
    assert!(!out.exec.fence.as_ref().unwrap().writes);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/out/a/c/y.txt").unwrap(), b"2");
    assert_eq!(read(&vfs, "/l/a/c/y.txt").unwrap(), b"2");
}

#[test]
fn a6_19_copyto_relative_drops_the_common_folders() {
    let (vfs, cmp) = two_folders();
    let op = copy_to(L, "/out", PathStructure::Relative);
    let out = plan_of(&vfs, &cmp, &[("a/b/x.txt", L), ("a/c/y.txt", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /out/b @0",
            "copy /out/b/x.txt @1",
            "create_dir /out/c @0",
            "copy /out/c/y.txt @1",
        ]
    );
    // One folder selected keeps its own name.
    let out = plan_of(&vfs, &cmp, &[("a/b", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        ["create_dir /out/b @0", "copy /out/b/x.txt @1"]
    );
}

#[test]
fn a6_19_copyto_common_prefix_is_in_whole_components() {
    let vfs = memory();
    file(&vfs, "/l/ab/x", b"1", 10);
    file(&vfs, "/l/ac/y", b"2", 10);
    vfs.insert_dir(&host("/out"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let op = copy_to(L, "/out", PathStructure::Relative);
    let out = plan_of(&vfs, &cmp, &[("ab/x", L), ("ac/y", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /out/ab @0",
            "copy /out/ab/x @1",
            "create_dir /out/ac @0",
            "copy /out/ac/y @1",
        ]
    );
}

#[test]
fn a6_19_copyto_flat_and_duplicates() {
    let vfs = memory();
    file(&vfs, "/l/a/same", b"1", 10);
    file(&vfs, "/l/b/same", b"2", 10);
    file(&vfs, "/r/c/same", b"3", 10);
    vfs.insert_dir(&host("/out"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let op = copy_to(SideSet::all(), "/out", PathStructure::None);
    let out = plan_of(
        &vfs,
        &cmp,
        &[("a/same", L), ("b/same", L), ("c/same", R)],
        &op,
    )
    .unwrap();
    assert_eq!(
        lines(&out),
        [
            "copy /out/same @0",
            "skip /out/same @0 !duplicate_target",
            "skip /out/same @0 !duplicate_target",
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/out/same").unwrap(), b"1");
}

#[test]
fn a6_19_copyto_rows_of_two_sides_use_their_own_prefix() {
    let vfs = memory();
    file(&vfs, "/l/left/deep/x", b"1", 10);
    file(&vfs, "/r/right/deep/y", b"2", 10);
    vfs.insert_dir(&host("/out"), meta(1)).unwrap();
    let cmp = compared(&vfs, &FolderRules::default());
    let op = copy_to(SideSet::all(), "/out", PathStructure::Relative);
    let out = plan_of(&vfs, &cmp, &[("left/deep/x", L), ("right/deep/y", R)], &op).unwrap();
    assert_eq!(lines(&out), ["copy /out/x @0", "copy /out/y @0"]);
}

#[test]
fn a6_19_copyto_the_overwrite_policy_looks_at_the_target() {
    let vfs = memory();
    file(&vfs, "/l/x", b"new", 10);
    file(&vfs, "/out/x", b"old", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let with = |overwrite| FolderOp::CopyTo {
        sides: L,
        target: mspec("/out"),
        folder: false,
        structure: PathStructure::Relative,
        overwrite,
    };
    let out = plan_of(&vfs, &cmp, &[("x", L)], &with(OverwritePolicy::Fail)).unwrap();
    assert_eq!(lines(&out), ["copy /out/x @0 !already_exists"]);
    let out = plan_of(&vfs, &cmp, &[("x", L)], &with(OverwritePolicy::Skip)).unwrap();
    assert_eq!(out.plan.items[0].skip, Some(SkipReason::Exists));
    let out = plan_of(&vfs, &cmp, &[("x", L)], &with(OverwritePolicy::Always)).unwrap();
    assert!(out.plan.items[0].replaces);
    assert_eq!(
        out.plan.items[0].expect.unwrap().target.unwrap().size,
        Some(3)
    );
}

#[test]
fn a6_19_a_new_target_folder_is_planned_at_depth_0_once() {
    let (vfs, cmp) = two_folders();
    let op = copy_to(L, "/out/new/deep", PathStructure::None);
    let out = plan_of(&vfs, &cmp, &[("a/b/x.txt", L), ("a/c/y.txt", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /out/new @0",
            "create_dir /out/new/deep @0",
            "copy /out/new/deep/x.txt @0",
            "copy /out/new/deep/y.txt @0",
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/out/new/deep/y.txt").unwrap(), b"2");
}

#[test]
fn a6_19_a_new_zip_target_is_created_once() {
    let world = world();
    world.mem.insert_dir(&host("/out"), meta(1)).unwrap();
    file(&world.mem, "/l/a/x", b"1", 10);
    file(&world.mem, "/l/a/y", b"2", 10);
    let cmp = world.compared(&FolderRules::default());
    let op = copy_to(L, "/out/new.zip", PathStructure::Relative);
    let out = plan_of(&world.vfs, &cmp, &[("a", L)], &op).unwrap();
    let creates = out
        .plan
        .items
        .iter()
        .filter(|item| item.action == Action::CreateArchive)
        .count();
    assert_eq!(creates, 1, "{:?}", lines(&out));
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let inside = Spec::archive_member(mspec("/out/new.zip"), "a/y");
    assert_eq!(world.vfs.stat(&inside, FollowLinks::No).unwrap().size, 1);
}

#[test]
fn a6_19_a_target_inside_a_selected_source_is_an_item_error() {
    let vfs = memory();
    file(&vfs, "/l/a/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let op = copy_to(L, "/l/a/inner", PathStructure::Relative);
    let out = plan_of(&vfs, &cmp, &[("a", L)], &op).unwrap();
    let error = out
        .plan
        .items
        .iter()
        .find_map(|item| item.error.as_ref())
        .unwrap_or_else(|| panic!("{:?}", lines(&out)));
    assert!(error.message.contains("inside the source"), "{error:?}");
    let report = run(&vfs, out);
    assert!(
        read(&vfs, "/l/a/inner/a/x").is_none(),
        "{:?}",
        report.results
    );
}

#[test]
fn a6_19_moveto_as_one_rename_then_row_by_row_across_devices() {
    let (vfs, cmp) = two_folders();
    let op = FolderOp::MoveTo {
        sides: L,
        target: mspec("/out"),
        folder: false,
        structure: PathStructure::Relative,
        overwrite: OverwritePolicy::Fail,
    };
    let out = plan_of(&vfs, &cmp, &[("a", L)], &op).unwrap();
    assert_eq!(lines(&out), ["rename /out/a @0"]);
    let (vfs, cmp) = two_folders();
    vfs.set_device(&host("/out"), 5);
    let out = plan_of(&vfs, &cmp, &[("a/b", L)], &op).unwrap();
    assert_eq!(
        lines(&out),
        [
            "create_dir /out/b @0",
            "move /out/b/x.txt @1",
            "remove_dir /l/a/b @0"
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(read(&vfs, "/l/a/b/x.txt").is_none());
    assert_eq!(read(&vfs, "/out/b/x.txt").unwrap(), b"1");
}

#[test]
fn a6_19_script_example_four() {
    // left.diff.files left.orphan.files, relative, a `.zip` named by date in the test.
    let world = world();
    world.mem.insert_dir(&host("/out"), meta(1)).unwrap();
    file(&world.mem, "/l/src/changed.txt", b"new text", 20);
    file(&world.mem, "/r/src/changed.txt", b"old", 10);
    file(&world.mem, "/l/src/added.txt", b"added", 10);
    file(&world.mem, "/l/src/same.txt", b"s", 10);
    file(&world.mem, "/r/src/same.txt", b"s", 10);
    let cmp = world.compared(&FolderRules::default());
    let read_tree = cmp.read();
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let view = SelectScope {
        display: &display,
        expanded: None,
    };
    let masks = [
        SelectMask::parse("left.diff.files").unwrap(),
        SelectMask::parse("left.orphan.files").unwrap(),
    ];
    let mut selection = Selection::default();
    select(&read_tree, &view, &masks, false, &mut selection);
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let op = FolderOp::CopyTo {
        sides: L,
        target: mspec("/out/2026-11-02.zip"),
        folder: false,
        structure: PathStructure::Relative,
        overwrite: OverwritePolicy::Fail,
    };
    let out = plan_op(
        &read_tree,
        &world.vfs,
        &scope,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read_tree);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let zip = mspec("/out/2026-11-02.zip");
    for name in ["added.txt", "changed.txt"] {
        let inside = Spec::archive_member(zip.clone(), name);
        assert!(world.vfs.stat(&inside, FollowLinks::No).is_ok(), "{name}");
    }
    let same = Spec::archive_member(zip, "same.txt");
    assert!(world.vfs.stat(&same, FollowLinks::No).is_err());
}

#[test]
fn a6_19_a_target_that_is_a_file_refuses_the_plan() {
    let vfs = memory();
    file(&vfs, "/l/x", b"1", 10);
    file(&vfs, "/out", b"a file", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let op = copy_to(L, "/out", PathStructure::Relative);
    assert!(plan_of(&vfs, &cmp, &[("x", L)], &op).is_err());
}

#[test]
fn a6_19_members_are_copied_out_of_an_archive() {
    let world = world();
    world.mem.insert_dir(&host("/out"), meta(1)).unwrap();
    file(&world.mem, "/l/x.txt", b"1", 10);
    let op = FolderOp::Mkdir {
        sides: L,
        parent: NodeId::ROOT,
        name: "made.zip".to_owned(),
        folder: false,
    };
    let cmp = world.compared(&FolderRules::default());
    run(&world.vfs, plan_of(&world.vfs, &cmp, &[], &op).unwrap());
    // A copy into the archive, then out of it, through opened rows.
    let mut rules = FolderRules::default();
    rules.handling.archives = ArchiveHandling::Always;
    let cmp = world.compared(&rules);
    let copy = FolderOp::CopyTo {
        sides: L,
        target: Spec::archive_member(mspec("/l/made.zip"), ""),
        folder: false,
        structure: PathStructure::Relative,
        overwrite: OverwritePolicy::Fail,
    };
    let report = run(
        &world.vfs,
        plan_of(&world.vfs, &cmp, &[("x.txt", L)], &copy).unwrap(),
    );
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let cmp = world.compared(&rules);
    let out = copy_to(L, "/out", PathStructure::Relative);
    let report = run(
        &world.vfs,
        plan_of(&world.vfs, &cmp, &[("made.zip/x.txt", L)], &out).unwrap(),
    );
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&world.mem, "/out/x.txt").unwrap(), b"1");
}

/// Files not named `*.txt` are hidden, so a folder `c.txt` shows while its `a` does not.
fn only_txt() -> FolderRules {
    let mut rules = FolderRules::default();
    let patch = hikasa_folder::FolderRulesPatch {
        name_filter: Some("*.txt".to_owned()),
        ..hikasa_folder::FolderRulesPatch::default()
    };
    rules.apply(&patch).unwrap();
    rules
}

/// Rule 16 (§1.6.25 W6b note 42): a folder planned only as the way to sources that all stay writes nothing, so it claims
/// no path. Left `c.txt/a` is filtered; the right's file `c.txt` takes `/o/t/c.txt`, and `/o/t` is made for it (P-OPS,
/// seed `a049c850`).
#[test]
fn a6_19_copyto_a_way_folder_with_nothing_in_it_is_no_duplicate() {
    let vfs = memory();
    vfs.insert_dir(&host("/o"), meta(1)).unwrap();
    file(&vfs, "/l/c.txt/a", b"hidden", 10);
    file(&vfs, "/l/a", b"hidden", 10);
    file(&vfs, "/r/c.txt", b"right", 10);
    file(&vfs, "/r/a", b"hidden", 10);
    let cmp = compared(&vfs, &only_txt());
    let read_tree = cmp.read();
    let mut selection = Selection::default();
    let both = SideSet::all();
    for id in read_tree.children(NodeId::ROOT).collect::<Vec<_>>() {
        let view = read_tree.node(id).unwrap();
        if view.is_folder() {
            for child in read_tree.children(id) {
                selection.insert(child, both);
            }
        } else {
            selection.insert(id, both);
        }
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let op = FolderOp::CopyTo {
        sides: both,
        target: mspec("/o/t"),
        folder: false,
        structure: PathStructure::Base,
        overwrite: OverwritePolicy::Always,
    };
    let out = plan_op(
        &read_tree,
        &vfs,
        &scope,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read_tree);
    assert_eq!(
        lines(&out),
        [
            "create_dir /o/t @0",
            "skip /o/t/c.txt/a @1 !filtered",
            "skip /o/t/a @0 !filtered",
            "skip /o/t/a @0 !filtered",
            "copy /o/t/c.txt @0",
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/o/t/c.txt").unwrap(), b"right");
}

/// The other half of note 42: a folder of the other side that lands on such a way folder is a source, and the folder is
/// made for it, though nothing goes into it.
#[test]
fn a6_19_copyto_a_folder_that_merges_into_a_way_folder_is_made() {
    let vfs = memory();
    vfs.insert_dir(&host("/o"), meta(1)).unwrap();
    file(&vfs, "/l/c.txt/a", b"hidden", 10);
    vfs.insert_dir(&host("/r/c.txt"), meta(1)).unwrap();
    let cmp = compared(&vfs, &only_txt());
    let out = plan_of(
        &vfs,
        &cmp,
        &[("c.txt/a", L), ("c.txt", R)],
        &FolderOp::CopyTo {
            sides: SideSet::all(),
            target: mspec("/o/t"),
            folder: false,
            structure: PathStructure::Base,
            overwrite: OverwritePolicy::Always,
        },
    )
    .unwrap();
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    let made = vfs.stat(&mspec("/o/t/c.txt"), FollowLinks::No).unwrap();
    assert_eq!(made.kind, EntryKind::Dir);
}

/// Note 42 on a back-end that folds case: the left's way folder `D` holds only a filtered file, the right's `d/y.txt` goes
/// through the same folder by its other spelling, so the folder is made.
#[test]
fn a6_19_copyto_a_way_folder_used_by_another_spelling_is_made() {
    let vfs = MemoryVfs::new(CaseSensitivity::Insensitive);
    for dir in ["/l", "/r", "/o"] {
        vfs.insert_dir(&host(dir), meta(1)).unwrap();
    }
    file(&vfs, "/l/D/a", b"hidden", 10);
    file(&vfs, "/r/d/y.txt", b"right", 10);
    let mut rules = only_txt();
    rules.alignment.case = hikasa_folder::CaseMode::Insensitive;
    let cmp = compared(&vfs, &rules);
    let out = plan_of(
        &vfs,
        &cmp,
        &[("D/a", L), ("D/y.txt", R)],
        &FolderOp::CopyTo {
            sides: SideSet::all(),
            target: mspec("/o/t"),
            folder: false,
            structure: PathStructure::Base,
            overwrite: OverwritePolicy::Always,
        },
    )
    .unwrap();
    let shown = lines(&out);
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{shown:?} {:?}", report.results);
    assert_eq!(
        read(&vfs, "/o/t/D/y.txt")
            .or_else(|| read(&vfs, "/o/t/d/y.txt"))
            .unwrap(),
        b"right"
    );
}

/// The mirror of note 42: the left's file `c.txt` goes first, and the right's filtered `c.txt/a` would need a folder there.
/// It writes nothing, so it is `skip filtered`, not an error of a row nobody sees.
#[test]
fn a6_19_copyto_a_filtered_row_behind_a_file_of_the_plan_is_skipped() {
    let vfs = memory();
    vfs.insert_dir(&host("/o"), meta(1)).unwrap();
    file(&vfs, "/l/c.txt", b"left", 10);
    file(&vfs, "/r/c.txt/a", b"hidden", 10);
    let cmp = compared(&vfs, &only_txt());
    let read_tree = cmp.read();
    let mut selection = Selection::default();
    for id in read_tree.children(NodeId::ROOT).collect::<Vec<_>>() {
        if read_tree.node(id).unwrap().is_folder() {
            for child in read_tree.children(id) {
                selection.insert(child, R);
            }
        } else {
            selection.insert(id, L);
        }
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let op = FolderOp::CopyTo {
        sides: SideSet::all(),
        target: mspec("/o/t"),
        folder: false,
        structure: PathStructure::Base,
        overwrite: OverwritePolicy::Always,
    };
    let out = plan_op(
        &read_tree,
        &vfs,
        &scope,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(read_tree);
    assert_eq!(
        lines(&out),
        [
            "create_dir /o/t @0",
            "copy /o/t/c.txt @0",
            "skip /o/t/c.txt/a @0 !filtered",
        ]
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&vfs, "/o/t/c.txt").unwrap(), b"left");
}
