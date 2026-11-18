//! Decoding (PX-001, §1.12.14, D-337) and PNG output (§1.12.20, D-343). The only module that names `image`
//! (check 2): containers by content, then by the name; normalization to RGBA8 (`normalize.rs`); the HDR/EXR tone
//! map through a pinned table ([`tonemap`]); the EXIF orientation applied; ICC profiles ignored with a warning;
//! the first frame; `max_pixels` checked from the header (and every GIF frame's descriptor) before any pixel buffer
//! is allocated, and the decoder's allocation budget derived from the checked sizes.

pub(crate) mod frames;
pub(crate) mod normalize;
pub mod png;
pub mod tonemap;

use std::io::{Cursor, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use hikasa_core::{Ctl, Pair, Phase, Progress, Spec, Unit};
use hikasa_vfs::{EntryKind, FollowLinks, Vfs, VfsError, VfsOp};
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};

pub use self::png::{PngLevel, encode_png, encode_png_to};
pub use self::tonemap::oetf_table;
use crate::error::PictureError;
use crate::geometry::{Orientation, orient};
use crate::model::{ImageFormatName, Picture, PictureInfo, SourceColor};
use crate::vocab::ToneMap;
use normalize::{Floats, to_rgba8};

/// Default `max_pixels`: 268,435,456 (256 MP, `--max-megapixels 256`).
pub const DEFAULT_MAX_PIXELS: u64 = 1 << 28;

/// Default `max_bytes`: 2 GiB.
pub const DEFAULT_MAX_BYTES: u64 = 2 << 30;

/// Options of [`decode`] and [`decode_bytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecodeOptions {
    /// Largest picture in pixels (`--max-megapixels`), checked from the header before allocation.
    pub max_pixels: u64,
    /// Largest file read (2 GiB by default): above it, `too_large` before reading.
    pub max_bytes: u64,
    /// The tone map of HDR and OpenEXR pictures.
    pub tone_map: ToneMap,
}

impl Default for DecodeOptions {
    fn default() -> DecodeOptions {
        DecodeOptions {
            max_pixels: DEFAULT_MAX_PIXELS,
            max_bytes: DEFAULT_MAX_BYTES,
            tone_map: ToneMap::Clamp,
        }
    }
}

/// The last component of a spec (the name hint of the container's extension).
fn spec_name(spec: &Spec) -> String {
    let canonical = spec.canonical();
    canonical
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&canonical)
        .to_owned()
}

/// Reads `spec` through `vfs` (cancellable; progress `reading` per MiB; bounded by `max_bytes`).
fn read_spec(
    vfs: &dyn Vfs,
    spec: &Spec,
    max_bytes: u64,
    ctl: &Ctl<'_>,
) -> Result<Vec<u8>, PictureError> {
    let display = spec.canonical();
    let too_large = || {
        PictureError::from_vfs(
            &display,
            VfsError::TooLarge {
                spec: display.clone(),
                limit: max_bytes,
            },
        )
    };
    let entry = vfs
        .stat(spec, FollowLinks::Yes)
        .map_err(|e| PictureError::from_vfs(&display, e))?;
    if entry.kind == EntryKind::Dir {
        return Err(PictureError::from_vfs(
            &display,
            VfsError::IsADirectory {
                spec: display.clone(),
            },
        ));
    }
    if entry.size > max_bytes {
        return Err(too_large());
    }
    let mut reader = vfs
        .open_read_with(spec, ctl.cancel)
        .map_err(|e| PictureError::from_vfs(&display, e))?;
    let mut out = Vec::with_capacity(usize::try_from(entry.size).unwrap_or(0));
    let mut buf = vec![0u8; 1 << 20];
    loop {
        ctl.check()?;
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(PictureError::from_vfs(
                    &display,
                    VfsError::from_io(display.as_str(), VfsOp::Read, e),
                ));
            }
        };
        if out.len() as u64 + n as u64 > max_bytes {
            return Err(too_large());
        }
        out.extend_from_slice(&buf[..n]);
        ctl.progress(Phase::Reading, || Progress {
            phase: Phase::Reading,
            done: out.len() as u64,
            total: Some(entry.size),
            unit: Unit::Bytes,
            current: Some(display.clone()),
        });
    }
    Ok(out)
}

/// Decodes the picture `spec` names: read through the VFS (bounded by `max_bytes`), then [`decode_bytes`] with
/// the spec's last component as the name hint. `PictureInfo::file_size` is the bytes read.
///
/// # Errors
///
/// [`PictureError`]: `Io` (kind of the VFS error; `too_large` above `max_bytes`), and those of
/// [`decode_bytes`].
pub fn decode(
    vfs: &dyn Vfs,
    spec: &Spec,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Result<Picture, PictureError> {
    let encoded = read_encoded(vfs, spec, opts, ctl)?;
    decode_vec(encoded.bytes, encoded.name.as_deref(), opts, ctl)
}

/// Reads the picture `spec` names through the VFS, bounded by `max_bytes`, with the spec's last component as the name
/// hint: what [`decode`] decodes ([`decode_two`] takes two of them).
///
/// # Errors
///
/// [`PictureError`]: `Io` (kind of the VFS error; `too_large` above `max_bytes`), [`PictureError::Cancelled`].
pub fn read_encoded(
    vfs: &dyn Vfs,
    spec: &Spec,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Result<Encoded, PictureError> {
    ctl.check()?;
    Ok(Encoded {
        bytes: read_spec(vfs, spec, opts.max_bytes, ctl)?,
        name: Some(spec_name(spec)),
    })
}

/// Reads both sides in parallel, then [`decode_two`]; each side's result is its own (RPT-004's per-side errors).
pub fn decode_pair(
    vfs: &dyn Vfs,
    specs: Pair<&Spec>,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Pair<Result<Picture, PictureError>> {
    let (left, right) = rayon::join(
        || read_encoded(vfs, specs.left, opts, ctl),
        || read_encoded(vfs, specs.right, opts, ctl),
    );
    match (left, right) {
        (Ok(l), Ok(r)) => decode_two(l, r, opts, ctl),
        (l, r) => Pair::new(
            l.and_then(|e| decode_vec(e.bytes, e.name.as_deref(), opts, ctl)),
            r.and_then(|e| decode_vec(e.bytes, e.name.as_deref(), opts, ctl)),
        ),
    }
}

/// The container of `bytes`: by content (the magic bytes), else by the extension of `name_hint`.
fn detect(bytes: &[u8], name_hint: Option<&str>) -> Result<ImageFormat, PictureError> {
    if let Ok(format) = image::guess_format(bytes) {
        return Ok(format);
    }
    let unsupported = |format: &str| PictureError::Unsupported {
        format: format.to_owned(),
    };
    let head = bytes.get(..bytes.len().min(256)).unwrap_or_default();
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && text.contains("<svg")) {
        return Err(unsupported("svg"));
    }
    let extension = name_hint
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match extension.as_deref() {
        Some("svg" | "svgz") => Err(unsupported("svg")),
        Some(ext) => ImageFormat::from_extension(ext).ok_or_else(|| unsupported("unknown")),
        None => Err(unsupported("unknown")),
    }
}

/// The supported container of `format`, or `Unsupported` naming it.
fn supported(format: ImageFormat) -> Result<ImageFormatName, PictureError> {
    Ok(match format {
        ImageFormat::Png => ImageFormatName::Png,
        ImageFormat::Jpeg => ImageFormatName::Jpeg,
        ImageFormat::Gif => ImageFormatName::Gif,
        ImageFormat::Bmp => ImageFormatName::Bmp,
        ImageFormat::Ico => ImageFormatName::Ico,
        ImageFormat::Tiff => ImageFormatName::Tiff,
        ImageFormat::WebP => ImageFormatName::WebP,
        ImageFormat::Tga => ImageFormatName::Tga,
        ImageFormat::Pnm => ImageFormatName::Pnm,
        ImageFormat::Qoi => ImageFormatName::Qoi,
        ImageFormat::OpenExr => ImageFormatName::OpenExr,
        ImageFormat::Hdr => ImageFormatName::Hdr,
        other => {
            let name = match other {
                ImageFormat::Dds => "dds",
                ImageFormat::Avif => "avif",
                ImageFormat::Farbfeld => "farbfeld",
                _ => other.extensions_str().first().copied().unwrap_or("unknown"),
            };
            return Err(PictureError::Unsupported {
                format: name.to_owned(),
            });
        }
    })
}

/// The engine's element of an EXIF orientation.
fn from_exif(orientation: image::metadata::Orientation) -> Orientation {
    use image::metadata::Orientation as Exif;
    match orientation {
        Exif::NoTransforms => Orientation::Identity,
        Exif::Rotate90 => Orientation::Cw90,
        Exif::Rotate180 => Orientation::Rot180,
        Exif::Rotate270 => Orientation::Ccw90,
        Exif::FlipHorizontal => Orientation::FlipH,
        Exif::FlipVertical => Orientation::FlipV,
        Exif::Rotate90FlipH => Orientation::Transpose,
        Exif::Rotate270FlipH => Orientation::Transverse,
    }
}

fn source_color(color: image::ColorType) -> SourceColor {
    use image::ColorType as C;
    match color {
        C::L8 => SourceColor::L8,
        C::La8 => SourceColor::La8,
        C::Rgb8 => SourceColor::Rgb8,
        C::L16 => SourceColor::L16,
        C::La16 => SourceColor::La16,
        C::Rgb16 => SourceColor::Rgb16,
        C::Rgba16 => SourceColor::Rgba16,
        C::Rgb32F => SourceColor::Rgb32f,
        C::Rgba32F => SourceColor::Rgba32f,
        _ => SourceColor::Rgba8,
    }
}

/// The error of an `image` failure on a `format` container.
fn image_error(format: ImageFormatName, max_pixels: u64, error: ImageError) -> PictureError {
    match error {
        ImageError::Limits(_) => PictureError::TooLarge {
            pixels: None,
            limit: max_pixels,
        },
        // A recognized container with a feature no decoder reads (a TIFF compression, arithmetic-coded JPEG):
        // `unsupported`, so the folder content test falls back to bytes (D-344).
        ImageError::Unsupported(_) => PictureError::Unsupported {
            format: format.as_str().to_owned(),
        },
        other => PictureError::Decode {
            format: format.as_str().to_owned(),
            message: other.to_string(),
        },
    }
}

/// The decoder's allocation budget for a picture of `pixels` pixels whose GIF frame has `frame_pixels`: 16 bytes
/// a pixel (RGBA of 32-bit floats, or an 8-bit picture and its intermediate copies), 4 bytes a frame pixel, and
/// 64 MiB for the decoder's own tables.
fn alloc_budget(pixels: u64, frame_pixels: u64) -> u64 {
    pixels
        .saturating_mul(16)
        .saturating_add(frame_pixels.saturating_mul(4))
        .saturating_add(64 << 20)
}

/// What one decode produced before normalization.
struct Decoded {
    image: DynamicImage,
    color: SourceColor,
    bit_depth: u8,
    orientation: Orientation,
    icc: bool,
}

fn decode_image(
    bytes: &[u8],
    image_format: ImageFormat,
    format: ImageFormatName,
    opts: &DecodeOptions,
) -> Result<Decoded, PictureError> {
    let fail = |e| image_error(format, opts.max_pixels, e);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format);
    let side = u32::try_from(opts.max_pixels).unwrap_or(u32::MAX);
    let limits = |max_alloc: u64| {
        let mut limits = Limits::default();
        limits.max_image_width = Some(side);
        limits.max_image_height = Some(side);
        limits.max_alloc = Some(max_alloc);
        limits
    };
    // A GIF decodes its first frame into a buffer of its own when it does not match the logical screen (which is
    // all `dimensions()` tells): every frame's descriptor is checked against `max_pixels` before decoding.
    let frame_pixels = if image_format == ImageFormat::Gif {
        frames::gif_largest_frame(bytes)
    } else {
        0
    };
    if frame_pixels > opts.max_pixels {
        return Err(PictureError::TooLarge {
            pixels: Some(frame_pixels),
            limit: opts.max_pixels,
        });
    }
    // Reading the header: the budget of the largest picture the limit allows.
    reader.limits(limits(alloc_budget(opts.max_pixels, 0)));
    let mut decoder = reader.into_decoder().map_err(fail)?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > opts.max_pixels {
        return Err(PictureError::TooLarge {
            pixels: Some(pixels),
            limit: opts.max_pixels,
        });
    }
    // Decoding: the budget of this picture (and of its GIF frame).
    decoder
        .set_limits(limits(alloc_budget(pixels, frame_pixels)))
        .map_err(fail)?;
    if pixels == 0 {
        return Err(PictureError::Decode {
            format: format.as_str().to_owned(),
            message: format!("the picture has no pixels ({width}x{height})"),
        });
    }
    let color = source_color(decoder.color_type());
    let original = decoder.original_color_type();
    let bit_depth = match original.channel_count() {
        0 => color.channel_bits(),
        n => (original.bits_per_pixel() / u16::from(n)).clamp(1, 32) as u8,
    };
    // A corrupt EXIF block or profile never fails the picture: no orientation, no profile.
    let orientation = decoder
        .orientation()
        .map(from_exif)
        .unwrap_or(Orientation::Identity);
    let icc = decoder
        .icc_profile()
        .ok()
        .flatten()
        .is_some_and(|profile| !frames::is_srgb_profile(&profile));
    let image = DynamicImage::from_decoder(decoder).map_err(fail)?;
    Ok(Decoded {
        image,
        color,
        bit_depth,
        orientation,
        icc,
    })
}

/// The bytes a decode reads: borrowed ([`decode_bytes`]) or owned ([`decode_vec`], dropped once decoded).
enum Input<'a> {
    Borrowed(&'a [u8]),
    Owned(Vec<u8>),
}

impl Input<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Input::Borrowed(b) => b,
            Input::Owned(v) => v,
        }
    }
}

/// Decodes a picture from memory: the container by content, then by `name_hint`'s extension; normalized to
/// RGBA8; the HDR/EXR tone map; the EXIF orientation applied; the first frame.
///
/// # Errors
///
/// [`PictureError::Unsupported`] (DDS, AVIF, SVG, unknown), [`PictureError::TooLarge`] (more than `max_pixels`,
/// from the header or a GIF frame's descriptor, or the decoder's allocation limit: `max_pixels · 16 + 64 MiB`
/// while reading the header, then `pixels · 16 + 64 MiB` plus 4 bytes a pixel of a GIF's largest frame),
/// [`PictureError::Decode`] (truncated or corrupt; a decoder panic is caught and reported so),
/// [`PictureError::Cancelled`] (checked before and after the decode, which is not interruptible).
pub fn decode_bytes(
    bytes: &[u8],
    name_hint: Option<&str>,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Result<Picture, PictureError> {
    decode_input(Input::Borrowed(bytes), name_hint, opts, ctl)
}

/// [`decode_bytes`] of bytes it owns: they are dropped as soon as the decoder has produced its pixels, before the
/// normalization (W12c review fixes, F3), so a side never holds its file, the decoded pixels and the RGBA8 result at
/// once.
///
/// # Errors
///
/// Those of [`decode_bytes`].
pub fn decode_vec(
    bytes: Vec<u8>,
    name_hint: Option<&str>,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Result<Picture, PictureError> {
    decode_input(Input::Owned(bytes), name_hint, opts, ctl)
}

fn decode_input(
    input: Input<'_>,
    name_hint: Option<&str>,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Result<Picture, PictureError> {
    ctl.check()?;
    let bytes = input.bytes();
    let image_format = detect(bytes, name_hint)?;
    let format = supported(image_format)?;
    let file_size = bytes.len() as u64;
    ctl.progress_now(Progress {
        phase: Phase::Decoding,
        done: 0,
        total: Some(file_size),
        unit: Unit::Bytes,
        current: name_hint.map(str::to_owned),
    });
    let frames = frames::count(format, bytes);
    let decoded = catch_unwind(AssertUnwindSafe(|| {
        decode_image(bytes, image_format, format, opts)
    }))
    .map_err(|_| PictureError::Decode {
        format: format.as_str().to_owned(),
        message: "the decoder failed on this file".into(),
    })??;
    // The file is no longer needed: its memory goes before the normalization allocates the result.
    drop(input);
    ctl.check()?;
    let floats = if format.is_hdr() {
        Floats::Linear(opts.tone_map)
    } else {
        Floats::Display
    };
    let (mut width, mut height) = (decoded.image.width(), decoded.image.height());
    let mut rgba = to_rgba8(decoded.image, floats);
    if decoded.orientation != Orientation::Identity {
        let oriented = orient(width, height, &rgba, decoded.orientation);
        drop(rgba);
        (width, height, rgba) = (oriented.width, oriented.height, Arc::from(oriented.rgba));
    }
    ctl.check()?;
    Ok(Picture {
        width,
        height,
        rgba,
        info: PictureInfo {
            format,
            width,
            height,
            color: decoded.color,
            bit_depth: decoded.bit_depth,
            has_alpha: decoded.color.has_alpha(),
            frames,
            orientation: (decoded.orientation != Orientation::Identity)
                .then_some(decoded.orientation),
            hdr: format.is_hdr().then_some(opts.tone_map),
            file_size,
            converted: false,
            icc: decoded.icc,
        },
    })
}

/// The decode of two pictures in parallel when their estimated peaks together stay within this (256 MiB); above it
/// one after the other ([`decode_two`]).
pub const PARALLEL_DECODE_BUDGET: u64 = 256 << 20;

/// An upper estimate of the memory one decode of `bytes` holds at its peak: the file, the decoder's pixel buffer
/// (its colour type's bytes, from the header) and the RGBA8 result (4 bytes a pixel). A header that cannot be read
/// counts the file only (the decode fails early).
pub fn decode_peak(bytes: &[u8], name_hint: Option<&str>, opts: &DecodeOptions) -> u64 {
    estimate(bytes, name_hint, opts).0
}

/// [`decode_peak`] and what stays once decoded (the RGBA8 result).
fn estimate(bytes: &[u8], name_hint: Option<&str>, opts: &DecodeOptions) -> (u64, u64) {
    let file = bytes.len() as u64;
    let Ok(image_format) = detect(bytes, name_hint) else {
        return (file, 0);
    };
    if supported(image_format).is_err() {
        return (file, 0);
    }
    let header = catch_unwind(AssertUnwindSafe(|| {
        let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format);
        let mut limits = Limits::default();
        limits.max_alloc = Some(alloc_budget(opts.max_pixels, 0));
        reader.limits(limits);
        let decoder = reader.into_decoder().ok()?;
        let (w, h) = decoder.dimensions();
        Some((decoder.total_bytes(), u64::from(w) * u64::from(h)))
    }));
    match header {
        Ok(Some((native, pixels))) => {
            let kept = pixels.saturating_mul(4);
            (file.saturating_add(native).saturating_add(kept), kept)
        }
        _ => (file, 0),
    }
}

/// One side to decode: its bytes and the name hint of the container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    /// The file's bytes (or a load conversion's output).
    pub bytes: Vec<u8>,
    /// The name whose extension is the container's hint (`None` for a conversion's output).
    pub name: Option<String>,
}

/// Decodes both sides, each from bytes it owns ([`decode_vec`]: a side's bytes are dropped once decoded): in
/// parallel (`rayon::join`) when the two [`decode_peak`]s together fit [`PARALLEL_DECODE_BUDGET`], else one after
/// the other in the order that holds less at once (the first side's picture stays while the second decodes). Each
/// side's result is its own (RPT-004's per-side errors); both are always decoded.
pub fn decode_two(
    left: Encoded,
    right: Encoded,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Pair<Result<Picture, PictureError>> {
    decode_two_within(left, right, PARALLEL_DECODE_BUDGET, opts, ctl)
}

/// [`decode_two`] with another parallel budget (tests: 0 decodes one side after the other).
pub fn decode_two_within(
    left: Encoded,
    right: Encoded,
    budget: u64,
    opts: &DecodeOptions,
    ctl: &Ctl<'_>,
) -> Pair<Result<Picture, PictureError>> {
    let estimate = |e: &Encoded| estimate(&e.bytes, e.name.as_deref(), opts);
    let ((pl, kl), (pr, kr)) = (estimate(&left), estimate(&right));
    let one = |e: Encoded| decode_vec(e.bytes, e.name.as_deref(), opts, ctl);
    if pl.saturating_add(pr) <= budget {
        let (l, r) = rayon::join(|| one(left), || one(right));
        return Pair::new(l, r);
    }
    // One after the other: while the first decodes, the second's bytes wait; while the second decodes, the first's
    // picture stays.
    let (fl, fr) = (left.bytes.len() as u64, right.bytes.len() as u64);
    let left_first = pl.saturating_add(fr).max(kl.saturating_add(pr))
        <= pr.saturating_add(fl).max(kr.saturating_add(pl));
    if left_first {
        let l = one(left);
        let r = one(right);
        Pair::new(l, r)
    } else {
        let r = one(right);
        let l = one(left);
        Pair::new(l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_by_content_then_name() {
        assert_eq!(
            detect(b"\x89PNG\r\n\x1a\n....", Some("a.jpg")).unwrap(),
            ImageFormat::Png
        );
        assert_eq!(
            detect(b"\0\0\x02\0", Some("x.TGA")).unwrap(),
            ImageFormat::Tga
        );
        assert!(matches!(
            detect(b"<svg xmlns='x'/>", Some("a.png")),
            Err(PictureError::Unsupported { format }) if format == "svg"
        ));
        assert!(matches!(
            supported(detect(b"DDS |....", None).unwrap()),
            Err(PictureError::Unsupported { format }) if format == "dds"
        ));
        assert!(matches!(
            detect(b"hello", Some("notes.txt")),
            Err(PictureError::Unsupported { format }) if format == "unknown"
        ));
    }
}
