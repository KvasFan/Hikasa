//! M2 W10b acceptance A10.14, first part (D-273): `Action::Write`, the save's write of a file. A3's fault injection on
//! `MemoryVfs`: a failure at every write call of a save (with and without a backup, replacing and creating) leaves the
//! target as it was; `expect` refuses a changed target; a target planned absent that appeared is refused; what is not a
//! regular file is refused; the backup is renamed aside and back; nothing of the source is applied (the replaced file's
//! mode and, on disk, a time of its own); a new target is created; members of an archive are one rebuild and one
//! archive backup. W10b fix F2: a save keeps the file (its hard links, its extended attributes and POSIX ACLs, a file in
//! a folder that takes no new file), in place where a rename would lose it, with the backup a copy.

mod common;

use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Mutex;

use common::archives::{OnMemory, TreeData, bytes_of, on_memory};
use common::{ft, host, mspec};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, Spec};
use hikasa_vfs::ops::{
    self, Action, BatchOptions, ExecOptions, Expect, ItemStamp, ItemStatus, OpKind, OpReport, Plan,
    PlanItem, PlanOptions, SkipReason,
};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, AtomicWrite, Attrs, Capabilities, Entry, EntryKind, Fault,
    FileName, FollowLinks, ListOptions, Listing, MemDump, MemMeta, MemoryVfs, ReadSeek, UnixMeta,
    Vfs, VfsError, VfsOp, WriteOptions,
};

// ---- the content a save streams -------------------------------------------------------------------------------

/// A read-only back-end that holds one content per spec, as a save's `SaveSource` does: `open_read(target)` streams it.
/// Reads can fail after a number of bytes.
struct Content {
    files: BTreeMap<String, Vec<u8>>,
    fail_after: Mutex<Option<usize>>,
}

impl Content {
    fn of(items: &[(&Spec, &[u8])]) -> Content {
        Content {
            files: items
                .iter()
                .map(|(spec, bytes)| (spec.canonical(), bytes.to_vec()))
                .collect(),
            fail_after: Mutex::new(None),
        }
    }

    fn get(&self, spec: &Spec) -> Result<&Vec<u8>, VfsError> {
        self.files
            .get(&spec.canonical())
            .ok_or_else(|| VfsError::NotFound {
                spec: spec.canonical(),
            })
    }
}

/// A reader that fails after `left` bytes.
struct Failing {
    data: Cursor<Vec<u8>>,
    left: usize,
}

impl std::io::Read for Failing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            return Err(std::io::Error::other("injected read fault"));
        }
        let room = buf.len().min(self.left);
        let n = self.data.read(&mut buf[..room])?;
        self.left -= n;
        Ok(n)
    }
}

impl std::io::Seek for Failing {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.data.seek(to)
    }
}

impl Vfs for Content {
    fn capabilities(&self, _at: &Spec) -> Result<Capabilities, VfsError> {
        Ok(Capabilities::empty())
    }
    fn stat(&self, spec: &Spec, _follow: FollowLinks) -> Result<Entry, VfsError> {
        let bytes = self.get(spec)?;
        let mut entry = Entry::new(
            FileName::new(&spec.file_name().unwrap_or_default())?,
            EntryKind::File,
        );
        entry.size = bytes.len() as u64;
        // A time and a mode of its own: a write that copied metadata from its source would show them.
        entry.mtime = Some(ft(5_000));
        entry.attrs = Attrs::Unix {
            meta: UnixMeta {
                mode: 0o777,
                uid: 0,
                gid: 0,
            },
        };
        Ok(entry)
    }
    fn list(
        &self,
        dir: &Spec,
        _opts: &ListOptions<'_>,
        _cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        Err(VfsError::NotADirectory {
            spec: dir.canonical(),
        })
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        let data = Cursor::new(self.get(spec)?.clone());
        match *self.fail_after.lock().unwrap() {
            Some(left) => Ok(Box::new(Failing { data, left })),
            None => Ok(Box::new(data)),
        }
    }
    fn open_write(
        &self,
        spec: &Spec,
        _opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        Err(VfsError::OperationUnsupported {
            spec: spec.canonical(),
            op: VfsOp::Write,
        })
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        Err(VfsError::NotFound {
            spec: format!("{}/{}", dir.canonical(), name.as_str()),
        })
    }
}

// ---- helpers ---------------------------------------------------------------------------------------------------

const NEW: &[u8] = b"the new content of the file";

fn meta(secs: i64, mode: u32) -> MemMeta {
    MemMeta {
        mtime: Some(ft(secs)),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode,
                uid: 0,
                gid: 0,
            },
        }),
        ..MemMeta::default()
    }
}

fn memory() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/d"), meta(10, 0o755)).unwrap();
    vfs
}

/// `/d/t` holds "old" (mode 0640, time 100).
fn replaced() -> MemoryVfs {
    let vfs = memory();
    vfs.insert_file(&host("/d/t"), &b"old"[..], meta(100, 0o640))
        .unwrap();
    vfs
}

fn stamp_of(vfs: &dyn Vfs, spec: &Spec) -> Option<ItemStamp> {
    let entry = vfs.stat(spec, FollowLinks::No).ok()?;
    Some(ItemStamp {
        kind: entry.kind,
        size: Some(entry.size),
        mtime: entry.mtime,
    })
}

/// The `write` item a save plans for `target` as `vfs` shows it now.
fn write_item(vfs: &dyn Vfs, target: &Spec, len: usize, backup: Option<&str>) -> PlanItem {
    let mut item = PlanItem::planned(
        Action::Write,
        EntryKind::File,
        Some(target.clone()),
        target.clone(),
        0,
    );
    item.size = Some(len as u64);
    let seen = stamp_of(vfs, target);
    item.replaces = seen.is_some();
    item.expect = seen.map(|stamp| Expect {
        source: None,
        target: Some(stamp),
    });
    item.backup = backup.map(mspec);
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

fn save(content: &Content, vfs: &dyn Vfs, items: Vec<PlanItem>) -> OpReport {
    ops::execute(content, vfs, batch(items), &Ctl::detached())
}

fn dump_of(vfs: &MemoryVfs, path: &str) -> Option<MemDump> {
    let path = host(path);
    vfs.dump().into_iter().find(|d| d.path == path)
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&host(path)).map(|bytes| bytes.to_vec())
}

fn message(report: &OpReport) -> String {
    report.results[0]
        .error
        .as_ref()
        .map(|e| e.message.clone())
        .unwrap_or_default()
}

// ---- the words -------------------------------------------------------------------------------------------------

#[test]
fn a10_14_the_write_action_and_the_no_change_skip() {
    assert_eq!(Action::Write.as_str(), "write");
    assert!(!Action::Write.needs_absent_target());
    assert_eq!(SkipReason::NoChange.as_str(), "no_change");
    let vfs = replaced();
    let target = mspec("/d/t");
    let plan = batch(vec![write_item(&vfs, &target, NEW.len(), None)]);
    assert_eq!(plan.bytes(), NEW.len() as u64, "a write counts its bytes");
    let text = common::json::to_pretty(&plan);
    assert!(text.contains("\"write\""), "{text}");
    let back: PlanItem = common::json::from_value(common::json::to_value(&plan.items[0])).unwrap();
    assert_eq!(back, plan.items[0]);
    assert_eq!(
        common::json::to_value(&SkipReason::NoChange),
        common::json::J::Str("no_change".into())
    );
}

// ---- execution -------------------------------------------------------------------------------------------------

/// A save replaces the content and applies nothing of its source: the mode stays the replaced file's, the source's
/// time and mode are not copied.
#[test]
fn a10_14_a_write_replaces_the_content_and_keeps_the_target_mode() {
    let target = mspec("/d/t");
    for backup in [None, Some("/d/t.bak")] {
        let vfs = replaced();
        let content = Content::of(&[(&target, NEW)]);
        let report = save(
            &content,
            &vfs,
            vec![write_item(&vfs, &target, NEW.len(), backup)],
        );
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(report.summary.bytes, NEW.len() as u64);
        assert_eq!(read(&vfs, "/d/t").unwrap(), NEW);
        let now = dump_of(&vfs, "/d/t").unwrap();
        assert_eq!(now.attrs, meta(0, 0o640).attrs.unwrap(), "{backup:?}");
        assert_ne!(
            now.mtime,
            Some(ft(5_000)),
            "the source's time is not copied"
        );
        if backup.is_some() {
            let kept = dump_of(&vfs, "/d/t.bak").unwrap();
            assert_eq!(read(&vfs, "/d/t.bak").unwrap(), b"old");
            assert_eq!(
                (kept.mtime, kept.attrs),
                (Some(ft(100)), meta(0, 0o640).attrs.unwrap())
            );
        }
    }
}

/// Stage, rename aside, commit; one generation: the next save replaces the backup.
#[test]
fn a10_14_the_backup_goes_aside_just_before_the_commit() {
    let target = mspec("/d/t");
    let vfs = replaced();
    vfs.set_journal(true);
    vfs.set_journal_writes(true);
    let content = Content::of(&[(&target, NEW)]);
    let report = save(
        &content,
        &vfs,
        vec![write_item(&vfs, &target, NEW.len(), Some("/d/t.bak"))],
    );
    assert!(report.ok(), "{:?}", report.results);
    let order: Vec<VfsOp> = vfs.journal().iter().map(|(op, _)| *op).collect();
    assert_eq!(
        order.get(..3),
        Some(&[VfsOp::Write, VfsOp::Rename, VfsOp::Commit][..]),
        "{order:?}"
    );
    let content = Content::of(&[(&target, b"third")]);
    let report = save(
        &content,
        &vfs,
        vec![write_item(&vfs, &target, 5, Some("/d/t.bak"))],
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(read(&vfs, "/d/t.bak").unwrap(), NEW);
    assert_eq!(read(&vfs, "/d/t").unwrap(), b"third");
}

/// A failed commit renames the old file back: the target as it was, no backup.
#[test]
fn a10_14_a_failed_commit_renames_the_backup_back() {
    let target = mspec("/d/t");
    let vfs = replaced();
    vfs.inject(Fault {
        path: host("/d/t"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    let before = vfs.dump();
    let content = Content::of(&[(&target, NEW)]);
    let report = save(
        &content,
        &vfs,
        vec![write_item(&vfs, &target, NEW.len(), Some("/d/t.bak"))],
    );
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(vfs.dump(), before);
    assert!(read(&vfs, "/d/t.bak").is_none());
}

/// The cases of the fault sweep: (backup, the target exists).
const CASES: [(Option<&str>, bool); 3] = [(None, true), (Some("/d/t.bak"), true), (None, false)];

fn fixture(exists: bool) -> MemoryVfs {
    if exists { replaced() } else { memory() }
}

/// A3's fault injection: every mutating call a successful save makes (each `open_write`, rename, commit, attribute
/// change), each failing once, and read and write faults after every few bytes. Each failure fails the item and leaves
/// the tree exactly as it was.
#[test]
fn a10_14_a_failure_at_every_write_call_leaves_the_target() {
    let target = mspec("/d/t");
    let content = Content::of(&[(&target, NEW)]);
    let mut swept = 0;
    for (backup, exists) in CASES {
        // The calls of a successful run.
        let probe = fixture(exists);
        probe.set_journal(true);
        probe.set_journal_writes(true);
        let report = save(
            &content,
            &probe,
            vec![write_item(&probe, &target, NEW.len(), backup)],
        );
        assert!(report.ok(), "{:?}", report.results);
        let calls = probe.journal();
        assert!(!calls.is_empty());
        for (op, spec) in calls {
            let vfs = fixture(exists);
            let item = write_item(&vfs, &target, NEW.len(), backup);
            // The journal names canonical specs; a `MemoryVfs` spec's text is its host path.
            vfs.inject(Fault {
                path: spec.clone(),
                op,
                kind: ErrorKind::Io,
            });
            let before = vfs.dump();
            let report = save(&content, &vfs, vec![item]);
            assert_eq!(
                report.results[0].status,
                ItemStatus::Failed,
                "{op:?} at {spec} ({backup:?}, exists {exists})"
            );
            assert!(!report.ok());
            assert_eq!(
                vfs.dump(),
                before,
                "{op:?} at {spec} ({backup:?}, exists {exists})"
            );
            swept += 1;
        }
        for after in [0, 1, 7, NEW.len() - 1] {
            for read_fault in [true, false] {
                let vfs = fixture(exists);
                let item = write_item(&vfs, &target, NEW.len(), backup);
                let content = Content::of(&[(&target, NEW)]);
                if read_fault {
                    *content.fail_after.lock().unwrap() = Some(after);
                } else {
                    vfs.inject_write_error_after(&host("/d/t"), after as u64, ErrorKind::Io);
                }
                let before = vfs.dump();
                let report = save(&content, &vfs, vec![item]);
                assert_eq!(report.results[0].status, ItemStatus::Failed);
                assert_eq!(vfs.dump(), before, "after {after} bytes, read {read_fault}");
                swept += 1;
            }
        }
    }
    assert!(swept >= 20, "{swept} faults");
}

/// The tree without the backup, and the backup's content: a save in place that failed after copying its backup leaves
/// that copy, which holds the file as it still is.
fn split_backup(vfs: &MemoryVfs) -> (Vec<MemDump>, Option<Vec<u8>>) {
    let rest = vfs
        .dump()
        .into_iter()
        .filter(|d| d.path != host("/d/t.bak"))
        .collect();
    (rest, read(vfs, "/d/t.bak"))
}

/// W10b F2 in memory: where a save writes in place (`MemoryVfs::save_in_place`, as `LocalVfs` for a file with another
/// hard link), the backup is a copy made just before the commit, never a rename of the file; the file keeps its
/// attributes. A3's fault sweep over every call of such a save, and read and write faults after every few bytes: each
/// failure fails the item and leaves the file as it was; a backup it made holds the file's content.
#[test]
fn a10_14_a_save_in_place_copies_its_backup_and_a_failure_leaves_the_file() {
    let target = mspec("/d/t");
    let content = Content::of(&[(&target, NEW)]);
    let fixture = || {
        let vfs = replaced();
        vfs.save_in_place("/d/t");
        vfs
    };
    let mut swept = 0;
    for backup in [None, Some("/d/t.bak")] {
        let probe = fixture();
        probe.set_journal(true);
        probe.set_journal_writes(true);
        let report = save(
            &content,
            &probe,
            vec![write_item(&probe, &target, NEW.len(), backup)],
        );
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(read(&probe, "/d/t").unwrap(), NEW);
        assert_eq!(
            dump_of(&probe, "/d/t").unwrap().attrs,
            meta(0, 0o640).attrs.unwrap()
        );
        let calls = probe.journal();
        assert!(
            calls.iter().all(|(op, _)| *op != VfsOp::Rename),
            "no rename: {calls:?}"
        );
        if backup.is_some() {
            assert_eq!(read(&probe, "/d/t.bak").unwrap(), b"old");
            let commits: Vec<&String> = calls
                .iter()
                .filter(|(op, _)| *op == VfsOp::Commit)
                .map(|(_, spec)| spec)
                .collect();
            assert_eq!(
                commits,
                [&host("/d/t.bak"), &host("/d/t")],
                "the backup first"
            );
        }
        for (op, spec) in calls {
            let vfs = fixture();
            let item = write_item(&vfs, &target, NEW.len(), backup);
            vfs.inject(Fault {
                path: spec.clone(),
                op,
                kind: ErrorKind::Io,
            });
            let (before, _) = split_backup(&vfs);
            let report = save(&content, &vfs, vec![item]);
            let what = format!("{op:?} at {spec} ({backup:?})");
            assert_eq!(report.results[0].status, ItemStatus::Failed, "{what}");
            let (after, kept) = split_backup(&vfs);
            assert_eq!(after, before, "{what}");
            assert!(kept.is_none_or(|kept| kept == b"old"), "{what}");
            swept += 1;
        }
        for after in [0, 1, 7, NEW.len() - 1] {
            for read_fault in [true, false] {
                let vfs = fixture();
                let item = write_item(&vfs, &target, NEW.len(), backup);
                let content = Content::of(&[(&target, NEW)]);
                if read_fault {
                    *content.fail_after.lock().unwrap() = Some(after);
                } else {
                    vfs.inject_write_error_after(&host("/d/t"), after as u64, ErrorKind::Io);
                }
                let before = vfs.dump();
                let report = save(&content, &vfs, vec![item]);
                assert_eq!(report.results[0].status, ItemStatus::Failed);
                assert_eq!(vfs.dump(), before, "after {after} bytes, read {read_fault}");
                swept += 1;
            }
        }
        // A cancel before the commit: nothing.
        let vfs = fixture();
        let before = vfs.dump();
        let cancel = CancelToken::new();
        cancel.cancel();
        let report = ops::execute(
            &content,
            &vfs,
            batch(vec![write_item(&vfs, &target, NEW.len(), backup)]),
            &common::ctl(&cancel),
        );
        assert!(report.cancelled);
        assert_eq!(vfs.dump(), before);
    }
    assert!(swept >= 20, "{swept} faults");
}

/// A cancel before the commit leaves the target.
#[test]
fn a10_14_a_cancel_leaves_the_target() {
    let target = mspec("/d/t");
    let vfs = replaced();
    let before = vfs.dump();
    let content = Content::of(&[(&target, NEW)]);
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = ops::execute(
        &content,
        &vfs,
        batch(vec![write_item(&vfs, &target, NEW.len(), None)]),
        &common::ctl(&cancel),
    );
    assert!(report.cancelled);
    assert_eq!(vfs.dump(), before);
}

/// `expect.target` refuses a target that changed since the plan was made; the change stays.
#[test]
fn a10_14_expect_refuses_a_changed_target() {
    let target = mspec("/d/t");
    let vfs = replaced();
    let item = write_item(&vfs, &target, NEW.len(), Some("/d/t.bak"));
    vfs.insert_file(&host("/d/t"), &b"changed by someone"[..], meta(200, 0o640))
        .unwrap();
    let content = Content::of(&[(&target, NEW)]);
    let report = save(&content, &vfs, vec![item]);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert!(
        message(&report).contains("changed since the plan was made"),
        "{}",
        message(&report)
    );
    assert_eq!(read(&vfs, "/d/t").unwrap(), b"changed by someone");
    assert!(read(&vfs, "/d/t.bak").is_none());
}

/// A target planned absent (Save As to a new file) that appeared since is not replaced.
#[test]
fn a10_14_a_target_planned_absent_that_appeared_is_refused() {
    let target = mspec("/d/t");
    let vfs = memory();
    let item = write_item(&vfs, &target, NEW.len(), None);
    assert!(!item.replaces);
    vfs.insert_file(&host("/d/t"), &b"appeared"[..], meta(200, 0o600))
        .unwrap();
    let content = Content::of(&[(&target, NEW)]);
    let report = save(&content, &vfs, vec![item]);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert!(message(&report).contains("changed since the plan was made"));
    assert_eq!(read(&vfs, "/d/t").unwrap(), b"appeared");
}

/// A folder, a special file or a link at the target is never written by a save.
#[test]
fn a10_14_what_is_not_a_regular_file_is_refused() {
    let target = mspec("/d/t");
    for kind in ["dir", "other", "link"] {
        let vfs = memory();
        vfs.insert_file(&host("/d/real"), &b"real"[..], meta(1, 0o600))
            .unwrap();
        match kind {
            "dir" => vfs.insert_dir(&host("/d/t"), meta(1, 0o755)).unwrap(),
            "other" => vfs.insert_other(&host("/d/t"), meta(1, 0o600)).unwrap(),
            _ => vfs
                .insert_symlink(&host("/d/t"), &host("/d/real"), meta(1, 0o777))
                .unwrap(),
        }
        let mut item = write_item(&vfs, &target, NEW.len(), None);
        item.expect = None;
        let before = vfs.dump();
        let content = Content::of(&[(&target, NEW)]);
        let report = save(&content, &vfs, vec![item]);
        assert_eq!(report.results[0].status, ItemStatus::Failed, "{kind}");
        let error = report.results[0].error.as_ref().unwrap();
        assert_eq!(error.kind, ErrorKind::InvalidInput, "{kind}: {error:?}");
        assert!(
            error.message.contains("is not a regular file"),
            "{kind}: {}",
            error.message
        );
        assert_eq!(vfs.dump(), before, "{kind}");
    }
}

/// Save As to a new file creates it; a missing folder fails the item.
#[test]
fn a10_14_a_new_target_is_created() {
    let target = mspec("/d/new.txt");
    let vfs = memory();
    let content = Content::of(&[(&target, NEW)]);
    let report = save(
        &content,
        &vfs,
        vec![write_item(&vfs, &target, NEW.len(), Some("/d/new.txt.bak"))],
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(read(&vfs, "/d/new.txt").unwrap(), NEW);
    assert!(
        read(&vfs, "/d/new.txt.bak").is_none(),
        "nothing was replaced"
    );
    assert_ne!(dump_of(&vfs, "/d/new.txt").unwrap().mtime, Some(ft(5_000)));

    let missing = mspec("/nowhere/new.txt");
    let content = Content::of(&[(&missing, NEW)]);
    let report = save(
        &content,
        &vfs,
        vec![write_item(&vfs, &missing, NEW.len(), None)],
    );
    assert_eq!(report.results[0].status, ItemStatus::Failed);
}

/// Several files are one plan; a failing item leaves the others done.
#[test]
fn a10_14_several_files_are_one_plan() {
    let (a, b) = (mspec("/d/a"), mspec("/d/b"));
    let vfs = memory();
    vfs.insert_file(&host("/d/a"), &b"a"[..], meta(1, 0o600))
        .unwrap();
    vfs.insert_file(&host("/d/b"), &b"b"[..], meta(1, 0o600))
        .unwrap();
    let items = vec![write_item(&vfs, &a, 3, None), write_item(&vfs, &b, 3, None)];
    vfs.inject(Fault {
        path: host("/d/a"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    let content = Content::of(&[(&a, b"AAA"), (&b, b"BBB")]);
    let report = save(&content, &vfs, items);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(report.results[1].status, ItemStatus::Done);
    assert_eq!(read(&vfs, "/d/a").unwrap(), b"a");
    assert_eq!(read(&vfs, "/d/b").unwrap(), b"BBB");
    assert_eq!(report.summary.bytes, 3);
}

// ---- archives --------------------------------------------------------------------------------------------------

fn zip(memory: &OnMemory) -> Spec {
    let tree: TreeData = vec![
        ("a.txt".into(), b"member a".to_vec()),
        ("b.txt".into(), b"member b".to_vec()),
        ("c.bin".into(), (0..=255u8).collect()),
    ];
    memory.put("z.zip", bytes_of(ArchiveFormat::Zip, &tree))
}

fn member_bytes(vfs: &dyn Vfs, spec: &Spec) -> Vec<u8> {
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut vfs.open_read(spec).unwrap(), &mut out).unwrap();
    out
}

/// Members of one archive are one rebuild and one backup of the archive file; the other members stay.
#[test]
fn a10_14_members_of_an_archive_are_one_rebuild_and_one_backup() {
    let memory = on_memory(ArchiveLimits::default());
    let file = zip(&memory);
    let old = memory.mem.read_back(&host("/z.zip")).unwrap();
    let vfs: &dyn Vfs = &*memory.vfs;
    let (a, b) = (
        Spec::archive_member(file.clone(), "a.txt"),
        Spec::archive_member(file.clone(), "b.txt"),
    );
    let c = Spec::archive_member(file.clone(), "c.bin");
    let c_before = member_bytes(vfs, &c);
    let items = vec![write_item(vfs, &a, 5, None), write_item(vfs, &b, 5, None)];
    let options = ExecOptions {
        archive_backups: [(file.canonical(), mspec("/z.zip.bak"))].into(),
        ..ExecOptions::default()
    };
    let content = Content::of(&[(&a, b"new a"), (&b, b"new b")]);
    let report = ops::execute_with(&content, vfs, batch(items), &options, &Ctl::detached());
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(memory.vfs.counters().rebuilds, 1);
    assert_eq!(memory.mem.read_back(&host("/z.zip.bak")).unwrap(), old);
    assert_eq!(member_bytes(vfs, &a), b"new a");
    assert_eq!(member_bytes(vfs, &b), b"new b");
    assert_eq!(member_bytes(vfs, &c), c_before);
}

/// A member whose write fails takes the rebuild with it: the archive as it was.
#[test]
fn a10_14_a_failing_member_leaves_the_archive() {
    let memory = on_memory(ArchiveLimits::default());
    let file = zip(&memory);
    let vfs: &dyn Vfs = &*memory.vfs;
    let a = Spec::archive_member(file.clone(), "a.txt");
    let item = write_item(vfs, &a, 5, None);
    let content = Content::of(&[(&a, b"new a")]);
    *content.fail_after.lock().unwrap() = Some(2);
    let before = memory.mem.dump();
    let report = ops::execute(&content, vfs, batch(vec![item]), &Ctl::detached());
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert_eq!(memory.mem.dump(), before);
}

/// Saves `bytes` into the member `path` of the archive `file` through a `write` item, as a save does.
fn save_member(memory: &OnMemory, file: &Spec, path: &str, bytes: &[u8]) {
    let vfs: &dyn Vfs = &*memory.vfs;
    let spec = Spec::archive_member(file.clone(), path);
    let content = Content::of(&[(&spec, bytes)]);
    let report = save(
        &content,
        vfs,
        vec![write_item(vfs, &spec, bytes.len(), None)],
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(member_bytes(vfs, &spec), bytes);
}

/// One tar member as the test reads it back: its header block, its data, and what its header says of the owner.
#[derive(Debug, PartialEq, Eq)]
struct TarSeen {
    path: String,
    header: Vec<u8>,
    data: Vec<u8>,
    owner: (u64, u64, Option<String>, Option<String>, u32),
    mtime: u64,
}

fn tar_members(stream: &[u8]) -> Vec<TarSeen> {
    let mut archive = tar::Archive::new(Cursor::new(stream));
    let mut out = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let header = entry.header().clone();
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data).unwrap();
        let text = |field: Option<&str>| field.map(str::to_owned);
        out.push(TarSeen {
            path: entry.path().unwrap().to_string_lossy().into_owned(),
            header: header.as_bytes().to_vec(),
            data,
            owner: (
                header.uid().unwrap(),
                header.gid().unwrap(),
                text(header.username().unwrap()),
                text(header.groupname().unwrap()),
                header.mode().unwrap(),
            ),
            mtime: header.mtime().unwrap(),
        });
    }
    out
}

/// W10b F6: a tar member written over by a save keeps its place and its header: uid, gid, the owner's names, the mode,
/// its type; its time is the save's (the clock's), its size and data the new ones. The other members stay byte for byte,
/// plain and through gzip.
#[test]
fn a10_14_a_rewritten_tar_member_keeps_its_header_and_place() {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, uid, user, mode) in [
        ("first.txt", 7, "root", 0o644),
        ("a.txt", 1234, "alice", 0o640),
        ("last.txt", 9, "bob", 0o600),
    ] {
        let mut header = tar::Header::new_ustar();
        let data = format!("the old content of {path}");
        header.set_size(data.len() as u64);
        header.set_uid(uid);
        header.set_gid(uid + 1);
        header.set_username(user).unwrap();
        header.set_groupname("staff").unwrap();
        header.set_mode(mode);
        header.set_mtime(1_500_000_000);
        builder
            .append_data(&mut header, path, data.as_bytes())
            .unwrap();
    }
    let plain = builder.into_inner().unwrap();
    let before = tar_members(&plain);
    for format in [ArchiveFormat::Tar, ArchiveFormat::TarGz] {
        let memory = on_memory(ArchiveLimits::default());
        let name = common::archives::name_of(format);
        let file = memory.put(name, common::archives::compress(format, &plain));
        save_member(&memory, &file, "a.txt", b"saved");
        let stored = memory.mem.read_back(&host(&format!("/{name}"))).unwrap();
        let after = tar_members(&common::archives::decompress(format, &stored));
        let paths: Vec<&str> = after.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["first.txt", "a.txt", "last.txt"], "{format:?}");
        assert_eq!(after[0], before[0], "{format:?}");
        assert_eq!(after[2], before[2], "{format:?}");
        assert_eq!(after[1].owner, before[1].owner, "{format:?}");
        assert_eq!(
            after[1].owner,
            (
                1234,
                1235,
                Some("alice".into()),
                Some("staff".into()),
                0o640
            )
        );
        assert_eq!(after[1].data, b"saved");
        assert_eq!(after[1].mtime, 0, "the save's time: the test clock's epoch");
    }
}

/// W10b F6: a tar hard link to a member written over keeps the old content, as before (it becomes a regular member at
/// its place), and a shadowed older record of the member goes with it: the new content stands at the winner's place.
#[test]
fn a10_14_a_rewritten_tar_member_leaves_its_links_and_takes_its_shadows() {
    use common::archives::{RawTar, raw_tar};
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put(
        "a.tar",
        raw_tar(&[
            RawTar::file(b"t.txt", b"shadowed"),
            RawTar::file(b"t.txt", b"old"),
            RawTar::link(b"l.txt", b'1', b"t.txt"),
            RawTar::file(b"z.txt", b"last"),
        ]),
    );
    save_member(&memory, &file, "t.txt", b"saved");
    let vfs: &dyn Vfs = &*memory.vfs;
    let read = |path: &str| member_bytes(vfs, &Spec::archive_member(file.clone(), path));
    assert_eq!(read("l.txt"), b"old");
    assert_eq!(read("z.txt"), b"last");
    let stored = memory.mem.read_back(&host("/a.tar")).unwrap();
    let paths: Vec<(String, Vec<u8>)> = tar_members(&stored)
        .into_iter()
        .map(|m| (m.path, m.data))
        .collect();
    assert_eq!(
        paths,
        [
            ("t.txt".to_owned(), b"saved".to_vec()),
            ("l.txt".to_owned(), b"old".to_vec()),
            ("z.txt".to_owned(), b"last".to_vec()),
        ]
    );
}

/// One central record of a zip as the test reads it: the name, the "made by" field, the external attributes, the
/// extra fields, the comment, and the bytes of its local record (header and data).
#[derive(Debug, PartialEq, Eq)]
struct ZipSeen {
    name: Vec<u8>,
    made_by: u16,
    flags: u16,
    external: u32,
    extra: Vec<u8>,
    comment: Vec<u8>,
    local: Vec<u8>,
}

fn zip_members(bytes: &[u8]) -> Vec<ZipSeen> {
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let end = bytes.len() - 22;
    assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
    let count = usize::from(u16_at(end + 10));
    let start = u32_at(end + 16) as usize;
    let mut at = start;
    let mut rows = Vec::new();
    for _ in 0..count {
        assert_eq!(&bytes[at..at + 4], b"PK\x01\x02");
        let (name_len, extra_len, comment_len) = (
            usize::from(u16_at(at + 28)),
            usize::from(u16_at(at + 30)),
            usize::from(u16_at(at + 32)),
        );
        let name = at + 46;
        rows.push((
            u32_at(at + 42) as usize,
            ZipSeen {
                name: bytes[name..name + name_len].to_vec(),
                made_by: u16_at(at + 4),
                flags: u16_at(at + 8),
                external: u32_at(at + 38),
                extra: bytes[name + name_len..name + name_len + extra_len].to_vec(),
                comment: bytes
                    [name + name_len + extra_len..name + name_len + extra_len + comment_len]
                    .to_vec(),
                local: Vec::new(),
            },
        ));
        at = name + name_len + extra_len + comment_len;
    }
    let mut offsets: Vec<usize> = rows.iter().map(|(offset, _)| *offset).collect();
    offsets.push(start);
    assert!(
        offsets.is_sorted(),
        "local order is central order: {offsets:?}"
    );
    rows.into_iter()
        .enumerate()
        .map(|(i, (offset, mut seen))| {
            seen.local = bytes[offset..offsets[i + 1]].to_vec();
            seen
        })
        .collect()
}

/// W10b F6: a zip member written over by a save keeps its place (in the central directory and among the local records)
/// and its central header: the name's bytes, the "made by" host, the external attributes as they were (here a mode
/// with no file type bits), the comment and the Unix owner field. The other members stay byte for byte.
#[test]
fn a10_14_a_rewritten_zip_member_keeps_its_place_and_attributes() {
    use common::archives::{RawEntry, raw_zip};
    // Info-ZIP's Unix owner field: version 1, 4-byte uid 1000, 4-byte gid 100.
    let mut owner = vec![0x75, 0x78, 11, 0, 1, 4];
    owner.extend_from_slice(&1000u32.to_le_bytes());
    owner.push(4);
    owner.extend_from_slice(&100u32.to_le_bytes());
    let a = RawEntry {
        version_made_by: 0x0314,
        external: 0o600 << 16,
        extra: owner.clone(),
        comment: b"a note".to_vec(),
        ..RawEntry::new(b"a.txt", b"the old content of a")
    };
    let entries = [
        RawEntry::new(b"first.txt", b"the first member").unix(0o100_644),
        a,
        RawEntry::new(b"last.txt", b"the last member").unix(0o100_600),
    ];
    let bytes = raw_zip(&entries);
    let before = zip_members(&bytes);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", bytes);
    save_member(&memory, &file, "a.txt", b"saved");
    let after = zip_members(&memory.mem.read_back(&host("/a.zip")).unwrap());
    let names: Vec<&[u8]> = after.iter().map(|m| m.name.as_slice()).collect();
    assert_eq!(names, [&b"first.txt"[..], b"a.txt", b"last.txt"]);
    assert_eq!(after[0], before[0]);
    assert_eq!(after[2], before[2]);
    let (old, new) = (&before[1], &after[1]);
    assert_eq!(new.external, 0o600 << 16, "{:o}", new.external >> 16);
    assert_eq!(new.made_by >> 8, 3, "made by Unix");
    assert_eq!(
        new.flags & (1 << 11),
        old.flags & (1 << 11),
        "the UTF-8 flag"
    );
    assert_eq!(new.comment, b"a note");
    assert!(
        new.extra.windows(owner.len()).any(|field| field == owner),
        "the owner field: {:?}",
        new.extra
    );
    let vfs: &dyn Vfs = &*memory.vfs;
    for (path, content) in [
        ("first.txt", &b"the first member"[..]),
        ("a.txt", b"saved"),
        ("last.txt", b"the last member"),
    ] {
        assert_eq!(
            member_bytes(vfs, &Spec::archive_member(file.clone(), path)),
            content
        );
    }
    // Another reader agrees, the CRC included.
    let stored = memory.mem.read_back(&host("/a.zip")).unwrap();
    let mut archive = zip::ZipArchive::new(Cursor::new(stored.to_vec())).unwrap();
    let mut saved = archive.by_name("a.txt").unwrap();
    assert_eq!(saved.unix_mode(), Some(0o600));
    let mut data = Vec::new();
    std::io::Read::read_to_end(&mut saved, &mut data).unwrap();
    assert_eq!(data, b"saved");
}

// ---- on disk ---------------------------------------------------------------------------------------------------

/// On disk: the replaced file's permissions are kept (with and without a backup), the time is the write's, never the
/// source's; a new file is created with the default mode.
#[cfg(unix)]
#[test]
fn a10_14_permissions_are_kept_on_disk() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let disk = hikasa_vfs::LocalVfs::default();
    for backup in [false, true] {
        let path = root.join(format!("t{backup}"));
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let spec = common::lspec_new(&path);
        let times = hikasa_vfs::SetTimes {
            modified: Some(ft(1_000)),
            created: None,
        };
        disk.set_times(&spec, &times).unwrap();
        let mut item = write_item(&disk, &spec, NEW.len(), None);
        let backup_path = root.join(format!("t{backup}.bak"));
        if backup {
            item.backup = Some(common::lspec_new(&backup_path));
        }
        let content = Content::of(&[(&spec, NEW)]);
        let report = ops::execute(&content, &disk, batch(vec![item]), &Ctl::detached());
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(std::fs::read(&path).unwrap(), NEW);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o640, "backup {backup}");
        let now = disk.stat(&spec, FollowLinks::No).unwrap();
        assert_ne!(now.mtime, Some(ft(5_000)), "the source's time");
        assert_ne!(now.mtime, Some(ft(1_000)), "the old time");
        if backup {
            assert_eq!(std::fs::read(&backup_path).unwrap(), b"old");
            let kept = disk
                .stat(&common::lspec_new(&backup_path), FollowLinks::No)
                .unwrap();
            assert_eq!(kept.mtime, Some(ft(1_000)));
        }
    }
    let new = root.join("new.txt");
    let spec = common::lspec_new(&new);
    let content = Content::of(&[(&spec, NEW)]);
    let report = ops::execute(
        &content,
        &disk,
        batch(vec![write_item(&disk, &spec, NEW.len(), None)]),
        &Ctl::detached(),
    );
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(std::fs::read(&new).unwrap(), NEW);
    let mode = std::fs::metadata(&new).unwrap().permissions().mode() & 0o7777;
    assert_ne!(mode, 0o777, "the source's mode is not copied");
}

/// On disk a link at the target is refused at execution (a save plans the file it leads to).
#[cfg(unix)]
#[test]
fn a10_14_a_link_at_the_target_is_refused_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let real = root.join("real");
    std::fs::write(&real, b"real").unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let disk = hikasa_vfs::LocalVfs::default();
    let spec = common::lspec_new(&link);
    let mut item = write_item(&disk, &spec, NEW.len(), None);
    item.expect = None;
    let content = Content::of(&[(&spec, NEW)]);
    let report = ops::execute(&content, &disk, batch(vec![item]), &Ctl::detached());
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    assert!(message(&report).contains("is not a regular file"));
    assert_eq!(std::fs::read(&real).unwrap(), b"real");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

// ---- on disk: a save keeps the file (W10b, F2) -----------------------------------------------------------------

/// Saves `content` into the file at `path` with the backup `backup`, as a save plans it.
#[cfg(unix)]
fn save_on_disk(
    path: &std::path::Path,
    content: &[u8],
    backup: Option<&std::path::Path>,
) -> OpReport {
    let disk = hikasa_vfs::LocalVfs::default();
    let spec = common::lspec_new(path);
    let mut item = write_item(&disk, &spec, content.len(), None);
    item.backup = backup.map(common::lspec_new);
    let source = Content::of(&[(&spec, content)]);
    ops::execute(&source, &disk, batch(vec![item]), &Ctl::detached())
}

/// The inode and the link count.
#[cfg(unix)]
fn inode(path: &std::path::Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).unwrap();
    (meta.ino(), meta.nlink())
}

/// A file with another hard link: the save writes into it, so the other name shows the new content, the inode and the
/// link count stay, the mode stays; a backup is a copy, a file of its own with the old content.
#[cfg(unix)]
#[test]
fn a10_14_a_hard_link_keeps_its_file_on_disk() {
    use std::os::unix::fs::PermissionsExt;
    for backup in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (file, link, kept) = (root.join("t"), root.join("link"), root.join("t.bak"));
        std::fs::write(&file, b"old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        std::fs::hard_link(&file, &link).unwrap();
        let before = inode(&file);
        assert_eq!(before.1, 2);
        let report = save_on_disk(&file, NEW, backup.then_some(kept.as_path()));
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(std::fs::read(&file).unwrap(), NEW);
        assert_eq!(std::fs::read(&link).unwrap(), NEW, "the other name");
        assert_eq!(inode(&file), before, "the same inode, two names");
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o640);
        if backup {
            assert_eq!(std::fs::read(&kept).unwrap(), b"old");
            assert_ne!(inode(&kept).0, before.0, "the backup is a copy");
        }
        let names: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|n| !n.starts_with(".hikasa")),
            "nothing left: {names:?}"
        );
    }
}

/// Sets the extended attribute `name` of `path`, or says why the file system refuses it.
#[cfg(target_os = "linux")]
fn set_xattr(path: &std::path::Path, name: &str, value: &[u8]) -> Result<(), String> {
    rustix::fs::setxattr(path, name, value, rustix::fs::XattrFlags::empty())
        .map_err(|e| format!("{name}: {e}"))
}

/// The extended attribute `name` of `path`.
#[cfg(target_os = "linux")]
fn xattr(path: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 4096];
    let len = rustix::fs::getxattr(path, name, &mut buffer[..]).ok()?;
    buffer.truncate(len);
    Some(buffer)
}

/// A POSIX ACL as Linux stores it in `system.posix_acl_access`: the file's owner, a named user, the file's group, the
/// mask, others.
#[cfg(target_os = "linux")]
fn acl_with_named_user(uid: u32) -> Vec<u8> {
    const UNDEFINED: u32 = u32::MAX;
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, perm, id) in [
        (0x01u16, 6u16, UNDEFINED),
        (0x02, 4, uid),
        (0x04, 4, UNDEFINED),
        (0x10, 4, UNDEFINED),
        (0x20, 0, UNDEFINED),
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&perm.to_le_bytes());
        acl.extend_from_slice(&id.to_le_bytes());
    }
    acl
}

/// A file of one name, renamed by the save as before (a new inode), keeps its `user.*` attribute and its POSIX ACL, with
/// and without a backup. A file system that refuses either is skipped for it, with a note.
#[cfg(target_os = "linux")]
#[test]
fn a10_14_extended_attributes_and_acls_are_kept_on_disk() {
    let acl = acl_with_named_user(54_321);
    for backup in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (file, kept) = (root.join("t"), root.join("t.bak"));
        std::fs::write(&file, b"old").unwrap();
        let mut set = Vec::new();
        for (name, value) in [
            ("user.hikasa.note", &b"kept"[..]),
            ("system.posix_acl_access", &acl[..]),
        ] {
            match set_xattr(&file, name, value) {
                Ok(()) => set.push((name, xattr(&file, name).unwrap())),
                Err(why) => eprintln!("skipped, the file system refuses it: {why}"),
            }
        }
        if set.is_empty() {
            return;
        }
        let before = inode(&file);
        let report = save_on_disk(&file, NEW, backup.then_some(kept.as_path()));
        assert!(report.ok(), "{:?}", report.results);
        assert_eq!(std::fs::read(&file).unwrap(), NEW);
        assert_ne!(
            inode(&file).0,
            before.0,
            "renamed: a new file took the name"
        );
        for (name, value) in &set {
            assert_eq!(
                xattr(&file, name).as_ref(),
                Some(value),
                "{name}, backup {backup}"
            );
        }
    }
}

/// A writable file in a folder that takes no new file is saved in place: without a backup, and with one in a folder that
/// does. A backup that would go into the read-only folder fails the item before anything is written: the error names
/// the backup, the file is as it was. Skipped where the folder's mode does not stop this process (root).
#[cfg(unix)]
#[test]
fn a10_14_a_file_in_a_read_only_folder_is_saved_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (locked, open) = (root.join("locked"), root.join("open"));
    std::fs::create_dir(&locked).unwrap();
    std::fs::create_dir(&open).unwrap();
    let file = locked.join("t");
    std::fs::write(&file, b"old").unwrap();
    let mode = |path: &std::path::Path, mode: u32| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    };
    mode(&locked, 0o555);
    if std::fs::write(locked.join("probe"), b"").is_ok() {
        mode(&locked, 0o755);
        eprintln!("skipped: the folder's mode does not stop this process");
        return;
    }
    let before = inode(&file);
    let report = save_on_disk(&file, NEW, None);
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(std::fs::read(&file).unwrap(), NEW);
    assert_eq!(inode(&file), before);

    let kept = open.join("t.bak");
    let report = save_on_disk(&file, b"third", Some(&kept));
    assert!(report.ok(), "{:?}", report.results);
    assert_eq!(std::fs::read(&file).unwrap(), b"third");
    assert_eq!(std::fs::read(&kept).unwrap(), NEW);

    let beside = locked.join("t.bak");
    let report = save_on_disk(&file, b"fourth", Some(&beside));
    mode(&locked, 0o755);
    assert_eq!(report.results[0].status, ItemStatus::Failed);
    let error = report.results[0].error.as_ref().unwrap();
    assert_eq!(error.kind, ErrorKind::PermissionDenied, "{error:?}");
    assert!(error.message.contains("t.bak"), "{}", error.message);
    assert_eq!(std::fs::read(&file).unwrap(), b"third");
    assert!(!beside.exists());
}
