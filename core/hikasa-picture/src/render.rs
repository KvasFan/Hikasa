//! Rendering (PX-003, PX-007, §1.12.18, D-341): the difference image in three modes, the left and right panes,
//! the checkerboard under transparent pixels, tiles at scale levels. Every rule is integer arithmetic with round
//! half up (`div_round(n, d) = (2n + d) / 2d`); a tile is a pure function of the comparison.
//!
//! Per position of the area:
//! - **Tolerance:** *same* → the `same` colour scaled by the luma `Y = (77r + 150g + 29b + 128) >> 8` of the left
//!   pixel (the right one where only it covers): each channel `same_c · (64 + 191·Y/255) / 255`, alpha 255;
//!   *similar* → `similar`; *different* → `different`; outside → transparent.
//! - **Range:** `d = 0` → opaque black; else `v = 32 + 223·d/255` → `(v, v, 0, 255)`.
//! - **Blend:** per channel, alpha included, `(L·(100 − p) + R·p) / 100`, a missing side transparent.
//! - **Left / Right:** the oriented (and scaled) picture placed in the area.

use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::{Ctl, Phase, Progress, Unit};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::compare::{PictureComparison, PixelClass, classify, pixel_distance};
use crate::error::PictureError;
use crate::model::{Rgba8, RgbaImage, RgbaTile};
pub use crate::vocab::Pane;
use crate::vocab::VocabError;

/// Largest image [`PictureComparison::render`] produces: 2²⁹ pixels (2 GiB).
pub const MAX_RENDER_PIXELS: u64 = 1 << 29;

/// Largest tile level: 1/2⁸.
pub const MAX_LEVEL: u8 = 8;

/// Cell size of the checkerboard in pixels.
pub const CHECKER_CELL: u32 = 8;

/// The display mode of the difference image (never changes the classes or the verdict).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffMode {
    /// The class colours, *same* shaded by luma (default).
    #[default]
    Tolerance,
    /// The degree of difference: black is same, yellow brightness the distance.
    Range,
    /// The two pictures blended.
    Blend {
        /// The right picture's weight, 0…100 (default 50).
        percent: u8,
    },
}

impl DiffMode {
    /// The default blend percentage.
    pub const DEFAULT_BLEND: u8 = 50;

    /// The stable word (`tolerance`, `range`, `blend`).
    pub const fn as_str(self) -> &'static str {
        match self {
            DiffMode::Tolerance => "tolerance",
            DiffMode::Range => "range",
            DiffMode::Blend { .. } => "blend",
        }
    }

    /// Parses a mode word (ASCII case ignored); `blend` takes [`DiffMode::DEFAULT_BLEND`].
    ///
    /// # Errors
    ///
    /// [`VocabError`] naming the words.
    pub fn parse(word: &str) -> Result<DiffMode, VocabError> {
        match word.trim().to_ascii_lowercase().as_str() {
            "tolerance" => Ok(DiffMode::Tolerance),
            "range" => Ok(DiffMode::Range),
            "blend" => Ok(DiffMode::Blend {
                percent: DiffMode::DEFAULT_BLEND,
            }),
            _ => Err(VocabError::new("mode", word, "tolerance, range, blend")),
        }
    }
}

/// The theme's `[picture]` colours (UI-023), as core's renderer uses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PictureColors {
    /// *Same* positions (shaded by luma in tolerance mode).
    pub same: Rgba8,
    /// *Similar* positions.
    pub similar: Rgba8,
    /// *Different* positions.
    pub different: Rgba8,
    /// The canvas background (W13's; not used by core's images).
    pub background: Rgba8,
    /// Light checkerboard cells.
    pub checker_light: Rgba8,
    /// Dark checkerboard cells.
    pub checker_dark: Rgba8,
}

impl Default for PictureColors {
    /// The built-in light scheme's `[picture]`.
    fn default() -> PictureColors {
        PictureColors {
            same: Rgba8::rgb(0x1f9d55),
            similar: Rgba8::rgb(0xd9a400),
            different: Rgba8::rgb(0xc62828),
            background: Rgba8::rgb(0xffffff),
            checker_light: Rgba8::rgb(0xffffff),
            checker_dark: Rgba8::rgb(0xe3e6eb),
        }
    }
}

/// A rectangle of a level's area in pixels of that level (`x`, `y` from the area's top-left corner).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileRect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// `n / d` rounded half up.
#[inline]
const fn div_round(n: u32, d: u32) -> u32 {
    (2 * n + d) / (2 * d)
}

/// The distance of a position from its covering pixels (one side: its alpha).
#[inline]
fn side_distance(l: Option<[u8; 4]>, r: Option<[u8; 4]>) -> u8 {
    match (l, r) {
        (Some(p), Some(q)) => pixel_distance(p, q),
        (Some(p), None) | (None, Some(p)) => p[3],
        (None, None) => 0,
    }
}

/// The `same` colour shaded by the luma of `p`.
#[inline]
fn shade(same: Rgba8, p: [u8; 4]) -> [u8; 4] {
    let y = (77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8;
    let f = 64 * 255 + 191 * y;
    let c = |s: u8| div_round(u32::from(s) * f, 255 * 255) as u8;
    [c(same.r), c(same.g), c(same.b), 255]
}

/// Composites the transparent pixels of `tile` source-over onto checkerboard cells of `cell` pixels anchored at
/// the area's origin; `origin` is the tile's top-left corner in area coordinates (PX-007). Per channel
/// `div_round(s·a + c·(255 − a), 255)`, alpha `a + div_round(c_a·(255 − a), 255)`.
pub fn composite_checkerboard(
    tile: &mut RgbaTile,
    origin: (u32, u32),
    colors: &PictureColors,
    cell: u32,
) {
    composite_rows(&mut tile.rgba, tile.width, origin, colors, cell);
}

/// [`composite_checkerboard`] on rows of `width` pixels.
fn composite_rows(
    rgba: &mut [u8],
    width: u32,
    origin: (u32, u32),
    colors: &PictureColors,
    cell: u32,
) {
    let cell = u64::from(cell.max(1));
    let width = width as usize;
    if width == 0 {
        return;
    }
    for (y, row) in rgba.chunks_exact_mut(width * 4).enumerate() {
        let cy = (u64::from(origin.1) + y as u64) / cell;
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let a = u32::from(px[3]);
            if a == 255 {
                continue;
            }
            let cx = (u64::from(origin.0) + x as u64) / cell;
            let bg = if (cx + cy).is_multiple_of(2) {
                colors.checker_light
            } else {
                colors.checker_dark
            }
            .to_array();
            for c in 0..3 {
                px[c] = div_round(u32::from(px[c]) * a + u32::from(bg[c]) * (255 - a), 255) as u8;
            }
            px[3] = (a + div_round(u32::from(bg[3]) * (255 - a), 255)) as u8;
        }
    }
}

impl PictureComparison {
    /// The rendered pixel of the area position `(ax, ay)` at level 0.
    #[inline]
    fn pixel(
        &self,
        pane: Pane,
        mode: DiffMode,
        colors: &PictureColors,
        ax: i64,
        ay: i64,
    ) -> [u8; 4] {
        let l = self.placed.left.pixel(ax, ay);
        let r = self.placed.right.pixel(ax, ay);
        match pane {
            Pane::Left => l.unwrap_or([0; 4]),
            Pane::Right => r.unwrap_or([0; 4]),
            Pane::Difference => {
                if l.is_none() && r.is_none() {
                    return [0; 4];
                }
                match mode {
                    DiffMode::Tolerance => {
                        match classify(side_distance(l, r), self.options.tolerance) {
                            PixelClass::Same => shade(colors.same, l.or(r).unwrap_or([0; 4])),
                            PixelClass::Similar => colors.similar.to_array(),
                            PixelClass::Different | PixelClass::Outside => {
                                colors.different.to_array()
                            }
                        }
                    }
                    DiffMode::Range => match side_distance(l, r) {
                        0 => [0, 0, 0, 255],
                        d => {
                            let v = (32 + div_round(223 * u32::from(d), 255)) as u8;
                            [v, v, 0, 255]
                        }
                    },
                    DiffMode::Blend { percent } => {
                        let p = u32::from(percent.min(100));
                        let (l, r) = (l.unwrap_or([0; 4]), r.unwrap_or([0; 4]));
                        let mix = |c: usize| {
                            div_round(u32::from(l[c]) * (100 - p) + u32::from(r[c]) * p, 100) as u8
                        };
                        [mix(0), mix(1), mix(2), mix(3)]
                    }
                }
            }
        }
    }

    /// The size of the area at `level` (`⌈W / 2ᵏ⌉ × ⌈H / 2ᵏ⌉`).
    pub fn level_size(&self, level: u8) -> (u32, u32) {
        let k = u32::from(level.min(MAX_LEVEL));
        let up = |n: u32| ((u64::from(n) + (1 << k) - 1) >> k) as u32;
        (up(self.area.width), up(self.area.height))
    }

    /// The level-`k` pixel `(x, y)`: the integer mean (round half up) of each channel over the block of `2ᵏ × 2ᵏ`
    /// level-0 positions it covers, clipped to the area; transparent beyond the area.
    fn level_pixel(
        &self,
        pane: Pane,
        mode: DiffMode,
        colors: &PictureColors,
        x: u32,
        y: u32,
        k: u32,
    ) -> [u8; 4] {
        let (w, h) = (i64::from(self.area.width), i64::from(self.area.height));
        let s = 1i64 << k;
        let (x0, y0) = (i64::from(x) * s, i64::from(y) * s);
        if x0 >= w || y0 >= h {
            return [0; 4];
        }
        if k == 0 {
            return self.pixel(pane, mode, colors, x0, y0);
        }
        let (x1, y1) = ((x0 + s).min(w), (y0 + s).min(h));
        // A block no picture touches is transparent without visiting it.
        let touches = |p: &crate::compare::Placed| {
            x0 < p.x + i64::from(p.width) && p.x < x1 && y0 < p.y + i64::from(p.height) && p.y < y1
        };
        if !touches(&self.placed.left) && !touches(&self.placed.right) {
            return [0; 4];
        }
        let mut sum = [0u64; 4];
        for ay in y0..y1 {
            for ax in x0..x1 {
                let p = self.pixel(pane, mode, colors, ax, ay);
                for c in 0..4 {
                    sum[c] += u64::from(p[c]);
                }
            }
        }
        let n = ((x1 - x0) * (y1 - y0)) as u64;
        sum.map(|v| ((2 * v + n) / (2 * n)) as u8)
    }

    /// Renders a tile of the area at scale `1/2^level` (`level` 0…8, larger values are 8) in `pane` and `mode`
    /// (W13 uploads tiles at the displayed zoom). Positions beyond the level's area are transparent. No
    /// checkerboard: [`composite_checkerboard`] adds it.
    pub fn render_tile(
        &self,
        pane: Pane,
        mode: DiffMode,
        colors: &PictureColors,
        tile: TileRect,
        level: u8,
    ) -> RgbaTile {
        let k = u32::from(level.min(MAX_LEVEL));
        let mut out = RgbaImage::transparent(tile.width, tile.height);
        let width = tile.width as usize;
        if width == 0 {
            return out;
        }
        for (j, row) in out.rgba.chunks_exact_mut(width * 4).enumerate() {
            let y = tile.y.saturating_add(j as u32);
            for (i, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let x = tile.x.saturating_add(i as u32);
                px.copy_from_slice(&self.level_pixel(pane, mode, colors, x, y, k));
            }
        }
        out
    }

    /// Renders the whole area at full resolution in `pane` and `mode` (bands of rows in parallel; cancellation
    /// and progress `writing` per band), with the checkerboard when asked (`--diff-output --checkerboard`, reports). Equal to its tiles
    /// assembled (P-TILES).
    ///
    /// # Errors
    ///
    /// [`PictureError::TooLarge`] for an area above [`MAX_RENDER_PIXELS`]; [`PictureError::Cancelled`].
    pub fn render(
        &self,
        pane: Pane,
        mode: DiffMode,
        colors: &PictureColors,
        checkerboard: bool,
        ctl: &Ctl<'_>,
    ) -> Result<RgbaImage, PictureError> {
        ctl.check()?;
        let (w, h) = (self.area.width, self.area.height);
        let pixels = u64::from(w) * u64::from(h);
        if pixels > MAX_RENDER_PIXELS {
            return Err(PictureError::TooLarge {
                pixels: Some(pixels),
                limit: MAX_RENDER_PIXELS,
            });
        }
        let mut out = RgbaImage::transparent(w, h);
        let band_rows = crate::stats::BAND as usize;
        let row_bytes = w as usize * 4;
        if row_bytes == 0 {
            return Ok(out);
        }
        let done = AtomicU64::new(0);
        let ctl_ = *ctl;
        out.rgba
            .par_chunks_mut(row_bytes * band_rows)
            .enumerate()
            .try_for_each(|(band, bytes)| {
                ctl_.check()?;
                let y0 = band * band_rows;
                for (j, row) in bytes.chunks_exact_mut(row_bytes).enumerate() {
                    let ay = (y0 + j) as i64;
                    for (ax, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        px.copy_from_slice(&self.pixel(pane, mode, colors, ax as i64, ay));
                    }
                }
                if checkerboard {
                    composite_rows(bytes, w, (0, y0 as u32), colors, CHECKER_CELL);
                }
                let rows = (bytes.len() / row_bytes) as u64;
                let rows = done.fetch_add(rows, Ordering::Relaxed) + rows;
                ctl_.progress(Phase::Writing, || Progress {
                    phase: Phase::Writing,
                    done: rows,
                    total: Some(u64::from(h)),
                    unit: Unit::Lines,
                    current: None,
                });
                Ok::<(), hikasa_core::Cancelled>(())
            })?;
        ctl.check()?;
        Ok(out)
    }
}
