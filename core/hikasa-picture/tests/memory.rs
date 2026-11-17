//! A12.12 (D-337, T9): an 80,000 × 80,000 PNG header is `too_large` without a large allocation — the pixel limit
//! is checked from the header before any pixel buffer exists. A counting allocator measures the peak live heap.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other
// memory tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use hikasa_core::{Classify, Ctl, ErrorKind};
use hikasa_picture::{DecodeOptions, decode_bytes};

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

/// The PNG header of a picture of `width` × `height` with a short IDAT (no real pixel data).
fn png_header(width: u32, height: u32) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend(hikasa_vfs::crc32_bytes(&body).to_be_bytes());
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(
        &mut png,
        b"IDAT",
        &[0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff, 0, 0, 0, 1],
    );
    chunk(&mut png, b"IEND", &[]);
    png
}

#[test]
fn an_80000_square_header_is_too_large_without_allocating() {
    // Warm up the lazily built tables and the thread pool before measuring.
    let _ = hikasa_picture::oetf_table();
    let header = png_header(80_000, 80_000);
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let err = decode_bytes(
        &header,
        Some("big.png"),
        &DecodeOptions::default(),
        &Ctl::detached(),
    )
    .map(|_| ())
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err}");
    let peak = PEAK.load(Ordering::Relaxed) - before;
    // 80,000² RGBA pixels would be 23.8 GiB; the decoder's buffers for the header are kilobytes.
    assert!(peak < 4 << 20, "peak live heap {peak} bytes");
}
