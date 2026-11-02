//! A5.5 and the cancellation gate of §1.5.16: location is tested in `hikasa-session` (`resolve_config_dir`); here the
//! layout, accounting, eviction, reuse, invalidation, exit and crash recovery of the temp cache.

mod common;

use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::archives::{
    RawEntry, RawTar, compress, config, name_of, on_disk, on_memory, raw_zip, tar_bytes,
};
use hikasa_core::{CancelToken, Classify, Ctl, ErrorKind, FileTime, Spec};
use hikasa_vfs::{
    ArchiveFormat, ArchiveLimits, ArchiveVfs, ListOptions, LocalVfs, MemMeta, Vfs, VfsError,
};

const MIB: usize = 1 << 20;
/// The identity at the start of each cache file (§1.5.6), which `cache_used` does not count.
const STAMP: u64 = 16;

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
    let session = dir.file_name().unwrap().to_str().unwrap();
    assert!(
        session.starts_with('s') && session[1..].bytes().all(|b| b.is_ascii_digit()),
        "{session}"
    );
    assert!(disk.cache.join(format!("{session}.lock")).is_file());
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
        let on_disk: u64 = files_in(&dir).iter().map(|(_, len)| len - STAMP).sum();
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
    assert!(disk.vfs.cache_dir().unwrap().is_dir());
    disk.vfs.cleanup();
    assert!(!disk.cache.exists(), "an empty root goes too");
    assert_eq!(disk.vfs.cache_dir(), None);
    assert_eq!(disk.vfs.cache_used(), 0);
    // It works again, and `Drop` removes it too.
    read(&*disk.vfs, &spec);
    let dir = disk.vfs.cache_dir().unwrap();
    assert!(dir.is_dir());
    let cache = disk.cache.clone();
    let vfs = disk.vfs;
    drop(vfs);
    assert!(!dir.exists());
    assert!(!cache.exists());
}

/// The folders above the root that a session had to make go with it, and a root that holds something else stays.
#[test]
fn a_session_leaves_no_empty_folder_and_touches_no_other_file() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let root = disk.cache.join("deep").join("archives");
    let vfs = fresh(&root);
    read(&vfs, &spec);
    assert!(root.is_dir());
    vfs.cleanup();
    assert!(!disk.cache.exists(), "all three were made by the session");

    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("notes"), b"not ours").unwrap();
    read(&vfs, &spec);
    vfs.cleanup();
    assert_eq!(files_in(&root).len(), 1);
}

/// A file at the path that is not the one written is never served (§1.5.6): the read decodes again.
#[test]
fn a_replaced_cache_file_is_decoded_again() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(2, 2 * MIB));
    let spec = Spec::archive_member(file, "m1");
    let want = read(&*disk.vfs, &spec);
    let dir = disk.vfs.cache_dir().unwrap();
    let (name, len) = files_in(&dir).pop().unwrap();
    assert_eq!(len, 2 * MIB as u64 + STAMP);
    fs::write(dir.join(&name), vec![9u8; len as usize]).unwrap();
    let decodes = disk.vfs.counters().member_decodes;
    assert_eq!(read(&*disk.vfs, &spec), want);
    assert_eq!(disk.vfs.counters().member_decodes, decodes + 1);

    let (name, len) = files_in(&dir).pop().unwrap();
    fs::write(dir.join(&name), vec![0u8; len as usize - 1]).unwrap();
    assert_eq!(read(&*disk.vfs, &spec), want, "a shorter file too");
}

/// What a reader opened stays its own, whatever the path meets later.
#[test]
fn a_reader_keeps_its_bytes_when_the_path_is_replaced() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(2, 2 * MIB));
    let mut reader = disk
        .vfs
        .open_read(&Spec::archive_member(file, "m1"))
        .unwrap();
    let dir = disk.vfs.cache_dir().unwrap();
    let (name, _) = files_in(&dir).pop().unwrap();
    let path = dir.join(&name);
    fs::rename(&path, dir.join("moved")).unwrap();
    fs::write(&path, vec![9u8; 2 * MIB]).unwrap();
    let mut out = Vec::new();
    reader.read_to_end(&mut out).unwrap();
    assert!(out.len() == 2 * MIB && out.iter().all(|&b| b == 1));
    drop(reader);
    fs::remove_file(dir.join("moved")).unwrap();
}

/// A5.5: sessions come and go on one root and each gets its own bytes. A `flock` or `LockFileEx` belongs to the open
/// file, so threads meet in the sweep like processes do. `cli_archive.rs` repeats this with real processes.
#[test]
fn concurrent_sessions_on_one_root_keep_their_bytes() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 120;
    let disk = on_disk(ArchiveLimits::default());
    let root = &disk.cache;
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let member = vec![thread as u8; MIB + 1];
            let bytes = raw_zip(&[RawEntry::new(b"big.bin", &member)]);
            let file = disk.put(&format!("a{thread}.zip"), &bytes);
            let spec = Spec::archive_member(file, "big.bin");
            scope.spawn(move || {
                for round in 0..ROUNDS {
                    // Each round is a whole session: claim, sweep, spool, read, cleanup.
                    let session = fresh(root);
                    let got = read(&session, &spec);
                    assert!(got == member, "thread {thread}, round {round}");
                    // Read twice, the second from the cache file.
                    assert!(read(&session, &spec) == member);
                }
            });
        }
    });
    assert!(!root.exists(), "the last session took the root with it");
}

/// Sweeps that lose a race must never take a live session's folder: live sessions hold files while others come and go.
#[test]
fn a_live_session_survives_a_crowd_of_sweeps() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 2 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let live = fresh(&disk.cache);
    let want = read(&live, &spec);
    let dir = live.cache_dir().unwrap();
    std::thread::scope(|scope| {
        for _ in 0..6 {
            let spec = &spec;
            let root = &disk.cache;
            scope.spawn(move || {
                for _ in 0..80 {
                    read(&fresh(root), spec);
                }
            });
        }
    });
    assert!(dir.is_dir());
    assert_eq!(read(&live, &spec), want);
    let decodes = live.counters().member_decodes;
    read(&live, &spec);
    assert_eq!(live.counters().member_decodes, decodes, "its file is there");
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
    assert!(!b.exists());
}

/// Keeps the largest `done` a pass reports.
#[derive(Default)]
struct BytesSeen(AtomicU64);

impl hikasa_core::ProgressSink for BytesSeen {
    fn report(&self, progress: hikasa_core::Progress) {
        self.0.fetch_max(progress.done, Ordering::Relaxed);
    }
}

/// What a pass may still do after `cancel` was asked: the step of G4, and one on its way. The bound is in bytes and not
/// in time because a machine under load delays a wake-up by any amount, and a pass that checks each step is prompt
/// however late it runs. The time gate of G4 is measured on a release build, in `docs/02`.
const AFTER_CANCEL: u64 = 8 << 20;

/// §1.5.16: `prepare` and a spool of a 1 GiB stream stop within a step of `cancel` (G4).
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
    let seen = BytesSeen::default();
    let ctl = Ctl {
        cancel: &cancel,
        progress: &seen,
    };
    std::thread::scope(|scope| {
        let (vfs, root) = (&memory.vfs, &root);
        let worker = scope.spawn(move || (vfs.prepare(root, &ctl), Instant::now()));
        while seen.0.load(Ordering::Relaxed) == 0 && !worker.is_finished() {
            std::thread::yield_now();
        }
        let asked = Instant::now();
        cancel.cancel();
        let at_cancel = seen.0.load(Ordering::Relaxed);
        let (result, stopped) = worker.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        let more = seen.0.load(Ordering::Relaxed) - at_cancel;
        println!(
            "prepare went on for {more} bytes, {:?}",
            stopped.saturating_duration_since(asked)
        );
        assert!(more <= AFTER_CANCEL, "{more}");
    });

    // A spool: the index is built, then a read decodes the stream to a cache file.
    let memory = on_memory(limits);
    let root = memory.root("big.bin.zst", stream);
    memory.vfs.prepare(&root, &Ctl::detached()).unwrap();
    let member = Spec::archive_member(common::mspec("/big.bin.zst"), "big.bin");
    let cancel = CancelToken::new();
    std::thread::scope(|scope| {
        let (vfs, member, token) = (&memory.vfs, &member, &cancel);
        let worker =
            scope.spawn(move || (vfs.open_read_with(member, token).map(drop), Instant::now()));
        // The spool grows in the cache as it goes.
        while memory.vfs.cache_used() == 0 && !worker.is_finished() {
            std::thread::yield_now();
        }
        let asked = Instant::now();
        cancel.cancel();
        let at_cancel = memory.vfs.cache_used();
        let mut most = at_cancel;
        while !worker.is_finished() {
            most = most.max(memory.vfs.cache_used());
        }
        let (result, stopped) = worker.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        println!(
            "the spool went on for {} bytes, {:?}",
            most - at_cancel,
            stopped.saturating_duration_since(asked)
        );
        assert!(most - at_cancel <= AFTER_CANCEL, "{}", most - at_cancel);
    });
    assert_eq!(
        memory.vfs.cache_used(),
        0,
        "a cancelled spool removes its file"
    );
}

/// A `tar.zst` of `gib` GiB of zeros, from frames of 1 GiB: its index takes a second or more in any build, and a test
/// stops it long before that.
fn slow_index(memory: &common::archives::OnMemory, gib: usize) -> Spec {
    let zeros = |encoder: &mut zstd::stream::write::Encoder<'_, Vec<u8>>, mib: usize| {
        for _ in 0..mib {
            std::io::Write::write_all(encoder, &[0u8; 1 << 20]).unwrap();
        }
    };
    let header = RawTar {
        size: Some((gib as u64) << 30),
        ..RawTar::file(b"big", b"")
    }
    .header();
    let mut first = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    std::io::Write::write_all(&mut first, &header).unwrap();
    zeros(&mut first, 1);
    let mut stream = first.finish().unwrap();
    let mut frame = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    zeros(&mut frame, 1024);
    stream.extend(frame.finish().unwrap().repeat(gib));
    memory.put("big.tar.zst", stream)
}

/// A decoder that panics leaves no reservation, no partial file and no lock behind: the next read works.
#[test]
fn a_panic_leaves_the_cache_usable() {
    let data: Vec<(String, Vec<u8>)> = (0u8..3)
        .map(|i| (format!("m{i}"), vec![i; 2 * MIB]))
        .collect();
    let memory = on_memory(ArchiveLimits::default());
    let format = ArchiveFormat::TarGz;
    let file = memory.put(name_of(format), compress(format, &tar_bytes(&data)));
    let member = |i: u32| Spec::archive_member(file.clone(), format!("m{i}"));
    assert_eq!(read(&*memory.vfs, &member(0)).len(), 2 * MIB);
    let used = memory.vfs.cache_used();
    memory.vfs.inject_decoder_panic();
    let failed = memory.vfs.open_read(&member(1)).map(drop);
    assert!(
        matches!(failed, Err(VfsError::ArchiveCorrupt { .. })),
        "{failed:?}"
    );
    assert_eq!(memory.vfs.cache_used(), used, "the reservation went back");
    assert_eq!(read(&*memory.vfs, &member(1)), vec![1u8; 2 * MIB]);
    assert_eq!(read(&*memory.vfs, &member(2)), vec![2u8; 2 * MIB]);
}

/// G4 for a call that waits: it does not wait for the pass of another call to end.
#[test]
fn cancel_of_a_call_waiting_for_an_index() {
    let limits = ArchiveLimits {
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = slow_index(&memory, 7);
    let root = Spec::archive_member(file, "");
    let stop_builder = CancelToken::new();
    let cancel = CancelToken::new();
    std::thread::scope(|scope| {
        let (vfs, spec, stop) = (&memory.vfs, &root, &stop_builder);
        let builder = scope.spawn(move || vfs.list(spec, &ListOptions::default(), stop).map(drop));
        std::thread::sleep(Duration::from_millis(20));
        let (vfs, spec, token) = (&memory.vfs, &root, &cancel);
        let waiter = scope.spawn(move || {
            let result = vfs.list(spec, &ListOptions::default(), token).map(drop);
            (result, Instant::now())
        });
        std::thread::sleep(Duration::from_millis(20));
        let asked = Instant::now();
        cancel.cancel();
        let (result, done) = waiter.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        println!("waiter stopped after {:?}", done.duration_since(asked));
        // The order is the claim: the pass of 7 GiB goes on, so the call did not wait for it.
        assert!(
            !builder.is_finished(),
            "the pass still ran: the wait was real"
        );
        stop_builder.cancel();
        assert!(matches!(builder.join().unwrap(), Err(VfsError::Cancelled)));
    });
}

/// A read that waits for the one decoder of a stream stops too.
#[test]
fn cancel_of_a_read_waiting_for_the_decoder() {
    let limits = ArchiveLimits {
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = slow_index(&memory, 1);
    let member = Spec::archive_member(file, "big");
    memory.vfs.prepare(&member, &Ctl::detached()).unwrap();
    let stop_reader = CancelToken::new();
    let cancel = CancelToken::new();
    std::thread::scope(|scope| {
        let (vfs, spec, stop) = (&memory.vfs, &member, &stop_reader);
        let reader = scope.spawn(move || vfs.open_read_with(spec, stop).map(drop));
        std::thread::sleep(Duration::from_millis(20));
        let (vfs, spec, token) = (&memory.vfs, &member, &cancel);
        let waiter = scope.spawn(move || {
            let result = vfs.open_read_with(spec, token).map(drop);
            (result, Instant::now())
        });
        std::thread::sleep(Duration::from_millis(20));
        let asked = Instant::now();
        cancel.cancel();
        let (result, done) = waiter.join().unwrap();
        assert!(matches!(result, Err(VfsError::Cancelled)), "{result:?}");
        println!("reader stopped after {:?}", done.duration_since(asked));
        // The order is the claim, as above: the decoder it waited for is still at work.
        assert!(!reader.is_finished(), "the wait was real");
        stop_reader.cancel();
        assert!(matches!(reader.join().unwrap(), Err(VfsError::Cancelled)));
    });
}

/// The tree of a zip of 400,000 entries: cancellation reaches the crate's directory read, the sorts and the loops of
/// `build_tree`, whichever of them a delay lands in.
#[test]
fn cancel_of_a_large_tree() {
    let entries: Vec<RawEntry> = (0..400_000)
        .map(|i| RawEntry::new(format!("dir{}/file{i}", i % 1000).as_bytes(), b""))
        .collect();
    let bytes = raw_zip(&entries);
    // The bound scales with the machine as it is now: a stop takes a fraction of the whole pass, at any load.
    let whole = {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root("big.zip", bytes.clone());
        let started = Instant::now();
        memory.vfs.prepare(&root, &Ctl::detached()).unwrap();
        started.elapsed()
    };
    for delay in [1, 5, 20, 60, 150] {
        let memory = on_memory(ArchiveLimits::default());
        let root = memory.root("big.zip", bytes.clone());
        let cancel = CancelToken::new();
        let ctl = Ctl {
            cancel: &cancel,
            progress: &hikasa_core::NullProgress,
        };
        let (late, stopped) = std::thread::scope(|scope| {
            let (vfs, root) = (&memory.vfs, &root);
            let worker = scope.spawn(move || (vfs.prepare(root, &ctl), Instant::now()));
            std::thread::sleep(Duration::from_millis(delay));
            let asked = Instant::now();
            cancel.cancel();
            let (result, done) = worker.join().unwrap();
            // A fast machine may have finished: the bound is on the time, not on the answer.
            assert!(
                result.is_ok() || matches!(result, Err(VfsError::Cancelled)),
                "{result:?}"
            );
            (done.saturating_duration_since(asked), result.is_err())
        });
        println!("tree, {delay} ms in: stopped {stopped} after {late:?}, the pass takes {whole:?}");
        assert!(
            late < whole.max(Duration::from_millis(100)),
            "{delay} ms in: {late:?}"
        );
    }
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

// ---- A6.13 `extract_to_cache` (D-186) ----------------------------------------------------------------------------

/// Where the session's extractions are: `<session>/x<k>/<name>`.
fn inside_session(vfs: &ArchiveVfs, path: &Path) -> String {
    let session = vfs.cache_dir().unwrap();
    let rest = path.strip_prefix(&session).unwrap();
    let mut parts = rest.iter().map(|part| part.to_string_lossy().into_owned());
    let folder = parts.next().unwrap();
    assert!(folder.starts_with('x'), "{folder}");
    assert!(folder[1..].bytes().all(|b| b.is_ascii_digit()), "{folder}");
    let name = parts.next().unwrap();
    assert_eq!(parts.next(), None);
    name
}

#[test]
fn a6_13_names_and_extensions() {
    let disk = on_disk(ArchiveLimits::default());
    let tar = common::archives::raw_tar(&[
        RawTar::file(b"plain.txt", b"plain"),
        RawTar::file(b"dir/report.final.pdf", b"pdf"),
        RawTar::file(b"ab:c.txt", b"colon"),
        RawTar::file(b"CON.txt", b"device"),
        RawTar::file(b"nul", b"bare device"),
        RawTar::file(b"trailing.", b"dot"),
    ]);
    let file = disk.put("a.tar", &tar);
    let rows = [
        ("plain.txt", "plain.txt", &b"plain"[..]),
        ("dir/report.final.pdf", "report.final.pdf", b"pdf"),
        ("ab:c.txt", "member.txt", b"colon"),
        ("CON.txt", "member.txt", b"device"),
        ("nul", "member", b"bare device"),
        ("trailing.", "member", b"dot"),
    ];
    let cancel = CancelToken::new();
    for (member, name, bytes) in rows {
        let spec = Spec::archive_member(file.clone(), member);
        let path = disk.vfs.extract_to_cache(&spec, &cancel).unwrap();
        assert_eq!(inside_session(&disk.vfs, &path), name, "{member}");
        assert_eq!(fs::read(&path).unwrap(), bytes, "{member}");
        let meta = fs::metadata(&path).unwrap();
        assert!(meta.permissions().readonly(), "{member}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(meta.permissions().mode() & 0o777, 0o400, "{member}");
        }
    }
}

/// The same member twice is one file; a new stamp is a new file, and the old one stays for the session.
#[test]
fn a6_13_same_member_one_file_new_stamp_new_file() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(2, 100));
    let spec = Spec::archive_member(file.clone(), "m1");
    let cancel = CancelToken::new();
    let first = disk.vfs.extract_to_cache(&spec, &cancel).unwrap();
    let again = disk.vfs.extract_to_cache(&spec, &cancel).unwrap();
    assert_eq!(first, again);
    assert_eq!(disk.vfs.cache_used(), 100);
    let time = FileTime::from_unix(1_000_000, 0);
    disk.put("a.zip", &stored(2, 200));
    let vfs: &dyn Vfs = &*disk.vfs;
    vfs.set_times(
        &file,
        &hikasa_vfs::SetTimes {
            modified: time,
            created: None,
        },
    )
    .unwrap();
    let new = disk.vfs.extract_to_cache(&spec, &cancel).unwrap();
    assert_ne!(new, first);
    assert_eq!(fs::read(&new).unwrap().len(), 200);
    assert_eq!(
        fs::read(&first).unwrap().len(),
        100,
        "valid for the session"
    );
}

/// Never evicted under pressure; a member over `cache_max` leaves nothing, and a member that only fits once its spool
/// is gone is taken.
#[test]
fn a6_13_never_evicted_and_over_the_limit() {
    let limits = ArchiveLimits {
        cache_max: 4 * MIB as u64,
        ..ArchiveLimits::default()
    };
    let disk = on_disk(limits);
    let file = disk.put("a.zip", &stored(3, 2 * MIB));
    let cancel = CancelToken::new();
    let member = |name: &str| Spec::archive_member(file.clone(), name);
    let kept = disk.vfs.extract_to_cache(&member("m0"), &cancel).unwrap();
    assert!(disk.vfs.cache_used() <= 4 * MIB as u64 + 2 * STAMP);
    // Reads go through the cache and must evict their own spools, never the extraction.
    for name in ["m1", "m2", "m1", "m2"] {
        read(&*disk.vfs, &member(name));
        assert!(kept.is_file(), "{name}");
    }
    let session = disk.vfs.cache_dir().unwrap();
    let big = disk.put("big.zip", &stored(1, 5 * MIB));
    let error = disk
        .vfs
        .extract_to_cache(&Spec::archive_member(big, "m0"), &cancel)
        .unwrap_err();
    assert!(
        matches!(error, VfsError::ArchiveLimit { limit: "cache", .. }),
        "{error}"
    );
    let folders = fs::read_dir(&session)
        .unwrap()
        .filter(|item| {
            let item = item.as_ref().unwrap();
            item.file_name().to_string_lossy().starts_with('x')
        })
        .count();
    assert_eq!(folders, 1, "the first extraction and nothing else");
    // The limit stands again: what was spooled for a member goes with its reader.
    assert!(disk.vfs.cache_used() <= 4 * MIB as u64);
}

/// A member that fills the cache alone: its own spool does not stop it.
#[test]
fn a6_13_a_member_of_the_whole_cache() {
    let limits = ArchiveLimits {
        cache_max: 3 * MIB as u64,
        ..ArchiveLimits::default()
    };
    let disk = on_disk(limits);
    let file = disk.put("a.zip", &stored(1, 3 * MIB));
    let spec = Spec::archive_member(file, "m0");
    let path = disk
        .vfs
        .extract_to_cache(&spec, &CancelToken::new())
        .unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), 3 * MIB as u64);
    assert_eq!(disk.vfs.cache_used(), 3 * MIB as u64);
}

/// `cleanup` removes it, on Windows after the read-only bit is cleared.
#[test]
fn a6_13_cleanup_removes_it() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 100));
    let spec = Spec::archive_member(file, "m0");
    let path = disk
        .vfs
        .extract_to_cache(&spec, &CancelToken::new())
        .unwrap();
    let session = disk.vfs.cache_dir().unwrap();
    disk.vfs.cleanup();
    assert!(!path.exists());
    assert!(!session.exists());
    assert_eq!(disk.vfs.cache_used(), 0);
    // A later call works, in a new session.
    let again = disk
        .vfs
        .extract_to_cache(&spec, &CancelToken::new())
        .unwrap();
    assert!(again.is_file());
}

/// A crashed session's extraction goes with its folder, on Windows too.
#[test]
fn a6_13_a_sweep_takes_read_only_files() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put("a.zip", &stored(1, 100));
    let spec = Spec::archive_member(file, "m0");
    let root = disk.cache.clone();
    fs::create_dir_all(root.join("s7/x1")).unwrap();
    let left = root.join("s7/x1/report.pdf");
    fs::write(&left, b"left over").unwrap();
    let mut permissions = fs::metadata(&left).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&left, permissions).unwrap();
    fs::write(root.join("s7.lock"), b"").unwrap();
    disk.vfs
        .extract_to_cache(&spec, &CancelToken::new())
        .unwrap();
    assert!(!root.join("s7").exists(), "swept with the first extraction");
    assert!(!root.join("s7.lock").exists());
}

/// Links, encrypted and sparse members give the errors of `open_read`.
#[test]
fn a6_13_what_cannot_be_read() {
    let disk = on_disk(ArchiveLimits::default());
    let tar = common::archives::raw_tar(&[
        RawTar::link(b"lnk", b'2', b"target"),
        RawTar::typed(b"sparse", b'S'),
        RawTar::typed(b"dev", b'3'),
        RawTar::file(b"dir/x", b"x"),
    ]);
    let file = disk.put("a.tar", &tar);
    let cancel = CancelToken::new();
    let spec = |name: &str| Spec::archive_member(file.clone(), name);
    for name in ["lnk", "sparse", "dev"] {
        let error = disk.vfs.extract_to_cache(&spec(name), &cancel).unwrap_err();
        let read = (&*disk.vfs as &dyn Vfs).open_read(&spec(name)).err();
        assert_eq!(Some(error.kind()), read.map(|e| e.kind()), "{name}");
    }
    let folder = disk
        .vfs
        .extract_to_cache(&spec("dir"), &cancel)
        .unwrap_err();
    assert_eq!(folder.kind(), ErrorKind::IsADirectory);
    let plain = disk.put("plain.txt", b"x");
    let error = disk.vfs.extract_to_cache(&plain, &cancel).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(disk.vfs.cache_dir(), None, "nothing was created");
}

#[test]
fn a6_13_an_encrypted_member() {
    let disk = on_disk(ArchiveLimits::default());
    let file = disk.put(
        "a.zip",
        &common::archives::raw_zip(&[{
            let mut entry = RawEntry::new(b"s", b"secret");
            entry.flags |= 1;
            entry
        }]),
    );
    let spec = Spec::archive_member(file, "s");
    let error = disk
        .vfs
        .extract_to_cache(&spec, &CancelToken::new())
        .unwrap_err();
    assert!(
        matches!(error, VfsError::ArchiveEncrypted { .. }),
        "{error}"
    );
}

/// A base whose readers cancel a token after a number of bytes.
struct CancelAfter {
    inner: Arc<dyn Vfs>,
    token: CancelToken,
    bytes: u64,
}

struct CancellingReader {
    inner: Box<dyn hikasa_vfs::ReadSeek>,
    token: CancelToken,
    left: u64,
}

impl Read for CancellingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.left = self.left.saturating_sub(n as u64);
        if self.left == 0 {
            self.token.cancel();
        }
        Ok(n)
    }
}

impl std::io::Seek for CancellingReader {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl Vfs for CancelAfter {
    fn capabilities(&self, at: &Spec) -> Result<hikasa_vfs::Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(
        &self,
        spec: &Spec,
        follow: hikasa_vfs::FollowLinks,
    ) -> Result<hikasa_vfs::Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<hikasa_vfs::Listing, VfsError> {
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn hikasa_vfs::ReadSeek>, VfsError> {
        Ok(Box::new(CancellingReader {
            inner: self.inner.open_read(spec)?,
            token: self.token.clone(),
            left: self.bytes,
        }))
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &hikasa_vfs::WriteOptions,
    ) -> Result<Box<dyn hikasa_vfs::AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }
    fn child(&self, dir: &Spec, name: &hikasa_vfs::FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }
}

/// Cancel mid-stream leaves nothing.
#[test]
fn a6_13_cancel_mid_stream_leaves_nothing() {
    let keep = tempfile::tempdir().unwrap();
    let root = keep.path().canonicalize().unwrap();
    let path = root.join("a.tar");
    let member = vec![5u8; 8 * MIB];
    fs::write(
        &path,
        common::archives::raw_tar(&[RawTar::file(b"big.bin", &member)]),
    )
    .unwrap();
    let file = common::archives::local(&path);
    let reading = CancelToken::new();
    let base = Arc::new(CancelAfter {
        inner: Arc::new(LocalVfs::default()),
        token: reading.clone(),
        bytes: (3 * MIB) as u64,
    });
    let vfs = ArchiveVfs::new(
        base as Arc<dyn Vfs>,
        config(&root.join("cache"), ArchiveLimits::default()),
    );
    let spec = Spec::archive_member(file, "big.bin");
    let error = vfs.extract_to_cache(&spec, &reading).unwrap_err();
    assert!(error.is_cancelled(), "{error}");
    let session = vfs.cache_dir().unwrap();
    assert_eq!(fs::read_dir(&session).unwrap().count(), 0);
    assert_eq!(vfs.cache_used(), 0);
}
