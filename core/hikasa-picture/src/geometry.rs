//! Geometry (PX-004, §1.12.17, D-340): each side's [`Orientation`] (the dihedral group D4 as exact index maps),
//! Auto Scale by the normative fixed-point bilinear ([`scale_bilinear`]), the offset of the right picture.
//! Geometry is view state: it never changes the image files.

use std::fmt;

use hikasa_core::Pair;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::model::{Picture, RgbaImage};
use crate::vocab::VocabError;

/// Largest offset component: ±2²⁰ pixels.
pub const MAX_OFFSET: i32 = 1 << 20;

/// An element of the dihedral group D4: what Rotate Clockwise/Counterclockwise and Flip Horizontally/Vertically
/// compose into. Each is an exact pixel index map (no resampling).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// As decoded.
    #[default]
    Identity,
    /// Rotated 90° clockwise.
    Cw90,
    /// Rotated 180°.
    Rot180,
    /// Rotated 90° counterclockwise.
    Ccw90,
    /// Mirrored left to right.
    FlipH,
    /// Mirrored top to bottom.
    FlipV,
    /// Mirrored over the main diagonal (EXIF 5).
    Transpose,
    /// Mirrored over the anti-diagonal (EXIF 7).
    Transverse,
}

/// The 2 × 2 signed permutation matrix `[m00, m01, m10, m11]` of an element in centred coordinates: the source
/// position of a destination position `(u, v)` is `(m00·u + m01·v, m10·u + m11·v)`.
type Matrix = [i8; 4];

impl Orientation {
    /// Every element, in declaration order.
    pub const ALL: [Orientation; 8] = [
        Orientation::Identity,
        Orientation::Cw90,
        Orientation::Rot180,
        Orientation::Ccw90,
        Orientation::FlipH,
        Orientation::FlipV,
        Orientation::Transpose,
        Orientation::Transverse,
    ];

    const fn matrix(self) -> Matrix {
        match self {
            Orientation::Identity => [1, 0, 0, 1],
            Orientation::Cw90 => [0, 1, -1, 0],
            Orientation::Rot180 => [-1, 0, 0, -1],
            Orientation::Ccw90 => [0, -1, 1, 0],
            Orientation::FlipH => [-1, 0, 0, 1],
            Orientation::FlipV => [1, 0, 0, -1],
            Orientation::Transpose => [0, 1, 1, 0],
            Orientation::Transverse => [0, -1, -1, 0],
        }
    }

    fn from_matrix(m: Matrix) -> Orientation {
        Orientation::ALL
            .into_iter()
            .find(|o| o.matrix() == m)
            .unwrap_or_default()
    }

    /// `self` first, then `next` (Rotate Clockwise after a flip, …).
    pub fn then(self, next: Orientation) -> Orientation {
        // dest → (next's source) → (self's source): M = M_self · M_next.
        let a = self.matrix();
        let b = next.matrix();
        Orientation::from_matrix([
            a[0] * b[0] + a[1] * b[2],
            a[0] * b[1] + a[1] * b[3],
            a[2] * b[0] + a[3] * b[2],
            a[2] * b[1] + a[3] * b[3],
        ])
    }

    /// The element that undoes `self` (`o.then(o.inverse()) == Identity`).
    pub fn inverse(self) -> Orientation {
        let m = self.matrix();
        Orientation::from_matrix([m[0], m[2], m[1], m[3]])
    }

    /// True when the width and height trade places (the quarter turns and the diagonal mirrors).
    pub const fn swaps_axes(self) -> bool {
        self.matrix()[0] == 0
    }

    /// The size of a `width` × `height` picture after the element.
    pub const fn oriented_size(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The source pixel index map of the element on a `width` × `height` picture: the oriented pixel `(x, y)` is
    /// the source pixel of index `origin + x · step_x + y · step_y` (row-major).
    pub(crate) fn index_map(self, width: u32, height: u32) -> IndexMap {
        let [m00, m01, m10, m11] = self.matrix().map(i64::from);
        let (w, h) = (i64::from(width), i64::from(height));
        let (ow, oh) = self.oriented_size(width, height);
        let (ow, oh) = (i64::from(ow), i64::from(oh));
        // sx = m00·x + m01·y + cx with cx = ((w−1) − m00·(W−1) − m01·(H−1)) / 2 (always even), likewise sy.
        let cx = ((w - 1) - m00 * (ow - 1) - m01 * (oh - 1)) / 2;
        let cy = ((h - 1) - m10 * (ow - 1) - m11 * (oh - 1)) / 2;
        IndexMap {
            origin: cy * w + cx,
            step_x: m00 + m10 * w,
            step_y: m01 + m11 * w,
        }
    }

    /// The stable word (`none`, `cw`, `ccw`, `180`, `flip-h`, `flip-v`, `transpose`, `transverse`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Orientation::Identity => "none",
            Orientation::Cw90 => "cw",
            Orientation::Rot180 => "180",
            Orientation::Ccw90 => "ccw",
            Orientation::FlipH => "flip-h",
            Orientation::FlipV => "flip-v",
            Orientation::Transpose => "transpose",
            Orientation::Transverse => "transverse",
        }
    }

    /// Parses one word (ASCII case ignored): the words of [`Orientation::as_str`] and the aliases `identity`,
    /// `cw90`, `90`, `rot180`, `ccw90`, `270`, `fliph`, `flipv`.
    ///
    /// # Errors
    ///
    /// [`VocabError`] naming the words.
    pub fn parse(word: &str) -> Result<Orientation, VocabError> {
        let lower = word.trim().to_ascii_lowercase();
        Ok(match lower.as_str() {
            "none" | "identity" => Orientation::Identity,
            "cw" | "cw90" | "90" => Orientation::Cw90,
            "180" | "rot180" => Orientation::Rot180,
            "ccw" | "ccw90" | "270" => Orientation::Ccw90,
            "flip-h" | "fliph" => Orientation::FlipH,
            "flip-v" | "flipv" => Orientation::FlipV,
            "transpose" => Orientation::Transpose,
            "transverse" => Orientation::Transverse,
            _ => {
                return Err(VocabError::new(
                    "orientation",
                    word,
                    "none, cw, ccw, 180, flip-h, flip-v, transpose, transverse",
                ));
            }
        })
    }

    /// Parses a comma list composed in order (`"cw,flip-h"`: rotate clockwise, then mirror). An empty list is an
    /// error; `none` alone is the identity.
    ///
    /// # Errors
    ///
    /// [`VocabError`] for an empty list or an unknown word.
    pub fn parse_list(text: &str) -> Result<Orientation, VocabError> {
        if text.trim().is_empty() {
            return Err(VocabError::new(
                "orientation",
                text,
                "a comma list of none, cw, ccw, 180, flip-h, flip-v, transpose, transverse",
            ));
        }
        text.split(',')
            .try_fold(Orientation::Identity, |acc, word| {
                Ok(acc.then(Orientation::parse(word)?))
            })
    }
}

impl fmt::Display for Orientation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A pixel index map: the pixel `(x, y)` of a view is the source pixel of index `origin + x·step_x + y·step_y`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexMap {
    pub(crate) origin: i64,
    pub(crate) step_x: i64,
    pub(crate) step_y: i64,
}

impl IndexMap {
    /// The identity map of a row-major image `width` wide.
    pub(crate) const fn row_major(width: u32) -> IndexMap {
        IndexMap {
            origin: 0,
            step_x: 1,
            step_y: width as i64,
        }
    }

    #[inline]
    pub(crate) const fn index(self, x: u32, y: u32) -> usize {
        (self.origin + x as i64 * self.step_x + y as i64 * self.step_y) as usize
    }
}

/// The view state of a comparison (§1.12.17): each side's orientation, Auto Scale, the right picture's origin in
/// left coordinates. Saved in a session's `[view]` (D-349), never written to the images.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Geometry {
    /// Each side's orientation.
    pub orientation: Pair<Orientation>,
    /// Resample the picture with fewer pixels to the other's size (off by default).
    pub auto_scale: bool,
    /// The right picture's origin in left coordinates, each component within ±[`MAX_OFFSET`].
    pub offset: (i32, i32),
}

impl Geometry {
    /// Checks the offset's range.
    ///
    /// # Errors
    ///
    /// [`VocabError`] (kind `invalid_input`) for a component beyond ±2²⁰.
    pub fn check(&self) -> Result<(), VocabError> {
        let (dx, dy) = self.offset;
        if (-MAX_OFFSET..=MAX_OFFSET).contains(&dx) && (-MAX_OFFSET..=MAX_OFFSET).contains(&dy) {
            Ok(())
        } else {
            Err(VocabError::new(
                "offset",
                &format!("{dx},{dy}"),
                format!("each component from {} to {MAX_OFFSET}", -MAX_OFFSET),
            ))
        }
    }

    /// The geometry seen from the other side (Swap Sides): orientations exchanged, the offset negated.
    pub fn swapped(&self) -> Geometry {
        Geometry {
            orientation: Pair::new(self.orientation.right, self.orientation.left),
            auto_scale: self.auto_scale,
            offset: (-self.offset.0, -self.offset.1),
        }
    }
}

/// The oriented copy of a picture's pixels (the EXIF orientation of the decoder, P-GEOM's reference).
pub fn orient(width: u32, height: u32, rgba: &[u8], orientation: Orientation) -> RgbaImage {
    let (ow, oh) = orientation.oriented_size(width, height);
    let map = orientation.index_map(width, height);
    let mut out = vec![0u8; ow as usize * oh as usize * 4];
    out.par_chunks_mut((ow as usize * 4).max(1))
        .enumerate()
        .for_each(|(y, row)| {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let at = map.index(x as u32, y as u32) * 4;
                if let Some(src) = rgba.get(at..at + 4) {
                    px.copy_from_slice(src);
                }
            }
        });
    RgbaImage {
        width: ow,
        height: oh,
        rgba: out,
    }
}

/// One axis of the normative bilinear (§1.12.17): for each destination coordinate, `(i0, i1, weight)` with
/// `f = ((2x + 1) · n · 2¹⁶) / (2N) − 2¹⁵` (floor) clamped to `[0, (n − 1) · 2¹⁶]`, `i0 = f >> 16`,
/// `i1 = min(i0 + 1, n − 1)`, `weight = f & 0xFFFF`.
fn axis(n: u32, dest: u32) -> Vec<(u32, u32, u64)> {
    let (n128, d128) = (u128::from(n), u128::from(dest));
    let max = u128::from(n.saturating_sub(1)) << 16;
    (0..d128)
        .map(|x| {
            let f = (((2 * x + 1) * n128) << 16) / (2 * d128);
            let f = f.saturating_sub(1 << 15).min(max);
            let i0 = (f >> 16) as u32;
            let i1 = (i0 + 1).min(n.saturating_sub(1));
            (i0, i1, (f & 0xFFFF) as u64)
        })
        .collect()
}

/// Auto Scale's normative fixed-point bilinear (§1.12.17) of a `width` × `height` RGBA8 view (pixel `(x, y)` at
/// `map.index(x, y)`) to exactly `to_w` × `to_h`, per straight channel
/// `v = ((c00·(2¹⁶−wx) + c10·wx)·(2¹⁶−wy) + (c01·(2¹⁶−wx) + c11·wx)·wy + 2³¹) >> 32`.
pub(crate) fn scale_view(
    rgba: &[u8],
    width: u32,
    height: u32,
    map: IndexMap,
    to_w: u32,
    to_h: u32,
) -> RgbaImage {
    let xs = axis(width, to_w);
    let ys = axis(height, to_h);
    let mut out = vec![0u8; to_w as usize * to_h as usize * 4];
    let px = |x: u32, y: u32| -> [u64; 4] {
        let at = map.index(x, y) * 4;
        match rgba.get(at..at + 4) {
            Some(c) => [c[0], c[1], c[2], c[3]].map(u64::from),
            None => [0; 4],
        }
    };
    out.par_chunks_mut((to_w as usize * 4).max(1))
        .zip(ys.par_iter())
        .for_each(|(row, &(y0, y1, wy))| {
            for (dst, &(x0, x1, wx)) in row.as_chunks_mut::<4>().0.iter_mut().zip(&xs) {
                let (c00, c10, c01, c11) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
                for c in 0..4 {
                    let top = c00[c] * ((1 << 16) - wx) + c10[c] * wx;
                    let bottom = c01[c] * ((1 << 16) - wx) + c11[c] * wx;
                    let v = (top * ((1 << 16) - wy) + bottom * wy + (1 << 31)) >> 32;
                    dst[c] = v.min(255) as u8;
                }
            }
        });
    RgbaImage {
        width: to_w,
        height: to_h,
        rgba: out,
    }
}

/// Auto Scale's bilinear of a whole image to `to_w` × `to_h` (the goldens of A12.14).
pub fn scale_bilinear(image: &RgbaImage, to_w: u32, to_h: u32) -> RgbaImage {
    scale_view(
        &image.rgba,
        image.width,
        image.height,
        IndexMap::row_major(image.width),
        to_w,
        to_h,
    )
}

/// Which side Auto Scale resamples when the oriented sizes differ: the one with fewer pixels, the right one on a
/// tie. `None` when the sizes are equal.
pub fn scaled_side(left: (u32, u32), right: (u32, u32)) -> Option<hikasa_core::Side> {
    if left == right {
        return None;
    }
    let pixels = |s: (u32, u32)| u64::from(s.0) * u64::from(s.1);
    if pixels(left) < pixels(right) {
        Some(hikasa_core::Side::Left)
    } else {
        Some(hikasa_core::Side::Right)
    }
}

/// True when the aspect ratios `w₁/h₁` and `w₂/h₂` differ by more than 1 % of the larger one (integer rule:
/// `100 · max(w₁h₂, w₂h₁) > 101 · min(w₁h₂, w₂h₁)`).
pub fn aspect_ratio_differs(a: (u32, u32), b: (u32, u32)) -> bool {
    let p = u128::from(a.0) * u128::from(b.1);
    let q = u128::from(b.0) * u128::from(a.1);
    let (lo, hi) = if p < q { (p, q) } else { (q, p) };
    hi * 100 > lo * 101
}

/// A picture's oriented size.
pub(crate) fn oriented_size(picture: &Picture, orientation: Orientation) -> (u32, u32) {
    orientation.oriented_size(picture.width, picture.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [i as u8, 0, 0, 255]).collect()
    }

    #[test]
    fn index_maps_match_the_definitions() {
        // dest(x, y) ← src per §1.12.17's elements on a 3 × 2 picture.
        let (w, h) = (3u32, 2u32);
        let src = |x: u32, y: u32| y * w + x;
        for o in Orientation::ALL {
            let (ow, oh) = o.oriented_size(w, h);
            let map = o.index_map(w, h);
            for y in 0..oh {
                for x in 0..ow {
                    let want = match o {
                        Orientation::Identity => src(x, y),
                        Orientation::Cw90 => src(y, h - 1 - x),
                        Orientation::Rot180 => src(w - 1 - x, h - 1 - y),
                        Orientation::Ccw90 => src(w - 1 - y, x),
                        Orientation::FlipH => src(w - 1 - x, y),
                        Orientation::FlipV => src(x, h - 1 - y),
                        Orientation::Transpose => src(y, x),
                        Orientation::Transverse => src(w - 1 - y, h - 1 - x),
                    };
                    assert_eq!(map.index(x, y), want as usize, "{o} at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn then_composes_like_materialized_images() {
        let (w, h) = (3u32, 2u32);
        let pixels = image(w, h);
        for a in Orientation::ALL {
            for b in Orientation::ALL {
                let once = orient(w, h, &pixels, a);
                let twice = orient(once.width, once.height, &once.rgba, b);
                let composed = orient(w, h, &pixels, a.then(b));
                assert_eq!(twice, composed, "{a} then {b}");
            }
            assert_eq!(a.then(a.inverse()), Orientation::Identity, "{a}");
        }
    }
}
