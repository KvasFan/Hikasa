//! Shared helpers of the picture tests: a seeded xorshift, pictures built in memory, encoders, thread pools.
#![allow(dead_code)]

use std::io::Cursor;
use std::sync::Arc;

use hikasa_core::{Ctl, Pair};
use hikasa_picture::geometry::{orient, scale_bilinear, scaled_side};
use hikasa_picture::{
    DecodeOptions, Geometry, Orientation, Picture, PictureComparison, PictureError, PictureOptions,
    PictureStats, Rect, RgbaImage, compare_pictures, decode_bytes,
};
use image::{DynamicImage, ExtendedColorType, ImageFormat};

/// A seeded xorshift64.
#[derive(Clone)]
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0.max(1);
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// A picture of RGBA bytes.
pub fn pic(width: u32, height: u32, rgba: Vec<u8>) -> Arc<Picture> {
    Arc::new(Picture::from_rgba(width, height, rgba).expect("width · height · 4 bytes"))
}

/// A picture of one colour.
pub fn flat(width: u32, height: u32, c: [u8; 4]) -> Arc<Picture> {
    pic(width, height, c.repeat(width as usize * height as usize))
}

/// A picture of random opaque-or-not pixels.
pub fn noise(width: u32, height: u32, seed: u64) -> Arc<Picture> {
    let mut rng = XorShift(seed);
    pic(
        width,
        height,
        rng.bytes(width as usize * height as usize * 4),
    )
}

/// Compares under `tolerance` and `geometry` (detached control).
pub fn compare(
    left: &Arc<Picture>,
    right: &Arc<Picture>,
    tolerance: u8,
    geometry: Geometry,
) -> PictureComparison {
    compare_pictures(
        Arc::clone(left),
        Arc::clone(right),
        &PictureOptions {
            tolerance,
            geometry,
        },
        &Ctl::detached(),
    )
    .expect("compare")
}

/// Runs `f` in a rayon pool of `threads` threads.
pub fn in_pool<T: Send>(threads: usize, f: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool")
        .install(f)
}

/// The thread counts every property runs with (§6.12).
pub const THREADS: [usize; 3] = [1, 4, 16];

/// Encodes raw samples with `image`'s encoder of `format`.
pub fn encode(
    format: ImageFormat,
    width: u32,
    height: u32,
    color: ExtendedColorType,
    raw: &[u8],
) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::write_buffer_with_format(&mut out, raw, width, height, color, format)
        .unwrap_or_else(|e| panic!("encode {format:?} {color:?}: {e}"));
    out.into_inner()
}

/// Encodes a dynamic image with `image`'s encoder of `format`.
pub fn encode_dynamic(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image
        .write_to(&mut out, format)
        .unwrap_or_else(|e| panic!("encode {format:?}: {e}"));
    out.into_inner()
}

/// Decodes with the default options.
pub fn decode(bytes: &[u8], name: &str) -> Result<Picture, PictureError> {
    decode_bytes(
        bytes,
        Some(name),
        &DecodeOptions::default(),
        &Ctl::detached(),
    )
}

/// The pair of a value.
pub fn both<T: Copy>(v: T) -> Pair<T> {
    Pair::new(v, v)
}

/// A local spec of `path`.
pub fn lspec(path: &std::path::Path) -> hikasa_core::Spec {
    hikasa_core::Spec::Local(
        hikasa_core::LocalPath::new(path.to_path_buf()).expect("absolute path"),
    )
}

/// A naive comparison: materialized orientation and scaling, every area position visited.
pub fn naive(left: &Picture, right: &Picture, tolerance: u8, geometry: Geometry) -> PictureStats {
    let orient_pic = |p: &Picture, o: Orientation| orient(p.width, p.height, &p.rgba, o);
    let mut l = orient_pic(left, geometry.orientation.left);
    let mut r = orient_pic(right, geometry.orientation.right);
    if geometry.auto_scale {
        match scaled_side((l.width, l.height), (r.width, r.height)) {
            Some(hikasa_core::Side::Left) => l = scale_bilinear(&l, r.width, r.height),
            Some(_) => r = scale_bilinear(&r, l.width, l.height),
            None => {}
        }
    }
    let (dx, dy) = (i64::from(geometry.offset.0), i64::from(geometry.offset.1));
    let (ox, oy) = (dx.min(0), dy.min(0));
    let w = i64::from(l.width).max(dx + i64::from(r.width)) - ox;
    let h = i64::from(l.height).max(dy + i64::from(r.height)) - oy;
    let at = |img: &RgbaImage, x: i64, y: i64| {
        (x >= 0 && y >= 0 && x < i64::from(img.width) && y < i64::from(img.height))
            .then(|| img.pixel(x as u32, y as u32).unwrap())
    };
    let mut s = PictureStats::default();
    let (mut x0, mut y0, mut x1, mut y1) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for ay in 0..h {
        for ax in 0..w {
            let p = at(&l, ax + ox, ay + oy);
            let q = at(&r, ax + ox - dx, ay + oy - dy);
            if p.is_none() && q.is_none() {
                continue;
            }
            s.pixels += 1;
            s.left_only += u64::from(q.is_none());
            s.right_only += u64::from(p.is_none());
            let d = match (p, q) {
                (Some(p), Some(q)) if p.a == 0 && q.a == 0 => 0,
                (Some(p), Some(q)) => [
                    p.r.abs_diff(q.r),
                    p.g.abs_diff(q.g),
                    p.b.abs_diff(q.b),
                    p.a.abs_diff(q.a),
                ]
                .into_iter()
                .max()
                .unwrap(),
                (Some(p), None) => p.a,
                (None, Some(q)) => q.a,
                (None, None) => unreachable!(),
            };
            s.distance_sum += u64::from(d);
            s.max_distance = s.max_distance.max(d);
            match d {
                0 => s.same += 1,
                d if d <= tolerance => s.similar += 1,
                _ => s.different += 1,
            }
            if d != 0 {
                (x0, y0, x1, y1) = (x0.min(ax), y0.min(ay), x1.max(ax), y1.max(ay));
            }
        }
    }
    s.bounds = (x0 <= x1).then(|| Rect {
        x: x0,
        y: y0,
        width: (x1 - x0 + 1) as u32,
        height: (y1 - y0 + 1) as u32,
    });
    s
}

/// A random small picture whose pixels come from a few values, so that equal and near pixels are common.
pub fn small_picture(rng: &mut XorShift, max: u64) -> Arc<Picture> {
    let (w, h) = (1 + rng.below(max) as u32, 1 + rng.below(max) as u32);
    let palette: Vec<u8> = vec![0, 1, 2, 128, 254, 255];
    let rgba = (0..w * h * 4)
        .map(|_| palette[rng.below(palette.len() as u64) as usize])
        .collect();
    pic(w, h, rgba)
}

pub fn random_geometry(rng: &mut XorShift, auto_scale: bool) -> Geometry {
    let o = |rng: &mut XorShift| Orientation::ALL[rng.below(8) as usize];
    Geometry {
        orientation: Pair::new(o(rng), o(rng)),
        auto_scale,
        offset: (rng.below(25) as i32 - 12, rng.below(25) as i32 - 12),
    }
}
