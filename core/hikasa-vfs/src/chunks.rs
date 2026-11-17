//! The chunk visitor (M2 W12a, D-325; HX-009): [`visit_chunks`], [`visit_chunk_pairs`], [`Visit`], [`VISIT_CHUNK`].
//!
//! One owner for every chunked pass over content: the binary comparison ([`crate::compare_sources`]), the CRC
//! ([`crate::crc32_source`]), and the hex engine's None scan, block chunking and find. The comparison and the CRC
//! visit only in place (mapped, or sources held in memory); a source that must be streamed is compared or hashed
//! by M1's streaming path with `buffer_size` buffers, which stops at the first difference after one buffer a side. A range is cut into
//! chunks of [`VISIT_CHUNK`] bytes starting at `range.start` (the last one shorter): the same cuts mapped,
//! streamed, in memory and with any thread count, so a result never depends on how the bytes were read (T2).
//!
//! ```text
//! visit(sources, range, f):
//!   1. range empty                              → no chunk (the CRC and the comparison still probe the end)
//!   2. every source a local regular file and opts.mmap.maps(range length)
//!        → mmap: chunks claimed by up to `threads` threads, values sorted by offset, lowest Break wins;
//!          a re-stat after the visit that shows a changed (length, mtime) discards it → 3
//!   3. streaming: one sequential reader per source (a source held in memory is read in place); each chunk is
//!      filled completely (reads of `buffer_size`) before `f` runs; up to `threads` chunks are filled, then `f`
//!      runs on them in parallel and their values are taken in order (a bounded window)
//!   a source that ends before range.end → VfsError::Truncated (kind unexpected_eof) naming it, never zeros
//! ```
//!
//! Cancellation is checked before every chunk and between the partial reads of a chunk. Progress is reported in
//! bytes of the range (`reading`, or `hashing` for the CRC), with one completion report at the end.

use std::io::Read;
use std::ops::{ControlFlow, Range};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use hikasa_core::{ContentSource, Ctl, ErrorKind, Phase, Progress, Unit, io_error_kind};

use crate::error::{VfsError, VfsOp};
use crate::mmap::{self, ChunkFn, MapRequest};
use crate::stream::{FillError, StreamOptions, fill, report_completion};

/// The chunk length of every visit (8 MiB, [`StreamOptions::MAPPED_CHUNK`]); fixed, never thread-dependent.
pub const VISIT_CHUNK: u64 = StreamOptions::MAPPED_CHUNK as u64;

/// The function of [`visit_chunks`]: `f(offset, bytes)` → the chunk's value, `Break` to end the visit.
pub type ChunkVisitor<'f, T> = dyn Fn(u64, &[u8]) -> ControlFlow<T, T> + Sync + 'f;

/// The function of [`visit_chunk_pairs`]: `f(offset, a, b)` with two slices of the same length.
pub type PairVisitor<'f, T> = dyn Fn(u64, &[u8], &[u8]) -> ControlFlow<T, T> + Sync + 'f;

/// Result of a visit: the per-chunk values in offset order, up to and including the first `Break`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visit<T> {
    /// One value per visited chunk, in offset order.
    pub values: Vec<T>,
    /// The last value came from a `Break`; the chunks after it were not visited (or their values discarded).
    pub broke: bool,
    /// The bytes were read through memory maps (else streamed or read in place from memory).
    pub mapped: bool,
}

/// Visits `range` of one source in chunks of [`VISIT_CHUNK`] bytes (the last one shorter). `f(offset, bytes)`
/// gets the chunk's absolute offset and runs on up to `opts.effective_threads()` threads; the values come back
/// in offset order; the lowest-offset `Break` wins and later chunks are not visited or are discarded (T2: the
/// result never depends on the thread count).
///
/// # Errors
///
/// `VfsError::Truncated` (kind `unexpected_eof`) when the source ends before `range.end`; the source's open or
/// read error; `VfsError::Cancelled`.
pub fn visit_chunks<T: Send>(
    src: &dyn ContentSource,
    range: Range<u64>,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkVisitor<'_, T>,
) -> Result<Visit<T>, VfsError> {
    let req = VisitRequest::new(range, VISIT_CHUNK, Phase::Reading);
    let g = |offset: u64, chunks: &[&[u8]]| f(offset, chunks.first().copied().unwrap_or_default());
    let visit = visit_sources(&[src], &req, opts, ctl, &g, None)?;
    report_completion(ctl, Phase::Reading, visit.visited);
    Ok(visit.visit)
}

/// Visits `range` of two sources pairwise, as [`visit_chunks`]; `f(offset, a, b)` gets two slices of the same
/// length. The caller bounds `range` by the shorter length.
///
/// # Errors
///
/// As [`visit_chunks`], naming the source that failed.
pub fn visit_chunk_pairs<T: Send>(
    a: &dyn ContentSource,
    b: &dyn ContentSource,
    range: Range<u64>,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &PairVisitor<'_, T>,
) -> Result<Visit<T>, VfsError> {
    let req = VisitRequest::new(range, VISIT_CHUNK, Phase::Reading);
    let visit = visit_sources(&[a, b], &req, opts, ctl, &pair_fn(f), None)?;
    report_completion(ctl, Phase::Reading, visit.visited);
    Ok(visit.visit)
}

/// `f` of a pair visit as a function of the slices.
fn pair_fn<'f, T>(
    f: &'f PairVisitor<'f, T>,
) -> impl Fn(u64, &[&[u8]]) -> ControlFlow<T, T> + Sync + 'f {
    move |offset, chunks| match chunks {
        [a, b] => f(offset, a, b),
        _ => f(offset, &[], &[]),
    }
}

/// What a visit covers (crate-private: the comparison and the CRC set `exact`, and the unit tests a small chunk).
#[derive(Debug, Clone)]
pub(crate) struct VisitRequest {
    /// The byte range of every source.
    pub(crate) range: Range<u64>,
    /// Chunk length (≥ 1).
    pub(crate) chunk: u64,
    /// Phase of the progress reports.
    pub(crate) phase: Phase,
    /// Every source must end exactly at `range.end`: bytes past it give [`Outcome::Longer`] (the comparison and
    /// the CRC then stream the whole sources, their reference implementation).
    pub(crate) exact: bool,
}

impl VisitRequest {
    pub(crate) fn new(range: Range<u64>, chunk: u64, phase: Phase) -> VisitRequest {
        VisitRequest {
            range,
            chunk: chunk.max(1),
            phase,
            exact: false,
        }
    }

    fn len(&self) -> u64 {
        self.range.end.saturating_sub(self.range.start)
    }
}

/// A finished visit and the bytes it visited (the completion report).
#[derive(Debug)]
pub(crate) struct Visited<T> {
    pub(crate) visit: Visit<T>,
    pub(crate) visited: u64,
}

/// Visits `req.range` of `sources` (module documentation). `before_restat` is the mmap re-stat test hook.
pub(crate) fn visit_sources<T: Send>(
    sources: &[&dyn ContentSource],
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
    before_restat: Option<&dyn Fn()>,
) -> Result<Visited<T>, VfsError> {
    match visit_outcome(sources, req, opts, ctl, f, before_restat)? {
        Outcome::Done(visited) => Ok(visited),
        // Only an exact visit sees bytes past the range; the error names the first source.
        Outcome::Longer => Err(VfsError::Io {
            spec: sources.first().map(|s| s.display()).unwrap_or_default(),
            op: VfsOp::Read,
            source: std::io::Error::other("longer than its reported length"),
        }),
    }
}

/// The outcome of a visit.
#[derive(Debug)]
pub(crate) enum Outcome<T> {
    /// Visited.
    Done(Visited<T>),
    /// An exact visit found bytes past `range.end` in a source (it grew, or reported a wrong length).
    Longer,
}

/// [`visit_sources`] that reports [`Outcome::Longer`] for an exact visit instead of failing.
pub(crate) fn visit_outcome<T: Send>(
    sources: &[&dyn ContentSource],
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
    before_restat: Option<&dyn Fn()>,
) -> Result<Outcome<T>, VfsError> {
    ctl.check()?;
    if let Some(visited) = visit_mapped(sources, req, opts, ctl, f, before_restat)? {
        return Ok(Outcome::Done(visited));
    }
    visit_streaming(sources, req, opts, ctl, f)
}

/// [`visit_outcome`] without read buffers, for the comparison and the CRC (W12 review F3): mapped under the policy,
/// or in place when every source holds its bytes in memory. `None` when neither applies or the mapped visit was
/// abandoned: the caller then streams with `buffer_size` buffers (M1's reference path), which reads one buffer a
/// side before it can stop at a difference, where a streaming visit would fill `threads` whole chunks a side.
pub(crate) fn visit_in_place<T: Send>(
    sources: &[&dyn ContentSource],
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
    before_restat: Option<&dyn Fn()>,
) -> Result<Option<Outcome<T>>, VfsError> {
    ctl.check()?;
    if !sources.is_empty() && sources.iter().all(|s| s.in_memory().is_some()) {
        return visit_streaming(sources, req, opts, ctl, f).map(Some);
    }
    Ok(visit_mapped(sources, req, opts, ctl, f, before_restat)?.map(Outcome::Done))
}

/// The mapped attempt: `None` when the policy or the sources rule it out, or when it was abandoned (the caller
/// streams).
fn visit_mapped<T: Send>(
    sources: &[&dyn ContentSource],
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
    before_restat: Option<&dyn Fn()>,
) -> Result<Option<Visited<T>>, VfsError> {
    let len = req.len();
    // The policy first: a range too small to be mapped never asks for a local path (a `stat` on LocalVfs).
    if len == 0 || !opts.mmap.maps(len) || sources.iter().any(|s| s.in_memory().is_some()) {
        return Ok(None);
    }
    let Some(paths) = sources
        .iter()
        .map(|s| s.local_path())
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let Ok(chunk) = usize::try_from(req.chunk) else {
        return Ok(None);
    };
    let map = MapRequest {
        range: req.range.clone(),
        exact: req.exact,
        chunk,
        threads: opts.effective_threads(),
        phase: req.phase,
    };
    match mmap::visit_files_mapped(&paths, &map, ctl, f, before_restat.unwrap_or(&|| {})) {
        Ok(Some(mapped)) => {
            let visited = if mapped.broke {
                chunk_end(req, mapped.values.len().saturating_sub(1) as u64)
            } else {
                len
            };
            Ok(Some(Visited {
                visit: Visit {
                    values: mapped.values,
                    broke: mapped.broke,
                    mapped: true,
                },
                visited: visited.max(mapped.processed.min(len)),
            }))
        }
        Ok(None) => Ok(None),
        Err(e) if io_error_kind(&e) == ErrorKind::Cancelled => Err(VfsError::Cancelled),
        // Streaming reports the failure with the source's spec (or succeeds).
        Err(_) => Ok(None),
    }
}

/// Bytes of the range up to the end of chunk `index` (relative).
fn chunk_end(req: &VisitRequest, index: u64) -> u64 {
    index
        .saturating_add(1)
        .saturating_mul(req.chunk)
        .min(req.len())
}

/// One source of a streaming visit: in place from memory, or a sequential reader.
enum Input<'s> {
    Memory(&'s [u8]),
    Reader {
        reader: Box<dyn Read + Send + 's>,
        display: String,
    },
}

/// The streaming visit (the reference implementation).
fn visit_streaming<T: Send>(
    sources: &[&dyn ContentSource],
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
    f: &ChunkFn<'_, T>,
) -> Result<Outcome<T>, VfsError> {
    let len = req.len();
    let mut inputs = Vec::with_capacity(sources.len());
    for src in sources {
        let display = src.display();
        if let Some(bytes) = src.in_memory() {
            let have = bytes.len() as u64;
            if have < req.range.end {
                return Err(truncated(display, have.max(req.range.start), req));
            }
            inputs.push(Input::Memory(bytes));
            continue;
        }
        let mut reader = src
            .open()
            .map_err(|e| VfsError::from_io(display.as_str(), VfsOp::Open, e))?;
        skip(&mut reader, req.range.start, &display, req, opts, ctl)?;
        inputs.push(Input::Reader { reader, display });
    }
    let window = opts.effective_threads();
    let chunks = len.div_ceil(req.chunk);
    // Buffers of the window: `slots[slot][input]` (memory inputs leave theirs empty).
    let mut slots: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut values = Vec::new();
    let mut broke = false;
    let mut index = 0u64;
    while index < chunks && !broke {
        let batch = (chunks - index).min(window as u64) as usize;
        if slots.len() < batch {
            slots.resize_with(batch, || vec![Vec::new(); inputs.len()]);
        }
        // Fill the window in offset order (one reader per source: sequential).
        let mut offsets = Vec::with_capacity(batch);
        for (k, slot) in slots.iter_mut().take(batch).enumerate() {
            ctl.check()?;
            let rel = (index + k as u64) * req.chunk;
            let offset = req.range.start + rel;
            let size = req.chunk.min(len - rel) as usize;
            for (input, buf) in inputs.iter_mut().zip(slot.iter_mut()) {
                if let Input::Reader { reader, display } = input {
                    buf.resize(size, 0);
                    read_chunk(reader, buf, offset, display, req, opts, ctl)?;
                }
            }
            offsets.push((offset, size));
        }
        let memory: Vec<Option<&[u8]>> = inputs
            .iter()
            .map(|input| match input {
                Input::Memory(bytes) => Some(*bytes),
                Input::Reader { .. } => None,
            })
            .collect();
        let flows = run_window(&memory, &slots[..batch], &offsets, window, f);
        for flow in flows {
            match flow {
                ControlFlow::Continue(value) => values.push(value),
                ControlFlow::Break(value) => {
                    values.push(value);
                    broke = true;
                    break;
                }
            }
        }
        index += batch as u64;
        let done = chunk_end(req, index - 1);
        ctl.progress(req.phase, || bytes_progress(req.phase, done, len));
    }
    if req.exact && !broke {
        for input in &mut inputs {
            if let Input::Reader { reader, display } = input {
                let mut probe = [0u8; 1];
                let extra = fill(reader.as_mut(), &mut probe, ctl.cancel)
                    .map_err(|e| fill_error(e, display))?;
                if extra > 0 {
                    return Ok(Outcome::Longer);
                }
            } else if let Input::Memory(bytes) = input
                && bytes.len() as u64 > req.range.end
            {
                return Ok(Outcome::Longer);
            }
        }
    }
    let visited = if broke {
        chunk_end(req, values.len().saturating_sub(1) as u64)
    } else {
        len
    };
    Ok(Outcome::Done(Visited {
        visit: Visit {
            values,
            broke,
            mapped: false,
        },
        visited,
    }))
}

/// Runs `f` on the filled chunks of a window: inline for one chunk or one thread, else on up to `threads` scoped
/// threads that claim chunks in order. The values come back in offset order.
fn run_window<T: Send>(
    memory: &[Option<&[u8]>],
    slots: &[Vec<Vec<u8>>],
    offsets: &[(u64, usize)],
    threads: usize,
    f: &ChunkFn<'_, T>,
) -> Vec<ControlFlow<T, T>> {
    let visit_one = |k: usize| {
        let (offset, size) = offsets[k];
        let slices: Vec<&[u8]> = memory
            .iter()
            .zip(&slots[k])
            .map(|(bytes, buf)| match bytes {
                Some(bytes) => usize::try_from(offset)
                    .ok()
                    .and_then(|start| bytes.get(start..start.checked_add(size)?))
                    .unwrap_or_default(),
                None => buf.get(..size).unwrap_or_default(),
            })
            .collect();
        f(offset, &slices)
    };
    let threads = threads.min(offsets.len()).max(1);
    if threads == 1 {
        return (0..offsets.len()).map(visit_one).collect();
    }
    let next = AtomicUsize::new(0);
    let mut results: Vec<Option<ControlFlow<T, T>>> = (0..offsets.len()).map(|_| None).collect();
    let claimed: Vec<Vec<(usize, ControlFlow<T, T>)>> = thread::scope(|scope| {
        let work = || {
            let mut done = Vec::new();
            loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                if k >= offsets.len() {
                    break done;
                }
                done.push((k, visit_one(k)));
            }
        };
        let helpers: Vec<_> = (1..threads)
            .filter_map(|_| {
                thread::Builder::new()
                    .name("hikasa-vfs-visit".into())
                    .spawn_scoped(scope, work)
                    .ok()
            })
            .collect();
        let mut all = vec![work()];
        for helper in helpers {
            // A helper that panicked loses its chunks; they are visited below.
            if let Ok(done) = helper.join() {
                all.push(done);
            }
        }
        all
    });
    for (k, flow) in claimed.into_iter().flatten() {
        results[k] = Some(flow);
    }
    results
        .into_iter()
        .enumerate()
        .map(|(k, flow)| flow.unwrap_or_else(|| visit_one(k)))
        .collect()
}

/// Reads and discards `n` bytes (a range that does not start at 0).
fn skip(
    reader: &mut Box<dyn Read + Send + '_>,
    mut n: u64,
    display: &str,
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    let mut buf = vec![
        0u8;
        opts.effective_buffer_size()
            .min(usize::try_from(n).unwrap_or(usize::MAX))
            .max(1)
    ];
    let mut at = 0u64;
    while n > 0 {
        ctl.check()?;
        let want = buf.len().min(usize::try_from(n).unwrap_or(usize::MAX));
        let got = fill(reader.as_mut(), &mut buf[..want], ctl.cancel)
            .map_err(|e| fill_error(e, display))?;
        if got == 0 {
            return Err(truncated(display.to_owned(), at, req));
        }
        n -= got as u64;
        at += got as u64;
    }
    Ok(())
}

/// Fills `buf` completely in reads of `buffer_size` bytes; a source that ends first is `Truncated`.
fn read_chunk(
    reader: &mut Box<dyn Read + Send + '_>,
    buf: &mut [u8],
    offset: u64,
    display: &str,
    req: &VisitRequest,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<(), VfsError> {
    let step = opts.effective_buffer_size();
    let mut filled = 0usize;
    for piece in buf.chunks_mut(step) {
        let want = piece.len();
        let got = fill(reader.as_mut(), piece, ctl.cancel).map_err(|e| fill_error(e, display))?;
        filled += got;
        if got < want {
            return Err(truncated(display.to_owned(), offset + filled as u64, req));
        }
        ctl.check()?;
    }
    Ok(())
}

fn fill_error(error: FillError, display: &str) -> VfsError {
    match error {
        FillError::Cancelled => VfsError::Cancelled,
        FillError::Io(e) => VfsError::from_io(display, VfsOp::Read, e),
    }
}

fn truncated(spec: String, at: u64, req: &VisitRequest) -> VfsError {
    VfsError::Truncated {
        spec,
        at,
        expected: req.range.end,
    }
}

fn bytes_progress(phase: Phase, done: u64, total: u64) -> Progress {
    Progress {
        phase,
        done,
        total: Some(total),
        unit: Unit::Bytes,
        current: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::MmapPolicy;

    struct Mem(Vec<u8>, bool);

    impl ContentSource for Mem {
        fn name(&self) -> &str {
            "m"
        }
        fn display(&self) -> String {
            "mem".into()
        }
        fn len(&self) -> Option<u64> {
            Some(self.0.len() as u64)
        }
        fn in_memory(&self) -> Option<&[u8]> {
            self.1.then_some(&self.0[..])
        }
        fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>> {
            Ok(Box::new(std::io::Cursor::new(&self.0[..])))
        }
        fn read_all(
            &self,
            _limit: u64,
            _cancel: &hikasa_core::CancelToken,
        ) -> Result<Vec<u8>, hikasa_core::ReadError> {
            Ok(self.0.clone())
        }
    }

    fn opts(threads: usize) -> StreamOptions {
        StreamOptions {
            buffer_size: 4096,
            mmap: MmapPolicy::Never,
            threads,
        }
    }

    /// Small chunks: the cuts start at `range.start`, the last chunk is shorter, values come in order, the lowest
    /// `Break` wins, with every thread count, streamed and in memory.
    #[test]
    fn small_chunks_cut_at_the_range_start_in_order() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 253) as u8).collect();
        for in_memory in [false, true] {
            let src = Mem(data.clone(), in_memory);
            for threads in [1, 3, 16] {
                let req = VisitRequest::new(5..998, 100, Phase::Reading);
                let f = |offset: u64, chunks: &[&[u8]]| {
                    let len = chunks[0].len() as u64;
                    assert_eq!(chunks[0][0], data[offset as usize]);
                    if offset >= 405 {
                        ControlFlow::Break((offset, len))
                    } else {
                        ControlFlow::Continue((offset, len))
                    }
                };
                let v = visit_sources(&[&src], &req, &opts(threads), &Ctl::detached(), &f, None)
                    .unwrap();
                assert_eq!(
                    v.visit.values,
                    [(5, 100), (105, 100), (205, 100), (305, 100), (405, 100)]
                );
                assert!(v.visit.broke && !v.visit.mapped);
                let all = |offset: u64, chunks: &[&[u8]]| -> ControlFlow<(u64, u64), (u64, u64)> {
                    ControlFlow::Continue((offset, chunks[0].len() as u64))
                };
                let v = visit_sources(&[&src], &req, &opts(threads), &Ctl::detached(), &all, None)
                    .unwrap();
                assert_eq!(v.visit.values.len(), 10);
                assert_eq!(v.visit.values.last(), Some(&(905, 93)));
                assert_eq!(v.visited, 993);
            }
        }
    }

    #[test]
    fn a_short_source_is_truncated() {
        let src = Mem(vec![1; 250], false);
        let req = VisitRequest::new(0..300, 100, Phase::Reading);
        let f = |_: u64, _: &[&[u8]]| ControlFlow::<(), ()>::Continue(());
        let err = visit_sources(&[&src], &req, &opts(1), &Ctl::detached(), &f, None).unwrap_err();
        assert!(
            matches!(&err, VfsError::Truncated { spec, at: 250, expected: 300 } if spec == "mem"),
            "{err:?}"
        );
        assert_eq!(hikasa_core::Classify::kind(&err), ErrorKind::UnexpectedEof);
        let mem = Mem(vec![1; 250], true);
        let err = visit_sources(&[&mem], &req, &opts(1), &Ctl::detached(), &f, None).unwrap_err();
        assert!(
            matches!(err, VfsError::Truncated { at: 250, .. }),
            "{err:?}"
        );
    }
}
