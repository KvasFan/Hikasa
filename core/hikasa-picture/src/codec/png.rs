//! PNG output (PX-009, §1.12.20, D-343): `image`'s PNG encoder, RGBA8, adaptive filter, fast compression by
//! default, balanced for clipboard images and best for reports. Writes `--diff-output`, the images of picture reports and clipboard captures
//! (`hikasa_session::clipboard::encode_png_rgba` delegates here).

use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use serde::{Deserialize, Serialize};

use crate::error::PictureError;
use crate::model::RgbaImage;

/// Compression effort of [`encode_png`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PngLevel {
    /// Fast compression (default: `--diff-output`).
    #[default]
    Fast,
    /// `image`'s default level: about a third of `Fast`'s size on flat images at three times its time (clipboard
    /// images, `encode_png_rgba`).
    Balanced,
    /// Best compression (reports).
    Best,
}

/// Encodes an RGBA8 image as a PNG (colour type 6, 8 bits, adaptive filter).
///
/// # Errors
///
/// [`PictureError::Decode`] (kind `invalid_data`, format `png`) when the image is empty or its buffer is not
/// `width · height · 4` bytes, the only inputs the encoder refuses.
pub fn encode_png(image: &RgbaImage, level: PngLevel) -> Result<Vec<u8>, PictureError> {
    let mut out = Vec::with_capacity(image.rgba.len() / 4 + 1024);
    encode_png_to(image, level, &mut out).map_err(|e| PictureError::Decode {
        format: "png".into(),
        message: e.to_string(),
    })?;
    Ok(out)
}

/// [`encode_png`] into `out` as it is compressed (W12c review fixes, F3: a file target never holds the whole PNG).
///
/// # Errors
///
/// `out`'s errors as they are; an image the encoder refuses (empty, a buffer that is not `width · height · 4`
/// bytes) as [`std::io::ErrorKind::InvalidInput`].
pub fn encode_png_to(
    image: &RgbaImage,
    level: PngLevel,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    let need = u64::from(image.width) * u64::from(image.height) * 4;
    if image.width == 0 || image.height == 0 || image.rgba.len() as u64 != need {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "an image of {}x{} pixels needs {need} bytes, got {}",
                image.width,
                image.height,
                image.rgba.len()
            ),
        ));
    }
    let compression = match level {
        PngLevel::Fast => CompressionType::Fast,
        PngLevel::Balanced => CompressionType::Default,
        PngLevel::Best => CompressionType::Best,
    };
    PngEncoder::new_with_quality(out, compression, FilterType::Adaptive)
        .write_image(
            &image.rgba,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| match e {
            image::ImageError::IoError(e) => e,
            other => std::io::Error::other(other.to_string()),
        })
}
