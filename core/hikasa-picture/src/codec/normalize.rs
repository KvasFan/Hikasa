//! Normalization to RGBA8 (normative, §1.12.14), by the decoder's colour type: `L8` → (l, l, l, 255); `La8` →
//! (l, l, l, a); `Rgb8` → (r, g, b, 255); `Rgba8` as is; 16-bit types per channel `(v · 255 + 32767) / 65535`
//! (round half up), then as the 8-bit type; float types of HDR and EXR through the tone map, of other formats
//! display-encoded (`clamp(v, 0, 1) · 255` rounded half up, NaN → 0).

use image::DynamicImage;

use crate::codec::tonemap::{channel, display, oetf_table};
use crate::vocab::ToneMap;

/// How float channels are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Floats {
    /// Linear light (HDR, OpenEXR): colour through the tone map, alpha display-encoded.
    Linear(ToneMap),
    /// Display-encoded (32-bit float TIFF and the like).
    Display,
}

/// One 16-bit channel to 8 bits, round half up.
#[inline]
pub(crate) const fn from16(v: u16) -> u8 {
    ((v as u32 * 255 + 32_767) / 65_535) as u8
}

/// The RGBA8 bytes of a decoded image. Consumes it, so an RGBA8 image is returned without a copy and the other
/// types hold the source and the result at once only while converting.
pub(crate) fn to_rgba8(image: DynamicImage, floats: Floats) -> Vec<u8> {
    match image {
        DynamicImage::ImageRgba8(buffer) => buffer.into_raw(),
        DynamicImage::ImageRgb8(buffer) => buffer
            .as_raw()
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        DynamicImage::ImageLuma8(buffer) => buffer
            .as_raw()
            .iter()
            .flat_map(|&l| [l, l, l, 255])
            .collect(),
        DynamicImage::ImageLumaA8(buffer) => buffer
            .as_raw()
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        DynamicImage::ImageRgba16(buffer) => buffer.as_raw().iter().map(|&v| from16(v)).collect(),
        DynamicImage::ImageRgb16(buffer) => buffer
            .as_raw()
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [from16(c[0]), from16(c[1]), from16(c[2]), 255])
            .collect(),
        DynamicImage::ImageLuma16(buffer) => buffer
            .as_raw()
            .iter()
            .flat_map(|&v| {
                let l = from16(v);
                [l, l, l, 255]
            })
            .collect(),
        DynamicImage::ImageLumaA16(buffer) => buffer
            .as_raw()
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|c| {
                let l = from16(c[0]);
                [l, l, l, from16(c[1])]
            })
            .collect(),
        DynamicImage::ImageRgb32F(buffer) => {
            let colour = colour_of(floats);
            buffer
                .as_raw()
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|c| [colour(c[0]), colour(c[1]), colour(c[2]), 255])
                .collect()
        }
        DynamicImage::ImageRgba32F(buffer) => {
            let colour = colour_of(floats);
            buffer
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|c| [colour(c[0]), colour(c[1]), colour(c[2]), display(c[3])])
                .collect()
        }
        // `DynamicImage` is non-exhaustive: a future type goes through `image`'s own conversion.
        other => other.into_rgba8().into_raw(),
    }
}

/// The colour-channel rule of `floats`.
fn colour_of(floats: Floats) -> impl Fn(f32) -> u8 {
    let table = oetf_table();
    move |v| match floats {
        Floats::Linear(map) => channel(map, table, v),
        Floats::Display => display(v),
    }
}
