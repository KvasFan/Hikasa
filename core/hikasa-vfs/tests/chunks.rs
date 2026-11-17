//! A12.1 (M2 W12a, D-325; HX-009): the chunk visitor.
//!
//! - P-VISIT: random pairs (equal, a prefix, one or two differing bytes at 0, at every chunk edge ± 1, at the
//!   last byte or anywhere; lengths around 0, 1, 8 MiB ± 1 and 16 MiB + 3) give the same values mapped, streamed
//!   and from memory with 1, 4 and 16 threads, equal to a naive loop over the same cuts; the lowest-offset
//!   `Break` wins;
//! - `compare_sources` and `crc32_source` equal a naive byte loop (the M1 results) on the same pairs;
//! - a truncation during a visit is `unexpected_eof` naming the source; cancellation is `Cancelled`.

mod common;

use std::fs;
use std::io::{self, Read};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::{CancelToken, Classify, ContentSource, Ctl, ErrorKind, ReadError};
use hikasa_vfs::{
    BinaryOutcome, MmapPolicy, StreamOptions, VISIT_CHUNK, VfsError, compare_sources, crc32_source,
    visit_chunk_pairs, visit_chunks,
};
use proptest::prelude::*;

use common::XorShift;

const CHUNK: usize = VISIT_CHUNK as usize;

/// A local file as a content source; `len` may lie (a truncation after the stat).
struct LocalFile {
    path: PathBuf,
    len: u64,
}

impl ContentSource for LocalFile {
    fn name(&self) -> &str {
        "f"
    }
    fn display(&self) -> String {
        self.path.display().to_string()
    }
    fn len(&self) -> Option<u64> {
        Some(self.len)
    }
    fn local_path(&self) -> Option<&Path> {
        Some(&self.path)
    }
    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(fs::File::open(&self.path)?))
    }
    fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Ok(fs::read(&self.path)?)
    }
}

/// Bytes as a content source, held in memory (`in_memory`) or only readable as a stream.
struct Bytes {
    name: &'static str,
    bytes: Arc<[u8]>,
    in_memory: bool,
    /// Reported length (may lie).
    len: u64,
}

impl Bytes {
    fn new(name: &'static str, bytes: &Arc<[u8]>, in_memory: bool) -> Bytes {
        Bytes {
            name,
            bytes: Arc::clone(bytes),
            in_memory,
            len: bytes.len() as u64,
        }
    }
}

impl ContentSource for Bytes {
    fn name(&self) -> &str {
        self.name
    }
    fn display(&self) -> String {
        format!("mem:{}", self.name)
    }
    fn len(&self) -> Option<u64> {
        Some(self.len)
    }
    fn in_memory(&self) -> Option<&[u8]> {
        self.in_memory.then_some(&self.bytes[..])
    }
    fn open(&self) -> io::Result<Box<dyn Read + Send + '_>> {
        Ok(Box::new(io::Cursor::new(&self.bytes[..])))
    }
    fn read_all(&self, _limit: u64, _cancel: &CancelToken) -> Result<Vec<u8>, ReadError> {
        Ok(self.bytes.to_vec())
    }
}

/// The value of a pair chunk: offset, length, the CRCs of both sides and the first differing offset; a
/// difference breaks the visit.
type PairValue = (u64, usize, u32, u32, Option<u64>);

fn pair_value(offset: u64, a: &[u8], b: &[u8]) -> ControlFlow<PairValue, PairValue> {
    let first = a.iter().zip(b).position(|(x, y)| x != y);
    let value = (
        offset,
        a.len(),
        crc32fast::hash(a),
        crc32fast::hash(b),
        first.map(|i| offset + i as u64),
    );
    if first.is_some() {
        ControlFlow::Break(value)
    } else {
        ControlFlow::Continue(value)
    }
}

/// The naive pair visit over the same cuts.
fn naive_pairs(a: &[u8], b: &[u8], len: usize) -> (Vec<PairValue>, bool) {
    let mut values = Vec::new();
    let mut start = 0;
    while start < len {
        let end = (start + CHUNK).min(len);
        match pair_value(start as u64, &a[start..end], &b[start..end]) {
            ControlFlow::Continue(v) => values.push(v),
            ControlFlow::Break(v) => {
                values.push(v);
                return (values, true);
            }
        }
        start = end;
    }
    (values, false)
}

/// The value of a single chunk: offset, length, CRC.
type OneValue = (u64, usize, u32);

fn one_value(offset: u64, a: &[u8]) -> ControlFlow<OneValue, OneValue> {
    ControlFlow::Continue((offset, a.len(), crc32fast::hash(a)))
}

fn naive_one(a: &[u8]) -> Vec<OneValue> {
    a.chunks(CHUNK)
        .enumerate()
        .map(|(k, c)| ((k * CHUNK) as u64, c.len(), crc32fast::hash(c)))
        .collect()
}

/// M1's binary outcome by a naive byte loop (§2.20: different known lengths both above 4096 bytes decide alone).
fn naive_outcome(a: &[u8], b: &[u8]) -> BinaryOutcome {
    if a.len() != b.len() && a.len() > 4096 && b.len() > 4096 {
        return BinaryOutcome::Different {
            first_difference: None,
        };
    }
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

fn options(mmap: MmapPolicy, threads: usize) -> StreamOptions {
    StreamOptions {
        buffer_size: 64 << 10,
        mmap,
        threads,
    }
}

const MAPPED: MmapPolicy = MmapPolicy::Auto { min_len: 0 };

/// A check of one way of reading: `(label, a, b, options)`.
type Check<'c> = dyn FnMut(&str, &dyn ContentSource, &dyn ContentSource, &StreamOptions) + 'c;

/// Every way of reading a pair: mapped files, streamed files, streamed bytes, bytes in memory; each with 1, 4 and
/// 16 threads. `check(label, a, b, opts)` runs on each.
fn every_reading(a: &Arc<[u8]>, b: &Arc<[u8]>, check: &mut Check<'_>) {
    let dir = tempfile::tempdir().unwrap();
    let (pa, pb) = (dir.path().join("a"), dir.path().join("b"));
    fs::write(&pa, a).unwrap();
    fs::write(&pb, b).unwrap();
    let fa = LocalFile {
        path: pa,
        len: a.len() as u64,
    };
    let fb = LocalFile {
        path: pb,
        len: b.len() as u64,
    };
    for threads in [1, 4, 16] {
        check("mapped", &fa, &fb, &options(MAPPED, threads));
        check("streamed", &fa, &fb, &options(MmapPolicy::Never, threads));
        let (sa, sb) = (Bytes::new("a", a, false), Bytes::new("b", b, false));
        check("stream-bytes", &sa, &sb, &options(MAPPED, threads));
        let (ma, mb) = (Bytes::new("a", a, true), Bytes::new("b", b, true));
        check("memory", &ma, &mb, &options(MAPPED, threads));
    }
}

/// A length around the interesting sizes: mostly small, sometimes around one or two chunks.
fn length() -> impl Strategy<Value = usize> {
    prop_oneof![
        8 => 0usize..=5000,
        1 => prop_oneof![
            Just(0usize),
            Just(1),
            Just(CHUNK - 1),
            Just(CHUNK),
            Just(CHUNK + 1),
            Just(2 * CHUNK + 3),
        ],
    ]
}

/// Where to plant a difference in a pair of `len` bytes: 0, a chunk edge ± 1, the last byte or anywhere.
fn place(len: usize, pick: u8, any: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let at = match pick % 6 {
        0 => 0,
        1 => CHUNK - 1,
        2 => CHUNK,
        3 => CHUNK + 1,
        4 => len - 1,
        _ => any % len,
    };
    Some(at.min(len - 1))
}

/// A pair: equal, a prefix (either side), one or two differing bytes.
fn pair() -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (
        length(),
        any::<u64>(),
        0u8..5,
        any::<u8>(),
        any::<u8>(),
        any::<usize>(),
        any::<usize>(),
        0usize..=4,
    )
        .prop_map(|(len, seed, mode, p1, p2, any1, any2, cut)| {
            let a = XorShift(seed | 1).bytes(len);
            let mut b = a.clone();
            match mode {
                0 => {}
                1 => b.truncate(len.saturating_sub(cut + 1)),
                2 => {
                    let mut a = a;
                    a.truncate(len.saturating_sub(cut + 1));
                    return (a, b);
                }
                _ => {
                    if let Some(at) = place(len, p1, any1) {
                        b[at] ^= 0x5A;
                    }
                    if mode == 4
                        && let Some(at) = place(len, p2, any2)
                    {
                        b[at] = b[at].wrapping_add(1);
                    }
                }
            }
            (a, b)
        })
}

proptest! {
    #![proptest_config(common::config_with("tests/proptest-regressions/chunks.txt", 96))]

    /// P-VISIT: the values of a pair visit, of a single visit, of `compare_sources` and of `crc32_source` are
    /// the same for every way of reading and every thread count, and equal the naive results.
    #[test]
    fn p_visit((a, b) in pair()) {
        let (a, b): (Arc<[u8]>, Arc<[u8]>) = (a.into(), b.into());
        let shorter = a.len().min(b.len());
        let (expected_pairs, expected_broke) = naive_pairs(&a, &b, shorter);
        let (expected_a, expected_b) = (naive_one(&a), naive_one(&b));
        let outcome = naive_outcome(&a, &b);
        let (crc_a, crc_b) = (crc32fast::hash(&a), crc32fast::hash(&b));
        let mut failures = Vec::new();
        every_reading(&a, &b, &mut |label, sa, sb, opts| {
            let threads = opts.threads;
            let ctl = Ctl::detached();
            let pairs = visit_chunk_pairs(sa, sb, 0..shorter as u64, opts, &ctl, &pair_value).unwrap();
            if (pairs.values.as_slice(), pairs.broke) != (expected_pairs.as_slice(), expected_broke) {
                failures.push(format!("{label}/{threads}: pairs {:?}", pairs.values.len()));
            }
            let mapped = label == "mapped" && shorter > 0;
            if pairs.mapped != mapped {
                failures.push(format!("{label}/{threads}: mapped {}", pairs.mapped));
            }
            for (src, expected) in [(sa, &expected_a), (sb, &expected_b)] {
                let len = src.len().unwrap();
                let one = visit_chunks(src, 0..len, opts, &ctl, &one_value).unwrap();
                if &one.values != expected || one.broke {
                    failures.push(format!("{label}/{threads}: single"));
                }
            }
            let got = compare_sources(sa, sb, opts, &ctl).unwrap();
            if got != outcome {
                failures.push(format!("{label}/{threads}: compare {got:?} != {outcome:?}"));
            }
            let crcs = (crc32_source(sa, opts, &ctl).unwrap(), crc32_source(sb, opts, &ctl).unwrap());
            if crcs != (crc_a, crc_b) {
                failures.push(format!("{label}/{threads}: crc"));
            }
        });
        prop_assert!(failures.is_empty(), "{failures:?}");
    }
}

/// The lowest-offset `Break` wins when every chunk breaks, and the values before it come in offset order, with
/// every thread count, mapped and streamed.
#[test]
fn the_lowest_break_wins() {
    let data: Arc<[u8]> = XorShift(11).bytes(5 * CHUNK + 17).into();
    let f = |offset: u64, bytes: &[u8]| {
        if offset >= 2 * VISIT_CHUNK {
            ControlFlow::Break((offset, bytes.len()))
        } else {
            ControlFlow::Continue((offset, bytes.len()))
        }
    };
    every_reading(&data, &data, &mut |label, sa, _, opts| {
        let visit = visit_chunks(sa, 0..data.len() as u64, opts, &Ctl::detached(), &f).unwrap();
        assert_eq!(
            visit.values,
            [(0, CHUNK), (VISIT_CHUNK, CHUNK), (2 * VISIT_CHUNK, CHUNK)],
            "{label} {}",
            opts.threads
        );
        assert!(visit.broke);
        // A range that does not start at 0 cuts at its own start; the last chunk is shorter.
        let range = 5..(2 * VISIT_CHUNK + 9);
        let all = |offset: u64, bytes: &[u8]| -> ControlFlow<(u64, usize), (u64, usize)> {
            ControlFlow::Continue((offset, bytes.len()))
        };
        let visit = visit_chunks(sa, range, opts, &Ctl::detached(), &all).unwrap();
        assert_eq!(
            visit.values,
            [
                (5, CHUNK),
                (VISIT_CHUNK + 5, CHUNK),
                (2 * VISIT_CHUNK + 5, 4)
            ],
            "{label} {}",
            opts.threads
        );
    });
}

/// A source that ends before the range: `Truncated` (kind `unexpected_eof`) naming it, mapped (the mapped
/// attempt gives way to streaming) and streamed, never zero-filled bytes.
#[test]
fn a_truncation_names_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("short");
    fs::write(&path, vec![7u8; CHUNK + 100]).unwrap();
    let file = LocalFile {
        path: path.clone(),
        len: (CHUNK + 1000) as u64,
    };
    let full: Arc<[u8]> = vec![7u8; CHUNK + 1000].into();
    let other = Bytes::new("other", &full, true);
    let f = |_: u64, _: &[u8]| ControlFlow::<(), ()>::Continue(());
    let g = |_: u64, _: &[u8], _: &[u8]| ControlFlow::<(), ()>::Continue(());
    for mmap in [MAPPED, MmapPolicy::Never] {
        for threads in [1, 4] {
            let opts = options(mmap, threads);
            let err = visit_chunks(&file, 0..file.len, &opts, &Ctl::detached(), &f).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
            assert_eq!(err.spec(), Some(path.display().to_string().as_str()));
            assert!(
                matches!(err, VfsError::Truncated { at, .. } if at == (CHUNK + 100) as u64),
                "{err:?}"
            );
            let err = visit_chunk_pairs(&other, &file, 0..file.len, &opts, &Ctl::detached(), &g)
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
            assert_eq!(err.spec(), Some(path.display().to_string().as_str()));
        }
    }
    // A source held in memory that is shorter than the range.
    let short: Arc<[u8]> = vec![1u8; 10].into();
    let mut lying = Bytes::new("lying", &short, true);
    lying.len = 20;
    let err = visit_chunks(&lying, 0..20, &options(MAPPED, 1), &Ctl::detached(), &f).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(err.spec(), Some("mem:lying"));
    // The comparison and the CRC keep M1's answers: the reference streams the whole inputs.
    assert_eq!(
        compare_sources(&file, &other, &options(MAPPED, 1), &Ctl::detached()).unwrap(),
        BinaryOutcome::Different {
            first_difference: Some((CHUNK + 100) as u64)
        }
    );
    assert_eq!(
        crc32_source(&file, &options(MAPPED, 4), &Ctl::detached()).unwrap(),
        crc32fast::hash(&vec![7u8; CHUNK + 100])
    );
}

/// Cancellation before or during a visit is `Cancelled`, mapped and streamed.
#[test]
fn cancellation_is_cancelled() {
    let data: Arc<[u8]> = XorShift(3).bytes(3 * CHUNK).into();
    every_reading(&data, &data, &mut |label, sa, sb, opts| {
        let token = CancelToken::new();
        token.cancel();
        let ctl = common::ctl(&token);
        let f = |_: u64, _: &[u8]| ControlFlow::<(), ()>::Continue(());
        let err = visit_chunks(sa, 0..data.len() as u64, opts, &ctl, &f).unwrap_err();
        assert!(matches!(err, VfsError::Cancelled), "{label}: {err:?}");
        // Cancelled by the function while the second chunk is visited (one thread: the third chunk is not
        // claimed or filled yet).
        if opts.threads > 1 {
            return;
        }
        let token = CancelToken::new();
        let ctl = common::ctl(&token);
        let g = |offset: u64, _: &[u8], _: &[u8]| {
            if offset >= VISIT_CHUNK {
                token.cancel();
            }
            ControlFlow::<(), ()>::Continue(())
        };
        let result = visit_chunk_pairs(sa, sb, 0..data.len() as u64, opts, &ctl, &g);
        assert!(
            matches!(result, Err(VfsError::Cancelled)),
            "{label} {}: {result:?}",
            opts.threads
        );
    });
}
