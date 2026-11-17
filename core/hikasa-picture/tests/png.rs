//! A12.17 (D-343, engine share): `encode_png` round-trips through the decoder for random images at every level;
//! empty images and buffers of the wrong length are refused. (The clipboard's share is in hikasa-session's
//! `tests/clipboard.rs`.)

mod common;

use common::{XorShift, decode};
use hikasa_core::{Classify, ErrorKind};
use hikasa_picture::{ImageFormatName, PngLevel, RgbaImage, SourceColor, encode_png};
use proptest::prelude::*;

proptest! {
    #![proptest_config(common::cases(64))]

    /// Random images (noise, flat runs) decode back to their exact pixels.
    #[test]
    fn encode_png_round_trips(seed in any::<u64>(), w in 1u32..200, h in 1u32..60, level in 0usize..3) {
        let mut rng = XorShift(seed);
        let n = (w * h * 4) as usize;
        let rgba = if rng.below(2) == 0 {
            rng.bytes(n)
        } else {
            let c = rng.bytes(4);
            (0..n).map(|i| if rng.below(50) == 0 { rng.next() as u8 } else { c[i % 4] }).collect()
        };
        let image = RgbaImage { width: w, height: h, rgba };
        let level = [PngLevel::Fast, PngLevel::Balanced, PngLevel::Best][level];
        let png = encode_png(&image, level).unwrap();
        prop_assert_eq!(&png, &encode_png(&image, level).unwrap(), "deterministic");
        let picture = decode(&png, "x.png").unwrap();
        prop_assert_eq!(picture.info.format, ImageFormatName::Png);
        prop_assert_eq!(picture.info.color, SourceColor::Rgba8);
        prop_assert_eq!((picture.width, picture.height), (w, h));
        prop_assert!(picture.rgba[..] == image.rgba[..]);
    }
}

#[test]
fn levels_compress_and_bad_buffers_are_refused() {
    let flat = RgbaImage {
        width: 500,
        height: 500,
        rgba: [7u8, 8, 9, 255].repeat(250_000),
    };
    let fast = encode_png(&flat, PngLevel::Fast).unwrap().len();
    let best = encode_png(&flat, PngLevel::Best).unwrap().len();
    assert!(best <= fast, "{best} > {fast}");
    assert!(fast < 1_000_000 / 20, "{fast}");
    for bad in [
        RgbaImage {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        },
        RgbaImage {
            width: 2,
            height: 2,
            rgba: vec![0; 15],
        },
        RgbaImage {
            width: 2,
            height: 2,
            rgba: vec![0; 17],
        },
    ] {
        let err = encode_png(&bad, PngLevel::Fast).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        // The engine's own check refuses a buffer of any other length (longer included), before the encoder.
        let need = bad.width * bad.height * 4;
        assert!(
            err.to_string()
                .contains(&format!("needs {need} bytes, got {}", bad.rgba.len())),
            "{err}"
        );
    }
}
