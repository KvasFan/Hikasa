//! M2 W5b, review of the write path: regressions of what the review of the part found, each built from bytes made here.
//! After every rebuild the result is read back by a new back-end and by the plain `zip` and `tar` crates.

mod common;

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::sync::Arc;

use common::archives::{
    CancellingBase, OnDisk, OnMemory, RawEntry, RawTar, Seen, TreeData, bytes_of, chain_of, config,
    decompress, local, name_of, on_disk, on_memory, pax, raw_tar, raw_zip, read_plain, seal,
    ustar_of, walk,
};
use common::{ctl, ft, host, mspec, read_to_vec};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, ErrorKind, Spec, WarningCode};
use hikasa_vfs::ops::{self, CopyOptions, ItemStatus, MoveOptions, OverwritePolicy, RemoveOptions};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, AtomicWrite, Capabilities, Entry, FileName, Finish,
    FollowLinks, ListOptions, Listing, LocalVfs, MemMeta, MemoryVfs, ReadSeek, SetTimes, Vfs,
    VfsError, VfsOp, WriteOptions,
};

const WRITABLE: [ArchiveFormat; 6] = [
    ArchiveFormat::Zip,
    ArchiveFormat::Tar,
    ArchiveFormat::TarGz,
    ArchiveFormat::TarBz2,
    ArchiveFormat::TarXz,
    ArchiveFormat::TarZst,
];

fn member(archive: &Spec, path: &str) -> Spec {
    Spec::archive_member(archive.clone(), path)
}

fn write_member(vfs: &dyn Vfs, spec: &Spec, bytes: &[u8]) -> Result<(), VfsError> {
    let options = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    let mut writer = vfs.open_write(spec, &options)?;
    writer.write_all(bytes).unwrap();
    writer.commit()
}

fn read_member(vfs: &dyn Vfs, spec: &Spec) -> Vec<u8> {
    read_to_vec(vfs.open_read(spec).unwrap())
}

fn stored(memory: &OnMemory, name: &str) -> Vec<u8> {
    memory
        .mem
        .read_back(&host(&format!("/{name}")))
        .unwrap()
        .to_vec()
}

/// A new back-end on the same bytes: it knows nothing of the rebuild.
fn fresh(memory: &OnMemory, keep: &tempfile::TempDir) -> ArchiveVfs {
    ArchiveVfs::new(
        Arc::clone(&memory.mem) as Arc<dyn Vfs>,
        config(keep.path(), ArchiveLimits::default()),
    )
}

fn fresh_view(memory: &OnMemory, file: &Spec) -> BTreeMap<String, Seen> {
    let dir = tempfile::tempdir().unwrap();
    walk(&fresh(memory, &dir), &member(file, ""))
}

fn file_of(bytes: &[u8]) -> Seen {
    Seen::File(bytes.to_vec())
}

/// `remove` through `ops`.
fn rm(vfs: &dyn Vfs, specs: &[Spec], recursive: bool) -> ops::OpReport {
    let opts = RemoveOptions {
        recursive,
        trash: false,
    };
    let plan = ops::plan_remove(vfs, specs, &opts, &Ctl::detached()).unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn mv(vfs: &dyn Vfs, from: &Spec, to: &Spec, overwrite: OverwritePolicy) -> ops::OpReport {
    let opts = MoveOptions {
        overwrite,
        ..MoveOptions::default()
    };
    let plan = ops::plan_move(
        vfs,
        vfs,
        &[(from.clone(), to.clone())],
        &opts,
        &Ctl::detached(),
    )
    .unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn cp(vfs: &dyn Vfs, from: &Spec, to: &Spec, overwrite: OverwritePolicy) -> ops::OpReport {
    let opts = CopyOptions {
        recursive: true,
        overwrite,
        ..CopyOptions::default()
    };
    let plan = ops::plan_copy(
        vfs,
        vfs,
        &[(from.clone(), to.clone())],
        &opts,
        &Ctl::detached(),
    )
    .unwrap();
    ops::execute(vfs, vfs, plan, &Ctl::detached())
}

fn siblings(disk: &OnDisk) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(&disk.dir)
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ---- writes below an untouched level ------------------------------------------------------------------------

const CHAINS: [&[ArchiveFormat]; 3] = [
    &[ArchiveFormat::Zip, ArchiveFormat::Zip, ArchiveFormat::Zip],
    &[ArchiveFormat::Zip, ArchiveFormat::Tar, ArchiveFormat::TarGz],
    &[
        ArchiveFormat::Zip,
        ArchiveFormat::TarZst,
        ArchiveFormat::Tar,
        ArchiveFormat::Zip,
    ],
];

fn leaf() -> TreeData {
    vec![
        ("deep.txt".into(), b"deep".to_vec()),
        ("keep.txt".into(), b"keep".to_vec()),
    ]
}

/// The spec of the deepest archive of a chain, on `file`.
fn deepest(file: &Spec, names: &[String]) -> Spec {
    names
        .iter()
        .skip(1)
        .fold(file.clone(), |at, name| member(&at, name))
}

/// The members of the deepest archive, as the plain crates read the stored bytes all the way down.
fn plain_deepest(
    mut bytes: Vec<u8>,
    formats: &[ArchiveFormat],
    names: &[String],
) -> BTreeMap<String, Seen> {
    for (at, format) in formats.iter().enumerate() {
        let seen = read_plain(*format, &bytes);
        if at + 1 == formats.len() {
            return seen;
        }
        assert_eq!(
            seen.get(&format!("side{at}")),
            Some(&file_of(format!("side of level {at}").as_bytes()))
        );
        match seen.get(&names[at + 1]) {
            Some(Seen::File(inner)) => bytes = inner.clone(),
            other => panic!("{other:?}"),
        }
    }
    unreachable!()
}

fn put_chain(memory: &OnMemory, formats: &[ArchiveFormat]) -> (Spec, Spec, Vec<String>) {
    let (names, bytes) = chain_of(formats, &leaf());
    let file = memory.put("outer.bin.zip", bytes);
    let deep = deepest(&file, &names);
    (file, deep, names)
}

/// A write three and four levels down, with no change at the levels between, reaches the file.
#[test]
fn a_write_below_untouched_levels_reaches_the_file() {
    for formats in CHAINS {
        if formats[0] != ArchiveFormat::Zip {
            continue;
        }
        let memory = on_memory(ArchiveLimits::default());
        let (file, deep, names) = put_chain(&memory, formats);
        memory.mem.set_journal(true);
        write_member(&*memory.vfs, &member(&deep, "new.txt"), b"written").unwrap();
        let commits = memory.mem.journal();
        let commits = commits.iter().filter(|(op, _)| *op == VfsOp::Commit);
        assert_eq!(commits.count(), 1, "{formats:?}: one rebuild");
        let plain = plain_deepest(stored(&memory, "outer.bin.zip"), formats, &names);
        assert_eq!(
            plain.get("new.txt"),
            Some(&file_of(b"written")),
            "{formats:?}"
        );
        assert_eq!(plain.get("keep.txt"), Some(&file_of(b"keep")));
        let dir = tempfile::tempdir().unwrap();
        let view = fresh(&memory, &dir);
        assert_eq!(read_member(&view, &member(&deep, "new.txt")), b"written");
        drop(file);
    }
}

/// A move into such a chain keeps its source until the file holds the copy; the review saw the source go and the copy
/// not arrive.
#[test]
fn a_move_below_untouched_levels_loses_nothing() {
    for formats in CHAINS {
        let memory = on_memory(ArchiveLimits::default());
        let (_, deep, names) = put_chain(&memory, formats);
        let meta = MemMeta::default();
        memory
            .mem
            .insert_file(&host("/only-copy"), &b"only copy"[..], meta)
            .unwrap();
        let vfs = &*memory.vfs;
        let source = mspec("/only-copy");
        let report = mv(vfs, &source, &member(&deep, "moved"), OverwritePolicy::Fail);
        assert!(report.ok(), "{:?}", report.results);
        let plain = plain_deepest(stored(&memory, "outer.bin.zip"), formats, &names);
        assert_eq!(
            plain.get("moved"),
            Some(&file_of(b"only copy")),
            "{formats:?}"
        );
        assert!(vfs.stat(&source, FollowLinks::No).is_err(), "it went");
    }
}

#[test]
fn a_remove_below_untouched_levels_reaches_the_file() {
    for formats in CHAINS {
        let memory = on_memory(ArchiveLimits::default());
        let (_, deep, names) = put_chain(&memory, formats);
        let report = rm(&*memory.vfs, &[member(&deep, "deep.txt")], false);
        assert!(report.ok(), "{:?}", report.results);
        let plain = plain_deepest(stored(&memory, "outer.bin.zip"), formats, &names);
        assert_eq!(plain.get("deep.txt"), None, "{formats:?}");
        assert_eq!(plain.get("keep.txt"), Some(&file_of(b"keep")));
    }
}

/// `seal_below` has the same walk: a level folded into a middle one that then moves, or is read whole, in one batch.
#[test]
fn a_seal_folds_through_untouched_levels() {
    let formats = [ArchiveFormat::Zip, ArchiveFormat::Zip, ArchiveFormat::Tar];
    let memory = on_memory(ArchiveLimits::default());
    let (file, deep, names) = put_chain(&memory, &formats);
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&deep, "sealed"), b"sealed").unwrap();
    let middle = member(&file, &names[1]);
    let bytes = read_member(vfs, &middle);
    let inner = read_plain(ArchiveFormat::Zip, &bytes);
    let Some(Seen::File(tar)) = inner.get(&names[2]) else {
        panic!("{inner:?}");
    };
    assert!(read_plain(ArchiveFormat::Tar, tar).contains_key("sealed"));
    vfs.finish(&file, Finish::Commit).unwrap();

    let memory = on_memory(ArchiveLimits::default());
    let (file, deep, names) = put_chain(&memory, &formats);
    let vfs = &*memory.vfs;
    vfs.begin(&file).unwrap();
    write_member(vfs, &member(&deep, "sealed"), b"sealed").unwrap();
    let moved = member(&file, "moved.zip");
    vfs.rename(&member(&file, &names[1]), &moved, false)
        .unwrap();
    vfs.finish(&file, Finish::Commit).unwrap();
    let outer = read_plain(ArchiveFormat::Zip, &stored(&memory, "outer.bin.zip"));
    let Some(Seen::File(zip)) = outer.get("moved.zip") else {
        panic!("{outer:?}");
    };
    let middle = read_plain(ArchiveFormat::Zip, zip);
    let Some(Seen::File(tar)) = middle.get(&names[2]) else {
        panic!("{middle:?}");
    };
    let plain = read_plain(ArchiveFormat::Tar, tar);
    assert_eq!(plain.get("sealed"), Some(&file_of(b"sealed")));
}

// ---- writers against writers --------------------------------------------------------------------------------

fn disk_view(disk: &OnDisk, format: ArchiveFormat, name: &str) -> BTreeMap<String, Seen> {
    read_plain(format, &std::fs::read(disk.dir.join(name)).unwrap())
}

fn second_back_end(disk: &OnDisk) -> (ArchiveVfs, tempfile::TempDir) {
    let cache = tempfile::tempdir().unwrap();
    let vfs = ArchiveVfs::new(
        Arc::new(LocalVfs::default()) as Arc<dyn Vfs>,
        config(cache.path(), ArchiveLimits::default()),
    );
    let _ = disk;
    (vfs, cache)
}

/// A second writer is told at once, whichever way it comes, and nobody's write is gone.
#[test]
fn a_second_writer_is_refused_while_a_batch_runs() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let (other, _keep) = second_back_end(&disk);
    disk.vfs.begin(&file).unwrap();
    write_member(&*disk.vfs, &member(&file, "first"), b"1").unwrap();
    let busy = |result: Result<(), VfsError>| {
        assert!(
            matches!(&result, Err(VfsError::Unavailable { reason, .. }) if reason.contains("another operation")),
            "{result:?}"
        );
    };
    busy(other.begin(&file));
    busy(write_member(&other, &member(&file, "second"), b"2"));
    assert_eq!(
        read_member(&other, &member(&file, "a")),
        b"a",
        "a reader does not care"
    );
    disk.vfs.finish(&file, Finish::Commit).unwrap();
    write_member(&other, &member(&file, "second"), b"2").unwrap();
    let plain = disk_view(&disk, ArchiveFormat::Zip, "a.zip");
    assert_eq!(plain.get("first"), Some(&file_of(b"1")));
    assert_eq!(plain.get("second"), Some(&file_of(b"2")));
}

/// Writers that race: each one that reports success is in the file at the end.
#[test]
fn racing_writers_lose_no_update() {
    for round in 0..40 {
        let disk = on_disk(ArchiveLimits::default());
        let tree: TreeData = vec![("base".into(), vec![7u8; 1 << 16])];
        let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
        let (other, _keep) = second_back_end(&disk);
        let backs: [&dyn Vfs; 2] = [&*disk.vfs, &other];
        let done: Vec<(String, bool)> = std::thread::scope(|scope| {
            let workers: Vec<_> = backs
                .iter()
                .enumerate()
                .map(|(at, vfs)| {
                    let (vfs, file) = (*vfs, &file);
                    scope.spawn(move || {
                        let name = format!("w{at}");
                        let ok = write_member(vfs, &member(file, &name), b"x").is_ok();
                        (name, ok)
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).collect()
        });
        let plain = disk_view(&disk, ArchiveFormat::Zip, "a.zip");
        for (name, ok) in done {
            if ok {
                assert!(plain.contains_key(&name), "round {round}: {name} was lost");
            }
        }
        assert_eq!(siblings(&disk), ["a.zip"], "round {round}");
    }
}

/// The lock file exists for the batch only, a batch that ends any way removes it, and one a dead writer left is taken
/// over. A link and the file it names share one lock.
#[test]
fn the_lock_file_lasts_as_long_as_the_batch() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let vfs = &*disk.vfs;
    vfs.list(
        &member(&file, ""),
        &ListOptions::default(),
        &CancelToken::never(),
    )
    .unwrap();
    assert_eq!(siblings(&disk), ["a.zip"], "a reader leaves nothing");
    vfs.begin(&file).unwrap();
    assert_eq!(siblings(&disk), [".a.zip.hikasa-lock", "a.zip"]);
    vfs.finish(&file, Finish::Abort).unwrap();
    assert_eq!(siblings(&disk), ["a.zip"]);
    std::fs::write(disk.dir.join(".a.zip.hikasa-lock"), b"").unwrap();
    write_member(vfs, &member(&file, "n"), b"n").unwrap();
    assert_eq!(siblings(&disk), ["a.zip"], "a dead writer's lock goes too");
}

#[cfg(unix)]
#[test]
fn a_link_and_its_file_share_the_lock() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("real.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let link = disk.dir.join("link.zip");
    std::os::unix::fs::symlink("real.zip", &link).unwrap();
    let (other, _keep) = second_back_end(&disk);
    disk.vfs.begin(&file).unwrap();
    let result = other.begin(&local(&link));
    assert!(
        matches!(result, Err(VfsError::Unavailable { .. })),
        "{result:?}"
    );
    disk.vfs.finish(&file, Finish::Abort).unwrap();
}

/// A base whose writer runs `on_sync` while it syncs: a change of the file in the window of an fsync.
struct SyncHook {
    mem: Arc<MemoryVfs>,
    on_sync: Arc<dyn Fn() + Send + Sync>,
}

struct HookedWriter {
    inner: Box<dyn AtomicWrite>,
    on_sync: Arc<dyn Fn() + Send + Sync>,
}

impl Write for HookedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for HookedWriter {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl AtomicWrite for HookedWriter {
    fn sync(&mut self) -> Result<(), VfsError> {
        (self.on_sync)();
        self.inner.sync()
    }
    fn commit(self: Box<Self>) -> Result<(), VfsError> {
        self.inner.commit()
    }
}

impl Vfs for SyncHook {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.mem.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.mem.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.mem.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.mem.open_read(spec)
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        Ok(Box::new(HookedWriter {
            inner: self.mem.open_write(spec, opts)?,
            on_sync: Arc::clone(&self.on_sync),
        }))
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.mem.child(dir, name)
    }
}

/// The file changes during the fsync of the rebuild, after every earlier look: the last look comes after the wait.
#[test]
fn a_change_during_the_sync_is_caught_before_the_rename() {
    let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let path = host("/a.zip");
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let other: TreeData = vec![("other".into(), b"other".to_vec())];
    let newer = bytes_of(ArchiveFormat::Zip, &other);
    let meta = |seconds| MemMeta {
        mtime: hikasa_core::FileTime::from_unix(seconds, 0),
        ..MemMeta::default()
    };
    mem.insert_file(
        &path,
        bytes_of(ArchiveFormat::Zip, &tree),
        meta(1_700_000_000),
    )
    .unwrap();
    let on_sync = {
        let (mem, path, newer) = (Arc::clone(&mem), path.clone(), newer.clone());
        Arc::new(move || {
            mem.insert_file(&path, newer.clone(), meta(1_800_000_000))
                .unwrap();
        })
    };
    let base = SyncHook {
        mem: Arc::clone(&mem),
        on_sync,
    };
    let vfs = ArchiveVfs::new(
        Arc::new(base) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    );
    let file = mspec("/a.zip");
    let result = write_member(&vfs, &member(&file, "n"), b"n");
    assert!(
        matches!(&result, Err(VfsError::InvalidTarget { reason, .. }) if reason.contains("changed since")),
        "{result:?}"
    );
    assert_eq!(
        mem.read_back(&path).unwrap().to_vec(),
        newer,
        "theirs stays"
    );
}

/// A file put in the place of the archive with the same size and time is another file.
#[cfg(unix)]
#[test]
fn a_file_of_the_same_size_and_time_in_its_place_is_seen() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let bytes = bytes_of(ArchiveFormat::Zip, &tree);
    let file = disk.put("a.zip", &bytes);
    let path = disk.dir.join("a.zip");
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    disk.vfs
        .list(
            &member(&file, ""),
            &ListOptions::default(),
            &CancelToken::never(),
        )
        .unwrap();
    disk.vfs.begin(&file).unwrap();
    disk.vfs.create_dir(&member(&file, "d"), false).unwrap();
    let swapped = disk.dir.join("swap");
    std::fs::write(&swapped, &bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&swapped)
        .unwrap()
        .set_modified(time)
        .unwrap();
    std::fs::rename(&swapped, &path).unwrap();
    let result = disk.vfs.finish(&file, Finish::Commit);
    assert!(
        matches!(result, Err(VfsError::InvalidTarget { .. })),
        "{result:?}"
    );
}

// ---- what a writer leaves next to the file ---------------------------------------------------------------------

/// What a dead writer left goes with the next rebuild; a live writer's file and a user's look-alikes stay.
#[test]
fn dead_temporary_files_go_and_nothing_else_does() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let local = Arc::new(LocalVfs::default());
    // A dead one: what a killed writer leaves. A writer that ends removes its file, so one is left by hand.
    let dead = disk.dir.join(".a.zip.hikasa-tmp");
    let private = |path: &std::path::Path| {
        std::fs::write(path, b"dead").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    };
    private(&dead);
    // A live one, on the next free name, through the same call a plain save makes.
    let options = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    let mut live = local.open_write(&file, &options).unwrap();
    live.write_all(b"being saved").unwrap();
    let live_name = ".a.zip.hikasa-tmp-1";
    assert!(disk.dir.join(live_name).exists());
    // Names that only look alike.
    for name in [
        ".a.zip.hikasa-tmp.bak",
        ".a.zip.hikasa-tmp-1x",
        "a.zip.hikasa-tmp",
        ".b.zip.hikasa-tmp",
        ".a.zip.hikasa-tmpx",
    ] {
        private(&disk.dir.join(name));
    }
    write_member(&*disk.vfs, &member(&file, "n"), b"n").unwrap();
    let left = siblings(&disk);
    assert!(!left.contains(&".a.zip.hikasa-tmp".to_owned()), "{left:?}");
    assert!(left.contains(&live_name.to_owned()), "{left:?}");
    assert_eq!(left.len(), 7, "{left:?}");
    drop(live);
}

/// A folder a user keeps a file of that name in: without the mode of a temporary file it is theirs.
#[cfg(unix)]
#[test]
fn a_file_with_a_temporary_name_and_a_wide_mode_stays() {
    use std::os::unix::fs::PermissionsExt;
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let theirs = disk.dir.join(".a.zip.hikasa-tmp");
    std::fs::write(&theirs, b"mine").unwrap();
    std::fs::set_permissions(&theirs, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_member(&*disk.vfs, &member(&file, "n"), b"n").unwrap();
    assert_eq!(std::fs::read(&theirs).unwrap(), b"mine");
}

/// The copy of a private file on its way to the rename is private, and the result has the right mode: the old file's,
/// or what a new file gets.
#[cfg(unix)]
#[test]
fn a_temporary_file_is_private_until_the_rename() {
    use std::os::unix::fs::PermissionsExt;
    let disk = on_disk(ArchiveLimits::default());
    let vfs = LocalVfs::default();
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let options = WriteOptions {
        overwrite: true,
        ..WriteOptions::default()
    };
    let secret = disk.dir.join("secret");
    std::fs::write(&secret, b"old").unwrap();
    std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut writer = vfs.open_write(&local(&secret), &options).unwrap();
    writer.write_all(b"new").unwrap();
    assert_eq!(mode(&disk.dir.join(".secret.hikasa-tmp")), 0o600);
    writer.commit().unwrap();
    assert_eq!(mode(&secret), 0o640, "the old file's");

    let plain = disk.dir.join("plain");
    std::fs::write(&plain, b"x").unwrap();
    let new = disk.dir.join("new");
    let writer = vfs
        .open_write(&common::lspec_new(&new), &WriteOptions::default())
        .unwrap();
    writer.commit().unwrap();
    assert_eq!(mode(&new), mode(&plain), "what a new file gets");
}

/// The M1 rule of local writes: the folder decides, and the mode stays.
#[cfg(unix)]
#[test]
fn a_read_only_archive_follows_the_local_write_rule() {
    use std::os::unix::fs::PermissionsExt;
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let file = disk.put("a.zip", &bytes_of(ArchiveFormat::Zip, &tree));
    let path = disk.dir.join("a.zip");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
    write_member(&*disk.vfs, &member(&file, "n"), b"n").unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o444);
    assert!(disk_view(&disk, ArchiveFormat::Zip, "a.zip").contains_key("n"));
}

/// A rename replaces a name: a second name of the same file keeps the old bytes (`docs/11`).
#[cfg(unix)]
#[test]
fn a_hard_linked_archive_loses_the_link_on_rebuild() {
    let disk = on_disk(ArchiveLimits::default());
    let tree: TreeData = vec![("a".into(), b"a".to_vec())];
    let bytes = bytes_of(ArchiveFormat::Zip, &tree);
    let file = disk.put("a.zip", &bytes);
    let other = disk.dir.join("b.zip");
    std::fs::hard_link(disk.dir.join("a.zip"), &other).unwrap();
    write_member(&*disk.vfs, &member(&file, "n"), b"n").unwrap();
    assert!(disk_view(&disk, ArchiveFormat::Zip, "a.zip").contains_key("n"));
    assert_eq!(std::fs::read(&other).unwrap(), bytes);
}

// ---- tar names ------------------------------------------------------------------------------------------------

/// A ustar member with its path split into `prefix` and `name`, as `git archive` writes one.
fn split_member(prefix: &[u8], name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut block = RawTar::file(name, data).header();
    block[345..345 + prefix.len()].copy_from_slice(prefix);
    seal(&mut block);
    let mut out = block.to_vec();
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(512), 0);
    out
}

/// What the plain crate reads of a `git archive` shape: it shows the global header as a member, which no other reader
/// does.
fn without_global(mut plain: BTreeMap<String, Seen>) -> BTreeMap<String, Seen> {
    plain.remove("pax_global_header");
    plain
}

fn git_shaped(long_dir: &str) -> Vec<u8> {
    let mut global = RawTar::file(
        b"pax_global_header",
        b"52 comment=0123456789012345678901234567890123456789\n",
    );
    global.type_flag = b'g';
    let mut out = global.bytes();
    out.extend(split_member(
        long_dir.as_bytes(),
        "q".repeat(90).as_bytes(),
        b"MOVED",
    ));
    out.extend(split_member(long_dir.as_bytes(), b"victim.txt", b"VICTIM"));
    out.extend_from_slice(&[0; 1024]);
    out
}

/// A member that loses its long path takes nothing of it along: the review saw `prefix` join the new name, and the
/// member land on another one.
#[test]
fn a_ustar_prefix_stays_behind_with_the_old_name() {
    let dir = "p".repeat(80);
    let long = format!("{dir}/{}", "q".repeat(90));
    let victim = format!("{dir}/victim.txt");
    for format in [
        ArchiveFormat::Tar,
        ArchiveFormat::TarGz,
        ArchiveFormat::TarXz,
    ] {
        let bytes = git_shaped(&dir);
        let bytes = if format == ArchiveFormat::Tar {
            bytes
        } else {
            common::archives::compress(format, &bytes)
        };
        let memory = on_memory(ArchiveLimits::default());
        let name = name_of(format);
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        vfs.rename(&member(&file, &long), &member(&file, "short.txt"), false)
            .unwrap();
        let plain = without_global(read_plain(format, &stored(&memory, name)));
        assert_eq!(
            plain.get("short.txt"),
            Some(&file_of(b"MOVED")),
            "{format:?}"
        );
        assert_eq!(plain.get(&victim), Some(&file_of(b"VICTIM")), "{format:?}");
        assert_eq!(fresh_view(&memory, &file), plain);
        assert_eq!(walk(vfs, &member(&file, "")), plain);
        let raw = decompress(format, &stored(&memory, name));
        assert_eq!(&raw[..1024][..17], b"pax_global_header");
    }
}

/// A rename of the folder, and a member renamed onto a shorter name in that folder, in every ustar shape.
#[test]
fn a_folder_of_split_names_moves_whole() {
    let dir = "p".repeat(80);
    for to in ["d2", "x/y"] {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put("a.tar", git_shaped(&dir));
        let vfs = &*memory.vfs;
        vfs.create_dir(&member(&file, "x"), false).unwrap();
        vfs.rename(&member(&file, &dir), &member(&file, to), false)
            .unwrap();
        let plain = without_global(read_plain(ArchiveFormat::Tar, &stored(&memory, "a.tar")));
        assert_eq!(
            plain.get(&format!("{to}/{}", "q".repeat(90))),
            Some(&file_of(b"MOVED"))
        );
        assert_eq!(
            plain.get(&format!("{to}/victim.txt")),
            Some(&file_of(b"VICTIM"))
        );
        assert_eq!(fresh_view(&memory, &file), plain);
    }
}

/// A touch leaves the name as it is, byte for byte, in every way a tar spells one.
#[test]
fn a_touch_leaves_the_name_blocks_alone() {
    let dir = "p".repeat(80);
    let long = "n".repeat(150);
    let mut records = split_member(dir.as_bytes(), b"split", b"a");
    let mut gnu_name = RawTar::file(b"././@LongLink", format!("{long}\0").as_bytes());
    gnu_name.type_flag = b'L';
    records.extend(gnu_name.bytes());
    records.extend(RawTar::file(long.get(..100).unwrap().as_bytes(), b"b").bytes());
    records.extend(pax(&[("path", format!("pax/{long}").as_bytes())]));
    records.extend(RawTar::file(b"truncated", b"c").bytes());
    records.extend(RawTar::file(b"./dot/f", b"d").bytes());
    records.extend_from_slice(&[0; 1024]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", records.clone());
    let vfs = &*memory.vfs;
    let before = walk(vfs, &member(&file, ""));
    let at = SetTimes {
        modified: Some(ft(1_000_000_000)),
        created: None,
    };
    let paths = [
        format!("{dir}/split"),
        long.clone(),
        format!("pax/{long}"),
        "dot/f".to_owned(),
    ];
    vfs.begin(&file).unwrap();
    for path in &paths {
        vfs.set_times(&member(&file, path), &at).unwrap();
    }
    vfs.finish(&file, Finish::Commit).unwrap();
    let after = stored(&memory, "a.tar");
    assert_eq!(after.len(), records.len(), "no block came or went");
    assert_eq!(fresh_view(&memory, &file), before, "the same members");
    let plain = read_plain(ArchiveFormat::Tar, &after);
    assert_eq!(
        plain.get("./dot/f"),
        Some(&file_of(b"d")),
        "the plain crate shows the spelling"
    );
    let spelled: Vec<u8> = b"./dot/f".to_vec();
    assert!(
        after.windows(spelled.len()).any(|w| w == spelled),
        "`./` stays"
    );
}

/// A name that does not decode is not rewritten from its text.
#[test]
fn a_touch_keeps_a_name_that_does_not_decode() {
    let raw: &[u8] = b"l\xe4s\xff.txt";
    let tar = raw_tar(&[RawTar::file(raw, b"BAD"), RawTar::file(b"ok", b"OK")]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("bad.tar", tar);
    let vfs = &*memory.vfs;
    let listing = vfs
        .list(
            &member(&file, ""),
            &ListOptions::default(),
            &CancelToken::never(),
        )
        .unwrap();
    let text = listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str().to_owned())
        .find(|name| name != "ok")
        .unwrap();
    let spec = member(&file, &text);
    let at = SetTimes {
        modified: Some(ft(1_000_000_000)),
        created: None,
    };
    vfs.set_times(&spec, &at).unwrap();
    let bytes = stored(&memory, "bad.tar");
    assert!(
        bytes.windows(raw.len()).any(|w| w == raw),
        "the raw name stays"
    );
    assert_eq!(read_member(vfs, &spec), b"BAD");
    assert_eq!(
        vfs.stat(&spec, FollowLinks::No).unwrap().mtime,
        Some(ft(1_000_000_000))
    );
    let other = format!("d/{text}");
    vfs.create_dir(&member(&file, "d"), false).unwrap();
    let moved = vfs.rename(&spec, &member(&file, &other), false);
    assert!(
        matches!(moved, Err(VfsError::OperationUnsupported { .. })),
        "the replacement character is no name: {moved:?}"
    );
    vfs.rename(&spec, &member(&file, "good.txt"), false)
        .unwrap();
    assert_eq!(read_member(vfs, &member(&file, "good.txt")), b"BAD");
}

// ---- shadowed duplicates --------------------------------------------------------------------------------------

const OLD: &[u8] = b"older version";
const NEW: &[u8] = b"current version";

fn dup_zip() -> Vec<u8> {
    raw_zip(&[
        RawEntry::new(b"doc", OLD),
        RawEntry::new(b"other", b"other"),
        RawEntry::new(b"doc", NEW),
    ])
}

fn dup_tar() -> Vec<u8> {
    raw_tar(&[
        RawTar::file(b"doc", OLD),
        RawTar::file(b"other", b"other"),
        RawTar::file(b"doc", NEW),
    ])
}

/// A zip whose shadow has another raw name, and so a record number of its own.
fn dot_zip() -> Vec<u8> {
    raw_zip(&[
        RawEntry::new(b"./doc", OLD),
        RawEntry::new(b"other", b"other"),
        RawEntry::new(b"doc", NEW),
    ])
}

fn has(bytes: &[u8], what: &[u8]) -> bool {
    bytes.windows(what.len()).any(|w| w == what)
}

fn dup_warnings(vfs: &dyn Vfs) -> Vec<String> {
    vfs.take_warnings()
        .into_iter()
        .filter(|w| w.code == WarningCode::ArchiveDuplicateMember)
        .map(|w| w.message)
        .collect()
}

/// Each archive with a shadowed `doc`, its name, and what a reader says of it.
fn dup_cases() -> [(&'static str, Vec<u8>); 3] {
    [
        ("hidden.zip", dup_zip()),
        ("numbered.zip", dot_zip()),
        ("a.tar", dup_tar()),
    ]
}

fn format_of(name: &str) -> ArchiveFormat {
    if name.ends_with(".zip") {
        ArchiveFormat::Zip
    } else {
        ArchiveFormat::Tar
    }
}

/// The records of `doc` in the plain reading of `bytes`, oldest first: the crates list every record.
fn versions(name: &str, bytes: &[u8], path: &str) -> Vec<Vec<u8>> {
    let wanted = |text: &str| text.trim_start_matches("./") == path;
    if format_of(name) == ArchiveFormat::Zip {
        let mut out = Vec::new();
        let mut at = 0;
        // The local headers, in order: a zip has no other list that shows a duplicate.
        while let Some(start) = bytes[at..]
            .windows(4)
            .position(|w| w == b"PK\x03\x04")
            .map(|p| p + at)
        {
            let size =
                u32::from_le_bytes(bytes[start + 18..start + 22].try_into().unwrap()) as usize;
            let name_len =
                u16::from_le_bytes(bytes[start + 26..start + 28].try_into().unwrap()) as usize;
            let extra =
                u16::from_le_bytes(bytes[start + 28..start + 30].try_into().unwrap()) as usize;
            let text = String::from_utf8_lossy(&bytes[start + 30..start + 30 + name_len]);
            let data = start + 30 + name_len + extra;
            if wanted(&text) {
                out.push(bytes[data..data + size].to_vec());
            }
            at = data + size;
        }
        return out;
    }
    let mut archive = tar::Archive::new(Cursor::new(bytes));
    let mut out = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let text = entry.path().unwrap().to_string_lossy().into_owned();
        if wanted(&text) {
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            out.push(data);
        }
    }
    out
}

/// A rebuild that leaves the winner alone keeps its shadow as raw bytes, without a word of a drop.
#[test]
fn a_shadow_stays_while_its_winner_stays() {
    for (name, bytes) in dup_cases() {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        write_member(vfs, &member(&file, "c"), b"c").unwrap();
        assert_eq!(
            dup_warnings(vfs)
                .iter()
                .filter(|m| m.contains("went"))
                .count(),
            0,
            "{name}"
        );
        let after = stored(&memory, name);
        assert_eq!(versions(name, &after, "doc"), [OLD, NEW], "{name}");
        assert_eq!(read_member(vfs, &member(&file, "doc")), NEW);
        let view = fresh_view(&memory, &file);
        assert_eq!(view.get("doc"), Some(&file_of(NEW)), "{name}");
        assert_eq!(view.get("c"), Some(&file_of(b"c")), "{name}");
        drop(read_plain(format_of(name), &after));
    }
}

/// So does a touch of the winner.
#[test]
fn a_shadow_stays_when_its_winner_is_touched() {
    for (name, bytes) in dup_cases() {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        let at = SetTimes {
            modified: Some(ft(1_000_000_000)),
            created: None,
        };
        vfs.set_times(&member(&file, "doc"), &at).unwrap();
        let after = stored(&memory, name);
        assert_eq!(versions(name, &after, "doc"), [OLD, NEW], "{name}");
        let dir = tempfile::tempdir().unwrap();
        let fresh = fresh(&memory, &dir);
        let entry = fresh.stat(&member(&file, "doc"), FollowLinks::No).unwrap();
        assert_eq!(entry.mtime, Some(ft(1_000_000_000)), "{name}");
    }
}

/// A shadow moves with its winner, and older still loses.
#[test]
fn a_shadow_follows_its_winner_to_a_new_path() {
    for (name, bytes) in dup_cases() {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        vfs.create_dir(&member(&file, "d"), false).unwrap();
        let to = "d/moved-to-a-longer-name";
        vfs.rename(&member(&file, "doc"), &member(&file, to), false)
            .unwrap();
        let after = stored(&memory, name);
        assert_eq!(versions(name, &after, to), [OLD, NEW], "{name}");
        assert_eq!(
            versions(name, &after, "doc"),
            Vec::<Vec<u8>>::new(),
            "{name}"
        );
        let view = fresh_view(&memory, &file);
        assert_eq!(view.get(to), Some(&file_of(NEW)), "{name}");
        assert!(!view.contains_key("doc"), "{name}");
    }
}

/// A removed or replaced winner takes its shadow with it, and the count is in a warning: what was overwritten must not
/// live on in an older record.
#[test]
fn a_shadow_goes_with_its_winner() {
    for (name, bytes) in dup_cases() {
        for replace in [false, true] {
            let memory = on_memory(ArchiveLimits::default());
            let file = memory.put(name, bytes.clone());
            let vfs = &*memory.vfs;
            if replace {
                write_member(vfs, &member(&file, "doc"), b"replacement").unwrap();
            } else {
                vfs.remove(&member(&file, "doc")).unwrap();
            }
            let warnings = dup_warnings(vfs);
            assert!(
                warnings.iter().any(|m| m.contains("1 shadowed")),
                "{name}: {warnings:?}"
            );
            let after = stored(&memory, name);
            assert!(!has(&after, OLD), "{name}");
            assert!(!has(&after, NEW), "{name}");
            let view = fresh_view(&memory, &file);
            let wanted = replace.then(|| file_of(b"replacement"));
            assert_eq!(view.get("doc").cloned(), wanted, "{name}");
        }
    }
}

/// An aborted batch or a failed rebuild drops nothing, so it says nothing.
#[test]
fn a_rebuild_that_fails_warns_of_no_drop() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", dup_tar());
    let vfs = &*memory.vfs;
    drop(vfs.take_warnings());
    memory.mem.inject(hikasa_vfs::Fault {
        path: host("/a.tar"),
        op: VfsOp::Commit,
        kind: ErrorKind::Io,
    });
    assert!(vfs.remove(&member(&file, "doc")).is_err());
    assert!(!dup_warnings(vfs).iter().any(|m| m.contains("went")));
}

/// A link to a kept shadow stays a link, and to one that goes it becomes a file with the bytes it always had.
#[test]
fn a_link_to_a_shadow_follows_the_shadow() {
    let tar = || {
        raw_tar(&[
            RawTar::file(b"a", OLD),
            RawTar::link(b"link", b'1', b"a"),
            RawTar::file(b"a", NEW),
            RawTar::file(b"z", b"z"),
        ])
    };
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", tar());
    let vfs = &*memory.vfs;
    assert_eq!(read_member(vfs, &member(&file, "link")), OLD);
    vfs.remove(&member(&file, "z")).unwrap();
    let after = stored(&memory, "a.tar");
    let mut archive = tar::Archive::new(Cursor::new(&after));
    let kinds: Vec<(String, tar::EntryType)> = archive
        .entries()
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.path().unwrap().to_string_lossy().into_owned(),
                entry.header().entry_type(),
            )
        })
        .collect();
    assert_eq!(
        kinds[1],
        ("link".to_owned(), tar::EntryType::Link),
        "{kinds:?}"
    );
    assert_eq!(fresh_view(&memory, &file).get("link"), Some(&file_of(OLD)));

    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", tar());
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "a")).unwrap();
    let view = fresh_view(&memory, &file);
    assert_eq!(view.get("link"), Some(&file_of(OLD)));
    assert!(!view.contains_key("a"));
    assert_eq!(
        read_plain(ArchiveFormat::Tar, &stored(&memory, "a.tar")).get("link"),
        Some(&file_of(OLD))
    );
}

/// A winner that is a link written as a file keeps its shadow; a shadowed link that loses what it names goes, since a
/// file in its place would win.
#[test]
fn a_winner_that_stops_being_a_link_keeps_its_shadow() {
    let memory = on_memory(ArchiveLimits::default());
    let tar = raw_tar(&[
        RawTar::file(b"base", b"base"),
        RawTar::file(b"x", OLD),
        RawTar::link(b"x", b'1', b"base"),
        RawTar::file(b"keep", b"keep"),
    ]);
    let file = memory.put("a.tar", tar);
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "base")).unwrap();
    let after = stored(&memory, "a.tar");
    assert_eq!(versions("a.tar", &after, "x"), [OLD, b"base".as_slice()]);
    assert_eq!(fresh_view(&memory, &file).get("x"), Some(&file_of(b"base")));

    let memory = on_memory(ArchiveLimits::default());
    let tar = raw_tar(&[
        RawTar::file(b"base", b"base"),
        RawTar::link(b"x", b'1', b"base"),
        RawTar::file(b"x", NEW),
    ]);
    let file = memory.put("a.tar", tar);
    let vfs = &*memory.vfs;
    drop(vfs.take_warnings());
    vfs.remove(&member(&file, "base")).unwrap();
    let warnings = dup_warnings(vfs);
    assert!(
        warnings.iter().any(|m| m.contains("1 shadowed")),
        "{warnings:?}"
    );
    let view = fresh_view(&memory, &file);
    assert_eq!(view.get("x"), Some(&file_of(NEW)));
    assert_eq!(view.len(), 1, "{view:?}");
}

/// A shadow that no rewrite can carry stops its winner's rename at the primitive, as a sparse winner does.
#[test]
fn a_sparse_shadow_stops_a_rename() {
    let mut sparse = RawTar::typed(b"s", b'S');
    sparse.data = b"sparse blocks".to_vec();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", raw_tar(&[sparse, RawTar::file(b"s", NEW)]));
    let vfs = &*memory.vfs;
    let result = vfs.rename(&member(&file, "s"), &member(&file, "t"), false);
    assert!(
        matches!(result, Err(VfsError::OperationUnsupported { .. })),
        "{result:?}"
    );
    let at = SetTimes {
        modified: Some(ft(1_000_000_000)),
        created: None,
    };
    vfs.set_times(&member(&file, "s"), &at).unwrap();
    assert!(
        has(&stored(&memory, "a.tar"), b"sparse blocks"),
        "and a touch keeps it"
    );
}

/// The next index counts the records a rebuild kept: what it could not read again is not written.
#[test]
fn shadows_count_against_the_entry_limit() {
    let limits = ArchiveLimits {
        entries_max: 3,
        ..ArchiveLimits::default()
    };
    for (name, bytes) in dup_cases() {
        let memory = on_memory(limits);
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        let result = write_member(vfs, &member(&file, "c"), b"c");
        assert!(
            matches!(
                result,
                Err(VfsError::ArchiveLimit {
                    limit: "entries",
                    ..
                })
            ),
            "{name}: {result:?}"
        );
    }
}

/// `--dry-run` says what a run would drop.
#[test]
fn a_dry_run_mentions_what_would_go() {
    for (name, bytes) in dup_cases() {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes);
        let vfs = &*memory.vfs;
        let opts = RemoveOptions::default();
        let mut plan =
            ops::plan_remove(vfs, &[member(&file, "doc")], &opts, &Ctl::detached()).unwrap();
        ops::note_shadow_drops(vfs, vfs, &mut plan);
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.code == WarningCode::ArchiveDuplicateMember),
            "{name}: {:?}",
            plan.warnings
        );
        let mut plan =
            ops::plan_remove(vfs, &[member(&file, "other")], &opts, &Ctl::detached()).unwrap();
        ops::note_shadow_drops(vfs, vfs, &mut plan);
        assert!(plan.warnings.is_empty(), "{name}: {:?}", plan.warnings);
    }
}

// ---- hard links -------------------------------------------------------------------------------------------------

/// A hard link written as a file keeps what its header said: owner, names, mode, a PAX record.
#[test]
fn a_materialized_link_keeps_its_owner() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut target = tar::Header::new_ustar();
    target.set_size(5);
    target.set_mode(0o644);
    builder
        .append_data(&mut target, "target", &b"bytes"[..])
        .unwrap();
    let mut link = tar::Header::new_ustar();
    link.set_entry_type(tar::EntryType::Link);
    link.set_size(0);
    link.set_uid(1234);
    link.set_gid(4321);
    link.set_mode(0o640);
    link.set_username("alice").unwrap();
    link.set_groupname("staff").unwrap();
    link.set_mtime(1_500_000_000);
    builder.append_link(&mut link, "link", "target").unwrap();
    let bytes = builder.into_inner().unwrap();
    let mut with_end = bytes;
    with_end.extend_from_slice(&[0; 1024]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.tar", with_end);
    memory.vfs.remove(&member(&file, "target")).unwrap();
    let after = stored(&memory, "a.tar");
    let mut archive = tar::Archive::new(Cursor::new(&after));
    let mut entries = archive.entries().unwrap();
    let mut entry = entries.next().unwrap().unwrap();
    let header = entry.header();
    assert_eq!(header.entry_type(), tar::EntryType::Regular);
    assert_eq!(header.uid().unwrap(), 1234);
    assert_eq!(header.gid().unwrap(), 4321);
    assert_eq!(header.mode().unwrap() & 0o777, 0o640);
    assert_eq!(header.username().unwrap(), Some("alice"));
    assert_eq!(header.groupname().unwrap(), Some("staff"));
    assert_eq!(header.mtime().unwrap(), 1_500_000_000);
    assert_eq!(header.link_name_bytes(), None);
    let mut data = Vec::new();
    entry.read_to_end(&mut data).unwrap();
    assert_eq!(data, b"bytes");
    assert!(entries.next().is_none());
}

/// It takes the place of its record, so a link that named it stays one.
#[test]
fn a_link_to_a_materialized_link_stays_a_link() {
    let memory = on_memory(ArchiveLimits::default());
    let tar = raw_tar(&[
        RawTar::file(b"target", b"bytes"),
        RawTar::link(b"link", b'1', b"target"),
        RawTar::link(b"link2", b'1', b"link"),
    ]);
    let file = memory.put("a.tar", tar);
    memory.vfs.remove(&member(&file, "target")).unwrap();
    let after = stored(&memory, "a.tar");
    let mut archive = tar::Archive::new(Cursor::new(&after));
    let kinds: Vec<tar::EntryType> = archive
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap().header().entry_type())
        .collect();
    assert_eq!(kinds, [tar::EntryType::Regular, tar::EntryType::Link]);
    let view = fresh_view(&memory, &file);
    assert_eq!(view.get("link2"), Some(&file_of(b"bytes")));
}

// ---- a member over its archive ----------------------------------------------------------------------------------

/// A member cannot replace the archive that holds it, or a level of its chain, whatever the policy.
#[test]
fn a_member_over_its_own_archive_is_refused() {
    let inner = raw_tar(&[RawTar::file(b"x", b"x"), RawTar::file(b"y", b"y")]);
    let bytes = raw_zip(&[
        RawEntry::new(b"x", b"X-only"),
        RawEntry::new(b"b.tar", &inner),
        RawEntry::new(b"precious", b"P"),
    ]);
    for policy in OverwritePolicy::ALL {
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put("o.zip", bytes.clone());
        let vfs = &*memory.vfs;
        let chain = member(&file, "b.tar");
        let pairs = [
            (member(&file, "x"), file.clone()),
            (member(&chain, "x"), file.clone()),
            (member(&chain, "x"), chain.clone()),
        ];
        for (from, to) in &pairs {
            for report in [mv(vfs, from, to, policy), cp(vfs, from, to, policy)] {
                assert_eq!(report.summary.failed, 1, "{policy:?}: {:?}", report.results);
                let error = report.results[0].error.as_ref().unwrap();
                assert!(error.message.contains("holds the source"), "{error:?}");
            }
        }
        assert_eq!(stored(&memory, "o.zip"), bytes, "{policy:?}");
    }
}

/// Through a link to it too, where the file's identity is known.
#[cfg(unix)]
#[test]
fn a_member_over_a_link_to_its_archive_is_refused() {
    let disk = on_disk(ArchiveLimits::default());
    let bytes = raw_zip(&[RawEntry::new(b"x", b"X"), RawEntry::new(b"p", b"P")]);
    let file = disk.put("o.zip", &bytes);
    let link = disk.dir.join("l.zip");
    std::os::unix::fs::symlink("o.zip", &link).unwrap();
    let vfs = &*disk.vfs;
    let to = local(&link);
    for report in [
        mv(vfs, &member(&file, "x"), &to, OverwritePolicy::Always),
        cp(vfs, &member(&file, "x"), &to, OverwritePolicy::Always),
    ] {
        assert_eq!(report.summary.failed, 1, "{:?}", report.results);
    }
    assert_eq!(std::fs::read(disk.dir.join("o.zip")).unwrap(), bytes);
}

/// Below a root as well: a member named like its archive, copied out next to it.
#[test]
fn a_copy_out_cannot_land_on_its_archive() {
    let memory = on_memory(ArchiveLimits::default());
    let bytes = raw_zip(&[
        RawEntry::new(b"o.zip", b"a member"),
        RawEntry::new(b"p", b"P"),
    ]);
    let file = memory.put("w/o.zip", bytes.clone());
    let vfs = &*memory.vfs;
    let report = cp(
        vfs,
        &member(&file, ""),
        &mspec("/w"),
        OverwritePolicy::Always,
    );
    let failed: Vec<_> = report
        .results
        .iter()
        .filter_map(|r| r.error.as_ref())
        .collect();
    assert_eq!(failed.len(), 1, "{:?}", report.results);
    assert!(failed[0].message.contains("holds the source"), "{failed:?}");
    assert_eq!(stored(&memory, "w/o.zip"), bytes);
    assert!(
        memory.mem.read_back(&host("/w/p")).is_some(),
        "the rest went out"
    );
}

/// A file moved into itself would be removed after the copy, with the copy in it.
#[test]
fn a_file_cannot_move_into_itself() {
    let inner = raw_tar(&[RawTar::file(b"x", b"x")]);
    let bytes = raw_zip(&[RawEntry::new(b"b.tar", &inner), RawEntry::new(b"p", b"P")]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("o.zip", bytes.clone());
    let vfs = &*memory.vfs;
    let chain = member(&file, "b.tar");
    for (from, to) in [
        (file.clone(), member(&file, "self")),
        (chain.clone(), member(&chain, "self")),
        (file.clone(), member(&chain, "self")),
    ] {
        let report = mv(vfs, &from, &to, OverwritePolicy::Fail);
        assert_eq!(report.summary.failed, 1, "{:?}", report.results);
        assert_eq!(stored(&memory, "o.zip"), bytes);
    }
    let report = cp(vfs, &file, &member(&file, "copy"), OverwritePolicy::Fail);
    assert!(report.ok(), "a copy of it is a copy: {:?}", report.results);
}

// ---- what a remove says, times, the ratio, odd records ----------------------------------------------------------

/// A record refused for a conflict stays in the archive, and reads again once its path is free. That is said.
#[test]
fn a_path_that_comes_free_is_reported() {
    let memory = on_memory(ArchiveLimits::default());
    let bytes = raw_zip(&[RawEntry::new(b"a", b"FILE-A"), RawEntry::new(b"a/b", b"B")]);
    let file = memory.put("rf.zip", bytes);
    let vfs = &*memory.vfs;
    let report = rm(vfs, &[member(&file, "a/b")], false);
    assert!(report.ok(), "{:?}", report.results);
    drop(vfs.take_warnings());
    let report = rm(vfs, &[member(&file, "a")], false);
    assert!(report.ok(), "{:?}", report.results);
    let warnings = vfs.take_warnings();
    assert!(
        warnings.iter().any(|w| w.code == WarningCode::ArchiveMemberRefused
            && w.message.contains("came free")),
        "{warnings:?}"
    );
    assert_eq!(read_member(vfs, &member(&file, "a")), b"FILE-A");

    let memory = on_memory(ArchiveLimits::default());
    let bytes = raw_zip(&[
        RawEntry::new(b"../x", b"evil"),
        RawEntry::new(b"x", b"X"),
        RawEntry::new(b"y", b"Y"),
    ]);
    let file = memory.put("t.zip", bytes);
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "x")).unwrap();
    assert_eq!(fresh_view(&memory, &file).len(), 1, "a name stays refused");
    assert!(has(&stored(&memory, "t.zip"), b"../x"));
}

/// A time the format cannot hold is stored as the nearest it can, and the operation says so.
#[test]
fn a_time_out_of_range_is_reported() {
    for (name, format, seconds) in [
        ("a.zip", ArchiveFormat::Zip, -86_400),
        ("b.zip", ArchiveFormat::Zip, 1 << 33),
        ("a.tar", ArchiveFormat::Tar, -86_400),
    ] {
        let tree: TreeData = vec![("m".into(), b"m".to_vec())];
        let memory = on_memory(ArchiveLimits::default());
        let file = memory.put(name, bytes_of(format, &tree));
        let vfs = &*memory.vfs;
        let at = SetTimes {
            modified: Some(ft(seconds)),
            created: None,
        };
        vfs.set_times(&member(&file, "m"), &at).unwrap();
        let warnings = vfs.take_warnings();
        assert!(
            warnings.iter().any(|w| w.message.contains("does not fit")),
            "{name}: {warnings:?}"
        );
        let in_range = SetTimes {
            modified: Some(ft(1_000_000_000)),
            created: None,
        };
        vfs.set_times(&member(&file, "m"), &in_range).unwrap();
        assert!(
            vfs.take_warnings()
                .iter()
                .all(|w| !w.message.contains("does not fit"))
        );
    }
}

/// A local 0x5455 field keeps the access and change times a touch does not set.
#[test]
fn a_touch_keeps_the_other_times_of_a_long_field() {
    let time = |seconds: u32| seconds.to_le_bytes();
    let mut local = vec![0x55, 0x54, 13, 0, 7];
    local.extend_from_slice(&time(1_500_000_000));
    local.extend_from_slice(&time(1_400_000_000));
    local.extend_from_slice(&time(1_300_000_000));
    let mut entry = RawEntry::new(b"m", b"m");
    entry.extra = local.clone();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", raw_zip(&[entry]));
    let vfs = &*memory.vfs;
    let at = SetTimes {
        modified: Some(ft(1_000_000_000)),
        created: None,
    };
    vfs.set_times(&member(&file, "m"), &at).unwrap();
    let after = stored(&memory, "a.zip");
    let mut expected = vec![0x55, 0x54, 13, 0, 7];
    expected.extend_from_slice(&time(1_000_000_000));
    expected.extend_from_slice(&time(1_400_000_000));
    expected.extend_from_slice(&time(1_300_000_000));
    assert!(has(&after, &expected));
    assert_eq!(
        fresh_view(&memory, &file).get("m"),
        Some(&file_of(b"m")),
        "and it reads"
    );
}

/// Taking the incompressible part out of an archive raises its ratio, which is no reason to keep it in.
#[test]
fn a_remove_does_not_fail_on_the_ratio() {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;
    let limits = ArchiveLimits {
        ratio_max: 20,
        ratio_floor: 1 << 20,
        ..ArchiveLimits::default()
    };
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("zero", options).unwrap();
    writer.write_all(&vec![0u8; 16 << 20]).unwrap();
    writer.start_file("noise", options).unwrap();
    let mut noise = common::XorShift(7);
    writer.write_all(&noise.bytes(1 << 20)).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let memory = on_memory(limits);
    let file = memory.put("r.zip", bytes);
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "noise")).unwrap();
    let warnings = vfs.take_warnings();
    assert!(
        warnings.iter().any(|w| w.message.contains("ratio_max")),
        "{warnings:?}"
    );
    let plain = read_plain(ArchiveFormat::Zip, &stored(&memory, "r.zip"));
    assert!(plain.contains_key("zero") && !plain.contains_key("noise"));
}

/// A parent rename keeps the bytes of a name in another code page: its text here is only a reading of them.
#[test]
fn a_rename_keeps_the_raw_bytes_it_does_not_change() {
    let shift_jis: &[u8] = b"\x93\xfa\x96\x7b\x8c\xea.txt";
    let mut raw = b"d/".to_vec();
    raw.extend_from_slice(shift_jis);
    let bytes = raw_zip(&[RawEntry::new(&raw, b"J"), RawEntry::new(b"other", b"O")]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("j.zip", bytes);
    let vfs = &*memory.vfs;
    vfs.rename(&member(&file, "d"), &member(&file, "e"), false)
        .unwrap();
    let after = stored(&memory, "j.zip");
    let mut wanted = b"e/".to_vec();
    wanted.extend_from_slice(shift_jis);
    assert!(has(&after, &wanted), "the raw name stays");
    let reader = zip::ZipArchive::new(Cursor::new(&after)).unwrap();
    let flagged = (0..reader.len()).any(|at| reader.name_for_index(at).is_none());
    assert!(!flagged);
    let names: Vec<_> = walk(vfs, &member(&file, "e")).into_keys().collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

/// A signature of the central directory signs what a rebuild changes. It cannot be kept, so that is said.
#[test]
fn a_directory_signature_is_reported() {
    let bytes = raw_zip(&[RawEntry::new(b"a", b"A"), RawEntry::new(b"b", b"B")]);
    let end = bytes.len() - 22;
    let mut signed = bytes[..end].to_vec();
    signed.extend_from_slice(b"PK\x05\x05\x04\x00SIGN");
    signed.extend_from_slice(&bytes[end..]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("s.zip", signed);
    let vfs = &*memory.vfs;
    vfs.remove(&member(&file, "b")).unwrap();
    let warnings = vfs.take_warnings();
    assert!(
        warnings.iter().any(|w| w.message.contains("signature")),
        "{warnings:?}"
    );
    let plain = read_plain(ArchiveFormat::Zip, &stored(&memory, "s.zip"));
    assert_eq!(plain.get("a"), Some(&file_of(b"A")));
}

/// A volume label's size field holds nothing, as GNU tar writes it, and the record is a valid one a rebuild keeps.
#[test]
fn a_volume_label_is_read_and_stays() {
    let mut label = [0u8; 512];
    label[..7].copy_from_slice(b"MYLABEL");
    label[136..147].copy_from_slice(b"15271500467");
    label[156] = b'V';
    seal(&mut label);
    let mut bytes = label.to_vec();
    bytes.extend(RawTar::file(b"a", b"A").bytes());
    bytes.extend(RawTar::file(b"b", b"B").bytes());
    bytes.extend_from_slice(&[0; 1024]);
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("v.tar", bytes);
    let vfs = &*memory.vfs;
    let view = walk(vfs, &member(&file, ""));
    assert_eq!(view.len(), 2, "{view:?}");
    vfs.remove(&member(&file, "a")).unwrap();
    let after = stored(&memory, "v.tar");
    assert_eq!(&after[..512], &label[..], "the label stays");
    assert_eq!(fresh_view(&memory, &file).get("b"), Some(&file_of(b"B")));
    let target = member(&file, "b");
    let at = SetTimes {
        modified: Some(ft(1_000_000_000)),
        created: None,
    };
    vfs.set_times(&target, &at).unwrap();
    assert_eq!(&stored(&memory, "v.tar")[..512], &label[..]);
}

// ---- several target roots -------------------------------------------------------------------------------------

/// A cancel between two rebuilds: the first went through, so its copies stay, and its sources do too. No item says
/// done with its source in place.
#[test]
fn a_cancel_after_one_rebuild_reports_no_finished_move() {
    let mem = Arc::new(MemoryVfs::new(CaseSensitivity::Sensitive));
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancelToken::new();
    let base = CancellingBase {
        mem: Arc::clone(&mem),
        token: cancel.clone(),
        after: 100,
    };
    let vfs = ArchiveVfs::new(
        Arc::new(base) as Arc<dyn Vfs>,
        config(dir.path(), ArchiveLimits::default()),
    );
    let tree: TreeData = vec![("old".into(), vec![1u8; 40])];
    let meta = MemMeta::default();
    mem.insert_file(
        &host("/a.zip"),
        bytes_of(ArchiveFormat::Zip, &tree),
        meta.clone(),
    )
    .unwrap();
    mem.insert_file(
        &host("/b.zip"),
        bytes_of(ArchiveFormat::Zip, &tree),
        meta.clone(),
    )
    .unwrap();
    mem.insert_file(&host("/one"), &b"1"[..], meta.clone())
        .unwrap();
    mem.insert_file(&host("/two"), &b"2"[..], meta).unwrap();
    let (a, b) = (mspec("/a.zip"), mspec("/b.zip"));
    let pairs = [
        (mspec("/one"), member(&a, "one")),
        (mspec("/two"), member(&b, "two")),
    ];
    let plan = ops::plan_move(
        &vfs,
        &vfs,
        &pairs,
        &MoveOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let report = ops::execute(&vfs, &vfs, plan, &ctl(&cancel));
    assert!(report.cancelled, "{:?}", report.results);
    for (item, result) in report.plan.items.iter().zip(&report.results) {
        let source = item.source.as_ref().unwrap();
        let kept = vfs.stat(source, FollowLinks::No).is_ok();
        assert_eq!(
            result.status == ItemStatus::Done,
            !kept,
            "{source:?}: {result:?}"
        );
    }
}

// ---- long names ---------------------------------------------------------------------------------------------------

/// Every format, a member past each field of a header, through a rename and back.
#[test]
fn long_names_survive_every_rewrite() {
    let long_dir = "d".repeat(120);
    let tree: TreeData = vec![
        (format!("{long_dir}/{}", "f".repeat(120)), b"long".to_vec()),
        (
            format!("{long_dir}/{}", "g".repeat(200)),
            b"longer".to_vec(),
        ),
        ("short".into(), b"short".to_vec()),
    ];
    for format in WRITABLE {
        let memory = on_memory(ArchiveLimits::default());
        let name = name_of(format);
        let file = memory.put(name, ustar_of(format, &tree));
        let vfs = &*memory.vfs;
        let mut want = walk(vfs, &member(&file, ""));
        let to = "e".repeat(140);
        vfs.rename(&member(&file, &long_dir), &member(&file, &to), false)
            .unwrap();
        want = want
            .into_iter()
            .map(|(path, seen)| (path.replace(long_dir.as_str(), to.as_str()), seen))
            .collect();
        assert_eq!(fresh_view(&memory, &file), want, "{format:?}");
        assert_eq!(
            read_plain(format, &stored(&memory, name)),
            want,
            "{format:?}"
        );
    }
}
