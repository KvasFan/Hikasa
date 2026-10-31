//! A5.8: memory stays bounded: peak allocation below 64 MiB for a stream of 1 GiB of zeros with a 16 MiB `stream_max`.
//! The counting `#[global_allocator]` lives in this test binary only (§6.1), so nothing else runs beside it under
//! `cargo nextest`, which runs each test in its own process.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::archives::on_memory;
use hikasa_core::CancelToken;
use hikasa_vfs::{ArchiveLimits, ListOptions, Vfs, VfsError};

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

#[test]
fn a_stream_of_1_gib_of_zeros_stays_small() {
    let zeros = vec![0u8; 1 << 20];
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    for _ in 0..1024 {
        encoder.write_all(&zeros).unwrap();
    }
    let stream = encoder.finish().unwrap();
    drop(zeros);
    let limits = ArchiveLimits {
        stream_max: 16 << 20,
        ratio_max: 0,
        ..ArchiveLimits::default()
    };
    let memory = on_memory(limits);
    let root = memory.root("big.bin.zst", stream);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let result = memory
        .vfs
        .list(&root, &ListOptions::default(), &CancelToken::never());
    assert!(
        matches!(
            result,
            Err(VfsError::ArchiveLimit {
                limit: "stream",
                ..
            })
        ),
        "{result:?}"
    );
    let peak = PEAK.load(Ordering::Relaxed) - base;
    assert!(peak < 64 << 20, "peak {peak} bytes");
}
