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
            PictureError::TooLarge { .. } => ErrorKind::TooLarge,
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
