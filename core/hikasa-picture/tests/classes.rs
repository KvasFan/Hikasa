//! A12.13 (D-339): pixel classes and the verdict — goldens at tolerance 0, 1 and 255, transparent pixels, uncovered
//! positions, Ignore Unimportant, every row of §1.12.16's table, and P-PIXEL (the statistics equal a naive per-pixel
//! loop for random pairs, geometries and tolerances, with 1, 4 and 16 threads).

mod common;

use std::convert::Infallible;
use std::sync::Arc;

use common::{
    THREADS, XorShift, compare, decode, encode, in_pool, naive, pic, random_geometry, small_picture,
};
use hikasa_core::{ContentKind, ExitCode};
use hikasa_picture::{
    Geometry, Picture, PictureStats, PictureVerdict, PixelClass, Rect, classify, distance,
};
use image::{ExtendedColorType, ImageFormat};
use proptest::prelude::*;

/// 20 pixel pairs: in each channel (R, G, B, A) the distances 0, 1, 2, 254, 255.
fn golden_pair() -> (Arc<Picture>, Arc<Picture>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for c in 0..4 {
        for d in [0u8, 1, 2, 254, 255] {
            let l = [0u8, 0, 0, 255];
            let mut r = l;
            if c < 3 {
                r[c] = d;
            } else {
                r[3] = 255 - d;
            }
            left.extend(l);
            right.extend(r);
        }
    }
    (pic(20, 1, left), pic(20, 1, right))
}

#[test]
fn goldens_at_tolerance_0_1_255() {
    let (left, right) = golden_pair();
    for (tolerance, same, similar, different) in [(0u8, 4, 0, 16), (1, 4, 4, 12), (255, 4, 16, 0)] {
        let stats = *compare(&left, &right, tolerance, Geometry::default()).stats();
        assert_eq!(
            (stats.pixels, stats.same, stats.similar, stats.different),
            (20, same, similar, different),
            "tolerance {tolerance}"
        );
        assert_eq!((stats.left_only, stats.right_only), (0, 0));
        assert_eq!(stats.max_distance, 255);
        assert_eq!(stats.distance_sum, 4 * (1 + 2 + 254 + 255));
        assert_eq!(
            stats.bounds,
            Some(Rect {
                x: 1,
                y: 0,
                width: 19,
                height: 1
            })
        );
    }
    let comparison = compare(&left, &right, 1, Geometry::default());
    let classes: Vec<PixelClass> = (0..5).map(|x| comparison.probe(x, 0).class).collect();
    use PixelClass::*;
    assert_eq!(classes, [Same, Similar, Different, Different, Different]);
    let distances: Vec<u8> = (15..20).map(|x| comparison.probe(x, 0).distance).collect();
    assert_eq!(distances, [0, 1, 2, 254, 255], "alpha distances");
}

#[test]
fn transparent_pixels_and_uncovered_positions() {
    assert_eq!(classify(0, 0), PixelClass::Same);
    let t = |c: [u8; 4]| Some(hikasa_picture::Rgba8::from_array(c));
    assert_eq!(distance(t([10, 20, 30, 0]), t([200, 0, 0, 0])), 0);
    assert_eq!(distance(t([10, 20, 30, 0]), t([10, 20, 30, 1])), 1);
    assert_eq!(distance(t([255, 255, 255, 77]), None), 77);
    assert_eq!(distance(None, t([255, 0, 0, 0])), 0);
    // Two fully transparent pixels of different colours are same.
    let left = pic(2, 1, vec![10, 20, 30, 0, 1, 2, 3, 4]);
    let right = pic(2, 1, vec![200, 0, 90, 0, 1, 2, 3, 4]);
    let stats = *compare(&left, &right, 0, Geometry::default()).stats();
    assert_eq!((stats.same, stats.different), (2, 0));
    // An uncovered position counts the covering pixel's alpha.
    let left = pic(3, 1, vec![0, 0, 0, 255, 255, 255, 255, 77, 9, 9, 9, 0]);
    let right = pic(1, 1, vec![0, 0, 0, 255]);
    let comparison = compare(&left, &right, 76, Geometry::default());
    let stats = *comparison.stats();
    assert_eq!((stats.pixels, stats.left_only, stats.right_only), (3, 2, 0));
    assert_eq!((stats.same, stats.similar, stats.different), (2, 0, 1));
    assert_eq!((stats.max_distance, stats.distance_sum), (77, 77));
    assert_eq!(comparison.probe(1, 0).distance, 77);
    assert_eq!(
        comparison.probe(2, 0).class,
        PixelClass::Same,
        "a transparent one-sided pixel"
    );
}

fn verdict(stats: &PictureStats, ignore: bool, bytes: bool) -> PictureVerdict {
    PictureVerdict::of(stats, ignore, || Ok::<_, Infallible>(bytes)).unwrap()
}

/// Every row of §1.12.16's table; the bytes are asked only when every position is same.
#[test]
fn every_row_of_the_verdict_table() {
    let mut rng = XorShift(41);
    let rgb: Vec<u8> = rng.bytes(6 * 4 * 3);
    let png = encode(ImageFormat::Png, 6, 4, ExtendedColorType::Rgb8, &rgb);
    let bmp = encode(ImageFormat::Bmp, 6, 4, ExtendedColorType::Rgb8, &rgb);
    let (a, b) = (
        Arc::new(decode(&png, "a.png").unwrap()),
        Arc::new(decode(&bmp, "a.bmp").unwrap()),
    );
    let comparison = compare(&a, &b, 0, Geometry::default());
    let identical = comparison
        .verdict(false, || Ok::<_, Infallible>(png == bmp))
        .unwrap();
    assert_eq!(
        (identical.kind, identical.exit_code),
        (ContentKind::RulesSame, ExitCode::RulesSame),
        "PNG vs BMP of the same pixels"
    );
    let same = compare(&a, &a, 0, Geometry::default());
    let v = same.verdict(false, || Ok::<_, Infallible>(true)).unwrap();
    assert_eq!(
        (v.kind, v.exit_code),
        (ContentKind::BinarySame, ExitCode::BinarySame)
    );
    // One channel off by 1: similar at tolerance 1, different at 0.
    let mut off = rgb.clone();
    off[0] = off[0].wrapping_add(if off[0] == 255 { 255 } else { 1 });
    let c = Arc::new(
        decode(
            &encode(ImageFormat::Png, 6, 4, ExtendedColorType::Rgb8, &off),
            "c.png",
        )
        .unwrap(),
    );
    let similar = *compare(&a, &c, 1, Geometry::default()).stats();
    let never =
        || -> Result<bool, Infallible> { panic!("bytes asked although a position differs") };
    let v = PictureVerdict::of(&similar, false, never).unwrap();
    assert_eq!(
        (v.kind, v.exit_code),
        (ContentKind::UnimportantOnly, ExitCode::Similar)
    );
    let v = PictureVerdict::of(&similar, true, never).unwrap();
    assert_eq!(
        (v.kind, v.exit_code),
        (ContentKind::UnimportantOnly, ExitCode::RulesSame)
    );
    let different = *compare(&a, &c, 0, Geometry::default()).stats();
    for ignore in [false, true] {
        let v = PictureVerdict::of(&different, ignore, never).unwrap();
        assert_eq!(
            (v.kind, v.exit_code),
            (ContentKind::ImportantDifferences, ExitCode::RulesDifferent)
        );
    }
    assert_eq!(
        verdict(&different, false, true).kind,
        ContentKind::ImportantDifferences
    );
    assert_eq!(PictureVerdict::ERROR.exit_code, ExitCode::UnknownError);
    // The display mode is not an input of the verdict: nothing to vary here but the tolerance and geometry.
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// P-PIXEL: the classes and statistics equal a naive per-pixel loop, with 1, 4 and 16 threads.
    #[test]
    fn p_pixel(seed in any::<u64>(), tolerance in any::<u8>(), auto_scale in any::<bool>()) {
        let mut rng = XorShift(seed);
        let left = small_picture(&mut rng, 12);
        let right = if rng.below(4) == 0 { Arc::clone(&left) } else { small_picture(&mut rng, 12) };
        let geometry = random_geometry(&mut rng, auto_scale);
        let want = naive(&left, &right, tolerance, geometry);
        for threads in THREADS {
            let got = in_pool(threads, || *compare(&left, &right, tolerance, geometry).stats());
            prop_assert_eq!(got, want, "threads {}", threads);
        }
    }
}
