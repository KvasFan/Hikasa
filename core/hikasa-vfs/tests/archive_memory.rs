//! A5.8: memory stays bounded: peak allocation below 64 MiB for a stream of 1 GiB of zeros with a 16 MiB `stream_max`.
//! The counting `#[global_allocator]` lives in this test binary only (§6.1), so nothing else runs beside it under
//! `cargo nextest`, which runs each test in its own process.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::archives::{RawEntry, RawTar, compress, name_of, on_memory, pax, raw_zip};
use hikasa_core::{CancelToken, Spec};
use hikasa_vfs::{ArchiveFormat, ArchiveLimits, ArchiveVfs, ListOptions, Listing, Vfs, VfsError};

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

/// `body` under a clean peak; returns the peak above what was live at the start.
fn peak_of<T>(body: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let value = body();
    (value, PEAK.load(Ordering::Relaxed) - base)
}

/// One GiB of zeros as `format`: a compressed block repeated. Every codec reads concatenated streams to the end.
fn zeros_of_1_gib(format: ArchiveFormat) -> Vec<u8> {
    const BLOCKS: usize = 16;
    let block = compress(format, &vec![0u8; (1 << 30) / BLOCKS]);
    block.repeat(BLOCKS)
}

fn listed(vfs: &ArchiveVfs, root: &Spec) -> Result<Listing, VfsError> {
    vfs.list(root, &ListOptions::default(), &CancelToken::never())
}

fn assert_limit<T: std::fmt::Debug>(result: &Result<T, VfsError>, want: &'static str) {
    assert!(
        matches!(result, Err(VfsError::ArchiveLimit { limit, .. }) if *limit == want),
        "{want}: {result:?}"
    );
}

/// A5.8 for one single stream, under both ways a bomb ends: the size and the ratio.
fn a_stream_of_zeros(format: ArchiveFormat, ratio_stops_it: bool) {
    let bytes = zeros_of_1_gib(format);
    let by_size = ArchiveLimits {
        stream_max: 16 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(by_size);
    let root = memory.root(name_of(format), bytes.clone());
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "stream");
    assert!(peak < 64 << 20, "size: peak {peak} bytes");
    assert_eq!(memory.vfs.cache_used(), 0);

    if ratio_stops_it {
        let by_ratio = ArchiveLimits {
            ratio_max: 100,
            ratio_floor: 1 << 20,
            ..ArchiveLimits::default()
        };
        let memory = on_memory(by_ratio);
        let root = memory.root(name_of(format), bytes);
        let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
        assert_limit(&result, "ratio");
        assert!(peak < 64 << 20, "ratio: peak {peak} bytes");
    }
}

#[test]
fn a_gz_of_zeros_stays_small() {
    a_stream_of_zeros(ArchiveFormat::Gz, true);
}

#[test]
fn a_bz2_of_zeros_stays_small() {
    a_stream_of_zeros(ArchiveFormat::Bz2, true);
}

#[test]
fn an_xz_of_zeros_stays_small() {
    a_stream_of_zeros(ArchiveFormat::Xz, true);
}

#[test]
fn a_stream_of_1_gib_of_zeros_stays_small() {
    a_stream_of_zeros(ArchiveFormat::Zst, true);
}

/// The index of a compressed tar walks the same guards as a stream.
#[test]
fn a_tar_of_zeros_stays_small() {
    let mut tar = RawTar {
        size: Some(1 << 30),
        ..RawTar::file(b"zeros", b"")
    }
    .header()
    .to_vec();
    tar.extend_from_slice(&vec![0u8; 64 << 20]);
    let limits = ArchiveLimits {
        stream_max: 16 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let root = memory.root("a.tar.zst", compress(ArchiveFormat::TarZst, &tar));
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "stream");
    assert!(peak < 64 << 20, "peak {peak} bytes");
}

/// A read of a zip member spools through the guards too, and a declared size does not allocate.
#[test]
fn a_zip_member_of_zeros_stays_small() {
    const SIZE: usize = 256 << 20;
    let zeros = vec![0u8; SIZE];
    let mut deflate =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    deflate.write_all(&zeros).unwrap();
    let mut entry = RawEntry::new(b"zeros", &deflate.finish().unwrap());
    entry.method = 8;
    entry.size = Some(SIZE as u32);
    entry.crc32 = Some(crc32fast::hash(&zeros));
    drop(zeros);
    let limits = ArchiveLimits {
        stream_max: 16 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let file = memory.put("a.zip", raw_zip(&[entry, RawEntry::new(b"small", b"fine")]));
    let spec = Spec::archive_member(file.clone(), "zeros");
    let (result, peak) = peak_of(|| memory.vfs.open_read(&spec).map(drop));
    assert_limit(&result, "stream");
    assert!(peak < 64 << 20, "peak {peak} bytes");
    assert_eq!(memory.vfs.cache_used(), 0, "no partial file");
    let neighbour = Spec::archive_member(file, "small");
    let mut out = Vec::new();
    let mut reader = memory.vfs.open_read(&neighbour).unwrap();
    reader.read_to_end(&mut out).unwrap();
    assert_eq!(out, b"fine");
}

/// A GNU long name and the header it applies to.
fn long_name(name: &[u8], type_flag: u8, out: &mut dyn Write) {
    let long = RawTar {
        data: name.to_vec(),
        ..RawTar::typed(b"././@LongLink", type_flag)
    };
    out.write_all(&long.bytes()).unwrap();
}

/// The tar of `write`, as a zstd stream, without holding the tar in memory.
fn tar_zst(write: impl FnOnce(&mut dyn Write)) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    write(&mut encoder);
    encoder.write_all(&[0u8; 1024]).unwrap();
    encoder.finish().unwrap()
}

/// A name of about 4,000 bytes that opens `depth` folders below a folder of its own.
fn deep_name(index: usize, depth: usize) -> Vec<u8> {
    let segment = "a".repeat(4000 / depth - 1);
    let mut name = format!("x{index}");
    for _ in 0..depth {
        name.push('/');
        name.push_str(&segment);
    }
    name.push_str("/f");
    name.into_bytes()
}

/// The review's bomb of 46 KiB: each of 3,000 names opens 2,000 folders of up to 4 KiB, 13.4 GiB as full paths.
#[test]
fn a_tree_deeper_than_the_cap_stops_at_once() {
    let bytes = tar_zst(|tar| {
        let header = RawTar::typed(b"longlink", b'0').bytes();
        for index in 0..3000 {
            long_name(&deep_name(index, 1995), b'L', tar);
            tar.write_all(&header).unwrap();
        }
    });
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("deep.tar.zst", bytes);
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "depth");
    assert!(peak < 8 << 20, "peak {peak} bytes");
}

/// Below the depth cap the same bomb stops at the text of its folders, and nothing is stored per folder.
#[test]
fn a_tree_of_long_paths_stops_at_the_paths_limit() {
    let bytes = tar_zst(|tar| {
        let header = RawTar::typed(b"longlink", b'0').bytes();
        for index in 0..3000 {
            long_name(&deep_name(index, 200), b'L', tar);
            tar.write_all(&header).unwrap();
        }
    });
    let memory = on_memory(ArchiveLimits::default());
    let root = memory.root("deep.tar.zst", bytes);
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "paths");
    assert!(peak < 48 << 20, "peak {peak} bytes");
}

/// The same through a zip, with folders that fit: the tree holds each path once.
#[test]
fn a_tree_that_fits_costs_its_members() {
    let entries: Vec<RawEntry> = (0..200)
        .map(|index| RawEntry::new(&deep_name(index, 200), b""))
        .collect();
    let memory = on_memory(ArchiveLimits::default());
    let file = memory.put("deep.zip", raw_zip(&entries));
    let root = Spec::archive_member(file.clone(), "");
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_eq!(result.unwrap().entries.len(), 200);
    assert!(peak < 32 << 20, "peak {peak} bytes");
    // 40,200 nodes. As full paths their text alone would be 80 MiB.
    let index = memory.vfs.index_memory(&file).unwrap();
    assert!(index < 8 << 20, "index {index} bytes");
}

/// The review's 3.3 MiB tar.gz of 9 GiB: link targets of 1 MiB. Targets stop at 4,096 bytes, as in a zip.
#[test]
fn long_link_targets_are_not_kept() {
    let target = vec![b't'; (1 << 20) - 200];
    for pax_header in [true, false] {
        let bytes = tar_zst(|tar| {
            for index in 0..300 {
                if pax_header {
                    tar.write_all(&pax(&[("linkpath", &target)])).unwrap();
                } else {
                    long_name(&target, b'K', tar);
                }
                let link = RawTar::typed(format!("l{index}").as_bytes(), b'2');
                tar.write_all(&link.bytes()).unwrap();
            }
        });
        let limits = ArchiveLimits {
            ratio_max: 0,
            ..ArchiveLimits::default()
        };
        let memory = on_memory(limits);
        let root = memory.root("links.tar.zst", bytes);
        let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
        let listing = result.unwrap();
        assert_eq!(listing.entries.len(), 300);
        assert!(
            listing.entries.iter().all(|e| e.link_target.is_none()),
            "a target past the cap is not shown"
        );
        assert!(peak < 16 << 20, "peak {peak} bytes");
    }
}

/// Long names within the entries limit stop at the bytes they cost.
#[test]
fn many_long_names_stop_at_the_paths_limit() {
    let bytes = tar_zst(|tar| {
        let header = RawTar::typed(b"longlink", b'0').bytes();
        for index in 0..20_000 {
            let mut name = vec![b'N'; 4000];
            name.extend_from_slice(index.to_string().as_bytes());
            long_name(&name, b'L', tar);
            tar.write_all(&header).unwrap();
        }
    });
    let limits = ArchiveLimits {
        paths_max: 64 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let root = memory.root("names.tar.zst", bytes);
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "paths");
    assert!(peak < 96 << 20, "peak {peak} bytes");
}

/// The crate keeps a central directory in memory and follows the count, not the stated size, so the bytes it may read
/// are what is bounded: 200 names of 60 KiB stop at the limit, before they reach the tree.
#[test]
fn a_central_directory_of_long_names_stops_at_the_paths_limit() {
    let entries: Vec<RawEntry> = (0..200)
        .map(|index| {
            let mut name = vec![b'n'; 60_000];
            name.extend_from_slice(format!("{index}").as_bytes());
            RawEntry::new(&name, b"")
        })
        .collect();
    let limits = ArchiveLimits {
        entries_max: 1000,
        paths_max: 1 << 20,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let root = memory.root("names.zip", raw_zip(&entries));
    let (result, peak) = peak_of(|| listed(&memory.vfs, &root));
    assert_limit(&result, "paths");
    assert!(peak < 8 << 20, "peak {peak} bytes");
}
