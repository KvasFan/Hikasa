//! `hikasa.picture_compare.v1` (§1.12.24, D-347): built from the picture model as `hikasa.table_compare.v1` is from
//! the table one (T-20). Counts are JSON numbers; percentages are numbers rounded to 4 decimals; arrays are never
//! `null`.

use hikasa_core::{ContentResult, Side, Warning};
use hikasa_picture::{DiffMode, PixelProbe, Rect, Rgba8};
use serde::Serialize;

use super::FormatJson;
use crate::picture::{PictureSource, class_word};

/// One side: `{spec, size, mtime, format, decoder, width, height, color, bit_depth, has_alpha, frames, orientation,
/// tone_map, converted}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PictureSideJson {
    /// The canonical spec.
    pub spec: String,
    /// File size in bytes.
    pub size: u64,
    /// Modification time, RFC 3339 UTC.
    pub mtime: Option<String>,
    /// The Hikasa format of the Picture kind, `null` when none matched.
    pub format: Option<FormatJson>,
    /// The container decoded (`png`, `jpeg`, …).
    pub decoder: &'static str,
    /// Width in pixels (after the EXIF orientation).
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The source colour type (`rgb8`, `rgba16`, …).
    pub color: &'static str,
    /// Bits a channel of the source.
    pub bit_depth: u8,
    /// The source has an alpha channel.
    pub has_alpha: bool,
    /// Frames of the container (`null`: unknown).
    pub frames: Option<u32>,
    /// The EXIF orientation applied (`null`: none).
    pub orientation: Option<&'static str>,
    /// The tone map of an HDR or OpenEXR picture (`null` otherwise).
    pub tone_map: Option<&'static str>,
    /// Decoded from its format's load conversion.
    pub converted: bool,
}

/// `{left, right}` orientations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrientationsJson {
    /// Left.
    pub left: &'static str,
    /// Right.
    pub right: &'static str,
}

/// The options: `{mode, tolerance, blend, ignore_unimportant, auto_scale, tone_map, orientation, offset}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PictureOptionsJson {
    /// `tolerance`, `range`, `blend`.
    pub mode: &'static str,
    /// 0…255.
    pub tolerance: u8,
    /// The blend percentage (50 outside blend mode).
    pub blend: u8,
    /// Similar pixels count as same for the exit code.
    pub ignore_unimportant: bool,
    /// Auto Scale.
    pub auto_scale: bool,
    /// `clamp`, `reinhard`.
    pub tone_map: &'static str,
    /// Each side's orientation word.
    pub orientation: OrientationsJson,
    /// The right picture's origin in left coordinates.
    pub offset: [i32; 2],
}

/// A rectangle: `{x, y, width, height}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RectJson {
    /// Left edge.
    pub x: i64,
    /// Top edge.
    pub y: i64,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

impl RectJson {
    fn new(r: Rect) -> RectJson {
        RectJson {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        }
    }
}

/// The geometry: `{area, scaled, scaled_to}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PictureGeometryJson {
    /// The area in left-picture coordinates.
    pub area: RectJson,
    /// The side Auto Scale resampled (`null`: none).
    pub scaled: Option<&'static str>,
    /// Its size after scaling.
    pub scaled_to: Option<[u32; 2]>,
}

/// Statistics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PictureStatsJson {
    /// Positions covered by at least one picture.
    pub pixels: u64,
    /// Same positions.
    pub same: u64,
    /// Similar positions.
    pub similar: u64,
    /// Different positions.
    pub different: u64,
    /// Positions only the left picture covers.
    pub left_only: u64,
    /// Positions only the right picture covers.
    pub right_only: u64,
    /// `same / pixels` in percent, 4 decimals.
    pub same_percent: f64,
    /// `similar / pixels` in percent.
    pub similar_percent: f64,
    /// `different / pixels` in percent.
    pub different_percent: f64,
    /// The largest distance.
    pub max_distance: u8,
    /// The mean distance, 4 decimals.
    pub mean_distance: f64,
    /// The bounds of the non-same positions in area coordinates (`null`: none).
    pub bounds: Option<RectJson>,
}

/// A side of a probe: `{x, y, rgba}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProbeSideJson {
    /// Column in the oriented (and scaled) picture.
    pub x: u32,
    /// Row.
    pub y: u32,
    /// `[r, g, b, a]`.
    pub rgba: [u8; 4],
}

/// A probe: `{x, y, left, right, distance, class}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PixelProbeJson {
    /// Area column.
    pub x: u32,
    /// Area row.
    pub y: u32,
    /// The left pixel there (`null`: not covered).
    pub left: Option<ProbeSideJson>,
    /// The right pixel there.
    pub right: Option<ProbeSideJson>,
    /// The distance.
    pub distance: u8,
    /// `same`, `similar`, `different`, `outside`.
    pub class: &'static str,
}

impl PixelProbeJson {
    /// The DTO of a probe.
    pub fn new(p: &PixelProbe) -> PixelProbeJson {
        let side = |s: &Option<(u32, u32, Rgba8)>| {
            s.map(|(x, y, c)| ProbeSideJson {
                x,
                y,
                rgba: c.to_array(),
            })
        };
        PixelProbeJson {
            x: p.area_x,
            y: p.area_y,
            left: side(&p.left),
            right: side(&p.right),
            distance: p.distance,
            class: class_word(p.class),
        }
    }
}

/// The image `--diff-output` wrote: `{target, pane, mode, width, height, checkerboard}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffOutputJson {
    /// The target as given.
    pub target: String,
    /// `difference`, `left`, `right`.
    pub pane: &'static str,
    /// `tolerance`, `range`, `blend`.
    pub mode: &'static str,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
    /// Transparent pixels over the checkerboard.
    pub checkerboard: bool,
}

/// `hikasa.picture_compare.v1` (§1.12.24).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PictureCompareJson {
    /// Left side.
    pub left: PictureSideJson,
    /// Right side.
    pub right: PictureSideJson,
    /// The options.
    pub options: PictureOptionsJson,
    /// The geometry.
    pub geometry: PictureGeometryJson,
    /// The verdict (`{"result": …}`).
    pub verdict: ContentResult,
    /// Statistics.
    pub stats: PictureStatsJson,
    /// `--probe` results in order.
    pub probes: Vec<PixelProbeJson>,
    /// The `--diff-output` image (`null`: none written).
    pub diff_output: Option<DiffOutputJson>,
    /// The comparison's warnings.
    pub warnings: Vec<Warning>,
}

fn side_json(src: &PictureSource<'_>, side: Side) -> PictureSideJson {
    let s = src.side(side);
    let info = src.info(side);
    PictureSideJson {
        spec: s.spec.clone(),
        size: s.size,
        mtime: s.mtime.map(|t| t.to_string()),
        format: s.format.clone(),
        decoder: info.format.as_str(),
        width: info.width,
        height: info.height,
        color: info.color.as_str(),
        bit_depth: info.bit_depth,
        has_alpha: info.has_alpha,
        frames: info.frames.count(),
        orientation: info.orientation.map(|o| o.as_str()),
        tone_map: info.hdr.map(|t| t.as_str()),
        converted: info.converted,
    }
}

/// Builds `hikasa.picture_compare.v1`.
pub fn picture_compare_json(src: &PictureSource<'_>) -> PictureCompareJson {
    let cmp = src.comparison;
    let o = cmp.options();
    let g = &o.geometry;
    let s = cmp.stats();
    let blend = match src.mode {
        DiffMode::Blend { percent } => percent,
        _ => DiffMode::DEFAULT_BLEND,
    };
    PictureCompareJson {
        left: side_json(src, Side::Left),
        right: side_json(src, Side::Right),
        options: PictureOptionsJson {
            mode: src.mode.as_str(),
            tolerance: o.tolerance,
            blend,
            ignore_unimportant: src.ignore_unimportant,
            auto_scale: g.auto_scale,
            tone_map: src.tone_map.as_str(),
            orientation: OrientationsJson {
                left: g.orientation.left.as_str(),
                right: g.orientation.right.as_str(),
            },
            offset: [g.offset.0, g.offset.1],
        },
        geometry: PictureGeometryJson {
            area: RectJson::new(cmp.area()),
            scaled: cmp.scaled().map(|(side, _)| side.as_str()),
            scaled_to: cmp.scaled().map(|(_, (w, h))| [w, h]),
        },
        verdict: src.verdict(),
        stats: PictureStatsJson {
            pixels: s.pixels,
            same: s.same,
            similar: s.similar,
            different: s.different,
            left_only: s.left_only,
            right_only: s.right_only,
            same_percent: s.percent(s.same),
            similar_percent: s.percent(s.similar),
            different_percent: s.percent(s.different),
            max_distance: s.max_distance,
            mean_distance: s.mean_distance(),
            bounds: s.bounds.map(RectJson::new),
        },
        probes: src.probes.iter().map(PixelProbeJson::new).collect(),
        diff_output: src.diff_output.map(|d| DiffOutputJson {
            target: d.target.clone(),
            pane: d.pane.as_str(),
            mode: d.mode.as_str(),
            width: d.width,
            height: d.height,
            checkerboard: d.checkerboard,
        }),
        warnings: src.warnings.to_vec(),
    }
}
