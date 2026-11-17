//! The tone map of HDR and OpenEXR pictures (PX-001, §1.12.14): linear float → `t(v)` → the sRGB OETF through a
//! pinned 65,536-entry table → 8 bits.

use std::sync::OnceLock;

use crate::vocab::ToneMap;

/// Entries of the OETF table: `t` is quantized to `round(t · 65535)`.
pub const OETF_ENTRIES: usize = 65_536;

/// The sRGB OETF over `i / 65535` for `i` in `0..65536`, computed once in `f64` (`12.92 x` below 0.0031308, else
/// `1.055 x^(1/2.4) − 0.055`) and rounded half up to `u8`. A test pins its CRC-32 on every platform, so a libm
/// difference fails CI instead of a golden.
pub fn oetf_table() -> &'static [u8; OETF_ENTRIES] {
    static TABLE: OnceLock<Box<[u8; OETF_ENTRIES]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = Box::new([0u8; OETF_ENTRIES]);
        for (i, slot) in table.iter_mut().enumerate() {
            let x = i as f64 / 65_535.0;
            let s = if x < 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            *slot = (s * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
        }
        table
    })
}

/// `t(v)` of the tone map: in `[0, 1]`; NaN → 0, +∞ → 1, negative → 0.
pub(crate) fn tone(map: ToneMap, v: f32) -> f64 {
    if v.is_nan() {
        return 0.0;
    }
    let v = f64::from(v);
    match map {
        ToneMap::Clamp => v.clamp(0.0, 1.0),
        ToneMap::Reinhard => {
            if v == f64::INFINITY {
                1.0
            } else if v >= 0.0 {
                v / (1.0 + v)
            } else {
                0.0
            }
        }
    }
}

/// One linear colour channel to 8 display bits: `OETF[round(t(v) · 65535)]`.
pub(crate) fn channel(map: ToneMap, table: &[u8; OETF_ENTRIES], v: f32) -> u8 {
    let t = tone(map, v);
    let index = (t * 65_535.0 + 0.5).floor() as usize;
    table[index.min(OETF_ENTRIES - 1)]
}

/// A float channel already display-encoded (alpha of HDR/EXR; every channel of float TIFF):
/// `clamp(v, 0, 1) · 255` rounded half up, NaN → 0.
pub(crate) fn display(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    (f64::from(v).clamp(0.0, 1.0) * 255.0 + 0.5).floor() as u8
}
