//! The hex view's thumbnail strip (M2 W13a, contract §1.13.8, D-360), toolkit-free: the pixel rows of a strip of any
//! height from the alignment's 4,096 buckets (`HexAlignment::thumbnail`, core's counts). Each pixel row sums the
//! buckets it covers and is *different* when it holds a different column, else *orphan* when it holds a one-sided
//! column, else *same*; a row covering only empty buckets is background. This is display only: the counts are core's.

use hikasa_hex::Thumbnail;

/// The class of one pixel row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbClass {
    /// No column.
    Empty,
    /// Only same columns.
    Same,
    /// A one-sided column and no different one.
    Orphan,
    /// A different column.
    Different,
}

/// The pixel rows of a strip `height` pixels high.
pub fn pixel_rows(thumbnail: &Thumbnail, height: u32) -> Vec<ThumbClass> {
    let n = thumbnail.buckets.len().max(1) as u64;
    let h = u64::from(height.max(1));
    (0..h)
        .map(|p| {
            let a = (p * n / h) as usize;
            let b = (((p + 1) * n / h) as usize).max(a + 1);
            let (mut same, mut diff, mut orphan) = (0u64, 0u64, 0u64);
            for bucket in thumbnail
                .buckets
                .get(a..b.min(thumbnail.buckets.len()))
                .unwrap_or(&[])
            {
                same += u64::from(bucket.same);
                diff += u64::from(bucket.different);
                orphan += u64::from(bucket.left_only) + u64::from(bucket.right_only);
            }
            if diff > 0 {
                ThumbClass::Different
            } else if orphan > 0 {
                ThumbClass::Orphan
            } else if same > 0 {
                ThumbClass::Same
            } else {
                ThumbClass::Empty
            }
        })
        .collect()
}

/// The pixel rows `[from, to)` of a strip `height` high framing the aligned columns `columns` of `total`.
pub fn frame_of(columns: std::ops::Range<u64>, total: u64, height: u32) -> (u32, u32) {
    if total == 0 {
        return (0, 0);
    }
    let h = f64::from(height);
    let y = |c: u64| ((c as f64 / total as f64) * h) as u32;
    let (a, b) = (y(columns.start), y(columns.end));
    (a.min(height.saturating_sub(1)), b.max(a + 1).min(height))
}
