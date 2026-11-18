//! Picture Compare for Hikasa (contract `docs/design/m2-contracts.md` §1.12, W12b): decoding to RGBA8, the
//! picture model, pixel classes and the verdict, geometry, rendering, statistics and the pixel probe, PNG output.
//!
//! | Module | Content |
//! |---|---|
//! | [`codec`] | [`decode`], [`decode_bytes`], [`decode_vec`], [`decode_pair`], [`decode_two`] (in parallel or one after the other by [`decode_peak`]): `image`'s decoders, normalization to RGBA8, the HDR/EXR tone map, EXIF orientation (D-337); [`encode_png`] (D-343) |
//! | [`model`] | [`Picture`], [`PictureInfo`], [`RgbaImage`], [`Rgba8`], [`Rect`] (D-338) |
//! | [`geometry`] | [`Orientation`] (the dihedral group), [`Geometry`], Auto Scale's fixed-point bilinear ([`geometry::scale_bilinear`]) (D-340) |
//! | [`compare`] | [`compare_pictures`], [`PictureComparison`], [`distance`], [`PixelClass`], [`PictureVerdict`] (D-339) |
//! | [`render`] | [`DiffMode`], [`Pane`], [`PictureColors`], [`TileRect`], [`composite_checkerboard`]; `render_tile` and `render` on the comparison (D-341) |
//! | [`stats`] | [`PictureStats`], [`PixelProbe`] (D-342) |
//! | [`comparator`] | [`PictureComparator`]: the Rules content test of Picture-kind formats (D-344) |
//! | [`vocab`] | the words of pictures and their parsers |
//!
//! Every read goes through `hikasa-vfs`; `image` is named only under `codec/` (D-352). Every class, render and
//! scaling rule is integer arithmetic, and the only floating-point rules (float sources, the tone map) go through
//! IEEE operations with a pinned table, so results are identical on every platform (T1). Parallel work (bands of
//! rows with `rayon`) never changes a result (T2): every sum is an integer.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod codec;
pub mod comparator;
pub mod compare;
pub mod error;
pub mod geometry;
pub mod model;
pub mod render;
pub mod stats;
pub mod vocab;

pub use codec::{
    DecodeOptions, Encoded, PARALLEL_DECODE_BUDGET, PngLevel, decode, decode_bytes, decode_pair,
    decode_peak, decode_two, decode_two_within, decode_vec, encode_png, encode_png_to, oetf_table,
    read_encoded,
};
pub use comparator::{PictureComparator, PictureConverter};
pub use compare::{
    PictureComparison, PictureOptions, PictureVerdict, PixelClass, classify, compare_pictures,
    distance,
};
pub use error::{PictureError, PictureWarning, PictureWarningCode};
pub use geometry::{Geometry, MAX_OFFSET, Orientation};
pub use model::{
    FrameCount, ImageFormatName, Picture, PictureInfo, Rect, Rgba8, RgbaImage, RgbaTile,
    SourceColor,
};
pub use render::{DiffMode, Pane, PictureColors, TileRect, composite_checkerboard};
pub use stats::{PictureStats, PixelProbe};
pub use vocab::{ToneMap, VocabError, Zoom, parse_zoom};
