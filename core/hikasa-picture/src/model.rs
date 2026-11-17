//! The picture model (§1.12.15, D-338): an immutable, shared [`Picture`] of RGBA8 pixels with its
//! [`PictureInfo`], and the small value types of the engine ([`Rgba8`], [`Rect`], [`RgbaImage`]).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{PictureWarning, PictureWarningCode};
use crate::geometry::Orientation;
use crate::vocab::ToneMap;

/// One RGBA pixel, 8 bits a channel, straight (not premultiplied) alpha.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba8 {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha (0 transparent, 255 opaque).
    pub a: u8,
}

impl Rgba8 {
    /// Fully transparent black, the pixel of a position no picture covers.
    pub const TRANSPARENT: Rgba8 = Rgba8::new(0, 0, 0, 0);

    /// A pixel.
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Rgba8 {
        Rgba8 { r, g, b, a }
    }

    /// An opaque pixel of `0xRRGGBB`.
    pub const fn rgb(hex: u32) -> Rgba8 {
        Rgba8::new((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255)
    }

    /// The four channels in memory order.
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// From four channels in memory order.
    pub const fn from_array(c: [u8; 4]) -> Rgba8 {
        Rgba8::new(c[0], c[1], c[2], c[3])
    }
}

/// A rectangle: `x`, `y` its top-left corner (area coordinates, or left-picture coordinates for the area itself),
/// `width` × `height` pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: i64,
    /// Top edge.
    pub y: i64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// An owned RGBA8 image, row-major, top to bottom (`rgba.len() == width · height · 4`): a render, a tile, a
/// scaled picture.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The pixels, four bytes each.
    pub rgba: Vec<u8>,
}

/// A rendered tile (§1.12.18): an [`RgbaImage`] of at most the tile's size.
pub type RgbaTile = RgbaImage;

impl RgbaImage {
    /// A transparent image of `width` × `height`.
    pub fn transparent(width: u32, height: u32) -> RgbaImage {
        RgbaImage {
            width,
            height,
            rgba: vec![0; width as usize * height as usize * 4],
        }
    }

    /// The pixel at `(x, y)`; `None` outside.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        let c = self.rgba.get(at..at + 4)?;
        Some(Rgba8::new(c[0], c[1], c[2], c[3]))
    }
}

impl fmt::Debug for RgbaImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RgbaImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// The container a picture was decoded from (by content first, D-337).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormatName {
    /// PNG (APNG: the default image).
    Png,
    /// JPEG, baseline and progressive.
    Jpeg,
    /// GIF (the first frame).
    Gif,
    /// BMP.
    Bmp,
    /// ICO (the largest entry).
    Ico,
    /// TIFF (the first page).
    Tiff,
    /// WebP, lossy and lossless (the first frame).
    #[serde(rename = "webp")]
    WebP,
    /// TGA (by the name only: it has no signature).
    Tga,
    /// PBM, PGM, PPM, PAM.
    Pnm,
    /// QOI.
    Qoi,
    /// OpenEXR (the first part), tone-mapped.
    #[serde(rename = "exr")]
    OpenExr,
    /// Radiance HDR, tone-mapped.
    Hdr,
}

impl ImageFormatName {
    /// Every format, in declaration order.
    pub const ALL: [ImageFormatName; 12] = [
        ImageFormatName::Png,
        ImageFormatName::Jpeg,
        ImageFormatName::Gif,
        ImageFormatName::Bmp,
        ImageFormatName::Ico,
        ImageFormatName::Tiff,
        ImageFormatName::WebP,
        ImageFormatName::Tga,
        ImageFormatName::Pnm,
        ImageFormatName::Qoi,
        ImageFormatName::OpenExr,
        ImageFormatName::Hdr,
    ];

    /// The stable lower-case name (`"png"`, `"jpeg"`, `"exr"`, …), identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            ImageFormatName::Png => "png",
            ImageFormatName::Jpeg => "jpeg",
            ImageFormatName::Gif => "gif",
            ImageFormatName::Bmp => "bmp",
            ImageFormatName::Ico => "ico",
            ImageFormatName::Tiff => "tiff",
            ImageFormatName::WebP => "webp",
            ImageFormatName::Tga => "tga",
            ImageFormatName::Pnm => "pnm",
            ImageFormatName::Qoi => "qoi",
            ImageFormatName::OpenExr => "exr",
            ImageFormatName::Hdr => "hdr",
        }
    }

    /// True for the linear-light formats that go through the tone map (HDR, OpenEXR).
    pub const fn is_hdr(self) -> bool {
        matches!(self, ImageFormatName::Hdr | ImageFormatName::OpenExr)
    }
}

impl fmt::Display for ImageFormatName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The decoder's pixel type before normalization (`"rgb8"`, `"rgba16"`, `"l8"`, `"rgb32f"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceColor {
    /// 8-bit grey.
    L8,
    /// 8-bit grey with alpha.
    La8,
    /// 8-bit RGB.
    Rgb8,
    /// 8-bit RGBA.
    Rgba8,
    /// 16-bit grey.
    L16,
    /// 16-bit grey with alpha.
    La16,
    /// 16-bit RGB.
    Rgb16,
    /// 16-bit RGBA.
    Rgba16,
    /// 32-bit float RGB.
    Rgb32f,
    /// 32-bit float RGBA.
    Rgba32f,
}

impl SourceColor {
    /// The stable lower-case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            SourceColor::L8 => "l8",
            SourceColor::La8 => "la8",
            SourceColor::Rgb8 => "rgb8",
            SourceColor::Rgba8 => "rgba8",
            SourceColor::L16 => "l16",
            SourceColor::La16 => "la16",
            SourceColor::Rgb16 => "rgb16",
            SourceColor::Rgba16 => "rgba16",
            SourceColor::Rgb32f => "rgb32f",
            SourceColor::Rgba32f => "rgba32f",
        }
    }

    /// True when the type carries alpha.
    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            SourceColor::La8
                | SourceColor::Rgba8
                | SourceColor::La16
                | SourceColor::Rgba16
                | SourceColor::Rgba32f
        )
    }

    /// Bits of one channel (8, 16 or 32).
    pub const fn channel_bits(self) -> u8 {
        match self {
            SourceColor::L8 | SourceColor::La8 | SourceColor::Rgb8 | SourceColor::Rgba8 => 8,
            SourceColor::L16 | SourceColor::La16 | SourceColor::Rgb16 | SourceColor::Rgba16 => 16,
            SourceColor::Rgb32f | SourceColor::Rgba32f => 32,
        }
    }
}

impl fmt::Display for SourceColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How many frames (or pages) the container holds; only the first is compared.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameCount {
    /// A still picture.
    #[default]
    One,
    /// An animation or a multi-page file of `n` (≥ 2) frames.
    Many(u32),
    /// The container's structure did not say.
    Unknown,
}

impl FrameCount {
    /// The count when known.
    pub const fn count(self) -> Option<u32> {
        match self {
            FrameCount::One => Some(1),
            FrameCount::Many(n) => Some(n),
            FrameCount::Unknown => None,
        }
    }
}

/// What the decoder found (PX-002's File Info data).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureInfo {
    /// The container (by content, else by the name).
    pub format: ImageFormatName,
    /// Width as decoded and oriented.
    pub width: u32,
    /// Height as decoded and oriented.
    pub height: u32,
    /// The decoder's pixel type.
    pub color: SourceColor,
    /// Bits a channel of the source (1 for a bilevel PBM, 16 for 16-bit PNG, 32 for float).
    pub bit_depth: u8,
    /// The source carries alpha.
    pub has_alpha: bool,
    /// Frames or pages of the container.
    pub frames: FrameCount,
    /// The EXIF orientation that was applied (`None`: none, or the identity).
    pub orientation: Option<Orientation>,
    /// The tone map applied (HDR and OpenEXR only).
    pub hdr: Option<ToneMap>,
    /// Bytes of the file.
    pub file_size: u64,
    /// The bytes were produced by a Picture format's load conversion (set by the Service).
    pub converted: bool,
    /// The file carried an ICC profile other than sRGB, which was ignored (`color_profile_ignored`).
    pub icc: bool,
}

/// A decoded picture: immutable, shared (`Arc`), RGBA8 row-major top to bottom. Geometry and comparison never
/// copy it (transforms are index maps, §1.12.17). Memory: 4 bytes a pixel plus the struct.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    /// Width in pixels (after the EXIF orientation).
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width · height · 4` bytes.
    pub rgba: Arc<[u8]>,
    /// What the decoder found.
    pub info: PictureInfo,
}

impl Picture {
    /// The pixel at `(x, y)`; `None` outside.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Rgba8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = (y as usize * self.width as usize + x as usize) * 4;
        let c = self.rgba.get(at..at + 4)?;
        Some(Rgba8::new(c[0], c[1], c[2], c[3]))
    }

    /// Pixels (`width · height`).
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// A picture of RGBA8 bytes with plain PNG-like info (tests, generated inputs, the Service's clipboard path).
    /// `None` when `rgba.len() != width · height · 4`.
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Picture> {
        if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return None;
        }
        let file_size = rgba.len() as u64;
        Some(Picture {
            width,
            height,
            rgba: rgba.into(),
            info: PictureInfo {
                format: ImageFormatName::Png,
                width,
                height,
                color: SourceColor::Rgba8,
                bit_depth: 8,
                has_alpha: true,
                frames: FrameCount::One,
                orientation: None,
                hdr: None,
                file_size,
                converted: false,
                icc: false,
            },
        })
    }

    /// The decoder's warnings for this picture (`color_profile_ignored`, `frames_ignored`,
    /// `orientation_applied`), naming `spec`.
    pub fn warnings(&self, spec: &str) -> Vec<PictureWarning> {
        let mut out = Vec::new();
        let warn = |code, message: String| PictureWarning {
            code,
            message,
            spec: Some(spec.to_owned()),
        };
        if self.info.icc {
            out.push(warn(
                PictureWarningCode::ColorProfileIgnored,
                format!("{spec}: the ICC colour profile was ignored (sRGB assumed)"),
            ));
        }
        if let FrameCount::Many(n) = self.info.frames {
            out.push(warn(
                PictureWarningCode::FramesIgnored,
                format!("{spec}: {n} frames; only the first is compared"),
            ));
        }
        if let Some(orientation) = self.info.orientation {
            out.push(warn(
                PictureWarningCode::OrientationApplied,
                format!("{spec}: the EXIF orientation {orientation} was applied"),
            ));
        }
        out
    }
}

impl fmt::Debug for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picture")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}
