//! A12.16 (D-342): statistics and the probe — counts, percentages with 4 decimals, the largest and mean distance,
//! the bounds of the non-same positions, `with_geometry` against a fresh comparison, the probe at corners, outside
//! the pictures and in an uncovered region.

mod common;

use common::{compare, flat, pic};
use hikasa_core::Ctl;
use hikasa_picture::{Geometry, PixelClass, PixelProbe, Rect, Rgba8};

fn golden() -> hikasa_picture::PictureComparison {
    let base = [50u8, 50, 50, 255];
    let left = flat(6, 5, base);
    let mut rgba = base.repeat(30);
    let mut set = |x: usize, y: usize, red: u8| rgba[(y * 6 + x) * 4] = red;
    set(1, 1, 53); // d = 3: different at tolerance 2
    set(4, 3, 150); // d = 100
    set(2, 4, 51); // d = 1, on the last row: similar
    compare(&left, &pic(6, 5, rgba), 2, Geometry::default())
}

#[test]
fn counts_percentages_distances_bounds() {
    let c = golden();
    let s = *c.stats();
    assert_eq!(
        (
            s.pixels,
            s.same,
            s.similar,
            s.different,
            s.left_only,
            s.right_only
        ),
        (30, 27, 1, 2, 0, 0)
    );
    assert_eq!((s.max_distance, s.distance_sum), (100, 104));
    assert_eq!(
        s.bounds,
        Some(Rect {
            x: 1,
            y: 1,
            width: 4,
            height: 4
        })
    );
    assert_eq!(s.percent(s.same), 90.0);
    assert_eq!(s.percent(s.similar), 3.3333);
    assert_eq!(s.percent(s.different), 6.6667);
    assert_eq!(s.mean_distance(), 3.4667);
    assert!(!s.all_same());
    // Identical pictures: no bounds, 100 %.
    let same = compare(
        &flat(3, 3, [1, 2, 3, 4]),
        &flat(3, 3, [1, 2, 3, 4]),
        0,
        Geometry::default(),
    );
    let s = *same.stats();
    assert_eq!(
        (s.bounds, s.percent(s.same), s.mean_distance()),
        (None, 100.0, 0.0)
    );
    assert!(s.all_same());
    // Rounding half up at the fourth decimal: 1 of 8 = 12.5 %, 1 of 3 = 33.3333 %, 2 of 3 = 66.6667 %.
    let s = hikasa_picture::PictureStats {
        pixels: 8,
        ..Default::default()
    };
    assert_eq!(s.percent(1), 12.5);
    let s = hikasa_picture::PictureStats {
        pixels: 3,
        ..Default::default()
    };
    assert_eq!((s.percent(1), s.percent(2)), (33.3333, 66.6667));
    assert_eq!(hikasa_picture::PictureStats::default().percent(0), 0.0);
}

#[test]
fn with_geometry_equals_a_fresh_comparison() {
    let c = golden();
    for offset in [(0, 0), (2, -1), (-6, 0), (7, 7)] {
        let geometry = Geometry {
            offset,
            ..Geometry::default()
        };
        let moved = c.with_geometry(geometry, &Ctl::detached()).unwrap();
        let fresh = compare(&c.pictures().left, &c.pictures().right, 2, geometry);
        assert_eq!(moved.stats(), fresh.stats(), "{offset:?}");
        assert_eq!(moved.area(), fresh.area());
    }
    let tolerant = c
        .with_options(
            hikasa_picture::PictureOptions {
                tolerance: 100,
                geometry: Geometry::default(),
            },
            &Ctl::detached(),
        )
        .unwrap();
    assert_eq!(
        (tolerant.stats().similar, tolerant.stats().different),
        (3, 0)
    );
}

#[test]
fn probe_at_corners_outside_and_uncovered() {
    let c = golden();
    let px = |r, g, b, a| Rgba8::new(r, g, b, a);
    assert_eq!(
        c.probe(0, 0),
        PixelProbe {
            area_x: 0,
            area_y: 0,
            left: Some((0, 0, px(50, 50, 50, 255))),
            right: Some((0, 0, px(50, 50, 50, 255))),
            distance: 0,
            class: PixelClass::Same,
        }
    );
    let corner = c.probe(5, 4);
    assert_eq!(
        (corner.left.map(|l| (l.0, l.1)), corner.class),
        (Some((5, 4)), PixelClass::Same)
    );
    let hot = c.probe(4, 3);
    assert_eq!((hot.distance, hot.class), (100, PixelClass::Different));
    assert_eq!(hot.right.unwrap().2, px(150, 50, 50, 255));
    let outside = c.probe(6, 0);
    assert_eq!(
        (outside.left, outside.right, outside.class),
        (None, None, PixelClass::Outside)
    );
    assert_eq!(c.probe(u32::MAX, u32::MAX).class, PixelClass::Outside);
    // An uncovered region inside the area: left 3 × 2 at (0, 0), right 2 × 2 at (4, 3).
    let c = compare(
        &flat(3, 2, [9, 9, 9, 255]),
        &flat(2, 2, [9, 9, 9, 200]),
        0,
        Geometry {
            offset: (4, 3),
            ..Geometry::default()
        },
    );
    assert_eq!((c.area().width, c.area().height), (6, 5));
    let gap = c.probe(3, 0);
    assert_eq!(
        (gap.left, gap.right, gap.distance, gap.class),
        (None, None, 0, PixelClass::Outside)
    );
    let only_left = c.probe(0, 0);
    assert_eq!((only_left.right, only_left.distance), (None, 255));
    let only_right = c.probe(5, 4);
    assert_eq!(only_right.right.map(|r| (r.0, r.1)), Some((1, 1)));
    assert_eq!((only_right.left, only_right.distance), (None, 200));
    let s = *c.stats();
    assert_eq!((s.pixels, s.left_only, s.right_only), (10, 6, 4));
    assert_eq!(
        s.bounds,
        Some(Rect {
            x: 0,
            y: 0,
            width: 6,
            height: 5
        })
    );
}
