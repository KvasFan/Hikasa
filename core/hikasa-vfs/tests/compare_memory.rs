//! W12 review F3: `compare_sources` and `crc32_source` of sources that are neither mapped nor held in memory keep
//! M1's figures — a comparison that differs early reads one `buffer_size` buffer a side before it stops, and the
//! buffers of a pair (of a CRC) are `2 × buffer_size` (`buffer_size`), whatever `threads` is. The counting
//! `#[global_allocator]` lives in this test binary only; each test holds `MEASURE` (`cargo test` runs them on
//! threads of one process, `cargo nextest` in processes of their own).
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Read;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use hikasa_core::{CancelToken, ContentSource, Ctl, ReadError};
use hikasa_vfs::{BinaryOutcome, MmapPolicy, StreamOptions, compare_sources, crc32_source};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct Counting;

// SAFETY: every call goes to `System` with the same arguments; only the counters are added.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract is forwarded unchanged.
        let block = unsafe { System.alloc(layout) };
        if !block.is_null() {
            let now = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        block
    }

    // SAFETY: forwarded to `System` with the caller's contract.
    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is forwarded unchanged.
        unsafe { System.dealloc(block, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

static MEASURE: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> MutexGuard<'static, ()> {
    MEASURE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `body` under a clean peak; returns the peak above what was live at the start.
fn peak_of<T>(body: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let value = body();
    (value, PEAK.load(Ordering::Relaxed).saturating_sub(base))
}

/// A streamed source (no local path, not in memory) of `len` bytes, all zero but `flip` (1 there), generated on
/// the fly; it counts the bytes it hands out.
struct Generated {
    len: u64,
    flip: Option<u64>,
    read: AtomicU64,
}

impl Generated {
    fn new(len: u64, flip: Option<u64>) -> Generated {
        Generated {
            len,
            flip,
            read: AtomicU64::new(0),
        }
    }

    fn read(&self) -> u64 {
        self.read.load(Ordering::Relaxed)
    }
}

struct Reader<'a> {
    src: &'a Generated,
    at: u64,
}

impl Read for Reader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (self.src.len - self.at).min(buf.len() as u64) as usize;
        buf[..n].fill(0);
        if let Some(flip) = self.src.flip
            && (self.at..self.at + n as u64).contains(&flip)
        {
            buf[(flip - self.at) as usize] = 1;
        }
        self.at += n as u64;
        self.src.read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

impl ContentSource for Generated {
    fn name(&self) -> &str {
        "g"
    }
    fn display(&self) -> String {
        "generated".into()
    }
    fn len(&self) -> Option<u64> {
        Some(self.len)
    }
    fn open(&self) -> std::io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(Reader { src: self, at: 0 }))
    }
    fn read_all(&self, _: u64, _: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Err(ReadError::Io(std::io::Error::other("not read whole")))
    }
}

const LEN: u64 = 64 << 20;
const MIB: u64 = 1 << 20;
/// Allocations beside the buffers (readers, boxes, the outcome).
const SLACK: usize = 64 << 10;

fn opts(threads: usize, mmap: MmapPolicy) -> StreamOptions {
    StreamOptions {
        threads,
        mmap,
        ..StreamOptions::default()
    }
}

#[test]
fn an_early_difference_reads_one_buffer_a_side() {
    let _guard = one_at_a_time();
    let ctl = Ctl::detached();
    for threads in [1, 4, 16] {
        for mmap in [MmapPolicy::Never, MmapPolicy::Auto { min_len: 0 }] {
            for (flip, most) in [(0, MIB), (5 * MIB + 3, 6 * MIB), (LEN - 1, LEN)] {
                let (a, b) = (Generated::new(LEN, None), Generated::new(LEN, Some(flip)));
                let (outcome, peak) =
                    peak_of(|| compare_sources(&a, &b, &opts(threads, mmap), &ctl));
                assert_eq!(
                    outcome.unwrap(),
                    BinaryOutcome::Different {
                        first_difference: Some(flip)
                    }
                );
                let label = format!("threads {threads} {mmap:?} flip {flip}");
                assert!(
                    a.read() <= most && b.read() <= most,
                    "{label}: read {} and {}",
                    a.read(),
                    b.read()
                );
                assert!(
                    peak <= 2 * MIB as usize + SLACK,
                    "{label}: peak live heap {peak} bytes"
                );
            }
            // Equal sources are read whole, with the same two buffers.
            let (a, b) = (Generated::new(LEN, None), Generated::new(LEN, None));
            let (outcome, peak) = peak_of(|| compare_sources(&a, &b, &opts(threads, mmap), &ctl));
            assert_eq!(outcome.unwrap(), BinaryOutcome::Same);
            assert_eq!((a.read(), b.read()), (LEN, LEN));
            assert!(
                peak <= 2 * MIB as usize + SLACK,
                "threads {threads}: peak {peak} bytes"
            );
        }
    }
}

#[test]
fn a_streamed_crc_holds_one_buffer() {
    let _guard = one_at_a_time();
    let ctl = Ctl::detached();
    let mut expected = crc32fast::Hasher::new();
    let mut piece = vec![0u8; MIB as usize];
    let source = Generated::new(LEN, Some(12_345));
    let mut reader = source.open().unwrap();
    for _ in 0..LEN / MIB {
        reader.read_exact(&mut piece).unwrap();
        expected.update(&piece);
    }
    drop((reader, piece));
    let expected = expected.finalize();
    for threads in [1, 4, 16] {
        let src = Generated::new(LEN, Some(12_345));
        let (crc, peak) =
            peak_of(|| crc32_source(&src, &opts(threads, MmapPolicy::Auto { min_len: 0 }), &ctl));
        assert_eq!(crc.unwrap(), expected);
        assert_eq!(src.read(), LEN);
        assert!(
            peak <= MIB as usize + SLACK,
            "threads {threads}: peak live heap {peak} bytes"
        );
    }
}
