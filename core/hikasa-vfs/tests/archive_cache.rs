//! A5.5 and the cancellation gate of §1.5.16: location is tested in `hikasa-session` (`resolve_config_dir`); here the
//! layout, accounting, eviction, reuse, invalidation, exit and crash recovery of the temp cache.

mod common;

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::archives::{
    RawEntry, compress, config, name_of, on_disk, on_memory, raw_zip, tar_bytes,
};
use hikasa_core::{CancelToken, Classify, Ctl, ErrorKind, FileTime, Spec};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, ListOptions, LocalVfs, MemMeta, Vfs, VfsError,
};

const MIB: usize = 1 << 20;

/// A stored zip of `count` members of `size` bytes.
fn stored(count: usize, size: usize) -> Vec<u8> {
    let entries: Vec<RawEntry> = (0..count)
        .map(|i| RawEntry::new(format!("m{i}").as_bytes(), &vec![i as u8; size]))
        .collect();
    raw_zip(&entries)
}

fn read(vfs: &dyn Vfs, spec: &Spec) -> Vec<u8> {
    let mut out = Vec::new();
    vfs.open_read(spec).unwrap().read_to_end(&mut out).unwrap();
    out
}

fn files_in(dir: &Path) -> Vec<(String, u64)> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .map(|items| {
            items
                .map(|item| {
                    let item = item.unwrap();
                    (
                        item.file_name().into_string().unwrap(),
                        item.metadata().unwrap().len(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn listing_creates_nothing() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(3, 100));
    let root = Spec::archive_member(file.clone(), "");
    disk.vfs
        .list(&root, &ListOptions::default(), &CancelToken::never())
        .unwrap();
    disk.vfs.prepare(&root, &Ctl::detached()).unwrap();
    read(&*disk.vfs, &Spec::archive_member(file, "m1"));
    assert!(!disk.cache.exists(), "a memory read needs no cache either");
    assert_eq!(disk.vfs.cache_dir(), None);
}

#[test]
fn layout_is_numeric() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("évil name.zip", &stored(2, MIB + 1));
    read(&*disk.vfs, &Spec::archive_member(file, "m0"));
    let dir = disk.vfs.cache_dir().unwrap();
    assert_eq!(dir, disk.cache.join("s1"));
    assert!(disk.cache.join("s1.lock").is_file());
    for (name, _) in files_in(&dir) {
        assert!(
            name.starts_with('m') && name[1..].bytes().all(|b| b.is_ascii_digit()),
            "{name}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&dir).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }
}

#[test]
fn used_equals_the_files() {
    let limits = ArchiveLimits {
        cache_max: 8 * MIB as u64,
        ..ArchiveLimits::default()
    };
    let disk = on_disk(limits);
    let file = disk.put("a.zip", &stored(9, 2 * MIB));
    let mut held = Vec::new();
    // A pseudo-random workload, some readers kept.
    let mut state = 12345usize;
    for step in 0..60 {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let member = Spec::archive_member(file.clone(), format!("m{}", (state >> 16) % 9));
        match disk.vfs.open_read(&member) {
            Ok(reader) => {
                if step % 7 == 0 {
                    held.push(reader);
                }
            }
            Err(error) => {
                assert!(
                    matches!(error, VfsError::ArchiveLimit { limit: "cache", .. }),
                    "{error}"
                );
                held.clear();
            }
        }
        let dir = disk.vfs.cache_dir().unwrap();
        let on_disk: u64 = files_in(&dir).iter().map(|(_, len)| len).sum();
        assert_eq!(disk.vfs.cache_used(), on_disk, "step {step}");
        assert!(on_disk <= 8 * MIB as u64);
    }
}

#[test]
fn eviction_is_lru_and_spares_held_files() {
    let limits = ArchiveLimits {
        cache_max: 5 * MIB as u64,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put("a.zip", stored(4, 2 * MIB));
    let member = |i: u32| Spec::archive_member(file.clone(), format!("m{i}"));
    let vfs = &memory.vfs;
    let held = vfs.open_read(&member(0)).unwrap();
    read(&**vfs, &member(1));
    let decodes = vfs.counters().member_decodes;
    read(&**vfs, &member(1));
    assert_eq!(vfs.counters().member_decodes, decodes, "a hit");
    // m2 needs room: m1 goes, m0 is held.
    read(&**vfs, &member(2));
    assert_eq!(vfs.cache_used(), 4 * MIB as u64);
    drop(held);
    let decodes = vfs.counters().member_decodes;
    read(&**vfs, &member(0));
    assert_eq!(vfs.counters().member_decodes, decodes, "m0 stayed");
    read(&**vfs, &member(1));
    assert_eq!(vfs.counters().member_decodes, decodes + 1, "m1 was evicted");
}

#[test]
fn a_member_over_the_limit_leaves_nothing() {
    let limits = ArchiveLimits {
        cache_max: 2 * MIB as u64,
        ..ArchiveLimits::default()
    };
    let disk = on_disk(limits);
    let file = disk.put("a.zip", &stored(1, 3 * MIB));
    let error = disk
        .vfs
        .open_read(&Spec::archive_member(file, "m0"))
        .map(drop)
        .unwrap_err();
    assert!(
        matches!(error, VfsError::ArchiveLimit { limit: "cache", .. }),
        "{error}"
    );
    assert_eq!(disk.vfs.cache_used(), 0);
    assert_eq!(disk.vfs.cache_dir(), None, "no session either");
}

#[test]
fn a_new_stamp_drops_the_index_and_the_files() {
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("a.zip", stored(2, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let vfs = &memory.vfs;
    assert_eq!(read(&**vfs, &spec)[0], 0);
    let before = vfs.counters();
    assert_eq!(vfs.cache_used(), 2 * MIB as u64);
    // The same path, a new content and time.
    let meta = MemMeta {
        mtime: FileTime::from_unix(2_000_000_000, 0),
        ..MemMeta::default()
    };
    let path = common::host("/a.zip");
    memory
        .mem
        .insert_file(&path, stored(1, 3 * MIB), meta)
        .unwrap();
    assert_eq!(read(&**vfs, &spec).len(), 3 * MIB);
    let after = vfs.counters();
    assert_eq!(after.index_passes, before.index_passes + 1);
    assert_eq!(after.member_decodes, before.member_decodes + 1);
    assert_eq!(vfs.cache_used(), 3 * MIB as u64, "the old file went");
}

#[test]
fn cleanup_and_drop_remove_everything() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    read(&*disk.vfs, &spec);
    assert!(disk.cache.join("s1").is_dir());
    disk.vfs.cleanup();
    assert!(!disk.cache.join("s1").exists());
    assert!(!disk.cache.join("s1.lock").exists());
    assert_eq!(disk.vfs.cache_used(), 0);
    // It works again, and `Drop` removes it too.
    read(&*disk.vfs, &spec);
    assert!(disk.cache.join("s1").is_dir());
    let cache = disk.cache.clone();
    let vfs = disk.vfs;
    drop(vfs);
    assert!(!cache.join("s1").exists());
    assert!(files_in(&cache).is_empty());
}

fn fresh(root: &Path) -> ArchiveVfs {
    ArchiveVfs::new(
        Arc::new(LocalVfs::default()) as Arc<dyn Vfs>,
        config(root, ArchiveLimits::default()),
    )
}

#[test]
fn a_crashed_session_is_swept_and_a_live_one_stays() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let root = disk.cache.clone();
    fs::create_dir_all(root.join("s7")).unwrap();
    fs::write(root.join("s7/m1"), b"left over").unwrap();
    fs::write(root.join("s7.lock"), b"").unwrap();
    // Held by "another process": a second handle in the test.
    fs::create_dir_all(root.join("s8")).unwrap();
    fs::write(root.join("s8/m1"), b"alive").unwrap();
    let held = fs::File::create(root.join("s8.lock")).unwrap();
    held.try_lock().unwrap();
    // Not ours to touch.
    fs::write(root.join("notes.lock"), b"").unwrap();
    fs::create_dir(root.join("s9x")).unwrap();

    read(&*disk.vfs, &spec);
    assert!(!root.join("s7").exists(), "swept at the first spool");
    assert!(!root.join("s7.lock").exists());
    assert_eq!(fs::read(root.join("s8/m1")).unwrap(), b"alive");
    assert!(root.join("notes.lock").exists() && root.join("s9x").exists());
    drop(held);
}

#[test]
fn two_back_ends_get_different_numbers() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let second = fresh(&disk.cache);
    read(&*disk.vfs, &spec);
    read(&second, &spec);
    let (a, b) = (disk.vfs.cache_dir().unwrap(), second.cache_dir().unwrap());
    assert_ne!(a, b);
    assert!(a.is_dir() && b.is_dir(), "neither swept the other");
    drop(second);
    assert!(a.is_dir());
}

/// §1.5.16: `prepare` and a spool of a 1 GiB stream stop within 100 ms of `cancel` (G4). Debug builds decode slower
/// than the release figure, so a step of 1 MiB still lands well inside it.
#[test]
fn cancel_of_prepare_and_spool_of_a_1_gib_stream() {
    let len = 1usize << 30;
    let zeros = vec![0u8; 1 << 20];
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    for _ in 0..(len >> 20) {
        std::io::Write::write_all(&mut encoder, &zeros).unwrap();
    }
    let stream = encoder.finish().unwrap();
    let limits = ArchiveLimits {
        ratio_max: 0,
        ..ArchiveLimits::default()
    };

    // `prepare`: the index pass.
    let memory = on_memory(limits);
    let root = memory.root("big.bin.zst", stream.clone());
    let cancel = CancelToken::new();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &hikasa_core::NullProgress,
    };
    let late = std::thread::scope(|scope| {
        let (vfs, root) = (&memory.vfs, &root);
        let worker = scope.spawn(move || {
            let result = vfs.prepare(root, &ctl);
            (result, Instant::now())
        });
        std::thread::sleep(Duration::from_millis(30));
        let asked = Instant::now();
        cancel.cancel();
        let (result, done) = worker.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        done.duration_since(asked)
    });
    assert!(late < Duration::from_millis(100), "{late:?}");

    // A spool: the index is built, then a read decodes the stream to a cache file.
    let memory = on_memory(limits);
    let root = memory.root("big.bin.zst", stream);
    memory.vfs.prepare(&root, &Ctl::detached()).unwrap();
    let member = Spec::archive_member(common::mspec("/big.bin.zst"), "big.bin");
    let cancel = CancelToken::new();
    let late = std::thread::scope(|scope| {
        let (vfs, member, token) = (&memory.vfs, &member, &cancel);
        let worker = scope.spawn(move || {
            let result = vfs.open_read_with(member, token).map(drop);
            (result, Instant::now())
        });
        std::thread::sleep(Duration::from_millis(30));
        let asked = Instant::now();
        cancel.cancel();
        let (result, done) = worker.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        done.duration_since(asked)
    });
    assert!(late < Duration::from_millis(100), "{late:?}");
    assert_eq!(
        memory.vfs.cache_used(),
        0,
        "a cancelled spool removes its file"
    );
}

#[test]
fn a_read_of_a_compressed_tar_is_cancelled_before_it_starts() {
    let data = vec![("m".to_owned(), vec![1u8; 4 * MIB])];
    let memory = on_memory(ArchiveLimits::default());
    let format = ArchiveFormat::TarGz;
    let file = memory.put(name_of(format), compress(format, &tar_bytes(&data)));
    let cancel = CancelToken::new();
    cancel.cancel();
    let spec = Spec::archive_member(file, "m");
    let error = memory
        .vfs
        .open_read_with(&spec, &cancel)
        .map(drop)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
}
