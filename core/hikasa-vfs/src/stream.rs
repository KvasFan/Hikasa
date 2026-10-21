//! Streaming CRC32 and binary comparison (§2.20; FC-025, VFS-020): [`StreamOptions`], [`MmapPolicy`],
//! [`crc32_reader`], [`compare_readers`].
//!
//! The streaming path is the reference implementation: the mapped path (`mmap`) must give the same results
//! (INV-VFS-2). Cancellation is checked once per buffer and between the partial reads that fill a buffer, so a
//! slow reader delays cancellation by at most one `read` call. Progress is reported lazily through
//! [`Ctl::progress`] (`reading` for comparisons, `hashing` for CRCs, both in bytes), and a finished CRC or
//! comparison always ends with one unconditional completion report (`done == total`, [`Ctl::progress_now`]).

use std::io::{self, Read};

use hikasa_core::{CancelToken, Cancelled, Ctl, Phase, Progress, Unit};

use crate::compare::BinaryOutcome;
use crate::error::{VfsError, VfsOp};

/// Streaming parameters. `Default` = 1 MiB buffers, `MmapPolicy::Auto { min_len: 16 MiB }`, one thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    /// Read buffer per side (OPT "Buffer size for binary compare"); clamped to [4 KiB, 64 MiB]. Default 1 MiB.
    pub buffer_size: usize,
    /// When to memory-map local files.
    pub mmap: MmapPolicy,
    /// Threads that process one memory-mapped file (CRC) or file pair (comparison): the calling thread plus up
    /// to `threads - 1` scoped helper threads that live only for that call (§5.1). Clamped to
    /// [1, [`MAX_THREADS`](Self::MAX_THREADS)] (0 counts as 1) and to the number of 8 MiB chunks. Default 1:
    /// the engine API never starts threads unless the caller asks; `hikasa-session` passes
    /// `min(4, its pool size)`. Results never depend on it (T2): chunk CRCs are combined exactly and a
    /// comparison reports the lowest differing offset. The streaming path always runs on the calling thread.
    pub threads: usize,
}

impl StreamOptions {
    /// Default read buffer per side (1 MiB).
    pub const DEFAULT_BUFFER_SIZE: usize = 1 << 20;
    /// Smallest effective buffer (4 KiB).
    pub const MIN_BUFFER_SIZE: usize = 4 << 10;
    /// Largest effective buffer (64 MiB).
    pub const MAX_BUFFER_SIZE: usize = 64 << 20;
    /// Chunk of a mapped comparison or CRC between two cancellation checks (8 MiB, §2.20).
    pub const MAPPED_CHUNK: usize = 8 << 20;
    /// Largest effective [`threads`](Self::threads) value (16).
    pub const MAX_THREADS: usize = 16;
    /// Reported sizes at or below this (4096 bytes) are not trusted to decide "different" alone: procfs reports
    /// 0 and sysfs 4096 for files with other contents, so such files are read (§2.20 step 2).
    pub const UNRELIABLE_SIZE_MAX: u64 = 4096;

    /// `buffer_size` clamped to [`MIN_BUFFER_SIZE`](Self::MIN_BUFFER_SIZE),
    /// [`MAX_BUFFER_SIZE`](Self::MAX_BUFFER_SIZE).
    pub fn effective_buffer_size(&self) -> usize {
        self.buffer_size
            .clamp(Self::MIN_BUFFER_SIZE, Self::MAX_BUFFER_SIZE)
    }

    /// `threads` clamped to [1, [`MAX_THREADS`](Self::MAX_THREADS)].
    pub fn effective_threads(&self) -> usize {
        self.threads.clamp(1, Self::MAX_THREADS)
    }

    /// True when two known lengths decide "different" without reading (§2.20 step 2): they differ and both are
    /// above [`UNRELIABLE_SIZE_MAX`](Self::UNRELIABLE_SIZE_MAX).
    pub(crate) fn lengths_differ(a: Option<u64>, b: Option<u64>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => {
                a != b && a > Self::UNRELIABLE_SIZE_MAX && b > Self::UNRELIABLE_SIZE_MAX
            }
            _ => false,
        }
    }
}

impl Default for StreamOptions {
    fn default() -> StreamOptions {
        StreamOptions {
            buffer_size: StreamOptions::DEFAULT_BUFFER_SIZE,
            mmap: MmapPolicy::Auto {
                min_len: MmapPolicy::DEFAULT_MIN_LEN,
            },
            threads: 1,
        }
    }
}

/// mmap policy (the streaming path is the reference implementation; INV-VFS-2: both give equal results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapPolicy {
    /// Never map.
    Never,
    /// Map local regular files whose length is at least `min_len` (default 16 MiB).
    Auto {
        /// Smallest length that is mapped.
        min_len: u64,
    },
}

impl MmapPolicy {
    /// Default `min_len` of [`MmapPolicy::Auto`] (16 MiB).
    pub const DEFAULT_MIN_LEN: u64 = 16 << 20;

    /// True when a local regular file of `len` bytes is mapped under this policy.
    pub fn maps(self, len: u64) -> bool {
        match self {
            MmapPolicy::Never => false,
            MmapPolicy::Auto { min_len } => len >= min_len,
        }
    }
}

/// A failure inside the streaming loops: cancellation, or an I/O error of one input (0 = first, 1 = second).
#[derive(Debug)]
pub(crate) enum StreamFail {
    /// The token was cancelled.
    Cancelled,
    /// Reading input `side` failed.
    Io {
        /// 0 = first input, 1 = second input.
        side: usize,
        /// The error.
        error: io::Error,
    },
}

impl From<Cancelled> for StreamFail {
    fn from(_: Cancelled) -> StreamFail {
        StreamFail::Cancelled
    }
}

impl StreamFail {
    /// The VFS error, naming the failed input with `specs[side]`.
    pub(crate) fn into_vfs(self, specs: [&str; 2]) -> VfsError {
        match self {
            StreamFail::Cancelled => VfsError::Cancelled,
            StreamFail::Io { side, error } => {
                let spec = specs.get(side).copied().unwrap_or_default();
                VfsError::from_io(spec, VfsOp::Read, error)
            }
        }
    }

    /// The failure of input `side` for a [`FillError`].
    fn of(side: usize) -> impl Fn(FillError) -> StreamFail {
        move |fail| match fail {
            FillError::Cancelled => StreamFail::Cancelled,
            FillError::Io(error) => StreamFail::Io { side, error },
        }
    }
}

/// Why [`fill`] stopped before the buffer was full or the reader at its end.
#[derive(Debug)]
pub(crate) enum FillError {
    /// The token was cancelled between two partial reads.
    Cancelled,
    /// A read failed.
    Io(io::Error),
}

/// Reads until `buf` is full or the reader is at its end; returns the number of bytes read. Retries
/// `Interrupted`. `cancel` is checked before every read but the first (the caller checks before calling), so a
/// slow reader that returns little per call delays cancellation by at most one `read`.
pub(crate) fn fill(
    r: &mut dyn Read,
    buf: &mut [u8],
    cancel: &CancelToken,
) -> Result<usize, FillError> {
    let mut filled = 0;
    let mut first = true;
    while let Some(rest) = buf.get_mut(filled..) {
        if rest.is_empty() {
            break;
        }
        if !first {
            cancel.check().map_err(|_| FillError::Cancelled)?;
        }
        first = false;
        match r.read(rest) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FillError::Io(e)),
        }
    }
    Ok(filled)
}

/// Index of the first differing byte of two equally long slices (`None` when they are equal).
pub(crate) fn first_mismatch(a: &[u8], b: &[u8]) -> Option<usize> {
    if a == b {
        return None;
    }
    const BLOCK: usize = 64;
    let block = a
        .chunks(BLOCK)
        .zip(b.chunks(BLOCK))
        .position(|(x, y)| x != y)?;
    let start = block * BLOCK;
    let x = a.get(start..).unwrap_or_default();
    let y = b.get(start..).unwrap_or_default();
    x.iter().zip(y).position(|(p, q)| p != q).map(|i| start + i)
}

/// Buffer length for a stream whose length is probably `hint`: never larger than `hint + 1` (one fill then
/// sees the end), never smaller than 1.
fn buffer_len(buf_len: usize, hint: Option<u64>) -> usize {
    let cap = hint
        .and_then(|h| usize::try_from(h.saturating_add(1)).ok())
        .unwrap_or(usize::MAX);
    buf_len.min(cap).max(1)
}

fn progress(phase: Phase, done: u64, total: Option<u64>) -> Progress {
    Progress {
        phase,
        done,
        total,
        unit: Unit::Bytes,
        current: None,
    }
}

/// The unconditional completion report of a finished CRC or comparison (`done == total == done`, §5.4): it
/// passes every throttle, so a progress bar always reaches its end. `done` is the number of bytes processed
/// (the length, or the offset that decided a comparison), which may differ from the length a caller expected.
pub(crate) fn report_completion(ctl: &Ctl<'_>, phase: Phase, done: u64) {
    ctl.progress_now(progress(phase, done, Some(done)));
}

/// CRC32 (IEEE) of everything `r` yields, reading `buf_len` bytes at a time (no clamping: tests use tiny
/// buffers). `total` is only a progress total and a buffer-size hint.
pub(crate) fn crc32_stream(
    r: &mut dyn Read,
    buf_len: usize,
    total: Option<u64>,
    ctl: &Ctl<'_>,
) -> Result<u32, StreamFail> {
    let full = buf_len.max(1);
    let mut buf = vec![0u8; buffer_len(buf_len, total)];
    let mut hasher = crc32fast::Hasher::new();
    let mut done = 0u64;
    loop {
        ctl.check()?;
        let n = fill(r, &mut buf, ctl.cancel).map_err(StreamFail::of(0))?;
        if n == 0 {
            break;
        }
        hasher.update(buf.get(..n).unwrap_or_default());
        done += n as u64;
        ctl.progress(Phase::Hashing, || progress(Phase::Hashing, done, total));
        if n < buf.len() {
            break;
        }
        if buf.len() < full {
            // The input is longer than its hint: continue with the full buffer.
            buf.resize(full, 0);
        }
    }
    report_completion(ctl, Phase::Hashing, done);
    Ok(hasher.finalize())
}

/// Byte-wise comparison (§2.20 step 4): fill `buf_a` fully, fill `buf_b` with the same length, compare; the
/// first unequal index gives the offset; the end of one input before the other gives
/// `Different { first_difference: Some(shorter length) }`. `buf_len` is not clamped (tests use 1–64 bytes);
/// `total` is only a progress total and a buffer-size hint.
pub(crate) fn compare_stream(
    a: &mut dyn Read,
    b: &mut dyn Read,
    buf_len: usize,
    total: Option<u64>,
    ctl: &Ctl<'_>,
) -> Result<BinaryOutcome, StreamFail> {
    let (outcome, done) = compare_stream_inner(a, b, buf_len, total, ctl)?;
    report_completion(ctl, Phase::Reading, done);
    Ok(outcome)
}

/// [`compare_stream`] without the completion report; also returns the bytes compared (the length, or the
/// offset that decided).
fn compare_stream_inner(
    a: &mut dyn Read,
    b: &mut dyn Read,
    buf_len: usize,
    total: Option<u64>,
    ctl: &Ctl<'_>,
) -> Result<(BinaryOutcome, u64), StreamFail> {
    let full = buf_len.max(1);
    let mut len = buffer_len(buf_len, total);
    let mut buf_a = vec![0u8; len];
    let mut buf_b = vec![0u8; len];
    let mut offset = 0u64;
    loop {
        ctl.check()?;
        if offset > 0 && len < full {
            // A full fill of a hinted buffer: the inputs are longer than the hint.
            len = full;
            buf_a.resize(full, 0);
            buf_b.resize(full, 0);
        }
        let n_a = fill(a, &mut buf_a, ctl.cancel).map_err(StreamFail::of(0))?;
        let n_b = fill(b, buf_b.get_mut(..n_a).unwrap_or_default(), ctl.cancel)
            .map_err(StreamFail::of(1))?;
        let common_a = buf_a.get(..n_b).unwrap_or_default();
        let common_b = buf_b.get(..n_b).unwrap_or_default();
        let different = |at: u64| {
            let outcome = BinaryOutcome::Different {
                first_difference: Some(at),
            };
            Ok((outcome, at))
        };
        if let Some(i) = first_mismatch(common_a, common_b) {
            return different(offset + i as u64);
        }
        if n_b < n_a {
            // The second input ended first.
            return different(offset + n_b as u64);
        }
        offset += n_a as u64;
        ctl.progress(Phase::Reading, || progress(Phase::Reading, offset, total));
        if n_a < len {
            // The first input ended: the second must end here too.
            let mut probe = [0u8; 1];
            let extra = fill(b, &mut probe, ctl.cancel).map_err(StreamFail::of(1))?;
            if extra == 0 {
                return Ok((BinaryOutcome::Same, offset));
            }
            return different(offset);
        }
    }
}

/// CRC32 (IEEE, `crc32fast`) of a reader. Progress in bytes; cancellation checked per buffer.
///
/// Read errors are reported as [`VfsError::Io`] with op `read` and the spec text `"<reader>"` (a reader has
/// no spec; [`crate::crc32_source`] names the source instead).
pub fn crc32_reader(
    r: &mut dyn Read,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<u32, VfsError> {
    crc32_stream(r, opts.effective_buffer_size(), None, ctl)
        .map_err(|fail| fail.into_vfs(["<reader>", "<reader>"]))
}

/// Byte-wise comparison of two readers (alternating fills of equal-sized chunks).
///
/// `first_difference` is the first differing offset, or the length of the shorter input when one input is a
/// prefix of the other. Read errors are reported as [`VfsError::Io`] with op `read` and the spec text
/// `"<first reader>"` or `"<second reader>"`.
pub fn compare_readers(
    a: &mut dyn Read,
    b: &mut dyn Read,
    opts: &StreamOptions,
    ctl: &Ctl<'_>,
) -> Result<BinaryOutcome, VfsError> {
    compare_stream(a, b, opts.effective_buffer_size(), None, ctl)
        .map_err(|fail| fail.into_vfs(["<first reader>", "<second reader>"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::FileFailurePersistence;
    use std::io::Cursor;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The proptest configuration of the unit-level properties: the case count of `ProptestConfig::default()`
    /// (256, or `PROPTEST_CASES`, §6.1) and failing seeds persisted next to the integration suites.
    fn config() -> ProptestConfig {
        ProptestConfig {
            failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
                "tests/proptest-regressions/stream.txt",
            ))),
            ..ProptestConfig::default()
        }
    }

    /// The naive whole-buffer comparison (the oracle of P-VFS-1 and INV-VFS-2).
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

    /// A reader that returns at most `step` bytes per call and an `Interrupted` error before every read.
    struct Choppy<'a> {
        data: &'a [u8],
        step: usize,
        interrupt: bool,
    }

    impl Read for Choppy<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.interrupt = !self.interrupt;
            if self.interrupt {
                return Err(io::Error::from(io::ErrorKind::Interrupted));
            }
            let n = self.step.min(buf.len()).min(self.data.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    fn content() -> impl Strategy<Value = Vec<u8>> {
        prop_oneof![
            proptest::collection::vec(any::<u8>(), 0..300),
            proptest::collection::vec(0u8..2, 0..300),
        ]
    }

    /// `b` derived from `a`: equal, one byte changed, truncated, extended, or unrelated.
    fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
        (content(), 0usize..5, any::<usize>(), any::<u8>(), content()).prop_map(
            |(a, mode, at, byte, other)| {
                let mut b = a.clone();
                match mode {
                    0 => {}
                    1 if !b.is_empty() => {
                        let i = at % b.len();
                        b[i] = b[i].wrapping_add(byte.max(1));
                    }
                    2 => b.truncate(at % (b.len() + 1)),
                    3 => b.push(byte),
                    _ => b = other,
                }
                (a, b)
            },
        )
    }

    proptest! {
        #![proptest_config(config())]

        /// P-VFS-1 (test-only path): 1–64 byte buffers, readers that return short reads and `Interrupted`.
        #[test]
        fn p_vfs_1_tiny_buffers((a, b) in pair(), buf in 1usize..=64, step_a in 1usize..9, step_b in 1usize..9) {
            let ctl = Ctl::detached();
            let mut ra = Choppy { data: &a, step: step_a, interrupt: false };
            let mut rb = Choppy { data: &b, step: step_b, interrupt: false };
            let got = compare_stream(&mut ra, &mut rb, buf, None, &ctl).unwrap();
            prop_assert_eq!(got, naive(&a, &b));
            let hinted = compare_stream(&mut Cursor::new(&a), &mut Cursor::new(&b), buf, Some(a.len() as u64), &ctl).unwrap();
            prop_assert_eq!(hinted, naive(&a, &b));
        }

        /// P-VFS-3 (test-only path): tiny buffers.
        #[test]
        fn p_vfs_3_tiny_buffers(a in content(), buf in 1usize..=64, step in 1usize..9) {
            let mut r = Choppy { data: &a, step, interrupt: false };
            let crc = crc32_stream(&mut r, buf, None, &Ctl::detached()).unwrap();
            prop_assert_eq!(crc, crc32fast::hash(&a));
        }

        #[test]
        fn first_mismatch_matches_the_naive_scan((a, b) in pair()) {
            let n = a.len().min(b.len());
            let naive_index = a[..n].iter().zip(&b[..n]).position(|(x, y)| x != y);
            prop_assert_eq!(first_mismatch(&a[..n], &b[..n]), naive_index);
        }
    }

    #[test]
    fn options_clamp_and_default() {
        let opts = StreamOptions::default();
        assert_eq!(opts.effective_buffer_size(), 1 << 20);
        assert_eq!(opts.mmap, MmapPolicy::Auto { min_len: 16 << 20 });
        assert_eq!(opts.threads, 1);
        assert_eq!(opts.effective_threads(), 1);
        let tiny = StreamOptions {
            buffer_size: 1,
            mmap: MmapPolicy::Never,
            threads: 0,
        };
        assert_eq!(tiny.effective_buffer_size(), 4096);
        assert_eq!(tiny.effective_threads(), 1);
        let huge = StreamOptions {
            buffer_size: usize::MAX,
            mmap: MmapPolicy::Never,
            threads: usize::MAX,
        };
        assert_eq!(huge.effective_buffer_size(), 64 << 20);
        assert_eq!(huge.effective_threads(), StreamOptions::MAX_THREADS);
        // Reported sizes up to 4096 bytes never decide "different" alone (procfs 0, sysfs 4096).
        let differ = StreamOptions::lengths_differ;
        assert!(differ(Some(4097), Some(5000)));
        assert!(!differ(Some(4097), Some(4097)));
        assert!(!differ(Some(0), Some(6)));
        assert!(!differ(Some(4096), Some(23)));
        assert!(!differ(Some(4096), Some(1 << 30)));
        assert!(!differ(None, Some(1 << 30)));
        assert!(!MmapPolicy::Never.maps(u64::MAX));
        assert!(MmapPolicy::Auto { min_len: 10 }.maps(10));
        assert!(!MmapPolicy::Auto { min_len: 10 }.maps(9));
    }

    #[test]
    fn cancellation_is_checked_per_buffer() {
        let token = CancelToken::new();
        token.cancel();
        let ctl = Ctl {
            cancel: &token,
            progress: &hikasa_core::NullProgress,
        };
        let data = vec![7u8; 100];
        let r = compare_readers(
            &mut Cursor::new(&data),
            &mut Cursor::new(&data),
            &StreamOptions::default(),
            &ctl,
        );
        assert!(matches!(r, Err(VfsError::Cancelled)));
        let r = crc32_reader(&mut Cursor::new(&data), &StreamOptions::default(), &ctl);
        assert!(matches!(r, Err(VfsError::Cancelled)));
    }

    #[test]
    fn read_errors_name_the_reader() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::InvalidData, "bad sector"))
            }
        }
        let ctl = Ctl::detached();
        let opts = StreamOptions::default();
        let err = compare_readers(&mut Cursor::new(b"abc"), &mut Broken, &opts, &ctl).unwrap_err();
        assert_eq!(err.to_string(), "<second reader>: Read failed: bad sector");
        let err = crc32_reader(&mut Broken, &opts, &ctl).unwrap_err();
        assert_eq!(err.to_string(), "<reader>: Read failed: bad sector");
    }

    #[test]
    fn progress_reports_bytes() {
        use std::sync::Mutex;
        let seen = Mutex::new(Vec::new());
        let sink = |p: Progress| seen.lock().unwrap().push((p.phase, p.done, p.total));
        let token = CancelToken::never();
        let ctl = Ctl {
            cancel: &token,
            progress: &sink,
        };
        let data = vec![1u8; 10_000];
        compare_stream(
            &mut Cursor::new(&data),
            &mut Cursor::new(&data),
            4096,
            Some(10_000),
            &ctl,
        )
        .unwrap();
        crc32_stream(&mut Cursor::new(&data), 4096, Some(10_000), &ctl).unwrap();
        let seen = seen.into_inner().unwrap();
        let reading: Vec<u64> = seen
            .iter()
            .filter(|(phase, _, _)| *phase == Phase::Reading)
            .map(|(_, done, _)| *done)
            .collect();
        // The last report is the unconditional completion (`done == total`).
        assert_eq!(reading, [4096, 8192, 10_000, 10_000]);
        let hashing: Vec<u64> = seen
            .iter()
            .filter(|(phase, _, _)| *phase == Phase::Hashing)
            .map(|(_, done, _)| *done)
            .collect();
        assert_eq!(hashing, [4096, 8192, 10_000, 10_000]);
        assert!(seen.iter().all(|(_, _, total)| *total == Some(10_000)));
    }

    /// A sink whose throttle never opens: only unconditional reports get through.
    struct Closed(Mutex<Vec<Progress>>);

    impl hikasa_core::ProgressSink for Closed {
        fn report(&self, progress: Progress) {
            self.0.lock().unwrap().push(progress);
        }
        fn due(&self, _phase: Phase) -> bool {
            false
        }
    }

    /// The completion report (`done == total`) passes a closed throttle, for every outcome.
    #[test]
    fn completion_is_reported_unconditionally() {
        let sink = Closed(Mutex::new(Vec::new()));
        let token = CancelToken::never();
        let ctl = Ctl {
            cancel: &token,
            progress: &sink,
        };
        let data = vec![5u8; 10_000];
        let mut other = data.clone();
        other[7000] = 0;
        let cases: [(&[u8], &[u8], BinaryOutcome, u64); 3] = [
            (&data, &data, BinaryOutcome::Same, 10_000),
            (
                &data,
                &other,
                BinaryOutcome::Different {
                    first_difference: Some(7000),
                },
                7000,
            ),
            (
                &data,
                &data[..9000],
                BinaryOutcome::Different {
                    first_difference: Some(9000),
                },
                9000,
            ),
        ];
        for (a, b, outcome, done) in cases {
            let got =
                compare_stream(&mut Cursor::new(a), &mut Cursor::new(b), 4096, None, &ctl).unwrap();
            assert_eq!(got, outcome);
            let last = sink.0.lock().unwrap().pop().unwrap();
            assert_eq!(
                (last.phase, last.done, last.total),
                (Phase::Reading, done, Some(done))
            );
        }
        crc32_stream(&mut Cursor::new(&data), 4096, Some(20_000), &ctl).unwrap();
        let reports = std::mem::take(&mut *sink.0.lock().unwrap());
        assert_eq!(reports.len(), 1, "only the completion passes");
        assert_eq!(
            (reports[0].phase, reports[0].done, reports[0].total),
            (Phase::Hashing, 10_000, Some(10_000)),
            "the completion names the bytes really hashed, not the hint"
        );
    }

    /// A reader that returns `step` bytes per call and cancels `token` on its `cancel_at`-th call; it counts
    /// its calls.
    struct CancelsAt<'a> {
        step: usize,
        calls: &'a AtomicUsize,
        cancel_at: usize,
        token: &'a CancelToken,
    }

    impl Read for CancelsAt<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed) + 1;
            if call == self.cancel_at {
                self.token.cancel();
            }
            let n = self.step.min(buf.len());
            buf[..n].fill(7);
            Ok(n)
        }
    }

    /// Cancellation is checked between the partial reads that fill one buffer: a slow reader (little per call)
    /// is not read again after `cancel()` (finding: 1 MiB buffers of 64 KiB reads waited for the whole buffer).
    #[test]
    fn cancellation_is_checked_between_partial_reads() {
        for cancel_at in [1, 2, 5] {
            let token = CancelToken::new();
            let calls = AtomicUsize::new(0);
            let ctl = Ctl {
                cancel: &token,
                progress: &hikasa_core::NullProgress,
            };
            let mut slow = CancelsAt {
                step: 64 << 10,
                calls: &calls,
                cancel_at,
                token: &token,
            };
            let mut fast = Cursor::new(vec![7u8; 4 << 20]);
            let r = compare_stream(&mut slow, &mut fast, 1 << 20, None, &ctl);
            assert!(matches!(r, Err(StreamFail::Cancelled)), "{r:?}");
            assert_eq!(
                calls.load(Ordering::Relaxed),
                cancel_at,
                "no read after cancel()"
            );

            let token = CancelToken::new();
            let calls = AtomicUsize::new(0);
            let ctl = Ctl {
                cancel: &token,
                progress: &hikasa_core::NullProgress,
            };
            let mut slow = CancelsAt {
                step: 64 << 10,
                calls: &calls,
                cancel_at,
                token: &token,
            };
            let r = crc32_stream(&mut slow, 64 << 20, None, &ctl);
            assert!(matches!(r, Err(StreamFail::Cancelled)), "{r:?}");
            assert_eq!(
                calls.load(Ordering::Relaxed),
                cancel_at,
                "no read after cancel()"
            );
        }
        // The second input is checked as well.
        let token = CancelToken::new();
        let calls = AtomicUsize::new(0);
        let ctl = Ctl {
            cancel: &token,
            progress: &hikasa_core::NullProgress,
        };
        let mut slow = CancelsAt {
            step: 64 << 10,
            calls: &calls,
            cancel_at: 3,
            token: &token,
        };
        let mut fast = Cursor::new(vec![7u8; 4 << 20]);
        let r = compare_stream(&mut fast, &mut slow, 1 << 20, None, &ctl);
        assert!(matches!(r, Err(StreamFail::Cancelled)), "{r:?}");
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}
