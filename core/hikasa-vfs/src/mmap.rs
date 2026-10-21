//! Memory-mapped comparison and CRC of local regular files (ARCH-081, §1.2.6, §2.20).
//!
//! This module is the only `unsafe` in Hikasa's library and program code (`lib.rs` allows `unsafe_code` on
//! `mod mmap;` alone). Its surface is crate-private and safe: mapped slices never leave it, a mapping covers one
//! chunk (8 MiB in production) and lives only while that chunk is compared or hashed, and both files are
//! re-stat'ed after the call; a changed length or modification time discards the mapped result (`Ok(None)`),
//! and the caller then streams (the reference implementation).
//!
//! On Linux every chunk is pre-faulted with `MADV_POPULATE_READ` (kernel 5.14+) before it is read. Populating a
//! range that a concurrent truncation has cut off fails with an error instead of raising `SIGBUS`, and the
//! chunk is then never touched: the mapped attempt is abandoned (`Ok(None)`) and the caller streams. This
//! narrows the residual risk of a truncation to the moment between populating a chunk and reading it. A kernel
//! without the advice (older than 5.14: `EINVAL`) falls back to demand faults, with the residual risk of other
//! systems.
//!
//! # Threads (§5.1)
//!
//! One mapped call may use `threads` threads: the calling thread plus `threads - 1` helper threads started with
//! `std::thread::scope`, which end before the call returns (no global state, no pool). The file is cut into
//! `chunk`-sized pieces that the threads claim in increasing order from a shared counter; every thread checks
//! the cancellation token before each claim, then maps, pre-faults, processes and unmaps the chunk it claimed
//! (so page-table set-up and tear-down run in parallel as well). The results do not depend on the thread count
//! (T2): a CRC combines the chunk CRCs in offset order with `crc32fast::Hasher::combine` (exact), and a
//! comparison reports the lowest differing offset (a chunk below the lowest difference found so far is always
//! processed completely). Only the calling thread reports progress, so `done` never decreases.

use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::SystemTime;

use hikasa_core::{Cancelled, Ctl, Phase, Progress, Unit};

use crate::compare::BinaryOutcome;
use crate::stream::{first_mismatch, report_completion};

/// Compares two local regular files by mapping them. Returns `Ok(None)` when either cannot be mapped
/// (empty file, not a regular file, mapping or pre-faulting refused) **or when a re-stat after the comparison
/// shows that the length or the modification time of either file changed**; the caller then streams (§2.20).
///
/// Cancellation is checked once per `chunk` bytes and reported as an `io::Error` that wraps [`Cancelled`]
/// (`hikasa_core::io_error_kind` maps it to `cancelled`).
///
/// `threads` (≥ 1) threads share the work (module documentation); the outcome does not depend on it.
pub(crate) fn compare_files_mapped(
    a: &Path,
    b: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<BinaryOutcome>> {
    compare_files_mapped_with(a, b, chunk, threads, ctl, &|| {})
}

/// [`compare_files_mapped`] with a hook that runs after every mapping is dropped and before the re-stat (the
/// re-stat test changes a file there).
pub(crate) fn compare_files_mapped_with(
    a: &Path,
    b: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
    before_restat: &dyn Fn(),
) -> io::Result<Option<BinaryOutcome>> {
    let Some(fa) = Opened::regular(a)? else {
        return Ok(None);
    };
    let Some(fb) = Opened::regular(b)? else {
        return Ok(None);
    };
    if fa.stamp.len != fb.stamp.len {
        // The files changed since the caller compared their lengths: let the streaming path decide.
        return Ok(None);
    }
    // Every chunk mapping is dropped by the thread that made it, before `compare_opened` returns.
    let Some(outcome) = compare_opened(&fa, &fb, chunk, threads, ctl)? else {
        return Ok(None);
    };
    before_restat();
    if fa.changed(a) || fb.changed(b) {
        return Ok(None);
    }
    Ok(Some(outcome))
}

/// CRC32 of a mapped local regular file (`Ok(None)` as above, including the re-stat check), on `threads`
/// threads (the CRC does not depend on it).
pub(crate) fn crc32_file_mapped(
    path: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<u32>> {
    crc32_file_mapped_with(path, chunk, threads, ctl, &|| {})
}

/// [`crc32_file_mapped`] with the re-stat test hook.
pub(crate) fn crc32_file_mapped_with(
    path: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
    before_restat: &dyn Fn(),
) -> io::Result<Option<u32>> {
    let Some(file) = Opened::regular(path)? else {
        return Ok(None);
    };
    let Some(crc) = crc32_opened(&file, chunk, threads, ctl)? else {
        return Ok(None);
    };
    before_restat();
    if file.changed(path) {
        return Ok(None);
    }
    Ok(Some(crc))
}

/// Length and modification time of a file (the re-stat key).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    mtime: Option<SystemTime>,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Stamp {
        Stamp {
            len: meta.len(),
            mtime: meta.modified().ok(),
        }
    }
}

/// An open, non-empty local regular file and its stamp at opening time.
struct Opened {
    file: File,
    stamp: Stamp,
}

impl Opened {
    /// Opens `path`; `None` when it is not a regular file or is empty (nothing to map).
    fn regular(path: &Path) -> io::Result<Option<Opened>> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() == 0 {
            return Ok(None);
        }
        Ok(Some(Opened {
            stamp: Stamp::of(&meta),
            file,
        }))
    }

    /// The length at opening time, the bytes a mapped call covers (`None` when it does not fit in `usize`).
    fn len(&self) -> Option<usize> {
        usize::try_from(self.stamp.len).ok()
    }

    /// A read-only mapping of the bytes `start..end` (`start < end`), or `None` when the mapping is refused.
    fn map(&self, start: usize, end: usize) -> Option<memmap2::Mmap> {
        let len = end.checked_sub(start).filter(|&len| len > 0)?;
        let map = map_read_only(&self.file, start as u64, len).ok()?;
        if map.len() != len {
            return None;
        }
        #[cfg(unix)]
        {
            // Advice only tunes read-ahead; a refusal changes nothing.
            let _ = map.advise(memmap2::Advice::Sequential);
        }
        Some(map)
    }

    /// True when the file at `path` no longer has the stamp it had when it was opened (or cannot be stat'ed).
    fn changed(&self, path: &Path) -> bool {
        match fs::metadata(path) {
            Ok(meta) => Stamp::of(&meta) != self.stamp,
            Err(_) => true,
        }
    }
}

fn map_read_only(file: &File, offset: u64, len: usize) -> io::Result<memmap2::Mmap> {
    // SAFETY: `memmap2::MmapOptions::map` is unsafe because another process may modify or truncate the file
    // while it is mapped. A modification can change the slice's bytes; a truncation can raise SIGBUS.
    // Mitigations: (1) the map is read-only and private to this module; (2) it is created only for
    // local *regular* files that `metadata()` confirms after opening; (3) the bytes are only compared
    // or hashed in bounded chunks, so changed bytes can make the *result* stale but cannot break memory
    // safety elsewhere; (4) a mapping covers one chunk and lives only while that chunk is processed, inside
    // one call; (5) after the call the file is re-stat'ed, and a changed length or mtime discards the mapped
    // result in favour of streaming; (6) on Linux each chunk is pre-faulted with MADV_POPULATE_READ before it
    // is read, which fails instead of raising SIGBUS when a truncation has cut the chunk off, and the attempt
    // is then abandoned; (7) helper threads are scoped (`thread::scope`): each one maps, reads and drops its
    // own chunk mappings, and all of them end before the call returns.
    // Residual risk (accepted, documented in docs/11 §3): a truncation between populating a chunk and
    // reading it (or any truncation on other systems, or on Linux before 5.14) can terminate the process.
    // MmapPolicy::Never disables this path.
    unsafe {
        memmap2::MmapOptions::new()
            .offset(offset)
            .len(len)
            .map(file)
    }
}

fn cancelled() -> io::Error {
    io::Error::other(Cancelled)
}

fn progress(phase: Phase, done: u64, total: usize) -> Progress {
    Progress {
        phase,
        done,
        total: Some(total as u64),
        unit: Unit::Bytes,
        current: None,
    }
}

/// Pre-faults chunk mappings before they are read (Linux `MADV_POPULATE_READ`; a no-op elsewhere).
struct Populate {
    /// False once the kernel said it does not know the advice (older than 5.14).
    #[cfg(target_os = "linux")]
    enabled: bool,
}

impl Populate {
    fn new() -> Populate {
        Populate {
            #[cfg(target_os = "linux")]
            enabled: true,
        }
    }

    /// Pre-faults the whole mapping. `false` means it must not be read (populating failed, for example
    /// because the file was truncated): the mapped attempt is abandoned.
    #[cfg(target_os = "linux")]
    fn chunk(&mut self, map: &memmap2::Mmap) -> bool {
        if !self.enabled {
            return true;
        }
        match map.advise(memmap2::Advice::PopulateRead) {
            Ok(()) => true,
            // A kernel without MADV_POPULATE_READ: demand faults from now on.
            Err(e) if e.kind() == io::ErrorKind::InvalidInput => {
                self.enabled = false;
                true
            }
            // A signal interrupted populating; demand faults read the chunk.
            Err(e) if e.kind() == io::ErrorKind::Interrupted => true,
            Err(_) => false,
        }
    }

    /// No pre-faulting outside Linux: every chunk may be read.
    #[cfg(not(target_os = "linux"))]
    fn chunk(&mut self, _map: &memmap2::Mmap) -> bool {
        true
    }
}

/// Shared state of the threads of one mapped call.
struct Work<'c> {
    /// Bytes to process.
    len: usize,
    /// Chunk length (≥ 1).
    chunk: usize,
    ctl: &'c Ctl<'c>,
    /// Phase of the progress reports.
    phase: Phase,
    /// Index of the next unclaimed chunk.
    next: AtomicUsize,
    /// Bytes processed so far (progress).
    done: AtomicU64,
    /// Lowest differing offset found so far (`usize::MAX`: none). Chunks at or above it are not processed.
    first_difference: AtomicUsize,
    /// Lowest offset of a chunk that could not be mapped or pre-faulted (`usize::MAX`: none).
    abandoned_at: AtomicUsize,
    /// Some thread observed cancellation.
    cancelled: AtomicBool,
}

impl<'c> Work<'c> {
    fn new(len: usize, chunk: usize, ctl: &'c Ctl<'c>, phase: Phase) -> Work<'c> {
        Work {
            len,
            chunk: chunk.max(1),
            ctl,
            phase,
            next: AtomicUsize::new(0),
            done: AtomicU64::new(0),
            first_difference: AtomicUsize::new(usize::MAX),
            abandoned_at: AtomicUsize::new(usize::MAX),
            cancelled: AtomicBool::new(false),
        }
    }

    /// Threads to use: at most one per chunk, at least one.
    fn threads(&self, requested: usize) -> usize {
        requested.min(self.len.div_ceil(self.chunk)).max(1)
    }

    /// Claims the next chunk `(index, start, end)`; `None` when every chunk is claimed, a difference below it
    /// was found, a chunk could not be mapped or pre-faulted, or the operation was cancelled.
    fn claim(&self) -> Option<(usize, usize, usize)> {
        if self.cancelled.load(Ordering::Relaxed)
            || self.abandoned_at.load(Ordering::Relaxed) != usize::MAX
        {
            return None;
        }
        if self.ctl.check().is_err() {
            self.cancelled.store(true, Ordering::Relaxed);
            return None;
        }
        let index = self.next.fetch_add(1, Ordering::Relaxed);
        let start = index.checked_mul(self.chunk)?;
        if start >= self.len || start >= self.first_difference.load(Ordering::Relaxed) {
            return None;
        }
        Some((index, start, start.saturating_add(self.chunk).min(self.len)))
    }

    /// Records that the chunk at `start` could not be mapped or pre-faulted: no thread claims another chunk.
    fn abandon(&self, start: usize) {
        self.abandoned_at.fetch_min(start, Ordering::Relaxed);
    }

    /// Records a difference at `offset`: no thread claims a chunk at or above it.
    fn differs_at(&self, offset: usize) {
        self.first_difference.fetch_min(offset, Ordering::Relaxed);
    }

    /// Records `bytes` as processed; the reporting thread forwards the total (lazily, throttled).
    fn processed(&self, bytes: usize, reporter: bool) {
        let done = self.done.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
        if reporter {
            self.ctl
                .progress(self.phase, || progress(self.phase, done, self.len));
        }
    }

    /// Runs `worker(reporter)` on the calling thread (`reporter = true`) and on up to `threads - 1` scoped
    /// helper threads. A helper that cannot be started is skipped: the other threads claim its chunks.
    fn run<T: Send>(&self, threads: usize, worker: impl Fn(bool) -> T + Sync) -> Vec<T> {
        let threads = self.threads(threads);
        if threads == 1 {
            return vec![worker(true)];
        }
        let mut results: Vec<Option<T>> = (0..threads).map(|_| None).collect();
        thread::scope(|scope| {
            let mut slots = results.iter_mut();
            let own = slots.next();
            for slot in slots {
                let worker = &worker;
                // A refused thread (`Err`) leaves its slot empty; its chunks are claimed by the others.
                let _ = thread::Builder::new()
                    .name("hikasa-vfs-map".into())
                    .spawn_scoped(scope, move || *slot = Some(worker(false)));
            }
            if let Some(own) = own {
                *own = Some(worker(true));
            }
        });
        results.into_iter().flatten().collect()
    }

    /// The final state: `Err` when cancelled, `Ok(false)` when a chunk below the first difference could not be
    /// mapped or pre-faulted (the attempt is abandoned), `Ok(true)` otherwise.
    fn finish(&self) -> io::Result<bool> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let abandoned_at = self.abandoned_at.load(Ordering::Relaxed);
        Ok(abandoned_at == usize::MAX
            || abandoned_at > self.first_difference.load(Ordering::Relaxed))
    }
}

/// Compares two opened files of equal length chunk by chunk on `threads` threads; `None` when a chunk below the
/// first difference could not be mapped or populated.
fn compare_opened(
    fa: &Opened,
    fb: &Opened,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<BinaryOutcome>> {
    let Some(len) = fa.len() else {
        return Ok(None);
    };
    let work = Work::new(len, chunk, ctl, Phase::Reading);
    let compared = work.run(threads, |reporter| {
        let mut populate = Populate::new();
        let mut compared = 0usize;
        while let Some((_, start, end)) = work.claim() {
            let maps = fa.map(start, end).zip(fb.map(start, end));
            let Some((ma, mb)) = maps.filter(|(ma, mb)| populate.chunk(ma) && populate.chunk(mb))
            else {
                work.abandon(start);
                break;
            };
            if let Some(i) = first_mismatch(&ma, &mb) {
                work.differs_at(start + i);
                break;
            }
            compared += end - start;
            work.processed(end - start, reporter);
            // Both chunk mappings are dropped here.
        }
        compared
    });
    if !work.finish()? {
        return Ok(None);
    }
    let decided_at = match work.first_difference.load(Ordering::Relaxed) {
        // Defensive: without a difference every chunk must have been compared.
        usize::MAX if compared.iter().sum::<usize>() != len => return Ok(None),
        usize::MAX => {
            report_completion(ctl, Phase::Reading, len as u64);
            return Ok(Some(BinaryOutcome::Same));
        }
        offset => offset as u64,
    };
    // Helpers may have compared chunks above the difference (and the calling thread may have reported them):
    // the completion never goes below what was reported.
    let processed = work.done.load(Ordering::Relaxed);
    report_completion(ctl, Phase::Reading, decided_at.max(processed));
    Ok(Some(BinaryOutcome::Different {
        first_difference: Some(decided_at),
    }))
}

/// One thread's CRC of consecutive chunks: `(index of the first chunk, hasher, bytes hashed)`.
type Run = (usize, crc32fast::Hasher, usize);

/// CRC32 of an opened file, chunk by chunk on `threads` threads; `None` when a chunk could not be mapped or
/// populated.
fn crc32_opened(
    file: &Opened,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<u32>> {
    let Some(len) = file.len() else {
        return Ok(None);
    };
    let work = Work::new(len, chunk, ctl, Phase::Hashing);
    // Consecutive chunks claimed by one thread share a hasher (one run for the whole file on one thread).
    let runs = work.run(threads, |reporter| {
        let mut populate = Populate::new();
        let mut runs: Vec<Run> = Vec::new();
        let mut last: Option<usize> = None;
        while let Some((index, start, end)) = work.claim() {
            let Some(map) = file.map(start, end).filter(|map| populate.chunk(map)) else {
                work.abandon(start);
                break;
            };
            match runs.last_mut() {
                Some((_, hasher, hashed)) if last.is_some_and(|l| l + 1 == index) => {
                    hasher.update(&map);
                    *hashed += map.len();
                }
                _ => {
                    let mut hasher = crc32fast::Hasher::new();
                    hasher.update(&map);
                    runs.push((index, hasher, map.len()));
                }
            }
            last = Some(index);
            work.processed(end - start, reporter);
        }
        runs
    });
    if !work.finish()? {
        return Ok(None);
    }
    let mut runs: Vec<Run> = runs.into_iter().flatten().collect();
    runs.sort_unstable_by_key(|(index, _, _)| *index);
    if runs.iter().map(|(_, _, hashed)| hashed).sum::<usize>() != len {
        // Defensive: the runs must cover the file exactly.
        return Ok(None);
    }
    let mut total = crc32fast::Hasher::new();
    for (_, hasher, _) in &runs {
        total.combine(hasher);
    }
    report_completion(ctl, Phase::Hashing, len as u64);
    Ok(Some(total.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::{CancelToken, ErrorKind, NullProgress, ProgressSink, io_error_kind};
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;
    use std::io::Write;
    use std::sync::Mutex;

    /// The largest thread count the tests use.
    const MAX_TEST_THREADS: usize = 8;

    fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, data).unwrap();
        path
    }

    /// The naive whole-buffer comparison (the oracle of INV-VFS-2).
    fn naive(a: &[u8], b: &[u8]) -> BinaryOutcome {
        match a.iter().zip(b).position(|(x, y)| x != y) {
            Some(i) => BinaryOutcome::Different {
                first_difference: Some(i as u64),
            },
            None if a.len() == b.len() => BinaryOutcome::Same,
            None => BinaryOutcome::Different {
                first_difference: Some(a.len().min(b.len()) as u64),
            },
        }
    }

    #[test]
    fn mapped_compare_and_crc() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let mut other = data.clone();
        other[77_777] ^= 1;
        let a = write(dir.path(), "a", &data);
        let b = write(dir.path(), "b", &data);
        let c = write(dir.path(), "c", &other);
        let ctl = Ctl::detached();
        for threads in [1, 4] {
            for chunk in [4096, 10_007, 1 << 20] {
                assert_eq!(
                    compare_files_mapped(&a, &b, chunk, threads, &ctl).unwrap(),
                    Some(BinaryOutcome::Same)
                );
                assert_eq!(
                    compare_files_mapped(&a, &c, chunk, threads, &ctl).unwrap(),
                    Some(BinaryOutcome::Different {
                        first_difference: Some(77_777)
                    })
                );
                assert_eq!(
                    crc32_file_mapped(&a, chunk, threads, &ctl).unwrap(),
                    Some(crc32fast::hash(&data))
                );
            }
        }
        // Chunks smaller than a page (mappings at unaligned offsets), on a smaller file.
        let small: Vec<u8> = data[..3000].to_vec();
        let mut small_other = small.clone();
        small_other[2222] ^= 1;
        let d = write(dir.path(), "d", &small);
        let e = write(dir.path(), "e", &small_other);
        for threads in [1, 4] {
            for chunk in [1, 7] {
                assert_eq!(
                    compare_files_mapped(&d, &e, chunk, threads, &ctl).unwrap(),
                    Some(BinaryOutcome::Different {
                        first_difference: Some(2222)
                    })
                );
                assert_eq!(
                    crc32_file_mapped(&e, chunk, threads, &ctl).unwrap(),
                    Some(crc32fast::hash(&small_other))
                );
            }
        }
    }

    /// T2: every thread count gives the same CRC and the same (lowest) differing offset, also when several
    /// chunks differ and the later ones are found first.
    #[test]
    fn results_do_not_depend_on_the_thread_count() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..300_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let mut other = data.clone();
        for at in [299_999, 250_001, 150_000, 64_123] {
            other[at] ^= 0x80;
        }
        let a = write(dir.path(), "a", &data);
        let c = write(dir.path(), "c", &other);
        let ctl = Ctl::detached();
        for threads in 1..=MAX_TEST_THREADS {
            for chunk in [4096, 10_000, 65_536] {
                assert_eq!(
                    compare_files_mapped(&a, &c, chunk, threads, &ctl).unwrap(),
                    Some(BinaryOutcome::Different {
                        first_difference: Some(64_123)
                    }),
                    "threads {threads}, chunk {chunk}"
                );
                assert_eq!(
                    crc32_file_mapped(&c, chunk, threads, &ctl).unwrap(),
                    Some(crc32fast::hash(&other)),
                    "threads {threads}, chunk {chunk}"
                );
            }
        }
    }

    #[test]
    fn unmappable_inputs_give_none() {
        let dir = tempfile::tempdir().unwrap();
        let empty = write(dir.path(), "empty", b"");
        let one = write(dir.path(), "one", b"x");
        let two = write(dir.path(), "two", b"xy");
        let ctl = Ctl::detached();
        assert_eq!(
            compare_files_mapped(&empty, &empty, 8, 1, &ctl).unwrap(),
            None
        );
        assert_eq!(compare_files_mapped(&one, &two, 8, 4, &ctl).unwrap(), None);
        assert_eq!(
            compare_files_mapped(dir.path(), &one, 8, 1, &ctl).unwrap(),
            None
        );
        assert_eq!(crc32_file_mapped(&empty, 8, 4, &ctl).unwrap(), None);
        assert_eq!(crc32_file_mapped(dir.path(), 8, 1, &ctl).unwrap(), None);
        let missing = dir.path().join("missing");
        assert_eq!(
            compare_files_mapped(&missing, &one, 8, 1, &ctl)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn restat_discards_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a", &[5u8; 10_000]);
        let b = write(dir.path(), "b", &[5u8; 10_000]);
        let ctl = Ctl::detached();
        let grow = || {
            let mut f = fs::OpenOptions::new().append(true).open(&a).unwrap();
            f.write_all(b"!").unwrap();
        };
        assert_eq!(
            compare_files_mapped_with(&a, &b, 4096, 2, &ctl, &grow).unwrap(),
            None
        );
        let touch = || {
            let later = filetime::FileTime::from_unix_time(2_000_000_000, 0);
            filetime::set_file_mtime(&b, later).unwrap();
        };
        assert_eq!(
            crc32_file_mapped_with(&b, 4096, 2, &ctl, &touch).unwrap(),
            None
        );
        // Unchanged files keep the mapped result.
        assert_eq!(
            crc32_file_mapped_with(&b, 4096, 2, &ctl, &|| {}).unwrap(),
            Some(crc32fast::hash(&[5u8; 10_000]))
        );
    }

    /// A file truncated after it was opened, before its chunks are mapped and read: populating fails, so the
    /// mapped attempt is abandoned (`None`, the caller streams) instead of raising SIGBUS — on one thread and on
    /// several.
    #[cfg(target_os = "linux")]
    #[test]
    fn truncation_before_reading_falls_back_without_sigbus() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a", &[3u8; 3 << 20]);
        let b = write(dir.path(), "b", &[3u8; 3 << 20]);
        let ctl = Ctl::detached();
        let fa = Opened::regular(&a).unwrap().unwrap();
        let fb = Opened::regular(&b).unwrap().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&a)
            .unwrap()
            .set_len(0)
            .unwrap();
        for threads in [1, 3] {
            assert_eq!(crc32_opened(&fa, 1 << 20, threads, &ctl).unwrap(), None);
            assert_eq!(
                compare_opened(&fa, &fb, 1 << 20, threads, &ctl).unwrap(),
                None
            );
            // Still intact: the other file maps and hashes normally.
            assert_eq!(
                crc32_opened(&fb, 1 << 20, threads, &ctl).unwrap(),
                Some(crc32fast::hash(&[3u8; 3 << 20]))
            );
        }
    }

    #[test]
    fn cancellation_wraps_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a", &[1u8; 1000]);
        let token = CancelToken::new();
        token.cancel();
        let ctl = Ctl {
            cancel: &token,
            progress: &NullProgress,
        };
        for threads in [1, 4] {
            let err = compare_files_mapped(&a, &a, 100, threads, &ctl).unwrap_err();
            assert_eq!(io_error_kind(&err), ErrorKind::Cancelled);
            let err = crc32_file_mapped(&a, 100, threads, &ctl).unwrap_err();
            assert_eq!(io_error_kind(&err), ErrorKind::Cancelled);
        }
    }

    /// A sink that records reports and cancels `token` after `cancel_after` reports.
    struct Recorder<'a> {
        seen: Mutex<Vec<Progress>>,
        token: &'a CancelToken,
        cancel_after: usize,
    }

    impl ProgressSink for Recorder<'_> {
        fn report(&self, progress: Progress) {
            let mut seen = self.seen.lock().unwrap();
            seen.push(progress);
            if seen.len() == self.cancel_after {
                self.token.cancel();
            }
        }
    }

    /// With helper threads, only the calling thread reports: `done` never decreases, and the last report is the
    /// completion (`done == total`; the length, or for a difference at least its offset).
    #[test]
    fn parallel_progress_is_monotonic_and_completes() {
        let dir = tempfile::tempdir().unwrap();
        let data = vec![9u8; 1 << 20];
        let a = write(dir.path(), "a", &data);
        let b = write(dir.path(), "b", &data);
        let mut early = data.clone();
        early[5] = 0;
        let c = write(dir.path(), "c", &early);
        for threads in [1, 4] {
            let token = CancelToken::new();
            let sink = Recorder {
                seen: Mutex::new(Vec::new()),
                token: &token,
                cancel_after: usize::MAX,
            };
            let ctl = Ctl {
                cancel: &token,
                progress: &sink,
            };
            assert_eq!(
                compare_files_mapped(&a, &c, 4096, threads, &ctl).unwrap(),
                Some(BinaryOutcome::Different {
                    first_difference: Some(5)
                })
            );
            let seen = sink.seen.into_inner().unwrap();
            assert!(seen.windows(2).all(|w| w[0].done <= w[1].done), "{threads}");
            let last = seen.last().unwrap();
            assert_eq!(last.total, Some(last.done));
            assert!(last.done >= 5);
        }
        for threads in [1, 4] {
            let token = CancelToken::new();
            let sink = Recorder {
                seen: Mutex::new(Vec::new()),
                token: &token,
                cancel_after: usize::MAX,
            };
            let ctl = Ctl {
                cancel: &token,
                progress: &sink,
            };
            assert_eq!(
                compare_files_mapped(&a, &b, 4096, threads, &ctl).unwrap(),
                Some(BinaryOutcome::Same)
            );
            assert_eq!(
                crc32_file_mapped(&a, 4096, threads, &ctl).unwrap(),
                Some(crc32fast::hash(&data))
            );
            let seen = sink.seen.into_inner().unwrap();
            for phase in [Phase::Reading, Phase::Hashing] {
                let done: Vec<(u64, Option<u64>)> = seen
                    .iter()
                    .filter(|p| p.phase == phase)
                    .map(|p| (p.done, p.total))
                    .collect();
                assert!(done.windows(2).all(|w| w[0].0 <= w[1].0), "{phase:?}");
                assert_eq!(done.last(), Some(&(1 << 20, Some(1 << 20))), "{phase:?}");
            }
        }
    }

    /// Cancellation observed by any thread stops every thread at its next chunk.
    #[test]
    fn cancellation_during_a_parallel_call() {
        let dir = tempfile::tempdir().unwrap();
        let data = vec![4u8; 4 << 20];
        let a = write(dir.path(), "a", &data);
        let b = write(dir.path(), "b", &data);
        for threads in [1, 4] {
            let token = CancelToken::new();
            let sink = Recorder {
                seen: Mutex::new(Vec::new()),
                token: &token,
                cancel_after: 3,
            };
            let ctl = Ctl {
                cancel: &token,
                progress: &sink,
            };
            let err = compare_files_mapped(&a, &b, 4096, threads, &ctl).unwrap_err();
            assert_eq!(io_error_kind(&err), ErrorKind::Cancelled);
            let token = CancelToken::new();
            let sink = Recorder {
                seen: Mutex::new(Vec::new()),
                token: &token,
                cancel_after: 3,
            };
            let ctl = Ctl {
                cancel: &token,
                progress: &sink,
            };
            let err = crc32_file_mapped(&a, 4096, threads, &ctl).unwrap_err();
            assert_eq!(io_error_kind(&err), ErrorKind::Cancelled);
            // No completion report after cancellation.
            let seen = sink.seen.into_inner().unwrap();
            assert!(seen.iter().all(|p| p.done < 4 << 20), "{threads}");
        }
    }

    /// The proptest configuration of the unit-level properties: the case count of `ProptestConfig::default()`
    /// (256, or `PROPTEST_CASES`, §6.1) and failing seeds persisted next to the integration suites.
    fn config() -> ProptestConfig {
        ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/proptest-regressions/mmap.txt",
            ))),
            ..ProptestConfig::default()
        }
    }

    /// `(a, b)` of equal length: equal, one byte changed, two bytes changed, or unrelated; random bytes or
    /// zeros.
    fn pair(max_len: usize) -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
        (
            any::<u64>(),
            0..=max_len,
            any::<bool>(),
            0usize..4,
            any::<usize>(),
            any::<usize>(),
        )
            .prop_map(|(seed, len, zeros, mode, at, at2)| {
                let mut state = seed | 1;
                let mut next = move || {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                };
                let a: Vec<u8> = (0..len).map(|_| if zeros { 0 } else { next() }).collect();
                let mut b = a.clone();
                match mode {
                    0 => {}
                    1 | 2 if !b.is_empty() => {
                        let i = at % b.len();
                        b[i] ^= 0x5A;
                        if mode == 2 {
                            let j = at2 % b.len();
                            b[j] = b[j].wrapping_add(1);
                        }
                    }
                    _ => b = (0..len).map(|_| next()).collect(),
                }
                (a, b)
            })
    }

    proptest! {
        #![proptest_config(config())]

        /// P-VFS-2 (INV-VFS-2) across many mapped chunks: random chunk sizes (1 byte to 64 KiB, so that a file
        /// spans up to 256 chunks) and thread counts give the naive outcome and `crc32fast::hash`.
        #[test]
        fn p_vfs_2_multi_chunk(
            chunk in prop_oneof![1usize..=64, 65usize..=4096, 4097usize..=65_536],
            (a, b) in pair(300_000),
            threads in 1usize..=MAX_TEST_THREADS,
        ) {
            let len = a.len().min(chunk.saturating_mul(256));
            let (a, b) = (&a[..len], &b[..len]);
            let dir = tempfile::tempdir().unwrap();
            let pa = write(dir.path(), "a", a);
            let pb = write(dir.path(), "b", b);
            let ctl = Ctl::detached();
            let expected = if a.is_empty() { None } else { Some(naive(a, b)) };
            prop_assert_eq!(compare_files_mapped(&pa, &pb, chunk, threads, &ctl).unwrap(), expected);
            let crc = (!a.is_empty()).then(|| crc32fast::hash(a));
            prop_assert_eq!(crc32_file_mapped(&pa, chunk, threads, &ctl).unwrap(), crc);
        }
    }
}
