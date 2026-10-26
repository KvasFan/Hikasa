//! Properties of §6.2.2:
//!
//! - P-VFS-1: `compare_readers` equals the naive whole-buffer comparison, including `first_difference`, for
//!   random contents, buffer sizes and short reads (the 1–64 byte buffers of the test-only path are covered by
//!   the unit tests of `stream.rs`);
//! - P-VFS-2: the mapped path equals the streaming path and the naive comparison (INV-VFS-2), for buffer sizes
//!   from 4 KiB to 1 MiB and 1–4 threads, through `compare_sources` and `crc32_source` on real files (each file
//!   is one 8 MiB chunk here; the unit property `mmap::tests::p_vfs_2_multi_chunk` covers files of up to 256
//!   chunks with 1–8 threads);
//! - P-VFS-3: `crc32_reader` equals `crc32fast::hash`;
//! - P-VFS-4: every `MemoryVfs` listing order gives the same entry sets.

mod common;

use std::fs;
use std::io::{self, Read};

use common::{host, mspec};
use hikasa_core::{CancelToken, Ctl, LocalPath, Spec};
use hikasa_vfs::{
    BinaryOutcome, EntryKind, FollowLinks, ListOptions, ListingOrder, LocalVfs, MemMeta, MemoryVfs,
    MmapPolicy, StreamOptions, Vfs, VfsSource, compare_readers, compare_sources, crc32_reader,
    crc32_source,
};
use proptest::prelude::*;

/// The naive whole-buffer comparison (the oracle).
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

/// A reader that returns at most `step` bytes per call.
struct Chunked {
    data: io::Cursor<Vec<u8>>,
    step: usize,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.step.min(buf.len());
        self.data.read(&mut buf[..n])
    }
}

/// Content from a seed: random bytes, or zeros (long equal runs), of length `len`.
fn content(seed: u64, len: usize, zeros: bool) -> Vec<u8> {
    if zeros {
        vec![0; len]
    } else {
        common::XorShift(seed | 1).bytes(len)
    }
}

/// `(a, b)`: `b` is `a` itself, `a` with one changed byte, `a` truncated, `a` extended, or unrelated.
fn pair(max_len: usize) -> impl Strategy<Value = (Vec<u8>, Vec<u8>)> {
    (
        any::<u64>(),
        0..=max_len,
        any::<bool>(),
        0usize..5,
        any::<usize>(),
        1u8..=255,
    )
        .prop_map(move |(seed, len, zeros, mode, at, delta)| {
            let a = content(seed, len, zeros);
            let mut b = a.clone();
            match mode {
                0 => {}
                1 if !b.is_empty() => {
                    let i = at % b.len();
                    b[i] = b[i].wrapping_add(delta);
                }
                2 => b.truncate(at % (b.len() + 1)),
                3 => b.extend(content(seed ^ 0xABCD, at % 5000 + 1, zeros)),
                _ => b = content(seed.rotate_left(17), at % (max_len + 1), !zeros),
            }
            (a, b)
        })
}

fn buffer_size() -> impl Strategy<Value = usize> {
    prop_oneof![
        Just(StreamOptions::MIN_BUFFER_SIZE),
        StreamOptions::MIN_BUFFER_SIZE..=1 << 20,
        Just(1usize << 20),
        // Below the minimum: clamped to 4 KiB.
        1usize..StreamOptions::MIN_BUFFER_SIZE,
    ]
}

fn local_source<'a>(vfs: &'a LocalVfs, path: &std::path::Path) -> VfsSource<'a> {
    let spec = Spec::Local(LocalPath::new(path.canonicalize().unwrap()).unwrap());
    let entry = vfs.stat(&spec, FollowLinks::No).unwrap();
    VfsSource::new(vfs, spec, entry)
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/props.txt"))]

    /// P-VFS-1.
    #[test]
    fn p_vfs_1_compare_readers_equals_naive(
        (a, b) in pair(150_000),
        buffer in buffer_size(),
        step_a in prop_oneof![Just(usize::MAX), 1usize..10_000],
        step_b in prop_oneof![Just(usize::MAX), 1usize..10_000],
    ) {
        let opts = StreamOptions { buffer_size: buffer, mmap: MmapPolicy::Never, threads: 1 };
        let mut ra = Chunked { data: io::Cursor::new(a.clone()), step: step_a };
        let mut rb = Chunked { data: io::Cursor::new(b.clone()), step: step_b };
        let got = compare_readers(&mut ra, &mut rb, &opts, &Ctl::detached()).unwrap();
        prop_assert_eq!(got, naive(&a, &b));
    }

    /// P-VFS-2 (INV-VFS-2): mapped = streamed = naive, on real files.
    #[test]
    fn p_vfs_2_mapped_equals_streamed((a, b) in pair(300_000), buffer in buffer_size(), threads in 1usize..=4) {
        let dir = tempfile::tempdir().unwrap();
        let (pa, pb) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&pa, &a).unwrap();
        fs::write(&pb, &b).unwrap();
        let vfs = LocalVfs::default();
        let (sa, sb) = (local_source(&vfs, &pa), local_source(&vfs, &pb));
        let ctl = Ctl::detached();
        let mapped = StreamOptions { buffer_size: buffer, mmap: MmapPolicy::Auto { min_len: 0 }, threads };
        let streamed = StreamOptions { buffer_size: buffer, mmap: MmapPolicy::Never, threads: 1 };
        // Known, different lengths above 4096 bytes are decided by length alone on every path; smaller reported
        // sizes are not trusted (pseudo-files), so those files are read.
        let trusted = |len: usize| len as u64 > StreamOptions::UNRELIABLE_SIZE_MAX;
        let expected = if a.len() != b.len() && trusted(a.len()) && trusted(b.len()) {
            BinaryOutcome::Different { first_difference: None }
        } else {
            naive(&a, &b)
        };
        prop_assert_eq!(compare_sources(&sa, &sb, &mapped, &ctl).unwrap(), expected);
        prop_assert_eq!(compare_sources(&sa, &sb, &streamed, &ctl).unwrap(), expected);
        prop_assert_eq!(crc32_source(&sa, &mapped, &ctl).unwrap(), crc32fast::hash(&a));
        prop_assert_eq!(crc32_source(&sb, &streamed, &ctl).unwrap(), crc32fast::hash(&b));
    }

    /// P-VFS-3.
    #[test]
    fn p_vfs_3_crc32_reader_equals_crc32fast(
        seed in any::<u64>(),
        len in 0usize..200_000,
        buffer in buffer_size(),
        step in prop_oneof![Just(usize::MAX), 1usize..10_000],
    ) {
        let data = content(seed, len, false);
        let opts = StreamOptions { buffer_size: buffer, mmap: MmapPolicy::Never, threads: 1 };
        let mut r = Chunked { data: io::Cursor::new(data.clone()), step };
        prop_assert_eq!(crc32_reader(&mut r, &opts, &Ctl::detached()).unwrap(), crc32fast::hash(&data));
    }

    /// P-VFS-4: every listing order gives the same entry sets (with and without following links and with a
    /// prefilter).
    #[test]
    fn p_vfs_4_listing_orders_agree(
        items in proptest::collection::vec((0u8..3, "[a-dA-D._]{1,4}"), 0..40),
        seed in any::<u64>(),
        follow in any::<bool>(),
        prefilter in any::<bool>(),
    ) {
        let vfs = MemoryVfs::default();
        vfs.insert_dir(&host("/t"), MemMeta::default()).unwrap();
        for (kind, name) in &items {
            if name == "." || name == ".." {
                continue;
            }
            let path = host(&format!("/t/{name}"));
            // Later inserts of another category fail (AlreadyExists); that is fine for the property.
            let _ = match kind {
                0 => vfs.insert_file(&path, name.as_bytes().to_vec(), MemMeta::default()),
                1 => vfs.insert_dir(&path, MemMeta::default()),
                _ => vfs.insert_symlink(&path, &name.to_lowercase(), MemMeta::default()),
            };
        }
        let accept_files = |_: &str, kind: EntryKind| kind == EntryKind::File;
        let opts = ListOptions {
            follow: if follow { FollowLinks::Yes } else { FollowLinks::No },
            prefilter: if prefilter { Some(&accept_files) } else { None },
        };
        let snapshot = |order: ListingOrder| {
            vfs.set_listing_order(order);
            let mut listing = vfs.list(&mspec("/t"), &opts, &CancelToken::never()).unwrap();
            listing.sort_by_name();
            (listing.entries, listing.name_only, listing.errors)
        };
        let sorted = snapshot(ListingOrder::Sorted);
        prop_assert_eq!(&snapshot(ListingOrder::Reversed), &sorted);
        prop_assert_eq!(&snapshot(ListingOrder::Shuffled { seed }), &sorted);
        prop_assert_eq!(&snapshot(ListingOrder::Shuffled { seed: seed.wrapping_add(1) }), &sorted);
    }
}
