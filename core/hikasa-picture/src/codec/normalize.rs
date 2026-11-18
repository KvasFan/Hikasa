//! Normalization to RGBA8 (normative, §1.12.14), by the decoder's colour type: `L8` → (l, l, l, 255); `La8` →
//! (l, l, l, a); `Rgb8` → (r, g, b, 255); `Rgba8` as is; 16-bit types per channel `(v · 255 + 32767) / 65535`
//! (round half up), then as the 8-bit type; float types of HDR and EXR through the tone map, of other formats
//! display-encoded (`clamp(v, 0, 1) · 255` rounded half up, NaN → 0).
//!
//! Memory (W12c review fixes, F3): the decoded buffer is consumed. A type of at most 4 bytes a pixel is written
//! straight into the shared buffer (one allocation; the source and the result are held at once, `≤ 8` bytes a
//! pixel). A wider type (16-bit RGB and RGBA, floats) is converted from its end in steps of [`STEP_PIXELS`] into a
//! zeroed buffer whose pages are touched only as they are written, the source shrinking behind each step, so the
//! two together never hold much more than the source alone; the result is then moved into the shared buffer.

use std::sync::Arc;

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

/// Pixels converted per step of a wide type's conversion (the source is shrunk after each step).
pub(crate) const STEP_PIXELS: usize = 1 << 18;

/// One 16-bit channel to 8 bits, round half up.
#[inline]
pub(crate) const fn from16(v: u16) -> u8 {
    ((v as u32 * 255 + 32_767) / 65_535) as u8
}

/// Converts `src` (`C` values a pixel) from its end into a new RGBA8 buffer, shrinking `src` behind each step.
fn from_the_end<T: Copy, const C: usize>(
    mut src: Vec<T>,
    f: impl Fn(&[T; C]) -> [u8; 4],
) -> Vec<u8> {
    let pixels = src.len() / C;
    // Zeroed: a large buffer comes from fresh pages, resident only once written.
    let mut out = vec![0u8; pixels * 4];
    let mut end = pixels;
    while end > 0 {
        let start = end.saturating_sub(STEP_PIXELS);
        let (from, _) = src[start * C..end * C].as_chunks::<C>();
        let (to, _) = out[start * 4..end * 4].as_chunks_mut::<4>();
        for (dst, px) in to.iter_mut().zip(from) {
            *dst = f(px);
        }
        src.truncate(start * C);
        src.shrink_to_fit();
        end = start;
    }
    out
}

/// The RGBA8 bytes of a decoded image, as the shared buffer of a [`crate::Picture`]. Consumes the image (see the
/// module's memory note).
pub(crate) fn to_rgba8(image: DynamicImage, floats: Floats) -> Arc<[u8]> {
    match image {
        DynamicImage::ImageRgba8(buffer) => Arc::from(buffer.into_raw()),
        DynamicImage::ImageRgb8(buffer) => {
            let raw = buffer.into_raw();
            (0..raw.len() / 3 * 4)
                .map(|i| match i & 3 {
                    3 => 255,
                    c => raw[(i >> 2) * 3 + c],
                })
                .collect()
        }
        DynamicImage::ImageLuma8(buffer) => {
            let raw = buffer.into_raw();
            (0..raw.len() * 4)
                .map(|i| if i & 3 == 3 { 255 } else { raw[i >> 2] })
                .collect()
        }
        DynamicImage::ImageLumaA8(buffer) => {
            let raw = buffer.into_raw();
            (0..raw.len() * 2)
                .map(|i| {
                    let at = (i >> 2) * 2;
                    if i & 3 == 3 { raw[at + 1] } else { raw[at] }
                })
                .collect()
        }
        DynamicImage::ImageLuma16(buffer) => {
            let raw = buffer.into_raw();
            (0..raw.len() * 4)
                .map(|i| if i & 3 == 3 { 255 } else { from16(raw[i >> 2]) })
                .collect()
        }
        DynamicImage::ImageLumaA16(buffer) => {
            let raw = buffer.into_raw();
            (0..raw.len() * 2)
                .map(|i| {
                    let at = (i >> 2) * 2;
                    from16(if i & 3 == 3 { raw[at + 1] } else { raw[at] })
                })
                .collect()
        }
        DynamicImage::ImageRgba16(buffer) => {
            Arc::from(from_the_end::<u16, 4>(buffer.into_raw(), |c| {
                [from16(c[0]), from16(c[1]), from16(c[2]), from16(c[3])]
            }))
        }
        DynamicImage::ImageRgb16(buffer) => {
            Arc::from(from_the_end::<u16, 3>(buffer.into_raw(), |c| {
                [from16(c[0]), from16(c[1]), from16(c[2]), 255]
            }))
        }
        DynamicImage::ImageRgb32F(buffer) => {
            let colour = colour_of(floats);
            Arc::from(from_the_end::<f32, 3>(buffer.into_raw(), |c| {
                [colour(c[0]), colour(c[1]), colour(c[2]), 255]
            }))
        }
        DynamicImage::ImageRgba32F(buffer) => {
            let colour = colour_of(floats);
            Arc::from(from_the_end::<f32, 4>(buffer.into_raw(), |c| {
                [colour(c[0]), colour(c[1]), colour(c[2]), display(c[3])]
            }))
        }
        // `DynamicImage` is non-exhaustive: a future type goes through `image`'s own conversion.
        other => Arc::from(other.into_rgba8().into_raw()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma, LumaA, Rgb, Rgba};

    #[test]
    fn every_type_and_the_steps_of_wide_types() {
        // More pixels than a step, and not a multiple of it, so the last step is partial.
        let n = STEP_PIXELS as u32 * 2 + 3;
        let rgb16 = ImageBuffer::<Rgb<u16>, _>::from_fn(n, 1, |x, _| {
            Rgb([x as u16, (x * 7) as u16, 65_535 - x as u16])
        });
        let out = to_rgba8(DynamicImage::ImageRgb16(rgb16.clone()), Floats::Display);
        assert_eq!(out.len(), n as usize * 4);
        for (x, px) in out.as_chunks::<4>().0.iter().enumerate() {
            let c = rgb16.get_pixel(x as u32, 0).0;
            assert_eq!(px, &[from16(c[0]), from16(c[1]), from16(c[2]), 255], "{x}");
        }
        let rgba16 = ImageBuffer::<Rgba<u16>, _>::from_fn(n, 1, |x, _| {
            Rgba([x as u16, 3, 32_767, (x * 13) as u16])
        });
        let out = to_rgba8(DynamicImage::ImageRgba16(rgba16.clone()), Floats::Display);
        for (x, px) in out.as_chunks::<4>().0.iter().enumerate() {
            let c = rgba16.get_pixel(x as u32, 0).0;
            assert_eq!(px, &c.map(from16), "{x}");
        }
        let small = |image: DynamicImage| to_rgba8(image, Floats::Display).to_vec();
        let rgb8 = ImageBuffer::<Rgb<u8>, _>::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(small(rgb8.into()), [1, 2, 3, 255, 4, 5, 6, 255]);
        let l8 = ImageBuffer::<Luma<u8>, _>::from_raw(2, 1, vec![7, 8]).unwrap();
        assert_eq!(small(l8.into()), [7, 7, 7, 255, 8, 8, 8, 255]);
        let la8 = ImageBuffer::<LumaA<u8>, _>::from_raw(2, 1, vec![7, 1, 8, 2]).unwrap();
        assert_eq!(small(la8.into()), [7, 7, 7, 1, 8, 8, 8, 2]);
        let l16 = ImageBuffer::<Luma<u16>, _>::from_raw(1, 1, vec![65_535]).unwrap();
        assert_eq!(small(l16.into()), [255, 255, 255, 255]);
        let la16 = ImageBuffer::<LumaA<u16>, _>::from_raw(1, 1, vec![0, 257]).unwrap();
        assert_eq!(small(la16.into()), [0, 0, 0, 1]);
        let rgba8 = ImageBuffer::<Rgba<u8>, _>::from_raw(1, 1, vec![9, 8, 7, 6]).unwrap();
        assert_eq!(small(rgba8.into()), [9, 8, 7, 6]);
        let f = ImageBuffer::<Rgba<f32>, _>::from_raw(1, 1, vec![1.0, 0.0, f32::NAN, 0.5]).unwrap();
        assert_eq!(small(f.into()), [255, 0, 0, 128]);
    }
}
