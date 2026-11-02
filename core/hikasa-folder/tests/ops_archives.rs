//! M2 W6b: operations with archives as sources and targets (rule 17, §1.6.5, §1.6.12), and what a plan does on a tree that
//! changed since the comparison: the cases where an operation could lose a file, end to end.

mod common;

use std::sync::Arc;

use common::{compared, default_now, engines, host, mspec};
use hikasa_core::{
    CancelToken, CaseSensitivity, Ctl, ErrorKind, FileTime, NullProgress, RelPath, Side, Spec,
    WarningCode,
};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, FolderOp, OpPlan, OpScope, OpSettings, PathStructure,
    ReadOnlySides, Recycle, plan_op,
};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::{
    ArchiveHandling, FolderComparison, FolderEnv, FolderModel, FolderRules, NullObserver,
    compare_into,
};
use hikasa_vfs::ops::{
    self, Action, BackupPolicy, BackupStyle, ItemStatus, OpReport, OverwritePolicy, SkipReason,
};
use hikasa_vfs::{
    ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, FollowLinks, MemMeta,
    MemoryVfs, Vfs,
};

const L: SideSet = SideSet::LEFT;
const R: SideSet = SideSet::RIGHT;

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

fn tar_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(name.as_bytes());
        header[100..107].copy_from_slice(b"0000644");
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[136..147].copy_from_slice(b"15000000000");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    out
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
        (ArchiveFormat::Tar, "*.tar".to_owned()),
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
    fn compared(&self, left: Spec, right: Spec, rules: &FolderRules) -> FolderComparison {
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
        let model = FolderModel::new(left, right, rules.clone()).unwrap();
        compare_into(&model, &env).unwrap()
    }

    fn tree(&self) -> FolderComparison {
        self.compared(mspec("/l"), mspec("/r"), &opened())
    }

    fn member(&self, archive: &str, member: &str) -> Option<Vec<u8>> {
        let spec = Spec::archive_member(mspec(archive), member);
        let mut reader = self.vfs.open_read(&spec).ok()?;
        let mut out = Vec::new();
        reader.read_to_end(&mut out).ok()?;
        Some(out)
    }
}

fn opened() -> FolderRules {
    let mut rules = FolderRules::default();
    rules.handling.archives = ArchiveHandling::Always;
    rules
}

fn plan_with(
    vfs: &dyn Vfs,
    cmp: &FolderComparison,
    rows: &[(&str, SideSet)],
    op: &FolderOp,
    settings: &OpSettings,
) -> OpPlan {
    let view = cmp.read();
    let mut selection = Selection::default();
    for (path, sides) in rows {
        let row = view
            .node_at(&RelPath::parse(path).unwrap())
            .unwrap_or_else(|| panic!("{path}"));
        selection.insert(row, *sides);
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    plan_op(&view, vfs, &scope, op, settings, &Ctl::detached()).unwrap()
}

fn plan(vfs: &dyn Vfs, cmp: &FolderComparison, rows: &[(&str, SideSet)], op: &FolderOp) -> OpPlan {
    plan_with(vfs, cmp, rows, op, &OpSettings::default())
}

fn run(vfs: &dyn Vfs, plan: OpPlan) -> OpReport {
    let plan = Confirmation::new(&plan, &ConfirmPolicy::ALL, ConfirmMode::YesToAll)
        .finish(plan)
        .unwrap();
    ops::execute_with(vfs, vfs, plan.plan, &plan.exec, &Ctl::detached())
}

const COPY: FolderOp = FolderOp::Copy {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};
const MOVE: FolderOp = FolderOp::Move {
    to: Side::Right,
    overwrite: OverwritePolicy::Always,
};

fn backups() -> OpSettings {
    OpSettings {
        backup: Some(BackupPolicy {
            style: BackupStyle::NameBak,
            folder: None,
        }),
        ..OpSettings::default()
    }
}

#[test]
fn a6_15_rule17_a_copy_within_opened_archives_rebuilds_once_with_one_backup() {
    let world = world();
    world
        .mem
        .insert_file(
            &host("/l/a.tar"),
            tar_of(&[("x", b"new1"), ("y", b"new2")]),
            meta(20),
        )
        .unwrap();
    world
        .mem
        .insert_file(
            &host("/r/a.tar"),
            tar_of(&[("x", b"old1"), ("y", b"old2"), ("z", b"keep")]),
            meta(10),
        )
        .unwrap();
    let cmp = world.tree();
    let out = plan_with(
        &world.vfs,
        &cmp,
        &[("a.tar/x", L), ("a.tar/y", L)],
        &COPY,
        &backups(),
    );
    // No per-member backups, one of the archive.
    assert!(out.plan.items.iter().all(|item| item.backup.is_none()));
    assert_eq!(out.exec.archive_backups.len(), 1);
    let backup = out.exec.archive_backups.values().next().unwrap().clone();
    let before = read(&world.mem, "/r/a.tar").unwrap();
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(world.member("/r/a.tar", "x").unwrap(), b"new1");
    assert_eq!(world.member("/r/a.tar", "z").unwrap(), b"keep");
    assert_eq!(
        world.mem.read_back(&backup.canonical()).unwrap().to_vec(),
        before
    );
}

#[test]
fn a6_19_a_move_into_a_tar_removes_its_sources_after_the_rebuild() {
    let world = world();
    file(&world.mem, "/l/d/x", b"1", 10);
    file(&world.mem, "/l/d/y", b"2", 10);
    let tar = tar_of(&[("keep", b"k")]);
    world
        .mem
        .insert_file(&host("/out.tar"), tar, meta(10))
        .unwrap();
    let cmp = world.tree();
    let op = FolderOp::MoveTo {
        sides: L,
        target: mspec("/out.tar"),
        folder: false,
        structure: PathStructure::Relative,
        overwrite: OverwritePolicy::Fail,
    };
    let view = cmp.read();
    let mut selection = Selection::default();
    selection.insert(view.node_at(&RelPath::parse("d").unwrap()).unwrap(), L);
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    // `/out.tar` as a folder is its root.
    let spec = Spec::archive_member(mspec("/out.tar"), "");
    let op = match op {
        FolderOp::MoveTo {
            sides,
            structure,
            overwrite,
            ..
        } => FolderOp::MoveTo {
            sides,
            target: spec,
            folder: false,
            structure,
            overwrite,
        },
        other => other,
    };
    let out = plan_op(
        &view,
        &world.vfs,
        &scope,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    drop(view);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(world.member("/out.tar", "d/x").unwrap(), b"1");
    assert_eq!(world.member("/out.tar", "keep").unwrap(), b"k");
    assert!(read(&world.mem, "/l/d/x").is_none());
    assert!(world.mem.stat(&mspec("/l/d"), FollowLinks::No).is_err());
}

#[test]
fn a6_19_a_move_out_of_an_archive_takes_the_member_out_of_it() {
    let world = world();
    world
        .mem
        .insert_file(
            &host("/l/a.tar"),
            tar_of(&[("x", b"1"), ("y", b"2")]),
            meta(20),
        )
        .unwrap();
    let cmp = world.tree();
    let out = plan(&world.vfs, &cmp, &[("a.tar/x", L)], &MOVE);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&world.mem, "/r/a.tar/x").unwrap(), b"1");
    assert!(world.member("/l/a.tar", "x").is_none());
    assert_eq!(world.member("/l/a.tar", "y").unwrap(), b"2");
}

#[test]
fn a6_15_an_opened_archive_that_goes_whole_goes_as_the_file_it_is() {
    let world = world();
    let bytes = tar_of(&[("x", b"1")]);
    world
        .mem
        .insert_file(&host("/l/a.tar"), bytes.clone(), meta(20))
        .unwrap();
    let cmp = world.tree();
    let out = plan(&world.vfs, &cmp, &[("a.tar", L)], &COPY);
    assert_eq!(out.plan.items.len(), 1, "{:?}", out.plan.items);
    assert_eq!(out.plan.items[0].action, Action::Copy);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&world.mem, "/r/a.tar").unwrap(), bytes);
    let delete = FolderOp::Delete {
        sides: L,
        recycle: Recycle::default(),
    };
    let cmp = world.tree();
    let out = plan(&world.vfs, &cmp, &[("a.tar", L)], &delete);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(read(&world.mem, "/l/a.tar").is_none());
}

#[test]
fn a6_19_copyto_of_an_opened_archive_copies_the_file() {
    let world = world();
    let bytes = tar_of(&[("x", b"1")]);
    world
        .mem
        .insert_file(&host("/l/a.tar"), bytes.clone(), meta(20))
        .unwrap();
    world.mem.insert_dir(&host("/out"), meta(1)).unwrap();
    let cmp = world.tree();
    let op = FolderOp::CopyTo {
        sides: L,
        target: mspec("/out"),
        folder: false,
        structure: PathStructure::Relative,
        overwrite: OverwritePolicy::Fail,
    };
    let out = plan(&world.vfs, &cmp, &[("a.tar", L)], &op);
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert_eq!(read(&world.mem, "/out/a.tar").unwrap(), bytes);
}

#[test]
fn a6_15_rule11_a_single_stream_archive_is_read_only_with_one_warning() {
    // A stream holds one member: what would write it is `skip capability`.
    let world = world();
    let tar = tar_of(&[("x", b"1")]);
    world
        .mem
        .insert_file(&host("/l/a.tar"), tar, meta(20))
        .unwrap();
    let mut gz = Vec::new();
    {
        // A stored deflate block in a gzip frame.
        gz.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3]);
        let data = b"text";
        gz.extend_from_slice(&[1, 4, 0, 0xfb, 0xff]);
        gz.extend_from_slice(data);
        let crc = crc32(data);
        gz.extend_from_slice(&crc.to_le_bytes());
        gz.extend_from_slice(&(data.len() as u32).to_le_bytes());
    }
    world
        .mem
        .insert_file(&host("/r/s.gz"), gz, meta(20))
        .unwrap();
    file(&world.mem, "/l/one", b"1", 10);
    file(&world.mem, "/l/two", b"2", 10);
    let cmp = world.compared(
        mspec("/l"),
        Spec::archive_member(mspec("/r/s.gz"), ""),
        &FolderRules::default(),
    );
    let out = plan(&world.vfs, &cmp, &[("one", L), ("two", L)], &COPY);
    assert!(
        out.plan
            .items
            .iter()
            .all(|item| item.skip == Some(SkipReason::Capability)),
        "{:?}",
        out.plan.items
    );
    let warned = out
        .warnings
        .iter()
        .filter(|warning| warning.code == WarningCode::CapabilityMissing)
        .count();
    assert_eq!(warned, 1, "{:?}", out.warnings);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn a6_15_rule13_members_of_two_archives_are_not_exchanged() {
    let world = world();
    world
        .mem
        .insert_file(&host("/l/a.tar"), tar_of(&[("x", b"1")]), meta(20))
        .unwrap();
    world
        .mem
        .insert_file(&host("/r/a.tar"), tar_of(&[("x", b"22")]), meta(20))
        .unwrap();
    let cmp = world.tree();
    let out = plan(&world.vfs, &cmp, &[("a.tar/x", L)], &FolderOp::Exchange);
    assert_eq!(
        out.plan.items[0].error.as_ref().unwrap().kind,
        ErrorKind::Unsupported
    );
    let report = run(&world.vfs, out);
    assert_eq!(report.summary.failed, 1);
    assert_eq!(world.member("/l/a.tar", "x").unwrap(), b"1");
}

#[test]
fn a6_15_a_file_changed_after_the_plan_is_not_deleted() {
    let vfs = memory();
    file(&vfs, "/l/x", b"seen", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let delete = FolderOp::Delete {
        sides: L,
        recycle: Recycle::default(),
    };
    let out = plan(&vfs, &cmp, &[("x", L)], &delete);
    file(&vfs, "/l/x", b"edited", 99);
    let report = run(&vfs, out);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/l/x").unwrap(), b"edited");
}

#[test]
fn a6_15_a_folder_swapped_for_a_link_after_the_plan_is_not_written_through() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"new", 20);
    file(&vfs, "/r/d/x", b"old", 10);
    file(&vfs, "/outside/x", b"outside", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let out = plan(&vfs, &cmp, &[("d/x", L)], &COPY);
    vfs.remove(&mspec("/r/d/x")).unwrap();
    vfs.remove(&mspec("/r/d")).unwrap();
    vfs.insert_symlink(&host("/r/d"), &host("/outside"), meta(10))
        .unwrap();
    let report = run(&vfs, out);
    assert_eq!(
        report.results[0].status,
        ItemStatus::Failed,
        "{:?}",
        report.results
    );
    assert_eq!(read(&vfs, "/outside/x").unwrap(), b"outside");
}

#[test]
fn a6_15_a_folder_and_its_child_on_both_sides() {
    let vfs = memory();
    file(&vfs, "/l/d/x", b"1", 10);
    file(&vfs, "/l/d/y", b"2", 10);
    file(&vfs, "/r/d/x", b"1", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let delete = FolderOp::Delete {
        sides: SideSet::all(),
        recycle: Recycle::default(),
    };
    // The left has more than the row selected: only that row goes there. The right has only it: its folder goes too.
    let out = plan(
        &vfs,
        &cmp,
        &[("d", SideSet::all()), ("d/x", SideSet::all())],
        &delete,
    );
    let report = run(&vfs, out);
    assert_eq!(report.summary.failed, 0, "{:?}", report.results);
    assert!(read(&vfs, "/l/d/x").is_none());
    assert_eq!(read(&vfs, "/l/d/y").unwrap(), b"2");
    assert!(vfs.stat(&mspec("/r/d"), FollowLinks::No).is_err());
}

#[test]
fn a6_15_a_selection_on_the_other_side_of_a_pair_stays_on_it() {
    let vfs = memory();
    file(&vfs, "/l/x", b"1", 10);
    file(&vfs, "/r/x", b"2", 10);
    let cmp = compared(&vfs, &FolderRules::default());
    let delete = FolderOp::Delete {
        sides: R,
        recycle: Recycle::default(),
    };
    let out = plan(&vfs, &cmp, &[("x", R)], &delete);
    run(&vfs, out);
    assert_eq!(read(&vfs, "/l/x").unwrap(), b"1");
    assert!(read(&vfs, "/r/x").is_none());
}
