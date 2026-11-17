//! A12.15 (D-341): rendering — each mode's pixel formulas on 4 × 4 goldens (computed independently from §1.12.18's
//! rules), the checkerboard's cells and anchoring, and P-TILES (a full render equals its tiles assembled, for
//! random sizes, tile edges, levels 0–3 and thread counts; a level-k tile equals the box filter of level 0).

mod common;

use std::sync::Arc;

use common::{THREADS, XorShift, compare, flat, in_pool, pic, small_picture};
use hikasa_core::{CancelToken, Ctl, NullProgress, Pair};
use hikasa_picture::{
    DiffMode, Geometry, Orientation, Pane, Picture, PictureColors, PictureComparison, Rgba8,
    RgbaImage, TileRect, composite_checkerboard,
};
use proptest::prelude::*;

/// The left picture of the goldens: pixel `i = 4y + x` is `(37i, 91i + 7, 53i + 11, i % 5 == 0 ? 0 : 255)`.
fn left_px(x: u32, y: u32) -> [u8; 4] {
    let i = y * 4 + x;
    [
        (i * 37 % 256) as u8,
        ((i * 91 + 7) % 256) as u8,
        ((i * 53 + 11) % 256) as u8,
        if i.is_multiple_of(5) { 0 } else { 255 },
    ]
}

/// The right picture, placed at offset (1, 0): the left's next pixel, red +5 (similar at 8) for `i % 3 == 0`, +40
/// (different) for `i % 3 == 1`.
fn right_px(x: u32, y: u32) -> [u8; 4] {
    let mut p = left_px(x + 1, y);
    let i = y * 4 + x;
    match i % 3 {
        0 => p[0] = p[0].saturating_add(5),
        1 => p[0] = p[0].wrapping_add(40),
        _ => {}
    }
    p
}

fn golden_comparison() -> PictureComparison {
    let picture =
        |f: fn(u32, u32) -> [u8; 4]| pic(4, 4, (0..16).flat_map(|i| f(i % 4, i / 4)).collect());
    let geometry = Geometry {
        offset: (1, 0),
        ..Geometry::default()
    };
    compare(&picture(left_px), &picture(right_px), 8, geometry)
}

const TOLERANCE: [u8; 80] = [
    8, 42, 23, 255, 217, 164, 0, 255, 198, 40, 40, 255, 14, 70, 38, 255, 198, 40, 40, 255, 198, 40,
    40, 255, 24, 122, 66, 255, 17, 85, 46, 255, 217, 164, 0, 255, 198, 40, 40, 255, 198, 40, 40,
    255, 15, 78, 42, 255, 19, 97, 53, 255, 198, 40, 40, 255, 198, 40, 40, 255, 198, 40, 40, 255,
    217, 164, 0, 255, 198, 40, 40, 255, 14, 72, 39, 255, 198, 40, 40, 255,
];

const RANGE: [u8; 80] = [
    0, 0, 0, 255, 36, 36, 0, 255, 67, 67, 0, 255, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255,
    0, 0, 0, 255, 0, 0, 0, 255, 36, 36, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 0, 0, 0, 255,
    0, 0, 0, 255, 67, 67, 0, 255, 255, 255, 0, 255, 255, 255, 0, 255, 36, 36, 0, 255, 67, 67, 0,
    255, 0, 0, 0, 255, 255, 255, 0, 255,
];

const BLEND_30: [u8; 80] = [
    0, 5, 8, 0, 39, 98, 64, 255, 86, 189, 117, 255, 111, 24, 170, 255, 46, 35, 67, 77, 104, 81,
    156, 179, 197, 206, 20, 0, 222, 41, 73, 255, 5, 132, 126, 255, 24, 67, 54, 77, 28, 156, 125,
    179, 77, 58, 232, 255, 116, 149, 29, 0, 163, 240, 82, 255, 56, 23, 41, 77, 132, 53, 95, 179,
    227, 166, 188, 255, 18, 1, 241, 255, 43, 92, 38, 0, 26, 55, 27, 77,
];

const LEFT: [u8; 80] = [
    0, 7, 11, 0, 37, 98, 64, 255, 74, 189, 117, 255, 111, 24, 170, 255, 0, 0, 0, 0, 148, 115, 223,
    255, 185, 206, 20, 0, 222, 41, 73, 255, 3, 132, 126, 255, 0, 0, 0, 0, 40, 223, 179, 255, 77,
    58, 232, 255, 114, 149, 29, 0, 151, 240, 82, 255, 0, 0, 0, 0, 188, 75, 135, 255, 225, 166, 188,
    255, 6, 1, 241, 255, 43, 92, 38, 0, 0, 0, 0, 0,
];

const RIGHT: [u8; 80] = [
    0, 0, 0, 0, 42, 98, 64, 255, 114, 189, 117, 255, 111, 24, 170, 255, 153, 115, 223, 255, 0, 0,
    0, 0, 225, 206, 20, 0, 222, 41, 73, 255, 8, 132, 126, 255, 80, 223, 179, 255, 0, 0, 0, 0, 77,
    58, 232, 255, 119, 149, 29, 0, 191, 240, 82, 255, 188, 75, 135, 255, 0, 0, 0, 0, 230, 166, 188,
    255, 46, 1, 241, 255, 43, 92, 38, 0, 85, 183, 91, 255,
];

fn render(c: &PictureComparison, pane: Pane, mode: DiffMode, checkerboard: bool) -> RgbaImage {
    c.render(
        pane,
        mode,
        &PictureColors::default(),
        checkerboard,
        &Ctl::detached(),
    )
    .unwrap()
}

#[test]
fn every_mode_on_the_golden_pair() {
    let c = golden_comparison();
    assert_eq!((c.area().width, c.area().height), (5, 4));
    let blend = DiffMode::Blend { percent: 30 };
    for (pane, mode, want) in [
        (Pane::Difference, DiffMode::Tolerance, &TOLERANCE),
        (Pane::Difference, DiffMode::Range, &RANGE),
        (Pane::Difference, blend, &BLEND_30),
        (Pane::Left, DiffMode::Tolerance, &LEFT),
        (Pane::Right, DiffMode::Range, &RIGHT),
    ] {
        let image = render(&c, pane, mode, false);
        assert_eq!((image.width, image.height), (5, 4));
        assert_eq!(image.rgba, want.to_vec(), "{pane} {mode:?}");
    }
    // The display mode never changes the classes.
    let stats = *c.stats();
    assert_eq!((stats.same, stats.similar, stats.different), (7, 3, 10));
}

#[test]
fn range_floor_and_blend_ends() {
    let a = flat(1, 1, [100, 0, 0, 255]);
    let b = flat(1, 1, [101, 0, 0, 255]);
    let c = compare(&a, &b, 0, Geometry::default());
    assert_eq!(
        render(&c, Pane::Difference, DiffMode::Range, false).rgba,
        [33, 33, 0, 255]
    );
    let full = compare(
        &flat(1, 1, [0, 0, 0, 255]),
        &flat(1, 1, [255, 0, 0, 255]),
        0,
        Geometry::default(),
    );
    assert_eq!(
        render(&full, Pane::Difference, DiffMode::Range, false).rgba,
        [255, 255, 0, 255]
    );
    // Blend 0 is the left picture, 100 the right one (F8's toggle).
    assert_eq!(
        render(&c, Pane::Difference, DiffMode::Blend { percent: 0 }, false).rgba,
        [100, 0, 0, 255]
    );
    assert_eq!(
        render(
            &c,
            Pane::Difference,
            DiffMode::Blend { percent: 100 },
            false
        )
        .rgba,
        [101, 0, 0, 255]
    );
    // White `same` gives shades of grey: luma 0 → 64, luma 255 → 255.
    let colors = PictureColors {
        same: Rgba8::rgb(0xffffff),
        ..PictureColors::default()
    };
    let shades = |p: [u8; 4]| {
        let c = compare(&flat(1, 1, p), &flat(1, 1, p), 0, Geometry::default());
        c.render(
            Pane::Difference,
            DiffMode::Tolerance,
            &colors,
            false,
            &Ctl::detached(),
        )
        .unwrap()
        .rgba
    };
    assert_eq!(shades([0, 0, 0, 255]), [64, 64, 64, 255]);
    assert_eq!(shades([255, 255, 255, 255]), [255, 255, 255, 255]);
    assert_eq!(
        shades([255, 0, 0, 255]),
        [122, 122, 122, 255],
        "Y(255, 0, 0) = 77"
    );
}

/// The checkerboard: 8-pixel cells anchored at the area's origin, transparent pixels composited source-over.
#[test]
fn checkerboard_cells_and_anchoring() {
    let colors = PictureColors {
        checker_light: Rgba8::rgb(0xffffff),
        checker_dark: Rgba8::rgb(0x000000),
        ..PictureColors::default()
    };
    let mut tile = RgbaImage::transparent(20, 10);
    composite_checkerboard(&mut tile, (0, 0), &colors, 8);
    let at = |t: &RgbaImage, x, y| t.pixel(x, y).unwrap().to_array();
    assert_eq!(at(&tile, 0, 0), [255, 255, 255, 255]);
    assert_eq!(at(&tile, 7, 7), [255, 255, 255, 255]);
    assert_eq!(at(&tile, 8, 0), [0, 0, 0, 255]);
    assert_eq!(at(&tile, 0, 8), [0, 0, 0, 255]);
    assert_eq!(at(&tile, 8, 8), [255, 255, 255, 255]);
    assert_eq!(at(&tile, 16, 9), [0, 0, 0, 255]);
    // Anchored at the area: a tile at (4, 0) starts in the middle of the first cell.
    let mut shifted = RgbaImage::transparent(8, 1);
    composite_checkerboard(&mut shifted, (4, 0), &colors, 8);
    let row: Vec<u8> = shifted.rgba.chunks(4).map(|c| c[0]).collect();
    assert_eq!(row, [255, 255, 255, 255, 0, 0, 0, 0]);
    // And in y: a tile at (0, 12) starts in the second row of cells (dark at x 0).
    let mut lower = RgbaImage::transparent(1, 8);
    composite_checkerboard(&mut lower, (0, 12), &colors, 8);
    let column: Vec<u8> = lower.rgba.chunks(4).map(|c| c[0]).collect();
    assert_eq!(column, [0, 0, 0, 0, 255, 255, 255, 255]);
    // Source-over: half-transparent red on white and on black; opaque pixels unchanged.
    let mut px = RgbaImage {
        width: 3,
        height: 1,
        rgba: vec![255, 0, 0, 128, 255, 0, 0, 128, 1, 2, 3, 255],
    };
    composite_checkerboard(&mut px, (7, 0), &colors, 8);
    assert_eq!(px.rgba, [255, 127, 127, 255, 128, 0, 0, 255, 1, 2, 3, 255]);
    // render(checkerboard) composites the same way, bands included.
    let c = compare(
        &flat(3, 70, [0, 0, 255, 0]),
        &flat(3, 70, [0, 0, 255, 0]),
        0,
        Geometry::default(),
    );
    let mut plain = render(&c, Pane::Left, DiffMode::Tolerance, false);
    composite_checkerboard(&mut plain, (0, 0), &PictureColors::default(), 8);
    assert_eq!(render(&c, Pane::Left, DiffMode::Tolerance, true), plain);
}

/// The box filter of a level-0 image: each level-k pixel the mean (round half up) of its block clipped to the image.
fn box_filter(image: &RgbaImage, k: u32) -> RgbaImage {
    let s = 1u32 << k;
    let (w, h) = (image.width.div_ceil(s), image.height.div_ceil(s));
    let mut out = RgbaImage::transparent(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u64; 4];
            let mut n = 0u64;
            for by in y * s..((y + 1) * s).min(image.height) {
                for bx in x * s..((x + 1) * s).min(image.width) {
                    let p = image.pixel(bx, by).unwrap().to_array();
                    for c in 0..4 {
                        sum[c] += u64::from(p[c]);
                    }
                    n += 1;
                }
            }
            let at = ((y * w + x) * 4) as usize;
            for (slot, total) in out.rgba[at..at + 4].iter_mut().zip(sum) {
                *slot = ((2 * total + n) / (2 * n)) as u8;
            }
        }
    }
    out
}

/// Assembles the tiles of `size` covering the level's area.
fn assembled(
    c: &PictureComparison,
    pane: Pane,
    mode: DiffMode,
    size: (u32, u32),
    level: u8,
) -> RgbaImage {
    let (w, h) = c.level_size(level);
    let mut out = RgbaImage::transparent(w, h);
    let colors = PictureColors::default();
    for ty in (0..h).step_by(size.1 as usize) {
        for tx in (0..w).step_by(size.0 as usize) {
            let rect = TileRect {
                x: tx,
                y: ty,
                width: size.0.min(w - tx),
                height: size.1.min(h - ty),
            };
            let tile = c.render_tile(pane, mode, &colors, rect, level);
            assert_eq!((tile.width, tile.height), (rect.width, rect.height));
            for row in 0..rect.height {
                let from = (row * rect.width * 4) as usize;
                let to = (((ty + row) * w + tx) * 4) as usize;
                out.rgba[to..to + (rect.width * 4) as usize]
                    .copy_from_slice(&tile.rgba[from..from + (rect.width * 4) as usize]);
            }
        }
    }
    out
}

/// A tile past the area is transparent; one straddling the edge is transparent beyond it.
#[test]
fn tiles_beyond_the_area() {
    let c = golden_comparison();
    let colors = PictureColors::default();
    let tile = c.render_tile(
        Pane::Left,
        DiffMode::Tolerance,
        &colors,
        TileRect {
            x: 3,
            y: 2,
            width: 4,
            height: 4,
        },
        0,
    );
    let full = render(&c, Pane::Left, DiffMode::Tolerance, false);
    assert_eq!(tile.pixel(0, 0), full.pixel(3, 2));
    assert_eq!(tile.pixel(1, 1), full.pixel(4, 3));
    assert_eq!(tile.pixel(2, 0).unwrap(), Rgba8::TRANSPARENT);
    assert_eq!(tile.pixel(0, 2).unwrap(), Rgba8::TRANSPARENT);
    let far = c.render_tile(
        Pane::Difference,
        DiffMode::Range,
        &colors,
        TileRect {
            x: 100,
            y: 100,
            width: 3,
            height: 3,
        },
        2,
    );
    assert!(far.rgba.iter().all(|&b| b == 0));
    assert_eq!(c.level_size(0), (5, 4));
    assert_eq!(c.level_size(1), (3, 2));
    assert_eq!(c.level_size(3), (1, 1));
}

/// Rendering is cancellable; an area above the limit is refused.
#[test]
fn render_cancel_and_limit() {
    let token = CancelToken::new();
    token.cancel();
    let ctl = Ctl {
        cancel: &token,
        progress: &NullProgress,
    };
    let c = golden_comparison();
    assert!(
        c.render(
            Pane::Difference,
            DiffMode::Tolerance,
            &PictureColors::default(),
            false,
            &ctl
        )
        .is_err()
    );
    let far = compare(
        &flat(1, 1, [0; 4]),
        &flat(1, 1, [0; 4]),
        0,
        Geometry {
            offset: (1 << 20, 1 << 20),
            ..Geometry::default()
        },
    );
    let err = far
        .render(
            Pane::Difference,
            DiffMode::Tolerance,
            &PictureColors::default(),
            false,
            &Ctl::detached(),
        )
        .unwrap_err();
    assert_eq!(
        hikasa_core::Classify::kind(&err),
        hikasa_core::ErrorKind::TooLarge
    );
}

fn random_comparison(rng: &mut XorShift) -> (Arc<Picture>, Arc<Picture>, Geometry, u8) {
    let big = rng.below(4) == 0;
    let (left, right) = if big {
        let mk = |rng: &mut XorShift| {
            let (w, h) = (1 + rng.below(700) as u32, 1 + rng.below(300) as u32);
            common::noise(w, h, rng.next())
        };
        (mk(rng), mk(rng))
    } else {
        (small_picture(rng, 40), small_picture(rng, 40))
    };
    let o = |rng: &mut XorShift| Orientation::ALL[rng.below(8) as usize];
    let geometry = Geometry {
        orientation: Pair::new(o(rng), o(rng)),
        auto_scale: rng.below(3) == 0,
        offset: (rng.below(61) as i32 - 30, rng.below(61) as i32 - 30),
    };
    (left, right, geometry, rng.next() as u8)
}

fn mode_of(i: u64) -> (Pane, DiffMode) {
    match i % 5 {
        0 => (Pane::Difference, DiffMode::Tolerance),
        1 => (Pane::Difference, DiffMode::Range),
        2 => (
            Pane::Difference,
            DiffMode::Blend {
                percent: (i / 5 % 101) as u8,
            },
        ),
        3 => (Pane::Left, DiffMode::Tolerance),
        _ => (Pane::Right, DiffMode::Range),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// P-TILES: a full render equals its tiles assembled (random sizes up to 700 × 300, tile sizes that cut the
    /// area anywhere, levels 0–3, 1/4/16 threads), and a level-k tile equals the box filter of the level-0 render.
    #[test]
    fn p_tiles(seed in any::<u64>()) {
        let mut rng = XorShift(seed);
        let (left, right, geometry, tolerance) = random_comparison(&mut rng);
        let (pane, mode) = mode_of(rng.next());
        let c = compare(&left, &right, tolerance, geometry);
        let full = in_pool(1, || render(&c, pane, mode, false));
        for threads in THREADS {
            let again = in_pool(threads, || render(&c, pane, mode, false));
            prop_assert!(again == full, "render with {} threads", threads);
        }
        let size = (1 + rng.below(300) as u32, 1 + rng.below(300) as u32);
        prop_assert!(assembled(&c, pane, mode, size, 0) == full, "tiles {:?}", size);
        prop_assert!(assembled(&c, pane, mode, (256, 256), 0) == full);
        let level = 1 + rng.below(3) as u8;
        let size = (1 + rng.below(80) as u32, 1 + rng.below(80) as u32);
        let tiles = assembled(&c, pane, mode, size, level);
        prop_assert!(tiles == box_filter(&full, u32::from(level)), "level {} tiles {:?}", level, size);
    }
}
