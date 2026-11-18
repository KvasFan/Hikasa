//! A12.2 and A12.4 (D-326, D-329): a counting allocator measures the live heap.
//! - the page cache holds at most `cache_pages` pages: after reading a 64 MiB file twice at random, the document
//!   holds at most 16 MiB of pages plus 16 KiB of bookkeeping;
//! - Fast on a 64 MiB pair (local files, mapped, one thread) peaks within the §1.12.6 bound: its blocks and
//!   interner, one visit chunk of right bytes for the statistics pass, and the refinement windows.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other
// memory tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use common::{Edit, XorShift, edited, lspec};
use hikasa_core::Ctl;
use hikasa_hex::{AlignMode, HexAlignOptions, HexDocument, HexLoadOptions, align};
use hikasa_vfs::{LocalVfs, MmapPolicy, StreamOptions};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn grew(by: isize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counters are static
// atomics, so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size() as isize);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }
    // SAFETY: `ptr` and `layout` come from `alloc`/`realloc` above, as `System::dealloc` requires.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
    // SAFETY: the caller's guarantees for `ptr`, `layout` and `new_size` are passed on unchanged.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grew(new_size as isize - layout.size() as isize);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// Both measurements run in one test: the counters are process-wide.
#[test]
fn memory_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let a = XorShift(1).bytes(64 << 20);
    let b = edited(
        &a,
        &[
            Edit::Insert {
                at: 1 << 20,
                len: 100,
            },
            Edit::Delete {
                at: 20 << 20,
                len: 3000,
            },
            Edit::Replace {
                at: 40 << 20,
                len: 5000,
            },
        ],
        2,
    );
    let (pa, pb) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::write(&pa, &a).unwrap();
    std::fs::write(&pb, &b).unwrap();
    drop((a, b));
    let vfs = LocalVfs::default();
    let ctl = Ctl::detached();

    // The page cache.
    let before = live();
    let doc = HexDocument::open(&vfs, &lspec(&pa), &HexLoadOptions::default(), &ctl).unwrap();
    let mut rng = XorShift(9);
    let mut buf = vec![0u8; 100];
    let base = live();
    for _ in 0..2 {
        for _ in 0..4096 {
            let at = rng.below(64 << 20);
            doc.read(at, &mut buf).unwrap();
        }
        for page in 0..(64u64 << 20) / (64 << 10) {
            doc.read(page * (64 << 10), &mut buf).unwrap();
        }
    }
    let held = live() - base;
    eprintln!("page cache after two random passes over 64 MiB: {held} bytes");
    assert!(
        held <= (16 << 20) + (16 << 10),
        "the page cache holds {held} bytes"
    );
    drop(doc);
    assert!(
        live() - before < 4096,
        "the document leaks {} bytes",
        live() - before
    );

    // Fast on the pair.
    let load = HexLoadOptions {
        unlocked_max: 0,
        ..HexLoadOptions::default()
    };
    let da = HexDocument::open(&vfs, &lspec(&pa), &load, &ctl).unwrap();
    let db = HexDocument::open(&vfs, &lspec(&pb), &load, &ctl).unwrap();
    let opts = HexAlignOptions {
        stream: StreamOptions {
            mmap: MmapPolicy::Auto { min_len: 0 },
            threads: 1,
            ..StreamOptions::default()
        },
        ..HexAlignOptions::with_mode(AlignMode::Fast)
    };
    let base = live();
    PEAK.store(base, Ordering::Relaxed);
    let al = align(&da, &db, &opts, &ctl).unwrap();
    let peak = PEAK.load(Ordering::Relaxed) - base;
    eprintln!("Fast on a 64 MiB pair: peak {peak} bytes over the documents");
    assert_eq!(
        (al.stats.left_only_bytes, al.stats.right_only_bytes),
        (3000, 100)
    );
    // Blocks: 24 bytes each (about 8,000 a side); the interner; one 8 MiB chunk of right bytes (and a short
    // read span); Myers windows of the small gaps.
    assert!(peak <= 20 << 20, "Fast peaked at {peak} bytes");
}
