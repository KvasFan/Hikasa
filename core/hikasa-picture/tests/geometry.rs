//! A12.14 (D-340): geometry — the composition table of `Orientation` and its inverses, `parse_list`, Auto Scale's
//! goldens and side rule, `aspect_ratio_differs`, offsets in every direction, P-GEOM and P-PICMIRROR.

mod common;

use std::sync::Arc;

use common::{
    THREADS, XorShift, compare, flat, in_pool, noise, pic, random_geometry, small_picture,
};
use hikasa_core::{Ctl, Pair, Side};
use hikasa_picture::geometry::{aspect_ratio_differs, orient, scale_bilinear, scaled_side};
use hikasa_picture::{
    DiffMode, Geometry, Orientation, Pane, PictureColors, PictureWarningCode, Rect, RgbaImage,
};
use proptest::prelude::*;

/// The 8 × 8 composition table: row `a`, column `b`, cell `a.then(b)` (rotate or flip `a` first, then `b`).
const TABLE: &str = "\
none cw 180 ccw flip-h flip-v transpose transverse
cw 180 ccw none transpose transverse flip-v flip-h
180 ccw none cw flip-v flip-h transverse transpose
ccw none cw 180 transverse transpose flip-h flip-v
flip-h transverse flip-v transpose none 180 ccw cw
flip-v transpose flip-h transverse 180 none cw ccw
transpose flip-h transverse flip-v cw ccw none 180
transverse flip-v transpose flip-h ccw cw 180 none";

#[test]
fn composition_table_and_inverses() {
    let got: Vec<String> = Orientation::ALL
        .iter()
        .map(|a| {
            Orientation::ALL
                .iter()
                .map(|b| a.then(*b).as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    assert_eq!(got.join("\n"), TABLE);
    for o in Orientation::ALL {
        assert_eq!(o.then(o.inverse()), Orientation::Identity, "{o}");
        assert_eq!(o.inverse().then(o), Orientation::Identity, "{o}");
    }
    let inverses: Vec<&str> = Orientation::ALL
        .iter()
        .map(|o| o.inverse().as_str())
        .collect();
    assert_eq!(
        inverses,
        [
            "none",
            "ccw",
            "180",
            "cw",
            "flip-h",
            "flip-v",
            "transpose",
            "transverse"
        ]
    );
}

#[test]
fn parse_list_of_every_word_and_lists() {
    for o in Orientation::ALL {
        assert_eq!(Orientation::parse_list(o.as_str()), Ok(o));
        assert_eq!(Orientation::parse(&o.as_str().to_ascii_uppercase()), Ok(o));
    }
    for (text, want) in [
        ("cw,flip-h", Orientation::Transpose),
        ("cw, cw", Orientation::Rot180),
        ("cw,cw,cw,cw", Orientation::Identity),
        ("flip-h,flip-v", Orientation::Rot180),
        ("ccw,flip-h", Orientation::Transverse),
        ("identity,90", Orientation::Cw90),
        ("270", Orientation::Ccw90),
        ("rot180,fliph,flipv", Orientation::Identity),
        ("none", Orientation::Identity),
    ] {
        assert_eq!(Orientation::parse_list(text), Ok(want), "{text}");
    }
    for bad in ["", " ", "cw,", "left", "cw,,ccw", "45"] {
        assert!(Orientation::parse_list(bad).is_err(), "{bad:?}");
    }
}

/// The 3 × 2 source of the first Auto Scale golden.
fn source_3x2() -> RgbaImage {
    let rgba = (0..6u32)
        .flat_map(|i| {
            [
                (i * 50 % 256) as u8,
                (255 - i * 40) as u8,
                (i * i * 20 % 256) as u8,
                if i % 2 == 0 { 255 } else { 128 },
            ]
        })
        .collect();
    RgbaImage {
        width: 3,
        height: 2,
        rgba,
    }
}

/// The 4 × 4 source of the second.
fn source_4x4() -> RgbaImage {
    let rgba = (0..16u32)
        .flat_map(|i| {
            [
                (i * 16) as u8,
                (255 - i * 16) as u8,
                (i * 37 % 256) as u8,
                255,
            ]
        })
        .collect();
    RgbaImage {
        width: 4,
        height: 4,
        rgba,
    }
}

const GOLDEN_7X5: [u8; 140] = [
    0, 255, 0, 255, 7, 249, 3, 237, 29, 232, 11, 182, 50, 215, 20, 128, 71, 198, 46, 182, 93, 181,
    71, 237, 100, 175, 80, 255, 15, 243, 18, 242, 22, 237, 19, 228, 44, 220, 22, 184, 65, 203, 24,
    141, 86, 186, 55, 184, 108, 169, 86, 228, 115, 163, 96, 242, 75, 195, 90, 192, 82, 189, 83,
    192, 104, 172, 63, 192, 125, 155, 42, 192, 146, 138, 93, 192, 168, 121, 145, 192, 175, 115,
    162, 192, 135, 147, 162, 141, 142, 141, 147, 155, 164, 124, 103, 199, 185, 107, 60, 242, 206,
    90, 132, 199, 228, 73, 204, 155, 235, 67, 228, 141, 150, 135, 180, 128, 157, 129, 163, 146,
    179, 112, 114, 201, 200, 95, 64, 255, 221, 78, 141, 201, 243, 61, 218, 146, 250, 55, 244, 128,
];

const GOLDEN_4X9: [u8; 144] = [
    0, 255, 0, 255, 16, 239, 37, 255, 32, 223, 74, 255, 48, 207, 111, 255, 11, 244, 25, 255, 27,
    228, 62, 255, 43, 212, 99, 255, 59, 196, 93, 255, 39, 216, 90, 255, 55, 200, 127, 255, 71, 184,
    164, 255, 87, 168, 45, 255, 68, 187, 142, 255, 84, 171, 179, 255, 100, 155, 216, 255, 116, 139,
    11, 255, 96, 159, 94, 255, 112, 143, 131, 255, 128, 127, 168, 255, 144, 111, 77, 255, 124, 131,
    46, 255, 140, 115, 83, 255, 156, 99, 120, 255, 172, 83, 143, 255, 153, 102, 98, 255, 169, 86,
    135, 255, 185, 70, 72, 255, 201, 54, 109, 255, 181, 74, 163, 255, 197, 58, 200, 255, 213, 42,
    24, 255, 229, 26, 61, 255, 192, 63, 188, 255, 208, 47, 225, 255, 224, 31, 6, 255, 240, 15, 43,
    255,
];

/// A12.14: Auto Scale's exact bytes (computed from §1.12.17's formulas independently), directly and through a
/// comparison's pane; the scaled side is the one with fewer pixels, the right one on a tie.
#[test]
fn auto_scale_goldens_and_the_side_rule() {
    assert_eq!(scale_bilinear(&source_3x2(), 7, 5).rgba, GOLDEN_7X5);
    assert_eq!(scale_bilinear(&source_4x4(), 4, 9).rgba, GOLDEN_4X9);
    // Scaling to the same size is the identity.
    assert_eq!(scale_bilinear(&source_4x4(), 4, 4), source_4x4());
    let small = pic(3, 2, source_3x2().rgba);
    let big = flat(7, 5, [0, 0, 0, 255]);
    let geometry = Geometry {
        auto_scale: true,
        ..Geometry::default()
    };
    let comparison = compare(&small, &big, 0, geometry);
    assert_eq!(comparison.scaled(), Some((Side::Left, (7, 5))));
    let left = comparison
        .render(
            Pane::Left,
            DiffMode::Tolerance,
            &PictureColors::default(),
            false,
            &Ctl::detached(),
        )
        .unwrap();
    assert_eq!(left.rgba, GOLDEN_7X5);
    assert_eq!(comparison.stats().pixels, 35);
    assert_eq!(
        comparison.area(),
        Rect {
            x: 0,
            y: 0,
            width: 7,
            height: 5
        }
    );
    // Swapped: the right one is scaled.
    let comparison = compare(&big, &small, 0, geometry);
    assert_eq!(comparison.scaled(), Some((Side::Right, (7, 5))));
    // Off: no scaling, the area is the union.
    let comparison = compare(&small, &big, 0, Geometry::default());
    assert_eq!(comparison.scaled(), None);
    assert_eq!(
        (comparison.stats().left_only, comparison.stats().right_only),
        (0, 29)
    );
    // A tie (2 × 8 and 4 × 4): the right one.
    assert_eq!(scaled_side((2, 8), (4, 4)), Some(Side::Right));
    assert_eq!(scaled_side((4, 4), (2, 8)), Some(Side::Right));
    assert_eq!(scaled_side((3, 3), (4, 4)), Some(Side::Left));
    assert_eq!(scaled_side((4, 4), (4, 4)), None);
    // The orientation applies before the scale.
    let tall = pic(2, 3, source_3x2().rgba);
    let geometry = Geometry {
        orientation: Pair::new(Orientation::Cw90, Orientation::Identity),
        auto_scale: true,
        offset: (0, 0),
    };
    let comparison = compare(&tall, &big, 0, geometry);
    assert_eq!(comparison.scaled(), Some((Side::Left, (7, 5))));
    let rotated = orient(2, 3, &source_3x2().rgba, Orientation::Cw90);
    let left = comparison
        .render(
            Pane::Left,
            DiffMode::Tolerance,
            &PictureColors::default(),
            false,
            &Ctl::detached(),
        )
        .unwrap();
    assert_eq!(left, scale_bilinear(&rotated, 7, 5));
}

#[test]
fn aspect_ratio_differs_past_one_percent() {
    assert!(!aspect_ratio_differs((100, 100), (200, 200)));
    assert!(!aspect_ratio_differs((101, 100), (100, 100)), "exactly 1 %");
    assert!(aspect_ratio_differs((102, 100), (100, 100)));
    assert!(aspect_ratio_differs((4, 3), (16, 9)));
    let warned = |a: (u32, u32), b: (u32, u32)| {
        let geometry = Geometry {
            auto_scale: true,
            ..Geometry::default()
        };
        compare(
            &flat(a.0, a.1, [1, 2, 3, 255]),
            &flat(b.0, b.1, [1, 2, 3, 255]),
            0,
            geometry,
        )
        .warnings()
        .iter()
        .any(|w| w.code == PictureWarningCode::AspectRatioDiffers)
    };
    assert!(warned((40, 30), (160, 90)));
    assert!(!warned((40, 30), (80, 60)));
    // Without Auto Scale nothing is resampled and nothing is said.
    assert!(
        compare(
            &flat(4, 3, [0; 4]),
            &flat(16, 9, [0; 4]),
            0,
            Geometry::default()
        )
        .warnings()
        .is_empty()
    );
}

/// A12.14: offsets positive, negative and beyond either picture (empty overlap).
#[test]
fn offsets_in_every_direction() {
    let left = flat(4, 3, [10, 10, 10, 255]);
    let right = flat(2, 2, [10, 10, 10, 255]);
    let at = |dx: i32, dy: i32| {
        compare(
            &left,
            &right,
            0,
            Geometry {
                offset: (dx, dy),
                ..Geometry::default()
            },
        )
    };
    // Inside: the area is the left picture.
    let c = at(1, 1);
    assert_eq!(
        c.area(),
        Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 3
        }
    );
    assert_eq!(
        c.placement(Side::Right),
        Some(Rect {
            x: 1,
            y: 1,
            width: 2,
            height: 2
        })
    );
    let s = *c.stats();
    assert_eq!((s.pixels, s.left_only, s.right_only, s.same), (12, 8, 0, 4));
    // Negative, overlapping one pixel.
    let c = at(-1, -1);
    assert_eq!(
        c.area(),
        Rect {
            x: -1,
            y: -1,
            width: 5,
            height: 4
        }
    );
    assert_eq!(
        c.placement(Side::Left),
        Some(Rect {
            x: 1,
            y: 1,
            width: 4,
            height: 3
        })
    );
    assert_eq!(
        c.placement(Side::Right),
        Some(Rect {
            x: 0,
            y: 0,
            width: 2,
            height: 2
        })
    );
    let s = *c.stats();
    assert_eq!((s.pixels, s.left_only, s.right_only), (15, 11, 3));
    let probe = c.probe(1, 1);
    assert_eq!(probe.left.map(|l| (l.0, l.1)), Some((0, 0)));
    assert_eq!(probe.right.map(|r| (r.0, r.1)), Some((1, 1)));
    // Beyond either picture: no overlap, an empty gap between them.
    for (dx, dy) in [(10, 0), (0, -7), (-5, -10), (4, 3)] {
        let c = at(dx, dy);
        let s = *c.stats();
        assert_eq!(
            (s.pixels, s.left_only, s.right_only),
            (16, 12, 4),
            "{dx},{dy}"
        );
        assert_eq!(s.different, 16, "every one-sided opaque pixel differs");
    }
    let c = at(10, 0);
    assert_eq!(
        c.area(),
        Rect {
            x: 0,
            y: 0,
            width: 12,
            height: 3
        }
    );
    assert_eq!(c.probe(6, 0).class, hikasa_picture::PixelClass::Outside);
    // The limit is ±2²⁰.
    let far = (1 << 20, -(1 << 20));
    let c = at(far.0, far.1);
    assert_eq!(c.stats().pixels, 16);
    let err = hikasa_picture::compare_pictures(
        Arc::clone(&left),
        Arc::clone(&right),
        &hikasa_picture::PictureOptions {
            tolerance: 0,
            geometry: Geometry {
                offset: ((1 << 20) + 1, 0),
                ..Geometry::default()
            },
        },
        &Ctl::detached(),
    )
    .unwrap_err();
    assert_eq!(
        hikasa_core::Classify::kind(&err),
        hikasa_core::ErrorKind::InvalidInput
    );
}

/// `with_geometry` keeps the pictures shared and equals a fresh comparison.
#[test]
fn with_geometry_equals_a_fresh_comparison() {
    let mut rng = XorShift(77);
    for _ in 0..50 {
        let left = small_picture(&mut rng, 10);
        let right = small_picture(&mut rng, 10);
        let scale = rng.below(2) == 0;
        let first = random_geometry(&mut rng, scale);
        let second = random_geometry(&mut rng, first.auto_scale);
        let tolerance = rng.next() as u8;
        let base = compare(&left, &right, tolerance, first);
        let moved = base.with_geometry(second, &Ctl::detached()).unwrap();
        let fresh = compare(&left, &right, tolerance, second);
        assert_eq!(moved.stats(), fresh.stats());
        assert_eq!(moved.area(), fresh.area());
        assert_eq!(moved.scaled(), fresh.scaled());
        assert!(Arc::ptr_eq(&moved.pictures().left, &left));
        // An offset change after a scale reuses the scaled copy and still equals a fresh comparison.
        let nudged = Geometry {
            offset: (second.offset.0 + 1, second.offset.1 - 1),
            ..second
        };
        assert_eq!(
            moved
                .with_geometry(nudged, &Ctl::detached())
                .unwrap()
                .stats(),
            compare(&left, &right, tolerance, nudged).stats()
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// P-GEOM: a picture oriented (materialized) and compared under the inverse orientation is `rules_same` with
    /// the original.
    #[test]
    fn p_geom(seed in any::<u64>(), which in 0usize..8) {
        let mut rng = XorShift(seed);
        let (w, h) = (1 + rng.below(11) as u32, 1 + rng.below(11) as u32);
        let original = noise(w, h, seed);
        let o = Orientation::ALL[which];
        let oriented = orient(w, h, &original.rgba, o);
        let turned = pic(oriented.width, oriented.height, oriented.rgba);
        let geometry = Geometry {
            orientation: Pair::new(Orientation::Identity, o.inverse()),
            ..Geometry::default()
        };
        for threads in THREADS {
            let c = in_pool(threads, || compare(&original, &turned, 0, geometry));
            let s = *c.stats();
            prop_assert_eq!((s.pixels, s.same), (u64::from(w * h), u64::from(w * h)), "{} threads {}", o, threads);
            let v = c.verdict(false, || Ok::<_, std::convert::Infallible>(false)).unwrap();
            prop_assert_eq!(v.kind, hikasa_core::ContentKind::RulesSame);
        }
    }

    /// P-PICMIRROR: swapping sides with the offset negated and the orientations swapped mirrors the statistics.
    #[test]
    fn p_picmirror(seed in any::<u64>(), tolerance in any::<u8>(), auto_scale in any::<bool>()) {
        let mut rng = XorShift(seed);
        let left = small_picture(&mut rng, 12);
        let right = small_picture(&mut rng, 12);
        let geometry = random_geometry(&mut rng, auto_scale);
        let sizes = |g: &Geometry| (
            g.orientation.left.oriented_size(left.width, left.height),
            g.orientation.right.oriented_size(right.width, right.height),
        );
        let (ls, rs) = sizes(&geometry);
        // Auto Scale's tie rule (the right side on equal pixel counts) is the one asymmetry.
        prop_assume!(!auto_scale || ls == rs || u64::from(ls.0 * ls.1) != u64::from(rs.0 * rs.1));
        for threads in THREADS {
            let (a, b) = in_pool(threads, || {
                (
                    *compare(&left, &right, tolerance, geometry).stats(),
                    *compare(&right, &left, tolerance, geometry.swapped()).stats(),
                )
            });
            prop_assert_eq!((a.same, a.similar, a.different), (b.same, b.similar, b.different));
            prop_assert_eq!((a.left_only, a.right_only), (b.right_only, b.left_only));
            prop_assert_eq!((a.pixels, a.max_distance, a.distance_sum, a.bounds), (b.pixels, b.max_distance, b.distance_sum, b.bounds));
        }
    }
}
