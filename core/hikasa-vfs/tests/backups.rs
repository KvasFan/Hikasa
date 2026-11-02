//! M2 W6a acceptance A6.6 (D-167, D-168): the names of backups and how they are made, by a copy, a move, a link and a
//! folder, with every fault of §1.6.5; archives, whose backup is one copy of the outermost file per operation.

mod common;

use common::archives::{OnMemory, TreeData, bytes_of, on_memory};
use common::{ft, host, mspec, read_to_vec};
use hikasa_core::{CaseSensitivity, Ctl, ErrorKind, RelPath, Spec};
use hikasa_vfs::ops::{
    self, Action, BackupNames, BackupPolicy, BackupStyle, BatchOptions, ExecOptions, ItemStatus,
    OpKind, OpReport, Plan, PlanItem, PlanOptions, write_backup,
};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, Attrs, EntryKind, Fault, LocalVfs, MemDump, MemMeta, MemoryVfs,
    UnixMeta, Vfs, VfsError, VfsOp,
};

fn memory() -> MemoryVfs {
    MemoryVfs::new(CaseSensitivity::Sensitive)
}

fn sibling(style: BackupStyle, name: &str) -> String {
    let policy = BackupPolicy {
        style,
        folder: None,
    };
    let backup = policy
        .target(&memory(), &mspec(&format!("/d/{name}")), None)
        .unwrap();
    assert_eq!(backup.parent(), Some(mspec("/d")));
    backup.file_name().unwrap()
}

/// The whole table of §1.6.5.
#[test]
fn a6_6_the_table_of_names() {
    use BackupStyle::{NameBak, NameTilde, StemBak, StemBakExt, StemTilde, TildeName};
    let rows = [
        (
            NameTilde,
            ["abc.txt~", "Makefile~", ".bashrc~", "a.tar.gz~"],
        ),
        (
            StemTilde,
            ["abc~.txt", "Makefile~", ".bashrc~", "a.tar~.gz"],
        ),
        (
            NameBak,
            ["abc.txt.bak", "Makefile.bak", ".bashrc.bak", "a.tar.gz.bak"],
        ),
        (
            TildeName,
            ["~abc.txt", "~Makefile", "~.bashrc", "~a.tar.gz"],
        ),
        (
            StemBak,
            ["abc.bak", "Makefile.bak", ".bashrc.bak", "a.tar.bak"],
        ),
        (
            StemBakExt,
            ["abc.bak.txt", "Makefile.bak", ".bashrc.bak", "a.tar.bak.gz"],
        ),
    ];
    for (style, names) in rows {
        for (name, expected) in ["abc.txt", "Makefile", ".bashrc", "a.tar.gz"]
            .iter()
            .zip(names)
        {
            assert_eq!(sibling(style, name), expected, "{style:?}");
        }
    }
}

/// A name with no stem, a trailing dot, and a name the style would give back unchanged.
#[test]
fn a6_6_odd_names() {
    assert_eq!(sibling(BackupStyle::StemBak, ".txt"), ".txt.bak");
    assert_eq!(sibling(BackupStyle::StemBakExt, ".txt"), ".txt.bak");
    assert_eq!(sibling(BackupStyle::StemBak, "a."), "a..bak");
    assert_eq!(sibling(BackupStyle::StemTilde, "a."), "a.~");
    assert_eq!(sibling(BackupStyle::StemBak, "..a"), "..bak");
    // A backup must not be what it keeps.
    assert_eq!(sibling(BackupStyle::StemBak, "abc.bak"), "abc-2.bak");
}

/// A non-UTF-8 name keeps its bytes.
#[cfg(unix)]
#[test]
fn a6_6_a_name_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let name = OsStr::from_bytes(b"caf\xE9.txt");
    let vfs = LocalVfs::default();
    let item = common::lspec(&root.join(name));
    let policy = BackupPolicy {
        style: BackupStyle::StemBakExt,
        folder: None,
    };
    let backup = policy.target(&vfs, &item, None).unwrap();
    let Spec::Local(path) = &backup else {
        panic!("{backup:?}")
    };
    assert_eq!(path.file_name().unwrap().as_bytes(), b"caf\xE9.bak.txt");
}

/// `Folder` with `rel` and without; without a folder it refuses.
#[test]
fn a6_6_the_folder_style() {
    let vfs = memory();
    let policy = BackupPolicy {
        style: BackupStyle::Folder,
        folder: Some(mspec("/bak")),
    };
    let item = mspec("/base/a/b/x.txt");
    let rel = RelPath::parse("a/b/x.txt").unwrap();
    assert_eq!(
        policy.target(&vfs, &item, Some(&rel)).unwrap(),
        mspec("/bak/a/b/x.txt")
    );
    assert_eq!(
        policy.target(&vfs, &item, None).unwrap(),
        mspec("/bak/x.txt")
    );
    let none = BackupPolicy {
        style: BackupStyle::Folder,
        folder: None,
    };
    let error = none.target(&vfs, &item, Some(&rel)).unwrap_err();
    assert!(
        error.to_string().contains("needs a backup folder"),
        "{error}"
    );
}

/// Names are final at planning: two items are told apart, in plan order, a folder there is an error.
#[test]
fn a6_6_numbering_and_folders_at_planning() {
    let vfs = memory();
    let stem = BackupPolicy {
        style: BackupStyle::StemBak,
        folder: None,
    };
    let mut names = BackupNames::default();
    let one = names
        .assign(&stem, &vfs, &mspec("/d/abc.txt"), None)
        .unwrap();
    let two = names
        .assign(&stem, &vfs, &mspec("/d/abc.doc"), None)
        .unwrap();
    let three = names.assign(&stem, &vfs, &mspec("/d/abc.c"), None).unwrap();
    assert_eq!(one, mspec("/d/abc.bak"));
    assert_eq!(two, mspec("/d/abc-2.bak"));
    assert_eq!(three, mspec("/d/abc-3.bak"));
    // `left` and `right` of one path with `folder`.
    let folder = BackupPolicy {
        style: BackupStyle::Folder,
        folder: Some(mspec("/bak")),
    };
    let rel = RelPath::parse("a/x.txt").unwrap();
    let mut names = BackupNames::default();
    let left = names
        .assign(&folder, &vfs, &mspec("/l/a/x.txt"), Some(&rel))
        .unwrap();
    let right = names
        .assign(&folder, &vfs, &mspec("/r/a/x.txt"), Some(&rel))
        .unwrap();
    assert_eq!(left, mspec("/bak/a/x.txt"));
    assert_eq!(right, mspec("/bak/a/x-2.txt"));
    // A file there is replaced; a folder there is the item's error.
    let vfs = memory();
    vfs.insert_file(&host("/d/abc.bak"), &b"old"[..], MemMeta::default())
        .unwrap();
    vfs.insert_dir(&host("/d/dir.bak"), MemMeta::default())
        .unwrap();
    let mut names = BackupNames::default();
    assert!(
        names
            .assign(&stem, &vfs, &mspec("/d/abc.txt"), None)
            .is_ok()
    );
    let error = names
        .assign(&stem, &vfs, &mspec("/d/dir.txt"), None)
        .unwrap_err();
    assert!(matches!(error, VfsError::IsADirectory { .. }), "{error}");
}

/// "A backup must not be what it keeps", in the `folder` style too, whose name can be the item's own.
#[test]
fn a6_6_the_folder_of_the_item_itself_is_numbered() {
    let vfs = memory();
    let policy = BackupPolicy {
        style: BackupStyle::Folder,
        folder: Some(mspec("/right")),
    };
    let target = mspec("/right/x.txt");
    assert_eq!(
        policy.target(&vfs, &target, None).unwrap(),
        mspec("/right/x-2.txt")
    );
    let rel = RelPath::parse("a/x.txt").unwrap();
    let target = mspec("/right/a/x.txt");
    assert_eq!(
        policy.target(&vfs, &target, Some(&rel)).unwrap(),
        mspec("/right/a/x-2.txt")
    );
    // By the back-end's case: `ABC.bak` is the item `abc.bak` there.
    let insensitive = MemoryVfs::new(CaseSensitivity::Insensitive);
    let stem = BackupPolicy {
        style: BackupStyle::StemBak,
        folder: None,
    };
    assert_eq!(
        stem.target(&insensitive, &mspec("/d/ABC.BAK"), None).unwrap(),
        mspec("/d/ABC-2.BAK")
    );
}

/// `x` and `x.bak` in one plan, with `name_bak`: reserved first, a backup numbers around every path of the plan, in
/// either order, and nothing is lost.
#[test]
fn a6_6_a_backup_numbers_around_the_paths_of_the_plan() {
    let policy = BackupPolicy {
        style: BackupStyle::NameBak,
        folder: None,
    };
    for order in [["x", "x.bak"], ["x.bak", "x"]] {
        let vfs = memory();
        for (side, text) in [("left", "new"), ("right", "old")] {
            for name in ["x", "x.bak"] {
                let path = format!("/{side}/{name}");
                let bytes = format!("{text} {name}").into_bytes();
                vfs.insert_file(&host(&path), bytes, meta(1)).unwrap();
            }
        }
        let mut names = BackupNames::default();
        for name in order {
            names.reserve(&vfs, &mspec(&format!("/left/{name}")));
            names.reserve(&vfs, &mspec(&format!("/right/{name}")));
        }
        let mut items = Vec::new();
        for name in order {
            let target = mspec(&format!("/right/{name}"));
            let mut item = PlanItem::planned(
                Action::Copy,
                EntryKind::File,
                Some(mspec(&format!("/left/{name}"))),
                target.clone(),
                0,
            );
            item.replaces = true;
            item.backup = Some(names.assign(&policy, &vfs, &target, None).unwrap());
            items.push(item);
        }
        let report = run(&vfs, batch(items));
        assert!(report.ok(), "{order:?}: {:?}", report.results);
        assert_eq!(content(&vfs, "/right/x"), b"new x");
        assert_eq!(content(&vfs, "/right/x.bak"), b"new x.bak");
        assert_eq!(content(&vfs, "/right/x-2.bak"), b"old x");
        assert_eq!(content(&vfs, "/right/x.bak.bak"), b"old x.bak");
    }
}

/// `assign` alone knows the items it has seen, so the later of the two is the one a reserve is for.
#[test]
fn a6_6_an_item_assigned_earlier_keeps_its_name() {
    let vfs = memory();
    let policy = BackupPolicy {
        style: BackupStyle::NameBak,
        folder: None,
    };
    let mut names = BackupNames::default();
    names.assign(&policy, &vfs, &mspec("/d/x.bak"), None).unwrap();
    assert_eq!(
        names.assign(&policy, &vfs, &mspec("/d/x"), None).unwrap(),
        mspec("/d/x-2.bak")
    );
}

// ---- execution -----------------------------------------------------------------------------------------------

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: Some(ft(secs)),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: 0o640,
                uid: 0,
                gid: 0,
            },
        }),
        ..MemMeta::default()
    }
}

fn replaced() -> MemoryVfs {
    let vfs = memory();
    vfs.insert_file(&host("/s"), &b"new content"[..], meta(200))
        .unwrap();
    vfs.insert_file(&host("/t"), &b"old"[..], meta(100))
        .unwrap();
    vfs
}

fn copy_with_backup() -> PlanItem {
    let mut item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(mspec("/s")),
        mspec("/t"),
        0,
    );
    item.replaces = true;
    item.backup = Some(mspec("/t.bak"));
    item
}

fn batch(items: Vec<PlanItem>) -> Plan {
    Plan {
        op: OpKind::Batch,
        options: PlanOptions::Batch(BatchOptions::default()),
        items,
        warnings: Vec::new(),
    }
}

fn run(vfs: &dyn Vfs, plan: Plan) -> OpReport {
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn dump_of(vfs: &MemoryVfs, path: &str) -> MemDump {
    let path = host(path);
    vfs.dump().into_iter().find(|d| d.path == path).unwrap()
}

fn content(vfs: &MemoryVfs, path: &str) -> Vec<u8> {
    vfs.read_back(&host(path)).unwrap().to_vec()
}

/// A replaced file ends at its backup with old content, time and mode; stage, rename, commit; a second run replaces it.
#[test]
fn a6_6_a_replaced_file_ends_at_its_backup() {
    let vfs = replaced();
    let old = dump_of(&vfs, "/t");
    vfs.set_journal(true);
    vfs.set_journal_writes(true);
    let report = run(&vfs, batch(vec![copy_with_backup()]));
    assert!(report.ok(), "{:?}", report.results);
    let kept = dump_of(&vfs, "/t.bak");
    assert_eq!(
        (kept.crc32, kept.mtime, kept.attrs),
        (old.crc32, old.mtime, old.attrs)
    );
    assert_eq!(content(&vfs, "/t"), b"new content");
    let order: Vec<VfsOp> = vfs.journal().iter().map(|(op, _)| *op).collect();
    assert_eq!(
        order.get(..3),
        Some(&[VfsOp::Write, VfsOp::Rename, VfsOp::Commit][..]),
        "{order:?}"
    );
    // One generation: the next run replaces the backup.
    vfs.insert_file(&host("/s"), &b"third"[..], meta(300))
        .unwrap();
    let report = run(&vfs, batch(vec![copy_with_backup()]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(content(&vfs, "/t.bak"), b"new content");
    assert_eq!(content(&vfs, "/t"), b"third");
}

/// The mode a target had stays with it when the metadata of the source is not asked for.
#[test]
fn a6_6_no_preserve_keeps_the_mode_of_the_target() {
    let vfs = replaced();
    let plan = Plan {
        op: OpKind::Batch,
        options: PlanOptions::Batch(BatchOptions {
            preserve: ops::Preserve::NONE,
            ..BatchOptions::default()
        }),
        items: vec![copy_with_backup()],
        warnings: Vec::new(),
    };
    assert!(run(&vfs, plan).ok());
    assert_eq!(dump_of(&vfs, "/t").attrs, dump_of(&vfs, "/t.bak").attrs);
}

/// A stream fault leaves target and old backup.
#[test]
fn a6_6_a_stream_fault_leaves_target_and_backup() {
    for read in [true, false] {
        let vfs = replaced();
        vfs.insert_file(&host("/t.bak"), &b"older"[..], meta(50))
            .unwrap();
        if read {
            vfs.inject_read_error_after(&host("/s"), 4, ErrorKind::Io);
        } else {
            vfs.inject_write_error_after(&host("/t"), 4, ErrorKind::Io);
        }
        let before = vfs.dump();
        let report = run(&vfs, batch(vec![copy_with_backup()]));
        assert_eq!(report.results[0].status, ItemStatus::Failed);
        assert_eq!(vfs.dump(), before);
    }
}

/// `Fault { op: Rename }` fails the item with its target intact.
#[test]
fn a6_6_a_failing_rename_fails_the_item() {
    let vfs = replaced();
    vfs.inject(Fault {
        path: host("/t"),
        op: VfsOp::Rename,
        kind: ErrorKind::PermissionDenied,
    });
    let before = vfs.dump();
    let report = run(&vfs, batch(vec![copy_with_backup()]));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(vfs.dump(), before);
}

/// `Fault { op: Commit }` puts the target back; a second fault there names the backup.
#[test]
fn a6_6_a_failed_commit_puts_the_target_back() {
    let vfs = replaced();
    vfs.inject(Fault {
        path: host("/t"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    let old = dump_of(&vfs, "/t");
    let report = run(&vfs, batch(vec![copy_with_backup()]));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    let now = dump_of(&vfs, "/t");
    assert_eq!((now.crc32, now.mtime), (old.crc32, old.mtime));
    assert!(vfs.read_back(&host("/t.bak")).is_none());

    let vfs = replaced();
    vfs.inject(Fault {
        path: host("/t"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    vfs.inject(Fault {
        path: host("/t.bak"),
        op: VfsOp::Rename,
        kind: ErrorKind::Io,
    });
    let report = run(&vfs, batch(vec![copy_with_backup()]));
    let error = report.results[0].error.as_ref().unwrap();
    assert!(
        error.message.contains(&mspec("/t.bak").canonical()),
        "{}",
        error.message
    );
    assert_eq!(content(&vfs, "/t.bak"), b"old", "the old file is there");
}

/// Another disk: the backup is a copy, made while the target is intact, and its failure fails the item.
#[test]
fn a6_6_cross_device_takes_the_copy_path() {
    let vfs = replaced();
    vfs.insert_dir(&host("/other"), MemMeta::default()).unwrap();
    vfs.set_device(&host("/other"), 9);
    let mut item = copy_with_backup();
    item.backup = Some(mspec("/other/deep/t"));
    let report = run(&vfs, batch(vec![item.clone()]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(content(&vfs, "/other/deep/t"), b"old");
    assert_eq!(dump_of(&vfs, "/other/deep/t").mtime, Some(ft(100)));
    assert_eq!(dump_of(&vfs, "/other/deep/t").attrs, meta(1).attrs.unwrap());
    assert_eq!(content(&vfs, "/t"), b"new content");

    let vfs = replaced();
    vfs.insert_dir(&host("/other/deep"), MemMeta::default())
        .unwrap();
    vfs.set_device(&host("/other"), 9);
    vfs.inject(Fault {
        path: host("/other/deep/t"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    let before = vfs.dump();
    let report = run(&vfs, batch(vec![item]));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(vfs.dump(), before, "nothing is replaced without its backup");
}

/// A hand-built plan whose backup would lose a file is refused before any item runs.
#[test]
fn a6_6_a_plan_whose_backup_loses_a_file_is_refused() {
    const ITEM: &str = "is the item it backs up";
    const PLAN: &str = "is a path of another item of the plan";
    let second = || {
        let mut item = PlanItem::planned(
            Action::Copy,
            EntryKind::File,
            Some(mspec("/s2")),
            mspec("/t2"),
            0,
        );
        item.replaces = true;
        item
    };
    let with = |backup: &str| {
        let mut item = copy_with_backup();
        item.backup = Some(mspec(backup));
        item
    };
    let mut twice = second();
    twice.backup = Some(mspec("/t.bak"));
    let rows: Vec<(Vec<PlanItem>, usize, &str)> = vec![
        (vec![with("/t")], 0, ITEM),
        (vec![with("/s")], 0, PLAN),
        (vec![second(), with("/t2")], 1, PLAN),
        (vec![with("/t2"), second()], 0, PLAN),
        (vec![with("/s2"), second()], 0, PLAN),
        (vec![copy_with_backup(), twice], 1, PLAN),
    ];
    for (items, at, reason) in rows {
        let vfs = replaced();
        vfs.insert_file(&host("/s2"), &b"new two"[..], meta(200))
            .unwrap();
        vfs.insert_file(&host("/t2"), &b"old two"[..], meta(100))
            .unwrap();
        let before = vfs.dump();
        vfs.set_journal(true);
        let report = run(&vfs, batch(items));
        assert!(!report.ok());
        for (index, result) in report.results.iter().enumerate() {
            if index == at {
                assert_eq!(result.status, ItemStatus::Failed);
                let error = result.error.as_ref().unwrap();
                assert_eq!(error.kind, ErrorKind::InvalidInput);
                assert!(error.message.contains(reason), "{}", error.message);
            } else {
                assert_eq!(result.status, ItemStatus::NotRun);
            }
        }
        assert_eq!(vfs.dump(), before);
        assert!(vfs.journal().iter().all(|(op, _)| *op == VfsOp::Stat), "no item ran");
    }
}

/// The same on a disk: the backup is the file itself, and the old file stays.
#[test]
fn a6_6_a_backup_onto_itself_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("right")).unwrap();
    std::fs::write(root.join("s"), b"new").unwrap();
    std::fs::write(root.join("right/x"), b"old").unwrap();
    let vfs = LocalVfs::default();
    let target = common::lspec_new(&root.join("right/x"));
    let mut item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(common::lspec_new(&root.join("s"))),
        target.clone(),
        0,
    );
    item.replaces = true;
    item.backup = Some(target);
    let report = run(&vfs, batch(vec![item]));
    assert!(!report.ok());
    assert_eq!(std::fs::read(root.join("right/x")).unwrap(), b"old");
    assert_eq!(std::fs::read(root.join("s")).unwrap(), b"new");
}

/// A move: the same path; `folder` parents are not items.
#[test]
fn a6_6_a_move_backs_up_and_parents_are_no_items() {
    let vfs = replaced();
    let mut item = PlanItem::planned(
        Action::Move,
        EntryKind::File,
        Some(mspec("/s")),
        mspec("/t"),
        0,
    );
    item.replaces = true;
    item.backup = Some(mspec("/bak/a/t"));
    let report = run(&vfs, batch(vec![item]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(report.summary.planned, 1);
    assert_eq!(content(&vfs, "/bak/a/t"), b"old");
    assert_eq!(content(&vfs, "/t"), b"new content");
    assert!(vfs.read_back(&host("/s")).is_none());
}

/// `link` and `create_dir` replacing: the hidden sibling becomes the backup.
#[test]
fn a6_6_link_and_folder_replacing() {
    let vfs = replaced();
    vfs.insert_symlink(&host("/l"), "s", MemMeta::default())
        .unwrap();
    let mut link = PlanItem::planned(
        Action::Link,
        EntryKind::Symlink,
        Some(mspec("/l")),
        mspec("/t"),
        0,
    );
    link.replaces = true;
    link.backup = Some(mspec("/t.bak"));
    let report = run(&vfs, batch(vec![link.clone()]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(dump_of(&vfs, "/t").link_target.as_deref(), Some("s"));
    assert_eq!(content(&vfs, "/t.bak"), b"old");

    let vfs = replaced();
    let mut folder = PlanItem::planned(Action::CreateDir, EntryKind::Dir, None, mspec("/t"), 0);
    folder.replaces = true;
    folder.backup = Some(mspec("/t.bak"));
    let report = run(&vfs, batch(vec![folder.clone()]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(dump_of(&vfs, "/t").kind, EntryKind::Dir);
    assert_eq!(content(&vfs, "/t.bak"), b"old");
    assert_eq!(
        vfs.dump().len(),
        4,
        "no hidden sibling left: {:?}",
        vfs.dump()
    );

    // A failed backup: the new item goes, the old one is back.
    let vfs = replaced();
    vfs.insert_dir(&host("/t.bak"), MemMeta::default()).unwrap();
    let before = vfs.dump();
    let report = run(&vfs, batch(vec![folder]));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(vfs.dump(), before);
}

/// Remove has no backup.
#[test]
fn a6_6_remove_has_no_backup() {
    let vfs = replaced();
    let mut item = PlanItem::planned(Action::Remove, EntryKind::File, None, mspec("/t"), 0);
    item.backup = Some(mspec("/t.bak"));
    assert!(run(&vfs, batch(vec![item])).ok());
    assert!(vfs.read_back(&host("/t.bak")).is_none());
}

/// `write_backup`: a link stays a link, a folder is refused, a backup that is there is replaced.
#[test]
fn a6_6_write_backup_directly() {
    let vfs = memory();
    vfs.insert_symlink(&host("/l"), "x", MemMeta::default())
        .unwrap();
    vfs.insert_symlink(&host("/bak"), "old", MemMeta::default())
        .unwrap();
    vfs.insert_dir(&host("/d"), MemMeta::default()).unwrap();
    let ctl = Ctl::detached();
    write_backup(&vfs, &mspec("/l"), &mspec("/bak"), &ctl).unwrap();
    assert_eq!(dump_of(&vfs, "/bak").link_target.as_deref(), Some("x"));
    assert_eq!(vfs.dump().len(), 4, "{:?}", vfs.dump());
    let error = write_backup(&vfs, &mspec("/d"), &mspec("/e"), &ctl).unwrap_err();
    assert!(matches!(error, VfsError::IsADirectory { .. }), "{error}");
}

// ---- archives ------------------------------------------------------------------------------------------------

fn archive(memory: &OnMemory, name: &str) -> Spec {
    let tree: TreeData = vec![
        ("a".into(), b"member a".to_vec()),
        ("b".into(), b"member b".to_vec()),
    ];
    memory.put(name, bytes_of(ArchiveFormat::Zip, &tree))
}

fn backups(outer: &Spec, backup: &str) -> ExecOptions {
    ExecOptions {
        archive_backups: [(outer.canonical(), mspec(backup))].into(),
        ..ExecOptions::default()
    }
}

fn removal(member: &Spec) -> PlanItem {
    PlanItem::planned(Action::Remove, EntryKind::File, None, member.clone(), 0)
}

fn exec(vfs: &dyn Vfs, items: Vec<PlanItem>, options: &ExecOptions) -> OpReport {
    ops::execute_with(vfs, vfs, batch(items), options, &Ctl::detached())
}

fn commits_of(memory: &OnMemory, path: &str) -> usize {
    let spec = mspec(path).canonical();
    memory
        .mem
        .journal()
        .iter()
        .filter(|(op, at)| *op == VfsOp::Commit && *at == spec)
        .count()
}

/// An archive's backup that is the archive, or a path of the plan, is refused before the archive is touched.
#[test]
fn a6_6_an_archive_backup_that_loses_a_file_is_refused() {
    for backup in ["/a.zip", "/other"] {
        let memory = on_memory(ArchiveLimits::default());
        let file = archive(&memory, "a.zip");
        memory
            .mem
            .insert_file(&host("/other"), &b"o"[..], MemMeta::default())
            .unwrap();
        let before = memory.mem.dump();
        let member = Spec::archive_member(file.clone(), "a");
        let touched = PlanItem::planned(
            Action::Touch,
            EntryKind::File,
            None,
            mspec("/other"),
            0,
        );
        let report = exec(
            &*memory.vfs,
            vec![removal(&member), touched],
            &backups(&file, backup),
        );
        assert_eq!(report.results[0].status, ItemStatus::Failed, "{backup}");
        assert_eq!(report.results[1].status, ItemStatus::NotRun);
        assert_eq!(memory.mem.dump(), before);
    }
}

/// One backup of the outermost file per operation, however many members change; a member has none of its own.
#[test]
fn a6_6_one_backup_per_archive_and_operation() {
    let memory = on_memory(ArchiveLimits::default());
    let file = archive(&memory, "a.zip");
    let old = memory.mem.read_back(&host("/a.zip")).unwrap();
    memory.mem.set_journal(true);
    let member = |name: &str| Spec::archive_member(file.clone(), name);
    let vfs: &dyn Vfs = &*memory.vfs;
    let report = exec(
        vfs,
        vec![removal(&member("a")), removal(&member("b"))],
        &backups(&file, "/a.zip.bak"),
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(memory.vfs.counters().rebuilds, 1);
    assert_eq!(commits_of(&memory, "/a.zip.bak"), 1);
    assert_eq!(memory.mem.read_back(&host("/a.zip.bak")).unwrap(), old);
    assert_ne!(memory.mem.read_back(&host("/a.zip")).unwrap(), old);
}

/// A nested chain has one backup, of the file outside it.
#[test]
fn a6_6_a_nested_chain_backs_up_the_outermost_file() {
    let memory = on_memory(ArchiveLimits::default());
    let inner: TreeData = vec![("x".into(), b"inner".to_vec()), ("y".into(), b"y".to_vec())];
    let tree: TreeData = vec![("c.zip".into(), bytes_of(ArchiveFormat::Zip, &inner))];
    let file = memory.put("a.zip", bytes_of(ArchiveFormat::Zip, &tree));
    let old = memory.mem.read_back(&host("/a.zip")).unwrap();
    memory.mem.set_journal(true);
    let vfs: &dyn Vfs = &*memory.vfs;
    let nested = Spec::archive_member(Spec::archive_member(file.clone(), "c.zip"), "x");
    let report = exec(vfs, vec![removal(&nested)], &backups(&file, "/a.bak"));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(commits_of(&memory, "/a.bak"), 1);
    assert_eq!(memory.mem.read_back(&host("/a.bak")).unwrap(), old);
}

/// A source-only root is backed up too: a move out of an archive rewrites it.
#[test]
fn a6_6_a_source_only_root() {
    let memory = on_memory(ArchiveLimits::default());
    let file = archive(&memory, "a.zip");
    let old = memory.mem.read_back(&host("/a.zip")).unwrap();
    memory.mem.set_journal(true);
    let vfs: &dyn Vfs = &*memory.vfs;
    let item = PlanItem::planned(
        Action::Move,
        EntryKind::File,
        Some(Spec::archive_member(file.clone(), "a")),
        mspec("/out"),
        0,
    );
    let report = exec(vfs, vec![item], &backups(&file, "/a.bak"));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(memory.mem.read_back(&host("/a.bak")).unwrap(), old);
    assert_eq!(
        memory.mem.read_back(&host("/out")).unwrap().as_ref(),
        b"member a"
    );
}

/// None when nothing was done, and none on `Abort`.
#[test]
fn a6_6_no_backup_when_nothing_was_done_or_on_abort() {
    let memory = on_memory(ArchiveLimits::default());
    let file = archive(&memory, "a.zip");
    let vfs: &dyn Vfs = &*memory.vfs;
    let missing = Spec::archive_member(file.clone(), "missing");
    let report = exec(vfs, vec![removal(&missing)], &backups(&file, "/a.bak"));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert!(
        memory.mem.read_back(&host("/a.bak")).is_none(),
        "nothing done"
    );

    let cancel = hikasa_core::CancelToken::new();
    let sink = |progress: hikasa_core::Progress| {
        if progress.phase == hikasa_core::Phase::Writing {
            cancel.cancel();
        }
    };
    let ctl = Ctl {
        cancel: &cancel,
        progress: &sink,
    };
    memory
        .mem
        .insert_file(&host("/big"), vec![1u8; 1 << 20], MemMeta::default())
        .unwrap();
    let item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(mspec("/big")),
        Spec::archive_member(file.clone(), "new"),
        0,
    );
    let options = backups(&file, "/a.bak");
    let report = ops::execute_with(vfs, vfs, batch(vec![item]), &options, &ctl);
    assert!(report.cancelled);
    assert!(
        memory.mem.read_back(&host("/a.bak")).is_none(),
        "a cancelled root"
    );
}

/// A failing backup aborts: the archive unchanged, items failed.
#[test]
fn a6_6_a_failing_backup_aborts_the_root() {
    let memory = on_memory(ArchiveLimits::default());
    let file = archive(&memory, "a.zip");
    let vfs: &dyn Vfs = &*memory.vfs;
    let member = Spec::archive_member(file.clone(), "a");
    memory.mem.inject(Fault {
        path: host("/a.bak"),
        op: VfsOp::Commit,
        kind: ErrorKind::PermissionDenied,
    });
    let before = memory.mem.dump();
    let report = exec(vfs, vec![removal(&member)], &backups(&file, "/a.bak"));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert!(!report.ok());
    assert_eq!(report.summary.failed, 1);
    assert_eq!(memory.mem.dump(), before);
    assert_eq!(memory.vfs.counters().rebuilds, 0);
    assert_eq!(read_to_vec(vfs.open_read(&member).unwrap()), b"member a");
}

/// `Replaces` inside an archive, with no backup of its own: the item's field stays `None`, and only the archive goes.
#[test]
fn a6_6_a_replacing_member_has_no_backup_of_its_own() {
    let memory = on_memory(ArchiveLimits::default());
    let file = archive(&memory, "a.zip");
    memory
        .mem
        .insert_file(&host("/new"), &b"new"[..], MemMeta::default())
        .unwrap();
    let vfs: &dyn Vfs = &*memory.vfs;
    let member = Spec::archive_member(file.clone(), "a");
    let mut item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(mspec("/new")),
        member.clone(),
        0,
    );
    item.replaces = true;
    assert_eq!(item.backup, None);
    let report = exec(vfs, vec![item], &backups(&file, "/a.bak"));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(read_to_vec(vfs.open_read(&member).unwrap()), b"new");
    assert!(memory.mem.read_back(&host("/a.bak")).is_some());
    assert_eq!(memory.mem.dump().len(), 4, "{:?}", memory.mem.dump());
}

/// Rename onto a read-only backup fails on Windows: the item fails and the target stays.
#[cfg(windows)]
#[test]
fn a6_6_a_read_only_backup_on_windows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (source, target, backup) = (root.join("s"), root.join("t"), root.join("t.bak"));
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    std::fs::write(&backup, b"older").unwrap();
    let mut permissions = std::fs::metadata(&backup).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&backup, permissions).unwrap();
    let vfs = LocalVfs::default();
    let mut item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(common::lspec(&source)),
        common::lspec(&target),
        0,
    );
    item.replaces = true;
    item.backup = Some(common::lspec(&backup));
    let report = run(&vfs, batch(vec![item]));
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    assert_eq!(std::fs::read(&backup).unwrap(), b"older");
}

/// On disk, a replaced file keeps content, time and mode at its backup.
#[test]
fn a6_6_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (source, target) = (root.join("s"), root.join("t"));
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let vfs = LocalVfs::default();
    let spec = |path: &std::path::Path| common::lspec_new(path);
    let times = hikasa_vfs::SetTimes {
        modified: Some(ft(1_000)),
        created: None,
    };
    vfs.set_times(&spec(&target), &times).unwrap();
    let policy = BackupPolicy {
        style: BackupStyle::NameBak,
        folder: None,
    };
    let mut item = PlanItem::planned(
        Action::Copy,
        EntryKind::File,
        Some(spec(&source)),
        spec(&target),
        0,
    );
    item.replaces = true;
    item.backup = Some(policy.target(&vfs, &spec(&target), None).unwrap());
    assert_eq!(
        item.backup.as_ref().unwrap().file_name().as_deref(),
        Some("t.bak")
    );
    let report = run(&vfs, batch(vec![item]));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(std::fs::read(&target).unwrap(), b"new");
    let backup = root.join("t.bak");
    assert_eq!(std::fs::read(&backup).unwrap(), b"old");
    let entry = vfs
        .stat(&spec(&backup), hikasa_vfs::FollowLinks::No)
        .unwrap();
    assert_eq!(entry.mtime, Some(ft(1_000)));
}
