//! A12.12 (D-337): decoders, normalization to RGBA8, the tone map, the EXIF orientation, ICC profiles, limits,
//! and P-DECODE (random bytes and mutated files never panic).
//!
//! Inputs are generated here with `image`'s own encoders, except two WebP files no encoder of `image` writes,
//! made once with Pillow and committed under `tests/fixtures/` (lossy: 118 bytes; animated, three lossless frames
//! of 5 × 4: 188 bytes).

mod common;

use common::{XorShift, decode, encode, encode_dynamic};
use hikasa_core::{CancelToken, Classify, Ctl, ErrorKind, NullProgress};
use hikasa_picture::{
    DecodeOptions, FrameCount, ImageFormatName, Orientation, PictureError, PictureWarningCode,
    SourceColor, ToneMap, decode_bytes, oetf_table,
};
use image::codecs::gif::GifEncoder;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ExtendedColorType, Frame, ImageEncoder, ImageFormat, RgbaImage};
use proptest::prelude::*;

const W: u32 = 7;
const H: u32 = 5;

/// One 16-bit channel to 8 bits by §1.12.14's rule.
fn n16(v: u16) -> u8 {
    ((u32::from(v) * 255 + 32_767) / 65_535) as u8
}

/// Samples of `color` (16-bit types as native-endian bytes), and the normalized RGBA8 of §1.12.14.
fn samples(color: ExtendedColorType, rng: &mut XorShift) -> (Vec<u8>, Vec<u8>) {
    let n = (W * H) as usize;
    let mut raw = Vec::new();
    let mut want = Vec::new();
    for _ in 0..n {
        match color {
            ExtendedColorType::L8 => {
                let l = rng.next() as u8;
                raw.push(l);
                want.extend([l, l, l, 255]);
            }
            ExtendedColorType::La8 => {
                let (l, a) = (rng.next() as u8, rng.next() as u8);
                raw.extend([l, a]);
                want.extend([l, l, l, a]);
            }
            ExtendedColorType::Rgb8 => {
                let c = [rng.next() as u8, rng.next() as u8, rng.next() as u8];
                raw.extend(c);
                want.extend([c[0], c[1], c[2], 255]);
            }
            ExtendedColorType::Rgba8 => {
                let c = [
                    rng.next() as u8,
                    rng.next() as u8,
                    rng.next() as u8,
                    rng.next() as u8,
                ];
                raw.extend(c);
                want.extend(c);
            }
            ExtendedColorType::L16 => {
                let l = rng.next() as u16;
                raw.extend(l.to_ne_bytes());
                want.extend([n16(l), n16(l), n16(l), 255]);
            }
            ExtendedColorType::La16 => {
                let (l, a) = (rng.next() as u16, rng.next() as u16);
                raw.extend(l.to_ne_bytes());
                raw.extend(a.to_ne_bytes());
                want.extend([n16(l), n16(l), n16(l), n16(a)]);
            }
            ExtendedColorType::Rgb16 => {
                let c = [rng.next() as u16, rng.next() as u16, rng.next() as u16];
                for v in c {
                    raw.extend(v.to_ne_bytes());
                }
                want.extend([n16(c[0]), n16(c[1]), n16(c[2]), 255]);
            }
            ExtendedColorType::Rgba16 => {
                let c = [
                    rng.next() as u16,
                    rng.next() as u16,
                    rng.next() as u16,
                    rng.next() as u16,
                ];
                for v in c {
                    raw.extend(v.to_ne_bytes());
                }
                want.extend(c.map(n16));
            }
            other => panic!("no samples for {other:?}"),
        }
    }
    (raw, want)
}

/// A12.12: every lossless format round-trips generated samples to the exact normalized bytes.
#[test]
fn lossless_formats_round_trip_to_the_normalized_bytes() {
    use ExtendedColorType as C;
    let cases: &[(ImageFormat, &str, ImageFormatName, &[ExtendedColorType])] = &[
        (
            ImageFormat::Png,
            "a.png",
            ImageFormatName::Png,
            &[
                C::L8,
                C::La8,
                C::Rgb8,
                C::Rgba8,
                C::L16,
                C::La16,
                C::Rgb16,
                C::Rgba16,
            ],
        ),
        (
            ImageFormat::Bmp,
            "a.bmp",
            ImageFormatName::Bmp,
            &[C::L8, C::Rgb8, C::Rgba8],
        ),
        (
            ImageFormat::Tiff,
            "a.tiff",
            ImageFormatName::Tiff,
            &[C::L8, C::Rgb8, C::Rgba8, C::L16, C::Rgb16, C::Rgba16],
        ),
        (
            ImageFormat::WebP,
            "a.webp",
            ImageFormatName::WebP,
            &[C::Rgb8, C::Rgba8],
        ),
        (
            ImageFormat::Tga,
            "a.tga",
            ImageFormatName::Tga,
            &[C::L8, C::La8, C::Rgb8, C::Rgba8],
        ),
        (
            ImageFormat::Pnm,
            "a.pnm",
            ImageFormatName::Pnm,
            &[C::L8, C::Rgb8, C::L16, C::Rgb16],
        ),
        (
            ImageFormat::Qoi,
            "a.qoi",
            ImageFormatName::Qoi,
            &[C::Rgb8, C::Rgba8],
        ),
        (ImageFormat::Ico, "a.ico", ImageFormatName::Ico, &[C::Rgba8]),
    ];
    let mut rng = XorShift(0x5EED);
    for &(format, name, expected, colors) in cases {
        for &color in colors {
            let (raw, want) = samples(color, &mut rng);
            let bytes = encode(format, W, H, color, &raw);
            let picture =
                decode(&bytes, name).unwrap_or_else(|e| panic!("{format:?} {color:?}: {e}"));
            assert_eq!(
                (picture.width, picture.height),
                (W, H),
                "{format:?} {color:?}"
            );
            assert_eq!(picture.info.format, expected);
            assert_eq!(picture.info.file_size, bytes.len() as u64);
            assert_eq!(picture.info.frames, FrameCount::One, "{format:?} {color:?}");
            assert_eq!(&picture.rgba[..], &want[..], "{format:?} {color:?}");
        }
    }
}

/// The colour type and bit depth of the source are reported.
#[test]
fn source_colour_and_bit_depth() {
    let mut rng = XorShift(9);
    for (color, source, depth, alpha) in [
        (ExtendedColorType::L8, SourceColor::L8, 8, false),
        (ExtendedColorType::La16, SourceColor::La16, 16, true),
        (ExtendedColorType::Rgba16, SourceColor::Rgba16, 16, true),
    ] {
        let (raw, _) = samples(color, &mut rng);
        let picture = decode(&encode(ImageFormat::Png, W, H, color, &raw), "x.png").unwrap();
        assert_eq!(picture.info.color, source);
        assert_eq!(picture.info.bit_depth, depth);
        assert_eq!(picture.info.has_alpha, alpha);
        assert_eq!(picture.info.hdr, None);
        assert!(!picture.info.icc && !picture.info.converted);
    }
}

/// The values of the tone-map goldens: 0, 0.0031308, 0.5, 1, 4, NaN, −1, +∞.
const VALUES: [f32; 8] = [
    0.0,
    0.003_130_8,
    0.5,
    1.0,
    4.0,
    f32::NAN,
    -1.0,
    f32::INFINITY,
];

fn exr(values: &[f32], alpha: Option<&[f32]>) -> Vec<u8> {
    let width = values.len() as u32;
    let (color, raw): (_, Vec<f32>) = match alpha {
        None => (
            ExtendedColorType::Rgb32F,
            values.iter().flat_map(|&v| [v, v, v]).collect(),
        ),
        Some(alpha) => (
            ExtendedColorType::Rgba32F,
            values
                .iter()
                .zip(alpha)
                .flat_map(|(&v, &a)| [v, v, v, a])
                .collect(),
        ),
    };
    let bytes: Vec<u8> = raw.iter().flat_map(|v| v.to_ne_bytes()).collect();
    encode(ImageFormat::OpenExr, width, 1, color, &bytes)
}

fn channel0(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks(4).map(|c| c[0]).collect()
}

/// A12.12: HDR and EXR through both tone maps at the golden values; the OETF table's CRC is pinned.
#[test]
fn tone_map_goldens_and_the_table_crc() {
    let table = oetf_table();
    let crc = hikasa_vfs::crc32_bytes(&table[..]);
    assert_eq!(
        crc, 0x9A17_3CA2,
        "the OETF table changed (a libm difference?)"
    );
    let bytes = exr(&VALUES, None);
    let golden = [
        (ToneMap::Clamp, [0u8, 10, 188, 255, 255, 0, 0, 255]),
        (ToneMap::Reinhard, [0u8, 10, 156, 188, 231, 0, 0, 255]),
    ];
    for (map, want) in golden {
        let opts = DecodeOptions {
            tone_map: map,
            ..DecodeOptions::default()
        };
        let picture = decode_bytes(&bytes, Some("x.exr"), &opts, &Ctl::detached()).unwrap();
        assert_eq!(picture.info.format, ImageFormatName::OpenExr);
        assert_eq!(picture.info.hdr, Some(map));
        assert_eq!(picture.info.color, SourceColor::Rgb32f);
        assert_eq!(picture.info.bit_depth, 32);
        assert_eq!(channel0(&picture.rgba), want, "{map}");
        assert!(
            picture
                .rgba
                .chunks(4)
                .all(|c| c[3] == 255 && c[0] == c[1] && c[1] == c[2])
        );
    }
    // Alpha is display-encoded (clamped, linear), never tone-mapped.
    let alpha = [0.0, 0.25, 0.5, 1.0, 4.0, f32::NAN, -1.0, f32::INFINITY];
    let picture = decode(&exr(&VALUES, Some(&alpha)), "x.exr").unwrap();
    let alphas: Vec<u8> = picture.rgba.chunks(4).map(|c| c[3]).collect();
    assert_eq!(alphas, [0, 64, 128, 255, 255, 0, 0, 255]);
    // Radiance HDR (RGBE holds 0, 0.5, 1 and 4 exactly).
    let hdr_values = [0.0f32, 0.5, 1.0, 4.0];
    let raw: Vec<u8> = hdr_values
        .iter()
        .flat_map(|&v| [v, v, v])
        .flat_map(f32::to_ne_bytes)
        .collect();
    let hdr = encode(ImageFormat::Hdr, 4, 1, ExtendedColorType::Rgb32F, &raw);
    for (map, want) in [
        (ToneMap::Clamp, [0u8, 188, 255, 255]),
        (ToneMap::Reinhard, [0u8, 156, 188, 231]),
    ] {
        let opts = DecodeOptions {
            tone_map: map,
            ..DecodeOptions::default()
        };
        let picture = decode_bytes(&hdr, Some("x.hdr"), &opts, &Ctl::detached()).unwrap();
        assert_eq!(picture.info.format, ImageFormatName::Hdr);
        assert_eq!(channel0(&picture.rgba), want, "hdr {map}");
    }
}

/// A float TIFF is display-encoded, never tone-mapped.
#[test]
fn float_tiff_is_display_encoded() {
    let raw: Vec<u8> = VALUES
        .iter()
        .flat_map(|&v| [v, v, v])
        .flat_map(f32::to_ne_bytes)
        .collect();
    let bytes = encode(ImageFormat::Tiff, 8, 1, ExtendedColorType::Rgb32F, &raw);
    for map in [ToneMap::Clamp, ToneMap::Reinhard] {
        let opts = DecodeOptions {
            tone_map: map,
            ..DecodeOptions::default()
        };
        let picture = decode_bytes(&bytes, Some("f.tif"), &opts, &Ctl::detached()).unwrap();
        assert_eq!(picture.info.hdr, None);
        assert_eq!(channel0(&picture.rgba), [0, 1, 128, 255, 255, 0, 0, 255]);
    }
}

/// A12.12: JPEG and lossy WebP decode to the right size; a `.png` holding a JPEG decodes by content; TGA by name.
#[test]
fn lossy_formats_and_detection_by_content() {
    let mut rng = XorShift(3);
    let (raw, _) = samples(ExtendedColorType::Rgb8, &mut rng);
    let jpeg = encode(ImageFormat::Jpeg, W, H, ExtendedColorType::Rgb8, &raw);
    for name in ["a.jpg", "a.png", "noextension"] {
        let picture = decode(&jpeg, name).unwrap();
        assert_eq!(picture.info.format, ImageFormatName::Jpeg, "{name}");
        assert_eq!((picture.width, picture.height), (W, H));
        assert!(picture.rgba.chunks(4).all(|c| c[3] == 255));
    }
    let webp = include_bytes!("fixtures/lossy.webp");
    let picture = decode(webp, "lossy.webp").unwrap();
    assert_eq!(picture.info.format, ImageFormatName::WebP);
    assert_eq!((picture.width, picture.height), (13, 7));
    // TGA has no signature: by name only.
    let (raw, want) = samples(ExtendedColorType::Rgba8, &mut rng);
    let tga = encode(ImageFormat::Tga, W, H, ExtendedColorType::Rgba8, &raw);
    assert_eq!(&decode(&tga, "x.TGA").unwrap().rgba[..], &want[..]);
    assert!(matches!(
        decode(&tga, "x.bin"),
        Err(PictureError::Unsupported { .. })
    ));
}

/// A12.12: GIF and animated WebP give the first frame and `frames_ignored`.
#[test]
fn first_frames() {
    let frames: Vec<Frame> = [[255u8, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]
        .iter()
        .map(|c| Frame::new(RgbaImage::from_raw(4, 3, c.repeat(12)).unwrap()))
        .collect();
    let mut gif = Vec::new();
    GifEncoder::new(&mut gif).encode_frames(frames).unwrap();
    let picture = decode(&gif, "a.gif").unwrap();
    assert_eq!(picture.info.format, ImageFormatName::Gif);
    assert_eq!(picture.info.frames, FrameCount::Many(3));
    assert_eq!(&picture.rgba[..], &[255u8, 0, 0, 255].repeat(12)[..]);
    let codes: Vec<_> = picture.warnings("a.gif").iter().map(|w| w.code).collect();
    assert_eq!(codes, [PictureWarningCode::FramesIgnored]);
    // A still GIF of few colours round-trips exactly.
    let still: Vec<u8> = (0..12)
        .flat_map(|i| {
            if i % 2 == 0 {
                [10, 20, 30, 255]
            } else {
                [200, 100, 0, 255]
            }
        })
        .collect();
    let bytes = encode(ImageFormat::Gif, 4, 3, ExtendedColorType::Rgba8, &still);
    let picture = decode(&bytes, "s.gif").unwrap();
    assert_eq!(picture.info.frames, FrameCount::One);
    assert_eq!(&picture.rgba[..], &still[..]);
    let anim = include_bytes!("fixtures/anim.webp");
    let picture = decode(anim, "anim.webp").unwrap();
    assert_eq!(picture.info.frames, FrameCount::Many(3));
    assert_eq!((picture.width, picture.height), (5, 4));
    assert_eq!(&picture.rgba[..], &[255u8, 0, 0, 255].repeat(20)[..]);
}

/// A12.12: ICO decodes its largest entry.
#[test]
fn ico_takes_the_largest_entry() {
    let small = [9u8, 9, 9, 255].repeat(16 * 16);
    let large = [1u8, 2, 3, 255].repeat(32 * 32);
    let frames = vec![
        IcoFrame::as_png(&small, 16, 16, ExtendedColorType::Rgba8).unwrap(),
        IcoFrame::as_png(&large, 32, 32, ExtendedColorType::Rgba8).unwrap(),
    ];
    let mut ico = Vec::new();
    IcoEncoder::new(&mut ico).encode_images(&frames).unwrap();
    let picture = decode(&ico, "a.ico").unwrap();
    assert_eq!(picture.info.format, ImageFormatName::Ico);
    assert_eq!((picture.width, picture.height), (32, 32));
    assert_eq!(&picture.rgba[..], &large[..]);
}

/// An EXIF block (little-endian TIFF) holding only the orientation `value`.
fn exif(value: u16) -> Vec<u8> {
    let mut e = b"II*\0".to_vec();
    e.extend(8u32.to_le_bytes());
    e.extend(1u16.to_le_bytes());
    e.extend(0x0112u16.to_le_bytes());
    e.extend(3u16.to_le_bytes());
    e.extend(1u32.to_le_bytes());
    e.extend(u32::from(value).to_le_bytes());
    e.extend(0u32.to_le_bytes());
    e
}

/// A12.12: the EXIF orientation comes out applied (PNG exactly, JPEG by size and against the same JPEG without
/// EXIF), with `orientation_applied`.
#[test]
fn exif_orientation_is_applied() {
    let mut rng = XorShift(17);
    let (raw, want) = samples(ExtendedColorType::Rgba8, &mut rng);
    for (value, orientation) in [
        (2, Orientation::FlipH),
        (3, Orientation::Rot180),
        (4, Orientation::FlipV),
        (5, Orientation::Transpose),
        (6, Orientation::Cw90),
        (7, Orientation::Transverse),
        (8, Orientation::Ccw90),
    ] {
        let mut png = Vec::new();
        let mut encoder = PngEncoder::new(&mut png);
        encoder.set_exif_metadata(exif(value)).unwrap();
        encoder
            .write_image(&raw, W, H, ExtendedColorType::Rgba8)
            .unwrap();
        let picture = decode(&png, "o.png").unwrap();
        let expected = hikasa_picture::geometry::orient(W, H, &want, orientation);
        assert_eq!(
            (picture.width, picture.height),
            (expected.width, expected.height)
        );
        assert_eq!(&picture.rgba[..], &expected.rgba[..], "EXIF {value}");
        assert_eq!(picture.info.orientation, Some(orientation));
        let codes: Vec<_> = picture.warnings("o.png").iter().map(|w| w.code).collect();
        assert_eq!(codes, [PictureWarningCode::OrientationApplied]);
    }
    let (rgb, _) = samples(ExtendedColorType::Rgb8, &mut rng);
    let jpeg = |exif_block: Option<Vec<u8>>| {
        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, 90);
        if let Some(block) = exif_block {
            encoder.set_exif_metadata(block).unwrap();
        }
        encoder
            .write_image(&rgb, W, H, ExtendedColorType::Rgb8)
            .unwrap();
        out
    };
    let plain = decode(&jpeg(None), "p.jpg").unwrap();
    let rotated = decode(&jpeg(Some(exif(6))), "r.jpg").unwrap();
    assert_eq!((rotated.width, rotated.height), (H, W));
    assert_eq!(rotated.info.orientation, Some(Orientation::Cw90));
    let expected = hikasa_picture::geometry::orient(W, H, &plain.rgba, Orientation::Cw90);
    assert_eq!(&rotated.rgba[..], &expected.rgba[..]);
}

/// A12.12: an ICC profile other than sRGB gives `color_profile_ignored`; an sRGB one does not.
#[test]
fn icc_profiles() {
    let raw = [1u8, 2, 3, 255].repeat(4);
    let with = |profile: &[u8]| {
        let mut png = Vec::new();
        let mut encoder = PngEncoder::new(&mut png);
        encoder.set_icc_profile(profile.to_vec()).unwrap();
        encoder
            .write_image(&raw, 2, 2, ExtendedColorType::Rgba8)
            .unwrap();
        decode(&png, "icc.png").unwrap()
    };
    let mut profile = vec![0u8; 132];
    profile.extend_from_slice(b"desc\0\0\0\0Display P3 (fake profile of a test)");
    let picture = with(&profile);
    assert!(picture.info.icc);
    let codes: Vec<_> = picture.warnings("icc.png").iter().map(|w| w.code).collect();
    assert_eq!(codes, [PictureWarningCode::ColorProfileIgnored]);
    assert_eq!(&picture.rgba[..], &raw[..], "the profile changes no pixel");
    let mut srgb = vec![0u8; 132];
    srgb.extend_from_slice(b"desc\0\0\0\0sRGB IEC61966-2.1");
    assert!(!with(&srgb).info.icc);
}

/// A12.12: DDS, AVIF, SVG and unknown content are `unsupported`; a truncated file is `invalid_data`.
#[test]
fn unsupported_and_truncated() {
    let kind = |r: Result<_, PictureError>| r.map(|_| ()).unwrap_err().kind();
    let unsupported = |bytes: &[u8], name: &str, format: &str| match decode(bytes, name) {
        Err(PictureError::Unsupported { format: f }) => assert_eq!(f, format, "{name}"),
        other => panic!("{name}: {other:?}"),
    };
    unsupported(b"DDS |\0\0\0\x07\x10\0\0", "a.dds", "dds");
    unsupported(b"\0\0\0\x1cftypavif\0\0\0\0", "a.avif", "avif");
    unsupported(b"<?xml version='1.0'?><svg/>", "a.svg", "svg");
    unsupported(b"<svg xmlns='http://www.w3.org/2000/svg'/>", "a.png", "svg");
    unsupported(b"plain text", "a.txt", "unknown");
    unsupported(b"", "empty", "unknown");
    assert_eq!(kind(decode(b"plain", "x.svg")), ErrorKind::Unsupported);
    let mut rng = XorShift(5);
    let (raw, _) = samples(ExtendedColorType::Rgba8, &mut rng);
    for (format, name) in [
        (ImageFormat::Png, "t.png"),
        (ImageFormat::Bmp, "t.bmp"),
        (ImageFormat::Qoi, "t.qoi"),
        (ImageFormat::Tiff, "t.tif"),
        (ImageFormat::WebP, "t.webp"),
    ] {
        let bytes = encode(format, W, H, ExtendedColorType::Rgba8, &raw);
        let cut = &bytes[..bytes.len() / 2];
        let err = decode(cut, name).map(|_| ()).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData, "{name}: {err}");
        assert!(matches!(err, PictureError::Decode { .. }), "{name}");
    }
}

/// The PNG header of a picture of `width` × `height` with one empty IDAT (no pixel data).
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

/// A12.12: more pixels than `max_pixels` is `too_large` from the header (the allocation bound is A12.12's
/// counting-allocator test in `memory.rs`).
#[test]
fn too_large_from_the_header() {
    let err = decode(&png_header(80_000, 80_000), "big.png")
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge, "{err}");
    let small = DecodeOptions {
        max_pixels: 99,
        ..DecodeOptions::default()
    };
    let raw = [0u8; 10 * 10 * 4];
    let png = encode(ImageFormat::Png, 10, 10, ExtendedColorType::Rgba8, &raw);
    match decode_bytes(&png, None, &small, &Ctl::detached()) {
        Err(PictureError::TooLarge { pixels, limit }) => {
            assert_eq!((pixels, limit), (Some(100), 99));
        }
        other => panic!("{other:?}"),
    }
    let exact = DecodeOptions {
        max_pixels: 100,
        ..DecodeOptions::default()
    };
    assert!(decode_bytes(&png, None, &exact, &Ctl::detached()).is_ok());
}

/// Decoding checks cancellation before it starts.
#[test]
fn a_cancelled_decode() {
    let token = CancelToken::new();
    token.cancel();
    let ctl = Ctl {
        cancel: &token,
        progress: &NullProgress,
    };
    let png = encode(ImageFormat::Png, 1, 1, ExtendedColorType::Rgba8, &[0; 4]);
    assert!(matches!(
        decode_bytes(&png, None, &DecodeOptions::default(), &ctl),
        Err(PictureError::Cancelled)
    ));
}

/// Through the VFS: the spec's name is the hint; a file above `max_bytes` is `too_large`; a folder is an error.
#[test]
fn through_the_vfs() {
    let dir = tempfile::tempdir().unwrap();
    let mut rng = XorShift(11);
    let (raw, want) = samples(ExtendedColorType::Rgba8, &mut rng);
    let tga = encode(ImageFormat::Tga, W, H, ExtendedColorType::Rgba8, &raw);
    let path = dir.path().join("p.tga");
    std::fs::write(&path, &tga).unwrap();
    let vfs = hikasa_vfs::LocalVfs::default();
    let spec = common::lspec(&path);
    let picture =
        hikasa_picture::decode(&vfs, &spec, &DecodeOptions::default(), &Ctl::detached()).unwrap();
    assert_eq!(&picture.rgba[..], &want[..]);
    let tight = DecodeOptions {
        max_bytes: tga.len() as u64 - 1,
        ..DecodeOptions::default()
    };
    let err = hikasa_picture::decode(&vfs, &spec, &tight, &Ctl::detached()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TooLarge);
    let folder = common::lspec(dir.path());
    assert!(
        hikasa_picture::decode(&vfs, &folder, &DecodeOptions::default(), &Ctl::detached()).is_err()
    );
    let pair = hikasa_picture::decode_pair(
        &vfs,
        hikasa_core::Pair::new(&spec, &folder),
        &DecodeOptions::default(),
        &Ctl::detached(),
    );
    assert!(pair.left.is_ok() && pair.right.is_err());
}

/// Valid files of every container, small (the seeds of P-DECODE's mutations).
fn seeds() -> Vec<(Vec<u8>, &'static str)> {
    let mut rng = XorShift(23);
    let (rgba, _) = samples(ExtendedColorType::Rgba8, &mut rng);
    let (rgb, _) = samples(ExtendedColorType::Rgb8, &mut rng);
    let image = DynamicImage::ImageRgba8(RgbaImage::from_raw(W, H, rgba.clone()).unwrap());
    let mut out = vec![
        (encode_dynamic(&image, ImageFormat::Png), "s.png"),
        (
            encode(ImageFormat::Jpeg, W, H, ExtendedColorType::Rgb8, &rgb),
            "s.jpg",
        ),
        (encode_dynamic(&image, ImageFormat::Gif), "s.gif"),
        (encode_dynamic(&image, ImageFormat::Bmp), "s.bmp"),
        (encode_dynamic(&image, ImageFormat::Ico), "s.ico"),
        (encode_dynamic(&image, ImageFormat::Tiff), "s.tif"),
        (encode_dynamic(&image, ImageFormat::WebP), "s.webp"),
        (encode_dynamic(&image, ImageFormat::Tga), "s.tga"),
        (
            encode(ImageFormat::Pnm, W, H, ExtendedColorType::Rgb8, &rgb),
            "s.ppm",
        ),
        (encode_dynamic(&image, ImageFormat::Qoi), "s.qoi"),
        (exr(&VALUES, None), "s.exr"),
        (include_bytes!("fixtures/lossy.webp").to_vec(), "l.webp"),
        (include_bytes!("fixtures/anim.webp").to_vec(), "a.webp"),
    ];
    let floats: Vec<u8> = [0.5f32, 1.0, 2.0]
        .repeat(4)
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    out.push((
        encode(ImageFormat::Hdr, 2, 2, ExtendedColorType::Rgb32F, &floats),
        "s.hdr",
    ));
    out
}

/// The result of a decode is a picture or a classified error, never a panic.
fn never_panics(bytes: &[u8], name: &str) {
    let result = std::panic::catch_unwind(|| decode(bytes, name));
    match result {
        Ok(Ok(p)) => assert_eq!(p.rgba.len() as u64, p.pixels() * 4),
        Ok(Err(e)) => assert!(
            matches!(
                e.kind(),
                ErrorKind::Unsupported | ErrorKind::InvalidData | ErrorKind::TooLarge
            ),
            "{name}: {e}"
        ),
        Err(_) => panic!("{name}: the decoder panicked"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// P-DECODE: random bytes never panic, with any name.
    #[test]
    fn p_decode_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048), ext in 0usize..6) {
        let name = ["a.png", "a.tga", "a.bin", "a.ico", "a.pnm", "a.exr"][ext];
        never_panics(&bytes, name);
    }

    /// P-DECODE: random mutations (flipped bytes, truncations, inserted runs) of valid files never panic.
    #[test]
    fn p_decode_mutated_files(which in 0usize..14, seed in any::<u64>(), edits in 1usize..8, cut in any::<bool>()) {
        let all = seeds();
        let (mut bytes, name) = all[which % all.len()].clone();
        let mut rng = XorShift(seed);
        for _ in 0..edits {
            let at = rng.below(bytes.len() as u64) as usize;
            match rng.below(3) {
                0 => bytes[at] ^= 1 << rng.below(8),
                1 => bytes[at] = rng.next() as u8,
                _ => {
                    let n = rng.below(16) as usize;
                    let run = rng.bytes(n);
                    bytes.splice(at..at, run);
                }
            }
        }
        if cut && !bytes.is_empty() {
            let keep = rng.below(bytes.len() as u64) as usize;
            bytes.truncate(keep);
        }
        never_panics(&bytes, name);
    }
}

/// The seeds themselves decode (so the mutations start from valid files).
#[test]
fn every_seed_decodes() {
    for (bytes, name) in seeds() {
        let picture = decode(&bytes, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(picture.pixels() > 0);
    }
}
