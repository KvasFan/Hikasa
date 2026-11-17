//! Gate G11, engine rows (§1.12.28, D-351), and the timed cancellation rows of A12.18. `#[ignore]`: run by hand in
//! release on an idle machine, figures in docs/02 and §1.12.33:
//!
//! ```text
//! cargo test --release -p hikasa-picture --test gates -- --ignored --nocapture --test-threads 1
//! ```
//!
//! - *diff*: two decoded 6000 × 4000 (24 MP) RGBA8 pictures differing in 1 % of their pixels (blobs and noise),
//!   tolerance 8: the statistics, then a full render of each mode — each < 0.5 s;
//! - *geometry*: a new offset on two 12 MP pictures (`with_geometry`) < 60 ms;
//! - *decode*: the decode of a 24 MP PNG (fast compression) < 1 s (§1.12.14's figure);
//! - *cancel*: a picture diff and a full render cancelled midway return within 100 ms.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::XorShift;
use hikasa_core::{CancelToken, Ctl, NullProgress};
use hikasa_picture::{
    DecodeOptions, DiffMode, Geometry, Pane, Picture, PictureColors, PictureOptions, PngLevel,
    RgbaImage, compare_pictures, decode_bytes, encode_png,
};

/// A photo-like picture: gradients plus low noise.
fn base(width: u32, height: u32, seed: u64) -> Vec<u8> {
    let mut rng = XorShift(seed);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let n = (rng.next() % 7) as u32;
            rgba.extend([
                ((x * 255 / width) + n).min(255) as u8,
                ((y * 255 / height) + n).min(255) as u8,
                (((x + y) * 128 / (width + height)) + n) as u8,
                255,
            ]);
        }
    }
    rgba
}

/// The pair of the diff row: the right picture differs in 1 % of the pixels — half in discs, half as noise, some
/// changes within the tolerance.
fn pair(width: u32, height: u32) -> (Arc<Picture>, Arc<Picture>) {
    let left = base(width, height, 1);
    let mut right = left.clone();
    let pixels = u64::from(width) * u64::from(height);
    let target = pixels / 100;
    let mut rng = XorShift(2);
    let mut changed = 0u64;
    while changed < target / 2 {
        let (cx, cy) = (
            rng.below(u64::from(width)) as i64,
            rng.below(u64::from(height)) as i64,
        );
        let r = 5 + rng.below(30) as i64;
        for y in (cy - r).max(0)..(cy + r).min(i64::from(height)) {
            for x in (cx - r).max(0)..(cx + r).min(i64::from(width)) {
                if (x - cx).pow(2) + (y - cy).pow(2) <= r * r {
                    let at = ((y * i64::from(width) + x) * 4) as usize;
                    right[at] = right[at].wrapping_add(90);
                    changed += 1;
                }
            }
        }
    }
    while changed < target {
        let at = (rng.below(pixels) * 4) as usize;
        right[at + 1] = right[at + 1].wrapping_add(if rng.below(2) == 0 { 5 } else { 60 });
        changed += 1;
    }
    let pic = |rgba| Arc::new(Picture::from_rgba(width, height, rgba).unwrap());
    (pic(left), pic(right))
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

#[test]
#[ignore = "G11: release, idle machine, by hand"]
fn g11_diff_and_renders() {
    let (left, right) = pair(6000, 4000);
    let options = PictureOptions {
        tolerance: 8,
        geometry: Geometry::default(),
    };
    let colors = PictureColors::default();
    // A warm-up of the thread pool.
    let _ = compare_pictures(
        Arc::clone(&left),
        Arc::clone(&right),
        &options,
        &Ctl::detached(),
    )
    .unwrap();
    let (comparison, diff) = time(|| {
        compare_pictures(
            Arc::clone(&left),
            Arc::clone(&right),
            &options,
            &Ctl::detached(),
        )
        .unwrap()
    });
    let s = *comparison.stats();
    eprintln!(
        "G11 diff 24 MP: {:.1} ms (same {}, similar {}, different {})",
        diff.as_secs_f64() * 1e3,
        s.same,
        s.similar,
        s.different
    );
    assert!(s.different > 0 && s.similar > 0);
    assert!(diff < Duration::from_millis(500), "diff {diff:?}");
    for (pane, mode) in [
        (Pane::Difference, DiffMode::Tolerance),
        (Pane::Difference, DiffMode::Range),
        (Pane::Difference, DiffMode::Blend { percent: 50 }),
        (Pane::Left, DiffMode::Tolerance),
    ] {
        let (image, took) = time(|| {
            comparison
                .render(pane, mode, &colors, false, &Ctl::detached())
                .unwrap()
        });
        eprintln!(
            "G11 render {pane} {}: {:.1} ms",
            mode.as_str(),
            took.as_secs_f64() * 1e3
        );
        assert_eq!((image.width, image.height), (6000, 4000));
        assert!(
            took < Duration::from_millis(500),
            "render {pane} {mode:?} {took:?}"
        );
    }
    let (_, took) = time(|| {
        comparison
            .render(
                Pane::Difference,
                DiffMode::Tolerance,
                &colors,
                true,
                &Ctl::detached(),
            )
            .unwrap()
    });
    eprintln!(
        "G11 render with the checkerboard: {:.1} ms",
        took.as_secs_f64() * 1e3
    );
}

#[test]
#[ignore = "G11: release, idle machine, by hand"]
fn g11_geometry() {
    let (left, right) = pair(4000, 3000);
    let options = PictureOptions {
        tolerance: 8,
        geometry: Geometry::default(),
    };
    let comparison = compare_pictures(left, right, &options, &Ctl::detached()).unwrap();
    let mut worst = Duration::ZERO;
    let mut all = Vec::new();
    for step in 1..=20 {
        let geometry = Geometry {
            offset: (step, -step / 2),
            ..Geometry::default()
        };
        let (moved, took) = time(|| {
            comparison
                .with_geometry(geometry, &Ctl::detached())
                .unwrap()
        });
        assert!(moved.stats().pixels > 12_000_000);
        worst = worst.max(took);
        all.push(took.as_secs_f64() * 1e3);
    }
    all.sort_by(f64::total_cmp);
    eprintln!(
        "G11 geometry 2 × 12 MP: median {:.1} ms, worst {:.1} ms",
        all[all.len() / 2],
        worst.as_secs_f64() * 1e3
    );
    assert!(worst < Duration::from_millis(60), "worst {worst:?}");
}

#[test]
#[ignore = "G11: release, idle machine, by hand"]
fn g11_decode_24mp() {
    let rgba = base(6000, 4000, 3);
    let png = encode_png(
        &RgbaImage {
            width: 6000,
            height: 4000,
            rgba,
        },
        PngLevel::Fast,
    )
    .unwrap();
    let (picture, took) = time(|| {
        decode_bytes(
            &png,
            Some("x.png"),
            &DecodeOptions::default(),
            &Ctl::detached(),
        )
        .unwrap()
    });
    eprintln!(
        "G11 decode 24 MP PNG ({} MiB): {:.1} ms",
        png.len() >> 20,
        took.as_secs_f64() * 1e3
    );
    assert_eq!(picture.pixels(), 24_000_000);
    assert!(took < Duration::from_secs(1), "decode {took:?}");
}

/// Cancels after `delay` from another thread and returns how long the operation took past the cancel.
fn cancel_latency(delay: Duration, op: impl FnOnce(&Ctl<'_>) -> bool + Send) -> Duration {
    let token = CancelToken::new();
    std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let ctl = Ctl {
                cancel: &token,
                progress: &NullProgress,
            };
            let cancelled = op(&ctl);
            (cancelled, Instant::now())
        });
        std::thread::sleep(delay);
        let at = Instant::now();
        token.cancel();
        let (cancelled, ended) = worker.join().unwrap();
        assert!(
            cancelled,
            "the operation ended before the cancel: lower the delay"
        );
        ended.saturating_duration_since(at)
    })
}

#[test]
#[ignore = "G4/A12.18: release, idle machine, by hand"]
fn cancel_within_100_ms() {
    let (left, right) = pair(6000, 4000);
    let options = PictureOptions {
        tolerance: 8,
        geometry: Geometry::default(),
    };
    let comparison = compare_pictures(
        Arc::clone(&left),
        Arc::clone(&right),
        &options,
        &Ctl::detached(),
    )
    .unwrap();
    let colors = PictureColors::default();
    // One thread so the operations take long enough to be cancelled midway.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let diff = cancel_latency(Duration::from_millis(20), |ctl| {
        pool.install(|| {
            compare_pictures(Arc::clone(&left), Arc::clone(&right), &options, ctl).is_err()
        })
    });
    let render = cancel_latency(Duration::from_millis(20), |ctl| {
        pool.install(|| {
            comparison
                .render(Pane::Difference, DiffMode::Tolerance, &colors, true, ctl)
                .is_err()
        })
    });
    eprintln!(
        "cancel: diff {:.1} ms, render {:.1} ms",
        diff.as_secs_f64() * 1e3,
        render.as_secs_f64() * 1e3
    );
    assert!(diff < Duration::from_millis(100) && render < Duration::from_millis(100));
}
