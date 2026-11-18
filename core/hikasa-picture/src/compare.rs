//! Pixel classes, the comparison and the verdict (PX-003, PX-004, §1.12.16–§1.12.17, D-339, D-340).
//!
//! **Distance (normative).** For left pixel `p` and right pixel `q`: 0 when both alphas are 0, else the largest
//! channel difference over R, G, B and A. A position covered by one picture only counts the covering pixel's
//! alpha (it compares against a transparent pixel). **Classes** with tolerance `t`: *same* `d = 0`, *similar*
//! `0 < d ≤ t`, *different* `d > t`.
//!
//! | Situation | `ContentKind` | Exit |
//! |---|---|---|
//! | bytes identical (checked only when every pixel is same) | `binary_same` | 1 |
//! | every compared position same | `rules_same` | 2 |
//! | no *different*, some *similar* | `unimportant_only` | 12 (2 with Ignore Unimportant) |
//! | any *different* | `important_differences` | 13 |
//! | any error | `error` | 100 |
//!
//! The classes depend on the tolerance and the geometry only, never on the display mode.

use std::sync::Arc;

use hikasa_core::{ContentKind, Ctl, ExitCode, Pair, Side};
use serde::{Deserialize, Serialize};

use crate::error::{PictureError, PictureWarning, PictureWarningCode};
use crate::geometry::{
    Geometry, IndexMap, aspect_ratio_differs, oriented_size, scale_view, scaled_side,
};
use crate::model::{Picture, Rect, Rgba8};
use crate::stats::{PictureStats, PixelProbe, scan};

/// The class of a position of the area (§1.12.16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelClass {
    /// `d = 0`.
    Same,
    /// `0 < d ≤ tolerance` (unimportant).
    Similar,
    /// `d > tolerance` (important).
    Different,
    /// Covered by neither picture (not counted, rendered transparent).
    Outside,
}

/// The distance of a position (§1.12.16): `left`/`right` are the covering pixels (`None`: not covered).
#[inline]
pub fn distance(left: Option<Rgba8>, right: Option<Rgba8>) -> u8 {
    match (left, right) {
        (Some(p), Some(q)) => pixel_distance(p.to_array(), q.to_array()),
        (Some(p), None) => p.a,
        (None, Some(q)) => q.a,
        (None, None) => 0,
    }
}

/// The distance of two covering pixels: 0 when both are fully transparent, else the largest channel difference.
#[inline]
pub(crate) fn pixel_distance(p: [u8; 4], q: [u8; 4]) -> u8 {
    if p[3] == 0 && q[3] == 0 {
        return 0;
    }
    p[0].abs_diff(q[0])
        .max(p[1].abs_diff(q[1]))
        .max(p[2].abs_diff(q[2]))
        .max(p[3].abs_diff(q[3]))
}

/// The class of a distance at `tolerance`.
#[inline]
pub const fn classify(distance: u8, tolerance: u8) -> PixelClass {
    if distance == 0 {
        PixelClass::Same
    } else if distance <= tolerance {
        PixelClass::Similar
    } else {
        PixelClass::Different
    }
}

/// What decides the classes (§1.12.16, §1.12.17); the display mode is not here, it never changes them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PictureOptions {
    /// 0…255 (default 0).
    pub tolerance: u8,
    /// Orientations, Auto Scale, offset.
    pub geometry: Geometry,
}

/// The answer of a picture comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PictureVerdict {
    /// The content result.
    pub kind: ContentKind,
    /// The exit code of `picture compare` (never 11).
    pub exit_code: ExitCode,
}

impl PictureVerdict {
    /// The verdict of any error (100).
    pub const ERROR: PictureVerdict = PictureVerdict {
        kind: ContentKind::Error,
        exit_code: ExitCode::UnknownError,
    };

    /// The verdict of statistics: `bytes_identical` is asked only when every position is same (the Service's
    /// `compare_sources` of the inputs).
    ///
    /// # Errors
    ///
    /// Those of `bytes_identical`.
    pub fn of<E>(
        stats: &PictureStats,
        ignore_unimportant: bool,
        bytes_identical: impl FnOnce() -> Result<bool, E>,
    ) -> Result<PictureVerdict, E> {
        let kind = if stats.different > 0 {
            ContentKind::ImportantDifferences
        } else if stats.similar > 0 {
            ContentKind::UnimportantOnly
        } else if bytes_identical()? {
            ContentKind::BinarySame
        } else {
            ContentKind::RulesSame
        };
        Ok(PictureVerdict {
            kind,
            exit_code: ExitCode::for_kind(kind, ignore_unimportant),
        })
    }
}

/// A side's picture as placed in the area: its pixels (the picture's, or Auto Scale's materialized copy), the
/// index map of its orientation, its placed size and its top-left corner in area coordinates.
#[derive(Clone)]
pub(crate) struct Placed {
    pub(crate) data: Arc<[u8]>,
    pub(crate) map: IndexMap,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) x: i64,
    pub(crate) y: i64,
}

impl Placed {
    /// The picture coordinates of the area position `(ax, ay)` when this side covers it.
    #[inline]
    pub(crate) fn covers(&self, ax: i64, ay: i64) -> Option<(u32, u32)> {
        let (x, y) = (ax - self.x, ay - self.y);
        (x >= 0 && y >= 0 && x < i64::from(self.width) && y < i64::from(self.height))
            .then_some((x as u32, y as u32))
    }

    /// The pixel at picture coordinates `(x, y)` (inside the placed size).
    #[inline]
    pub(crate) fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let at = self.map.index(x, y) * 4;
        match self.data.get(at..at + 4) {
            Some(c) => [c[0], c[1], c[2], c[3]],
            None => [0; 4],
        }
    }

    /// The pixel covering the area position, if any.
    #[inline]
    pub(crate) fn pixel(&self, ax: i64, ay: i64) -> Option<[u8; 4]> {
        self.covers(ax, ay).map(|(x, y)| self.at(x, y))
    }

    /// The area columns `[x0, x1)` this side covers on area row `ay`.
    #[inline]
    pub(crate) fn row_span(&self, ay: i64) -> Option<(i64, i64)> {
        (ay >= self.y && ay < self.y + i64::from(self.height))
            .then_some((self.x, self.x + i64::from(self.width)))
    }

    /// The area rectangle of this side.
    pub(crate) fn rect(&self) -> Rect {
        Rect {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

/// Auto Scale's materialized side, kept while the orientations and Auto Scale stay (an offset change reuses it).
#[derive(Clone)]
struct Scaled {
    side: Side,
    orientation: Pair<crate::geometry::Orientation>,
    width: u32,
    height: u32,
    data: Arc<[u8]>,
}

/// A comparison of two pictures under a tolerance and a geometry: the placed sides, the area and the statistics.
/// The pictures stay shared (`Arc`); only Auto Scale materializes one side (4 bytes a pixel). Rendering
/// (`render_tile`, `render`) and the probe are pure functions of it.
#[derive(Clone)]
pub struct PictureComparison {
    pub(crate) pictures: Pair<Arc<Picture>>,
    pub(crate) options: PictureOptions,
    pub(crate) placed: Pair<Placed>,
    scaled: Option<Scaled>,
    /// The area in left-picture coordinates.
    pub(crate) area: Rect,
    stats: PictureStats,
    warnings: Vec<PictureWarning>,
}

impl std::fmt::Debug for PictureComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PictureComparison")
            .field("options", &self.options)
            .field("area", &self.area)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// Compares two decoded pictures: places them under `options.geometry`, then counts the classes (bands of rows in
/// parallel, integer sums). Cancellation is checked per band.
///
/// # Errors
///
/// [`PictureError::Vocab`] for an offset beyond ±2²⁰, [`PictureError::AreaTooLarge`] when the area is wider or taller
/// than 2³² − 1 pixels, [`PictureError::Cancelled`].
pub fn compare_pictures(
    left: Arc<Picture>,
    right: Arc<Picture>,
    options: &PictureOptions,
    ctl: &Ctl<'_>,
) -> Result<PictureComparison, PictureError> {
    build(Pair::new(left, right), *options, None, ctl)
}

fn build(
    pictures: Pair<Arc<Picture>>,
    options: PictureOptions,
    reuse: Option<&Scaled>,
    ctl: &Ctl<'_>,
) -> Result<PictureComparison, PictureError> {
    ctl.check()?;
    options.geometry.check()?;
    let geometry = options.geometry;
    let sizes = Pair::new(
        oriented_size(&pictures.left, geometry.orientation.left),
        oriented_size(&pictures.right, geometry.orientation.right),
    );
    let mut warnings = Vec::new();
    let scaled = match geometry
        .auto_scale
        .then(|| scaled_side(sizes.left, sizes.right))
        .flatten()
    {
        None => None,
        Some(side) => {
            let (target, source) = match side {
                Side::Left => (sizes.right, sizes.left),
                _ => (sizes.left, sizes.right),
            };
            if aspect_ratio_differs(sizes.left, sizes.right) {
                warnings.push(PictureWarning {
                    code: PictureWarningCode::AspectRatioDiffers,
                    message: format!(
                        "the aspect ratios differ ({}x{} and {}x{}): Auto Scale stretched the {} picture",
                        sizes.left.0,
                        sizes.left.1,
                        sizes.right.0,
                        sizes.right.1,
                        side.as_str()
                    ),
                    spec: None,
                });
            }
            match reuse.filter(|r| {
                r.side == side
                    && r.orientation == geometry.orientation
                    && (r.width, r.height) == target
            }) {
                Some(kept) => Some(kept.clone()),
                None => {
                    let picture = if side == Side::Left {
                        &pictures.left
                    } else {
                        &pictures.right
                    };
                    let orientation = if side == Side::Left {
                        geometry.orientation.left
                    } else {
                        geometry.orientation.right
                    };
                    let map = orientation.index_map(picture.width, picture.height);
                    let image =
                        scale_view(&picture.rgba, source.0, source.1, map, target.0, target.1);
                    ctl.check()?;
                    Some(Scaled {
                        side,
                        orientation: geometry.orientation,
                        width: image.width,
                        height: image.height,
                        data: Arc::from(image.rgba),
                    })
                }
            }
        }
    };
    let place =
        |side: Side, picture: &Arc<Picture>, size: (u32, u32)| -> (Arc<[u8]>, IndexMap, u32, u32) {
            match &scaled {
                Some(s) if s.side == side => (
                    Arc::clone(&s.data),
                    IndexMap::row_major(s.width),
                    s.width,
                    s.height,
                ),
                _ => {
                    let orientation = if side == Side::Left {
                        geometry.orientation.left
                    } else {
                        geometry.orientation.right
                    };
                    (
                        Arc::clone(&picture.rgba),
                        orientation.index_map(picture.width, picture.height),
                        size.0,
                        size.1,
                    )
                }
            }
        };
    let (ldata, lmap, lw, lh) = place(Side::Left, &pictures.left, sizes.left);
    let (rdata, rmap, rw, rh) = place(Side::Right, &pictures.right, sizes.right);
    let (dx, dy) = (i64::from(geometry.offset.0), i64::from(geometry.offset.1));
    let (ox, oy) = (dx.min(0), dy.min(0));
    let right_edge = i64::from(lw).max(dx + i64::from(rw));
    let bottom_edge = i64::from(lh).max(dy + i64::from(rh));
    let (width, height) = (right_edge - ox, bottom_edge - oy);
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(PictureError::AreaTooLarge {
            width: width as u64,
            height: height as u64,
            pixels: (width as u64).saturating_mul(height as u64),
            limit: u64::from(u32::MAX),
        });
    };
    let placed = Pair::new(
        Placed {
            data: ldata,
            map: lmap,
            width: lw,
            height: lh,
            x: -ox,
            y: -oy,
        },
        Placed {
            data: rdata,
            map: rmap,
            width: rw,
            height: rh,
            x: dx - ox,
            y: dy - oy,
        },
    );
    let stats = scan(&placed, options.tolerance, ctl)?;
    Ok(PictureComparison {
        pictures,
        options,
        placed,
        scaled,
        area: Rect {
            x: ox,
            y: oy,
            width,
            height,
        },
        stats,
        warnings,
    })
}

impl PictureComparison {
    /// The comparison under a new geometry (rotate, flip, Auto Scale, offset): the pictures stay shared and Auto
    /// Scale's copy is reused when only the offset changed. A job (≤ 60 ms for 2 × 12 MP, G11).
    ///
    /// # Errors
    ///
    /// As [`compare_pictures`].
    pub fn with_geometry(
        &self,
        geometry: Geometry,
        ctl: &Ctl<'_>,
    ) -> Result<PictureComparison, PictureError> {
        self.with_options(
            PictureOptions {
                geometry,
                ..self.options
            },
            ctl,
        )
    }

    /// The comparison under new options (a tolerance or a geometry).
    ///
    /// # Errors
    ///
    /// As [`compare_pictures`].
    pub fn with_options(
        &self,
        options: PictureOptions,
        ctl: &Ctl<'_>,
    ) -> Result<PictureComparison, PictureError> {
        build(self.pictures.clone(), options, self.scaled.as_ref(), ctl)
    }

    /// The two pictures.
    pub fn pictures(&self) -> &Pair<Arc<Picture>> {
        &self.pictures
    }

    /// The options.
    pub fn options(&self) -> &PictureOptions {
        &self.options
    }

    /// The statistics (§1.12.19).
    pub fn stats(&self) -> &PictureStats {
        &self.stats
    }

    /// Warnings of the geometry (`aspect_ratio_differs`); the decoders' are [`Picture::warnings`].
    pub fn warnings(&self) -> &[PictureWarning] {
        &self.warnings
    }

    /// The area: the union of the left rectangle at `(0, 0)` and the right one at the offset, `x`/`y` in left
    /// coordinates (≤ 0). Area coordinates (tiles, probes, bounds) count from its top-left corner.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Where a side lies in area coordinates, with its placed (oriented, scaled) size. `None` for `Center` and
    /// `Output`.
    pub fn placement(&self, side: Side) -> Option<Rect> {
        self.placed.get(side).map(Placed::rect)
    }

    /// The side Auto Scale resampled and its new size.
    pub fn scaled(&self) -> Option<(Side, (u32, u32))> {
        self.scaled.as_ref().map(|s| (s.side, (s.width, s.height)))
    }

    /// The verdict (§1.12.16); `bytes_identical` is asked only when every position is same.
    ///
    /// # Errors
    ///
    /// Those of `bytes_identical`.
    pub fn verdict<E>(
        &self,
        ignore_unimportant: bool,
        bytes_identical: impl FnOnce() -> Result<bool, E>,
    ) -> Result<PictureVerdict, E> {
        PictureVerdict::of(&self.stats, ignore_unimportant, bytes_identical)
    }

    /// The pixel probe at area coordinates `(x, y)` (PX-008's status bar).
    pub fn probe(&self, x: u32, y: u32) -> PixelProbe {
        let (ax, ay) = (i64::from(x), i64::from(y));
        let side = |placed: &Placed| {
            placed
                .covers(ax, ay)
                .map(|(px, py)| (px, py, Rgba8::from_array(placed.at(px, py))))
        };
        let left = side(&self.placed.left);
        let right = side(&self.placed.right);
        let d = distance(left.map(|l| l.2), right.map(|r| r.2));
        let class = if left.is_none() && right.is_none() {
            PixelClass::Outside
        } else {
            classify(d, self.options.tolerance)
        };
        PixelProbe {
            area_x: x,
            area_y: y,
            left,
            right,
            distance: d,
            class,
        }
    }
}
