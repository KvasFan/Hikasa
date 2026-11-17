//! Statistics and the pixel probe (PX-008, §1.12.19, D-342): counts of the classes, one-sided positions, the
//! largest and summed distance and the bounds of the non-same positions, computed in parallel bands of rows with
//! integer sums (T2: any thread count gives the same numbers); percentages are derived at output.

use hikasa_core::{Ctl, Pair};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::compare::{PixelClass, Placed, pixel_distance};
use crate::error::PictureError;
use crate::model::{Rect, Rgba8};

/// Rows of a band (the unit of parallel work and of cancellation).
pub(crate) const BAND: i64 = 64;

/// The statistics of a comparison (§1.12.19). `bounds` is in area coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PictureStats {
    /// Positions covered by at least one picture.
    pub pixels: u64,
    /// *Same* positions.
    pub same: u64,
    /// *Similar* positions.
    pub similar: u64,
    /// *Different* positions.
    pub different: u64,
    /// Positions covered by the left picture only (also counted in a class).
    pub left_only: u64,
    /// Positions covered by the right picture only (also counted in a class).
    pub right_only: u64,
    /// The largest distance.
    pub max_distance: u8,
    /// The sum of the distances.
    pub distance_sum: u64,
    /// The bounding rectangle of the non-same positions.
    pub bounds: Option<Rect>,
}

/// `count / total` as a number with 4 decimals, rounded half up in integers (0 when `total` is 0).
fn ratio4(count: u64, total: u64, scale: u128) -> f64 {
    if total == 0 {
        return 0.0;
    }
    let n = (u128::from(count) * scale * 2 + u128::from(total)) / (u128::from(total) * 2);
    n as f64 / 10_000.0
}

impl PictureStats {
    /// `count` as a percentage of [`PictureStats::pixels`], 4 decimals (rounded half up).
    pub fn percent(&self, count: u64) -> f64 {
        ratio4(count, self.pixels, 1_000_000)
    }

    /// The mean distance over the covered positions, 4 decimals (rounded half up).
    pub fn mean_distance(&self) -> f64 {
        ratio4(self.distance_sum, self.pixels, 10_000)
    }

    /// True when every covered position is *same*.
    pub fn all_same(&self) -> bool {
        self.same == self.pixels
    }
}

/// The pixel probe (§1.12.19): both sides' pixels at an area position, with picture coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PixelProbe {
    /// Area column.
    pub area_x: u32,
    /// Area row.
    pub area_y: u32,
    /// The left picture's coordinates and pixel there.
    pub left: Option<(u32, u32, Rgba8)>,
    /// The right picture's coordinates and pixel there.
    pub right: Option<(u32, u32, Rgba8)>,
    /// The distance (0 outside).
    pub distance: u8,
    /// The class.
    pub class: PixelClass,
}

/// One band's sums.
#[derive(Clone, Copy)]
struct Acc {
    pixels: u64,
    same: u64,
    similar: u64,
    different: u64,
    left_only: u64,
    right_only: u64,
    max: u8,
    sum: u64,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
}

impl Acc {
    const EMPTY: Acc = Acc {
        pixels: 0,
        same: 0,
        similar: 0,
        different: 0,
        left_only: 0,
        right_only: 0,
        max: 0,
        sum: 0,
        min_x: i64::MAX,
        min_y: i64::MAX,
        max_x: i64::MIN,
        max_y: i64::MIN,
    };

    fn merge(mut self, o: Acc) -> Acc {
        self.pixels += o.pixels;
        self.same += o.same;
        self.similar += o.similar;
        self.different += o.different;
        self.left_only += o.left_only;
        self.right_only += o.right_only;
        self.max = self.max.max(o.max);
        self.sum += o.sum;
        self.min_x = self.min_x.min(o.min_x);
        self.min_y = self.min_y.min(o.min_y);
        self.max_x = self.max_x.max(o.max_x);
        self.max_y = self.max_y.max(o.max_y);
        self
    }

    /// Counts the positions `[x0, x1)` of row `ay` with distances `d(ax)`; returns the first and last non-same
    /// columns seen.
    #[inline]
    fn run(
        &mut self,
        x0: i64,
        x1: i64,
        tolerance: u8,
        row: &mut (i64, i64),
        mut d: impl FnMut(i64) -> u8,
    ) {
        if x0 >= x1 {
            return;
        }
        self.pixels += (x1 - x0) as u64;
        for ax in x0..x1 {
            let d = d(ax);
            self.sum += u64::from(d);
            if d == 0 {
                self.same += 1;
                continue;
            }
            self.max = self.max.max(d);
            if d <= tolerance {
                self.similar += 1;
            } else {
                self.different += 1;
            }
            row.0 = row.0.min(ax);
            row.1 = row.1.max(ax);
        }
    }
}

/// `a` minus `b`: at most two intervals (empty ones have `start >= end`).
fn minus(a: (i64, i64), b: Option<(i64, i64)>) -> [(i64, i64); 2] {
    match b {
        None => [a, (0, 0)],
        Some(b) => [(a.0, a.1.min(b.0)), (a.0.max(b.1), a.1)],
    }
}

/// One area row.
fn row(placed: &Pair<Placed>, ay: i64, tolerance: u8, acc: &mut Acc) {
    let (l, r) = (&placed.left, &placed.right);
    let ls = l.row_span(ay);
    let rs = r.row_span(ay);
    let mut seen = (i64::MAX, i64::MIN);
    if let (Some(a), Some(b)) = (ls, rs) {
        let (x0, x1) = (a.0.max(b.0), a.1.min(b.1));
        let (ly, ry) = ((ay - l.y) as u32, (ay - r.y) as u32);
        acc.run(x0, x1, tolerance, &mut seen, |ax| {
            pixel_distance(l.at((ax - l.x) as u32, ly), r.at((ax - r.x) as u32, ry))
        });
    }
    if let Some(a) = ls {
        let ly = (ay - l.y) as u32;
        for (x0, x1) in minus(a, rs) {
            let before = acc.pixels;
            acc.run(x0, x1, tolerance, &mut seen, |ax| {
                l.at((ax - l.x) as u32, ly)[3]
            });
            acc.left_only += acc.pixels - before;
        }
    }
    if let Some(b) = rs {
        let ry = (ay - r.y) as u32;
        for (x0, x1) in minus(b, ls) {
            let before = acc.pixels;
            acc.run(x0, x1, tolerance, &mut seen, |ax| {
                r.at((ax - r.x) as u32, ry)[3]
            });
            acc.right_only += acc.pixels - before;
        }
    }
    if seen.0 <= seen.1 {
        acc.min_x = acc.min_x.min(seen.0);
        acc.max_x = acc.max_x.max(seen.1);
        acc.min_y = acc.min_y.min(ay);
        acc.max_y = acc.max_y.max(ay);
    }
}

/// The bands `[y0, y1)` of the rows at least one side covers (an empty gap between the two sides is skipped).
pub(crate) fn bands(placed: &Pair<Placed>) -> Vec<(i64, i64)> {
    let a = (placed.left.y, placed.left.y + i64::from(placed.left.height));
    let b = (
        placed.right.y,
        placed.right.y + i64::from(placed.right.height),
    );
    let spans = if a.1 < b.0 || b.1 < a.0 {
        vec![a.min(b), a.max(b)]
    } else {
        vec![(a.0.min(b.0), a.1.max(b.1))]
    };
    spans
        .into_iter()
        .flat_map(|(y0, y1)| {
            (y0..y1)
                .step_by(BAND as usize)
                .map(move |s| (s, (s + BAND).min(y1)))
        })
        .collect()
}

/// The statistics of placed sides at `tolerance` (cancellation per band).
pub(crate) fn scan(
    placed: &Pair<Placed>,
    tolerance: u8,
    ctl: &Ctl<'_>,
) -> Result<PictureStats, PictureError> {
    let cancel = ctl.cancel;
    let sums = bands(placed)
        .into_par_iter()
        .map(|(y0, y1)| {
            cancel.check()?;
            let mut acc = Acc::EMPTY;
            for ay in y0..y1 {
                row(placed, ay, tolerance, &mut acc);
            }
            Ok(acc)
        })
        .collect::<Result<Vec<Acc>, hikasa_core::Cancelled>>()?;
    ctl.check()?;
    let acc = sums.into_iter().fold(Acc::EMPTY, Acc::merge);
    let bounds = (acc.min_x <= acc.max_x).then(|| Rect {
        x: acc.min_x,
        y: acc.min_y,
        width: (acc.max_x - acc.min_x + 1) as u32,
        height: (acc.max_y - acc.min_y + 1) as u32,
    });
    Ok(PictureStats {
        pixels: acc.pixels,
        same: acc.same,
        similar: acc.similar,
        different: acc.different,
        left_only: acc.left_only,
        right_only: acc.right_only,
        max_distance: acc.max,
        distance_sum: acc.sum,
        bounds,
    })
}
