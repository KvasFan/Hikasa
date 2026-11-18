//! A12.12 (D-337, T9): an 80,000 × 80,000 PNG header is `too_large` without a large allocation — the pixel limit
//! is checked from the header before any pixel buffer exists. W12c review fixes (F3): a decode holds the file and the
//! decoder's pixels, then the pixels and the RGBA8 result, never the three at once (and a wide type not even two
//! of them). A counting allocator measures the peak live heap; each test holds `MEASURE` (`cargo test` runs them
//! on threads of one process).
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other
// memory tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Mutex;
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

static MEASURE: Mutex<()> = Mutex::new(());

fn measure() -> std::sync::MutexGuard<'static, ()> {
    MEASURE.lock().unwrap_or_else(|e| e.into_inner())
}

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
    let _measure = measure();
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

/// A GIF of a `screen` logical screen with one image descriptor a frame (all at 0, 0; each a 1-pixel LZW stream:
/// no real pixel data for larger frames).
fn gif(screen: (u16, u16), frames: &[(u16, u16)]) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend(screen.0.to_le_bytes());
    g.extend(screen.1.to_le_bytes());
    // A global colour table of two entries.
    g.extend([0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
    for &(width, height) in frames {
        g.extend([0x2C, 0, 0, 0, 0]);
        g.extend(width.to_le_bytes());
        g.extend(height.to_le_bytes());
        // No local table; LZW minimum code size 2; codes clear, 0, end of information (3 bits each).
        g.extend([0, 2, 2, 0x44, 0x01, 0]);
    }
    g.push(0x3B);
    g
}

/// W12 review F4 (D-337): a GIF whose frame is larger than its logical screen is checked by the frame. A 1 × 1
/// screen with a frame above 256 MP (or a later frame above it) is `too_large` before the decoder allocates the
/// frame (1 GiB for 16,385², 16 GiB for 65,535²).
#[test]
fn a_gif_frame_above_the_limit_is_too_large_without_allocating() {
    let _measure = measure();
    let _ = hikasa_picture::oetf_table();
    // The small GIF decodes (the walk accepts what it refuses only by size).
    let small = gif((1, 1), &[(1, 1)]);
    let picture = decode_bytes(
        &small,
        Some("s.gif"),
        &DecodeOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!((picture.width, picture.height), (1, 1));
    for (frames, pixels) in [
        (&[(16_385, 16_385)][..], 16_385 * 16_385),
        (&[(65_535, 65_535)][..], 65_535 * 65_535),
        (&[(1, 1), (40_000, 40_000)][..], 40_000 * 40_000),
    ] {
        let bytes = gif((1, 1), frames);
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let err = decode_bytes(
            &bytes,
            Some("bomb.gif"),
            &DecodeOptions::default(),
            &Ctl::detached(),
        )
        .map(|_| ())
        .unwrap_err();
        let peak = PEAK.load(Ordering::Relaxed) - before;
        assert!(
            matches!(
                err,
                hikasa_picture::PictureError::TooLarge { pixels: Some(p), limit }
                    if p == pixels && limit == 1 << 28
            ),
            "{err:?}"
        );
        assert!(peak < 4 << 20, "{frames:?}: peak live heap {peak} bytes");
    }
}

/// Seeded noise (PNG cannot compress it much, so the file is about as large as the pixels).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// F3: the file is dropped once the decoder has its pixels; an 8-bit type is written straight into the shared
/// buffer (the pixels and the result, no third copy); a 16-bit RGBA picture's result is copied into the shared buffer
/// only once its source is gone. Before: file + pixels + result + the shared copy.
#[test]
fn a_decode_holds_two_buffers_at_most() {
    let _measure = measure();
    let _ = hikasa_picture::oetf_table();
    let (w, h) = (1024u32, 1024u32);
    let px = (w * h) as isize;
    let rgb8 = image::DynamicImage::ImageRgb8(
        image::RgbImage::from_raw(w, h, noise(px as usize * 3, 7)).unwrap(),
    );
    let rgba16 = image::DynamicImage::ImageRgba16(
        image::ImageBuffer::from_raw(
            w,
            h,
            noise(px as usize * 8, 9)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect(),
        )
        .unwrap(),
    );
    // A smooth RGBA16 picture compresses to little, so its peak is the conversion's, not the file's.
    let smooth = image::DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(w, h, |x, y| {
        image::Rgba([(x * 64) as u16, (y * 64) as u16, 1000, 65_535])
    }));
    let slack = 1536 << 10;
    for (image, native, wide) in [
        (rgb8, px * 3, false),
        (rgba16, px * 8, true),
        (smooth, px * 8, true),
    ] {
        let mut file = std::io::Cursor::new(Vec::new());
        image.write_to(&mut file, image::ImageFormat::Png).unwrap();
        let file = file.into_inner();
        drop(image);
        let f = file.len() as isize;
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let picture = hikasa_picture::decode_vec(
            file,
            Some("n.png"),
            &DecodeOptions::default(),
            &Ctl::detached(),
        )
        .unwrap();
        // The file was allocated before measuring and is freed inside: count it.
        let peak = PEAK.load(Ordering::Relaxed) - before + f;
        let after = LIVE.load(Ordering::Relaxed) - before + f;
        drop(picture);
        let rgba = px * 4;
        // A wide type's zeroed result counts whole here (the allocator cannot see that its pages become resident
        // only as the source shrinks: that is G11's RSS row); its copy into the shared buffer comes after the
        // source is gone.
        let _ = wide;
        let bound = (f + native).max(native + rgba);
        assert!(
            peak <= bound + slack,
            "wide {wide}: peak live heap {peak} > {bound} + {slack} (file {f}, pixels {native})"
        );
        assert!(after <= rgba + (64 << 10), "kept {after}");
    }
}

/// F3: above the parallel budget the two sides decode one after the other: the first side's picture stays while the
/// second decodes, never both decoders' buffers at once. The pictures equal the ones decoded alone.
#[test]
fn two_sides_above_the_budget_decode_one_after_the_other() {
    let _measure = measure();
    let _ = hikasa_picture::oetf_table();
    let (w, h) = (1024u32, 1024u32);
    let px = (w * h) as isize;
    let file = |seed: u64| {
        let image = image::DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(w, h, noise(px as usize * 3, seed)).unwrap(),
        );
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let (l, r) = (file(3), file(5));
    let alone = |bytes: &[u8]| {
        hikasa_picture::decode_bytes(bytes, None, &DecodeOptions::default(), &Ctl::detached())
            .unwrap()
    };
    let (la, ra) = (alone(&l), alone(&r));
    let f = (l.len() + r.len()) as isize;
    let encoded = |bytes: Vec<u8>| hikasa_picture::Encoded {
        bytes,
        name: Some("n.png".into()),
    };
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let pair = hikasa_picture::decode_two_within(
        encoded(l),
        encoded(r),
        0,
        &DecodeOptions::default(),
        &Ctl::detached(),
    );
    // Both files were allocated before measuring and are freed inside: count them.
    let peak = PEAK.load(Ordering::Relaxed) - before + f;
    let (lp, rp) = (pair.left.unwrap(), pair.right.unwrap());
    assert!(lp.rgba == la.rgba && rp.rgba == ra.rgba);
    // One side decoding (its pixels and its result) beside the other's picture and, at most, its file.
    let (native, rgba) = (px * 3, px * 4);
    let bound = rgba + native + rgba;
    assert!(
        peak <= bound + (768 << 10),
        "peak live heap {peak} > {bound} + 768 KiB"
    );
}
