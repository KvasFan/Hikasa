//! [`PictureError`], [`PictureWarning`] and [`PictureWarningCode`] (§1.12.22, D-345).

use hikasa_core::{Cancelled, Classify, ErrorKind};
use hikasa_vfs::VfsError;
use serde::{Deserialize, Serialize};

use crate::vocab::VocabError;

/// Failure of a picture operation. Every variant classifies to an [`ErrorKind`]; the CLI exits 100 with the side's
/// spec (RPT-004).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PictureError {
    /// A container the decoders do not read (DDS, AVIF, SVG, anything unrecognized). Kind `unsupported`.
    #[error("{format}: this picture format is not supported")]
    Unsupported {
        /// The container's name (`dds`, `avif`, `svg`, `unknown`, …).
        format: String,
    },
    /// A truncated or corrupt file. Kind `invalid_data`.
    #[error("cannot decode the {format} picture: {message}")]
    Decode {
        /// The container's name.
        format: String,
        /// What the decoder reported.
        message: String,
    },
    /// More pixels than `max_pixels`, found from the header before any pixel buffer was allocated, or an image
    /// to render larger than the renderer's limit. Kind `too_large`.
    #[error("{}", too_large_text(*.pixels, *.limit))]
    TooLarge {
        /// The pixel count when the header told it (`None` when the decoder refused a dimension first).
        pixels: Option<u64>,
        /// The limit.
        limit: u64,
    },
    /// The compared area (the union of both placed pictures, §1.12.17) is larger than an image rendered whole may
    /// be (W12c review fixes, F11: it is not one of the pictures). Kind `too_large`.
    #[error(
        "the compared area is {width} × {height} = {pixels} pixels, more than the {limit} pixels an image may have"
    )]
    AreaTooLarge {
        /// Width of the area.
        width: u64,
        /// Height of the area.
        height: u64,
        /// `width · height` (saturating).
        pixels: u64,
        /// The limit.
        limit: u64,
    },
    /// The VFS could not stat, open or read an input (including a file above `max_bytes`, kind `too_large`).
    #[error("{source}")]
    Io {
        /// Canonical spec of the input.
        spec: String,
        /// The VFS error.
        #[source]
        source: VfsError,
    },
    /// A word, offset or option that names nothing (kind `invalid_input`).
    #[error(transparent)]
    Vocab(#[from] VocabError),
    /// Cancelled (never a partial result).
    #[error("cancelled")]
    Cancelled,
}

fn too_large_text(pixels: Option<u64>, limit: u64) -> String {
    match pixels {
        Some(pixels) => format!("the picture has {pixels} pixels, more than the limit of {limit}"),
        None => format!("the picture is larger than the limit of {limit} pixels"),
    }
}

impl PictureError {
    /// The error of a VFS failure on `spec`: a cancellation becomes [`PictureError::Cancelled`].
    pub fn from_vfs(spec: &str, error: VfsError) -> PictureError {
        if error.is_cancelled() {
            return PictureError::Cancelled;
        }
        PictureError::Io {
            spec: error.spec().unwrap_or(spec).to_owned(),
            source: error,
        }
    }

    /// True for [`PictureError::Cancelled`].
    pub fn is_cancelled(&self) -> bool {
        matches!(self, PictureError::Cancelled)
    }
}

impl From<Cancelled> for PictureError {
    fn from(_: Cancelled) -> PictureError {
        PictureError::Cancelled
    }
}

impl Classify for PictureError {
    fn kind(&self) -> ErrorKind {
        match self {
            PictureError::Unsupported { .. } => ErrorKind::Unsupported,
            PictureError::Decode { .. } => ErrorKind::InvalidData,
            PictureError::TooLarge { .. } | PictureError::AreaTooLarge { .. } => {
                ErrorKind::TooLarge
            }
            PictureError::Io { source, .. } => source.kind(),
            PictureError::Vocab(_) => ErrorKind::InvalidInput,
            PictureError::Cancelled => ErrorKind::Cancelled,
        }
    }
}

/// Codes of the picture warnings; W12c adds the same names to `hikasa_core::WarningCode` (§1.12.22). The
/// snake_case names are the stable codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PictureWarningCode {
    /// The file carries an ICC profile other than sRGB; it was ignored (sRGB assumed, PX-001).
    ColorProfileIgnored,
    /// Auto Scale resampled a picture whose aspect ratio differs from the other's by more than 1 %.
    AspectRatioDiffers,
    /// An animated or multi-page input: only the first frame was compared (informational).
    FramesIgnored,
    /// The file's EXIF orientation was applied (informational).
    OrientationApplied,
}

impl PictureWarningCode {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            PictureWarningCode::ColorProfileIgnored => "color_profile_ignored",
            PictureWarningCode::AspectRatioDiffers => "aspect_ratio_differs",
            PictureWarningCode::FramesIgnored => "frames_ignored",
            PictureWarningCode::OrientationApplied => "orientation_applied",
        }
    }

    /// True for the informational codes, shown with `--verbose` only (`frames_ignored`, `orientation_applied`).
    pub const fn is_informational(self) -> bool {
        matches!(
            self,
            PictureWarningCode::FramesIgnored | PictureWarningCode::OrientationApplied
        )
    }
}

/// A warning of a picture operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PictureWarning {
    /// The code.
    pub code: PictureWarningCode,
    /// Message (English, stable for tests).
    pub message: String,
    /// The input concerned, if any.
    pub spec: Option<String>,
}

impl PictureWarning {
    /// The warning as the envelope's [`hikasa_core::Warning`] (the codes map one to one, §1.12.22).
    pub fn to_warning(&self) -> hikasa_core::Warning {
        use hikasa_core::WarningCode;
        hikasa_core::Warning {
            code: match self.code {
                PictureWarningCode::ColorProfileIgnored => WarningCode::ColorProfileIgnored,
                PictureWarningCode::AspectRatioDiffers => WarningCode::AspectRatioDiffers,
                PictureWarningCode::FramesIgnored => WarningCode::FramesIgnored,
                PictureWarningCode::OrientationApplied => WarningCode::OrientationApplied,
            },
            message: self.message.clone(),
            spec: self.spec.clone(),
        }
    }
}
