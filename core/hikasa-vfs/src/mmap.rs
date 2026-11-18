//! Memory-mapped visits of local regular files (ARCH-081, §1.2.6, §2.20; M2 W12a, D-325).
//!
//! This module is the only `unsafe` in Hikasa's library and program code (`lib.rs` allows `unsafe_code` on
//! `mod mmap;` alone). Its surface is crate-private and safe: one entry, [`visit_files_mapped`], which the chunk
//! visitor (`chunks.rs`) calls for the binary comparison, the CRC and every bulk pass of the hex engine. A mapping
//! covers one chunk (8 MiB in production) and lives only while the visitor's function reads that chunk: the
//! function borrows the mapped slices for one call and cannot keep them. Every file is re-stat'ed after the
//! visit; a changed length or modification time discards the mapped result (`Ok(None)`), and the caller then
//! streams (the reference implementation).
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
//! `std::thread::scope`, which end before the call returns (no global state, no pool). The range is cut into
//! `chunk`-sized pieces that the threads claim in increasing order from a shared counter; every thread checks
//! the cancellation token before each claim, then maps, pre-faults, visits and unmaps the chunk it claimed
//! (so page-table set-up and tear-down run in parallel as well). The results do not depend on the thread count
//! (T2): the values come back in offset order, and the lowest-offset `Break` wins (a chunk below the lowest
//! `Break` found so far is always visited; chunks above it are not claimed, and values of chunks above it that
//! were visited meanwhile are discarded). Only the calling thread reports progress, so `done` never decreases.

use std::fs::{self, File};
use std::io;
use std::ops::{ControlFlow, Range};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread;
use std::time::SystemTime;

use hikasa_core::{Cancelled, Ctl, Phase, Progress, Unit};

/// The function of a visit: `f(offset, chunks)` gets the chunk's absolute offset and one slice per file, all of
/// the chunk's length.
pub(crate) type ChunkFn<'f, T> = dyn Fn(u64, &[&[u8]]) -> ControlFlow<T, T> + Sync + 'f;

/// What one mapped visit covers.
#[derive(Debug, Clone)]
pub(crate) struct MapRequest {
    /// The byte range of every file.
    pub(crate) range: Range<u64>,
    /// Every file must be exactly `range.end` bytes long (the binary comparison and the CRC: bytes past the
    /// range would change their answer); otherwise at least `range.end`.
    pub(crate) exact: bool,
    /// Chunk length (8 MiB in production; tests use small chunks).
    pub(crate) chunk: usize,
    /// Threads (≥ 1; module documentation).
    pub(crate) threads: usize,
    /// Phase of the progress reports.
    pub(crate) phase: Phase,
}

/// The result of a mapped visit: the per-chunk values in offset order, up to and including the first `Break`.
#[derive(Debug)]
pub(crate) struct MappedVisit<T> {
    /// Values in offset order.
    pub(crate) values: Vec<T>,
    /// The last value is a `Break`.
    pub(crate) broke: bool,
    /// Bytes visited (helpers may have visited chunks above a `Break`; for progress only).
    pub(crate) processed: u64,
}

/// Visits `req.range` of the local regular files `paths` chunk by chunk (D-325). `Ok(None)` when a file cannot be
/// mapped (not a regular file, empty, shorter than the range or, with `req.exact`, of another length), when a
/// chunk below the first `Break` cannot be mapped or pre-faulted, **or when a re-stat after the visit shows that
/// the length or the modification time of a file changed**; the caller then streams (§2.20's rule).
///
/// Cancellation is checked once per chunk and reported as an `io::Error` that wraps [`Cancelled`]
/// (`hikasa_core::io_error_kind` maps it to `cancelled`). `before_restat` runs after every mapping is dropped and
/// before the re-stat (the re-stat tests change a file there).
pub(crate) fn visit_files_mapped<T: Send>(
    paths: &[&Path],
    req: &MapRequest,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
    before_restat: &dyn Fn(),
) -> io::Result<Option<MappedVisit<T>>> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(file) = Opened::regular(path)? else {
            return Ok(None);
        };
        let fits = if req.exact {
            file.stamp.len == req.range.end
        } else {
            file.stamp.len >= req.range.end
        };
        if !fits {
            // The file changed since the caller took its length (or is shorter than the range): streaming decides.
            return Ok(None);
        }
        files.push(file);
    }
    // Every chunk mapping is dropped by the thread that made it, before `visit_opened` returns.
    let Some(visit) = visit_opened(&files, req, ctl, f)? else {
        return Ok(None);
    };
    before_restat();
    if files
        .iter()
        .zip(paths)
        .any(|(file, path)| file.changed(path))
    {
        return Ok(None);
    }
    Ok(Some(visit))
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
        let file = match File::open(path) {
            Ok(file) => file,
            // Windows cannot open a folder as a file ("access denied"): a folder is still "not a regular file".
            Err(_) if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) => return Ok(None),
            Err(e) => return Err(e),
        };
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() == 0 {
            return Ok(None);
        }
        Ok(Some(Opened {
            stamp: Stamp::of(&meta),
            file,
        }))
    }

    /// A read-only mapping of the bytes `start..end` (`start < end`), or `None` when the mapping is refused.
    fn map(&self, start: u64, end: u64) -> Option<memmap2::Mmap> {
        let len = end
            .checked_sub(start)
            .filter(|&len| len > 0)
            .and_then(|len| usize::try_from(len).ok())?;
        let map = map_read_only(&self.file, start, len).ok()?;
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
    // local *regular* files that `metadata()` confirms after opening; (3) the bytes are only read, in bounded
    // chunks, by the visitor's function, which borrows them for one call and cannot keep them, so changed
    // bytes can make the *result* stale but cannot break memory safety elsewhere; (4) a mapping covers one
    // chunk and lives only while that chunk is visited, inside one call; (5) after the call the file is
    // re-stat'ed, and a changed length or mtime discards the mapped result in favour of streaming; (6) on Linux
    // each chunk is pre-faulted with MADV_POPULATE_READ before it is read, which fails instead of raising SIGBUS
    // when a truncation has cut the chunk off, and the attempt is then abandoned; (7) helper threads are scoped
    // (`thread::scope`): each one maps, reads and drops its own chunk mappings, and all of them end before the
    // call returns.
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

/// Shared state of the threads of one mapped call. Offsets are relative to the start of the visited range.
struct Work<'c> {
    /// Bytes to visit.
    len: usize,
    /// Chunk length (≥ 1).
    chunk: usize,
    ctl: &'c Ctl<'c>,
    /// Phase of the progress reports.
    phase: Phase,
    /// Index of the next unclaimed chunk.
    next: AtomicUsize,
    /// Bytes visited so far (progress).
    done: AtomicU64,
    /// Start of the lowest chunk whose value was a `Break` (`usize::MAX`: none). Chunks above it are not visited.
    first_break: AtomicUsize,
    /// Lowest start of a chunk that could not be mapped or pre-faulted (`usize::MAX`: none).
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
            first_break: AtomicUsize::new(usize::MAX),
            abandoned_at: AtomicUsize::new(usize::MAX),
            cancelled: AtomicBool::new(false),
        }
    }

    /// Threads to use: at most one per chunk, at least one.
    fn threads(&self, requested: usize) -> usize {
        requested.min(self.len.div_ceil(self.chunk)).max(1)
    }

    /// Claims the next chunk `(index, start, end)`; `None` when every chunk is claimed, a `Break` below it was
    /// found, a chunk could not be mapped or pre-faulted, or the operation was cancelled.
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
        if start >= self.len || start > self.first_break.load(Ordering::Relaxed) {
            return None;
        }
        Some((index, start, start.saturating_add(self.chunk).min(self.len)))
    }

    /// Records that the chunk at `start` could not be mapped or pre-faulted: no thread claims another chunk.
    fn abandon(&self, start: usize) {
        self.abandoned_at.fetch_min(start, Ordering::Relaxed);
    }

    /// Records a `Break` of the chunk at `start`: no thread claims a chunk above it.
    fn breaks_at(&self, start: usize) {
        self.first_break.fetch_min(start, Ordering::Relaxed);
    }

    /// Records `bytes` as visited; the reporting thread forwards the total (lazily, throttled).
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

    /// The final state: `Err` when cancelled, `Ok(false)` when a chunk below the first `Break` could not be
    /// mapped or pre-faulted (the attempt is abandoned), `Ok(true)` otherwise.
    fn finish(&self) -> io::Result<bool> {
        if self.cancelled.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let abandoned_at = self.abandoned_at.load(Ordering::Relaxed);
        Ok(abandoned_at == usize::MAX || abandoned_at > self.first_break.load(Ordering::Relaxed))
    }
}

/// One thread's visited chunks: `(chunk index, value)`.
type Visited<T> = Vec<(usize, ControlFlow<T, T>)>;

/// Visits `req.range` of opened files (each at least `req.range.end` long) chunk by chunk on `req.threads`
/// threads; `None` when a chunk below the first `Break` could not be mapped or populated.
fn visit_opened<T: Send>(
    files: &[Opened],
    req: &MapRequest,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
) -> io::Result<Option<MappedVisit<T>>> {
    let base = req.range.start;
    let Some(len) = req
        .range
        .end
        .checked_sub(base)
        .and_then(|len| usize::try_from(len).ok())
    else {
        return Ok(None);
    };
    let work = Work::new(len, req.chunk, ctl, req.phase);
    let visited = work.run(req.threads, |reporter| {
        let mut populate = Populate::new();
        let mut visited: Visited<T> = Vec::new();
        while let Some((index, start, end)) = work.claim() {
            let (from, to) = (base + start as u64, base + end as u64);
            let maps: Option<Vec<memmap2::Mmap>> = files
                .iter()
                .map(|file| file.map(from, to).filter(|map| populate.chunk(map)))
                .collect();
            let Some(maps) = maps else {
                work.abandon(start);
                break;
            };
            let slices: Vec<&[u8]> = maps.iter().map(|map| &map[..]).collect();
            let flow = f(from, &slices);
            let broke = flow.is_break();
            visited.push((index, flow));
            work.processed(end - start, reporter);
            if broke {
                work.breaks_at(start);
                break;
            }
            // The chunk mappings are dropped here.
        }
        visited
    });
    if !work.finish()? {
        return Ok(None);
    }
    let mut visited: Visited<T> = visited.into_iter().flatten().collect();
    visited.sort_unstable_by_key(|(index, _)| *index);
    let mut values = Vec::with_capacity(visited.len());
    let mut broke = false;
    for (expected, (index, flow)) in visited.into_iter().enumerate() {
        if index != expected {
            // Defensive: every chunk below the first `Break` must have been visited.
            return Ok(None);
        }
        match flow {
            ControlFlow::Continue(value) => values.push(value),
            ControlFlow::Break(value) => {
                values.push(value);
                broke = true;
                break;
            }
        }
    }
    if !broke && values.len() != len.div_ceil(work.chunk) {
        // Defensive: without a `Break` every chunk must have been visited.
        return Ok(None);
    }
    Ok(Some(MappedVisit {
        values,
        broke,
        processed: work.done.load(Ordering::Relaxed),
    }))
}

// ---- the comparison and the CRC as mapped visits (the unit tests' entries, §2.20) ----------------------------

#[cfg(test)]
use crate::compare::BinaryOutcome;
#[cfg(test)]
use crate::stream::{first_mismatch, report_completion};

/// Compares two local regular files by mapping them (`Ok(None)` as [`visit_files_mapped`]; also when their
/// lengths differ). The outcome does not depend on `threads`.
#[cfg(test)]
fn compare_files_mapped(
    a: &Path,
    b: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<BinaryOutcome>> {
    compare_files_mapped_with(a, b, chunk, threads, ctl, &|| {})
}

/// [`compare_files_mapped`] with the re-stat hook.
#[cfg(test)]
fn compare_files_mapped_with(
    a: &Path,
    b: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
    before_restat: &dyn Fn(),
) -> io::Result<Option<BinaryOutcome>> {
    let (Some(len_a), Some(len_b)) = (
        Opened::regular(a)?.map(|f| f.stamp.len),
        Opened::regular(b)?.map(|f| f.stamp.len),
    ) else {
        return Ok(None);
    };
    if len_a != len_b {
        return Ok(None);
    }
    let req = MapRequest {
        range: 0..len_a,
        exact: true,
        chunk,
        threads,
        phase: Phase::Reading,
    };
    let visit = visit_files_mapped(&[a, b], &req, ctl, &mismatch, before_restat)?;
    Ok(visit.map(|visit| compared(ctl, &visit, len_a)))
}

/// The comparison's chunk function: the first differing offset is a `Break`.
#[cfg(test)]
fn mismatch(offset: u64, chunks: &[&[u8]]) -> ControlFlow<Option<u64>, Option<u64>> {
    match first_mismatch(chunks[0], chunks[1]) {
        Some(i) => ControlFlow::Break(Some(offset + i as u64)),
        None => ControlFlow::Continue(None),
    }
}

/// The outcome of a comparison visit, with its completion report.
#[cfg(test)]
fn compared(ctl: &Ctl<'_>, visit: &MappedVisit<Option<u64>>, len: u64) -> BinaryOutcome {
    match visit
        .values
        .last()
        .copied()
        .flatten()
        .filter(|_| visit.broke)
    {
        Some(at) => {
            report_completion(ctl, Phase::Reading, at.max(visit.processed));
            BinaryOutcome::Different {
                first_difference: Some(at),
            }
        }
        None => {
            report_completion(ctl, Phase::Reading, len);
            BinaryOutcome::Same
        }
    }
}

/// CRC32 of a mapped local regular file (`Ok(None)` as above); the CRC does not depend on `threads`.
#[cfg(test)]
fn crc32_file_mapped(
    path: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<u32>> {
    crc32_file_mapped_with(path, chunk, threads, ctl, &|| {})
}

/// [`crc32_file_mapped`] with the re-stat hook.
#[cfg(test)]
fn crc32_file_mapped_with(
    path: &Path,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
    before_restat: &dyn Fn(),
) -> io::Result<Option<u32>> {
    let Some(len) = Opened::regular(path)?.map(|f| f.stamp.len) else {
        return Ok(None);
    };
    let req = MapRequest {
        range: 0..len,
        exact: true,
        chunk,
        threads,
        phase: Phase::Hashing,
    };
    let visit = visit_files_mapped(&[path], &req, ctl, &chunk_crc, before_restat)?;
    Ok(visit.map(|visit| {
        report_completion(ctl, Phase::Hashing, len);
        combine(&visit.values)
    }))
}

/// The CRC's chunk function: `(crc, length)` of the chunk.
#[cfg(test)]
fn chunk_crc(_offset: u64, chunks: &[&[u8]]) -> ControlFlow<(u32, u64), (u32, u64)> {
    ControlFlow::Continue((crc32fast::hash(chunks[0]), chunks[0].len() as u64))
}

/// Chunk CRCs combined in offset order.
#[cfg(test)]
fn combine(values: &[(u32, u64)]) -> u32 {
    let mut total = crc32fast::Hasher::new();
    for &(crc, len) in values {
        total.combine(&crc32fast::Hasher::new_with_initial_len(crc, len));
    }
    total.finalize()
}

/// [`compare_files_mapped`] on opened files (the Linux truncation test opens them first).
#[cfg(all(test, target_os = "linux"))]
fn compare_opened(
    fa: &Opened,
    fb: &Opened,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<BinaryOutcome>> {
    let len = fa.stamp.len;
    let req = MapRequest {
        range: 0..len,
        exact: true,
        chunk,
        threads,
        phase: Phase::Reading,
    };
    let files = [
        Opened {
            file: fa.file.try_clone()?,
            stamp: fa.stamp,
        },
        Opened {
            file: fb.file.try_clone()?,
            stamp: fb.stamp,
        },
    ];
    let visit = visit_opened(&files, &req, ctl, &mismatch)?;
    Ok(visit.map(|visit| compared(ctl, &visit, len)))
}

/// [`crc32_file_mapped`] on an opened file.
#[cfg(all(test, target_os = "linux"))]
fn crc32_opened(
    file: &Opened,
    chunk: usize,
    threads: usize,
    ctl: &Ctl<'_>,
) -> io::Result<Option<u32>> {
    let req = MapRequest {
        range: 0..file.stamp.len,
        exact: true,
        chunk,
        threads,
        phase: Phase::Hashing,
    };
    let files = [Opened {
        file: file.file.try_clone()?,
        stamp: file.stamp,
    }];
    let visit = visit_opened(&files, &req, ctl, &chunk_crc)?;
    Ok(visit.map(|visit| combine(&visit.values)))
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
