//! The words of pictures (§1.12.23's flags, the session keys of W12c): [`VocabError`], [`ToneMap`], [`Pane`] and
//! the parsers of the numbers (tolerance, blend, megapixels, offsets, probe points). `Orientation::parse_list` and
//! `DiffMode::parse` live beside their types. Adapters reach them through the Service (check 7).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::geometry::MAX_OFFSET;

/// A word or number that names nothing (kind `invalid_input`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} {word:?}: expected {expected}")]
pub struct VocabError {
    /// What was parsed (`tone map`, `tolerance`, …).
    pub what: &'static str,
    /// The rejected text.
    pub word: String,
    /// What is accepted.
    pub expected: String,
}

impl VocabError {
    pub(crate) fn new(what: &'static str, word: &str, expected: impl Into<String>) -> VocabError {
        VocabError {
            what,
            word: word.to_owned(),
            expected: expected.into(),
        }
    }
}

/// Defines a word enum with `ALL`, `as_str`, `parse` (ASCII case-insensitive) and `Display`.
macro_rules! words {
    ($(#[$m:meta])* $name:ident, $what:literal { $($(#[$vm:meta])* $variant:ident = $word:literal $(| $alias:literal)*),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($(#[$vm])* $variant),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The stable word.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $word),+
                }
            }

            /// Parses a word (ASCII case ignored; aliases accepted).
            ///
            /// # Errors
            ///
            /// [`VocabError`] naming the accepted words.
            pub fn parse(word: &str) -> Result<$name, VocabError> {
                let lower = word.trim().to_ascii_lowercase();
                $(if lower == $word $(|| lower == $alias)* {
                    return Ok($name::$variant);
                })+
                Err(VocabError::new($what, word, [$($word),+].join(", ")))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

words! {
    /// The tone map of HDR and OpenEXR pictures (PX-001, §1.12.14): linear light to display values before the
    /// sRGB transfer function.
    ToneMap, "tone map" {
        /// `clamp(v, 0, 1)`: SDR content in EXR shows as authored (default).
        #[default]
        Clamp = "clamp",
        /// `v / (1 + v)`: highlights kept.
        Reinhard = "reinhard",
    }
}

words! {
    /// What an image shows (§1.12.18): a side's picture placed in the area, or the difference image.
    Pane, "pane" {
        /// The left picture, oriented and scaled, placed in the area.
        Left = "left",
        /// The right picture, oriented and scaled, placed in the area.
        Right = "right",
        /// The difference image in the chosen mode (default).
        #[default]
        Difference = "difference" | "diff",
    }
}

/// Parses an integer within `min..=max` (ASCII digits, an optional sign when `min < 0`).
fn bounded(what: &'static str, text: &str, min: i64, max: i64) -> Result<i64, VocabError> {
    let expected = || format!("an integer from {min} to {max}");
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(VocabError::new(what, text, expected()));
    }
    match trimmed.parse::<i64>() {
        Ok(v) if (min..=max).contains(&v) => Ok(v),
        _ => Err(VocabError::new(what, text, expected())),
    }
}

/// Parses `--tolerance`: 0…255.
///
/// # Errors
///
/// [`VocabError`] outside the range or for anything but digits.
pub fn parse_tolerance(text: &str) -> Result<u8, VocabError> {
    bounded("tolerance", text, 0, 255).map(|v| v as u8)
}

/// Parses `--blend`: a percentage 0…100.
///
/// # Errors
///
/// [`VocabError`] outside the range or for anything but digits.
pub fn parse_blend(text: &str) -> Result<u8, VocabError> {
    bounded("blend percentage", text, 0, 100).map(|v| v as u8)
}

/// Largest `--max-megapixels`: 4,096 (2³² pixels, more than any decoder's dimensions allow).
pub const MAX_MEGAPIXELS: u64 = 4096;

/// Parses `--max-megapixels` (1…4,096) and returns the pixel limit (`n · 2²⁰`: 256 is the default 268,435,456).
///
/// # Errors
///
/// [`VocabError`] outside the range.
pub fn parse_max_megapixels(text: &str) -> Result<u64, VocabError> {
    bounded("megapixel limit", text, 1, MAX_MEGAPIXELS as i64).map(|v| (v as u64) << 20)
}

/// Splits `a,b` (spaces around either number allowed).
fn pair<'t>(
    what: &'static str,
    text: &'t str,
    expected: &str,
) -> Result<(&'t str, &'t str), VocabError> {
    match text.split_once(',') {
        Some((a, b)) if !b.contains(',') => Ok((a, b)),
        _ => Err(VocabError::new(what, text, expected)),
    }
}

/// Parses `--offset <dx>,<dy>`: each component within ±2²⁰ ([`MAX_OFFSET`]).
///
/// # Errors
///
/// [`VocabError`] for anything but two signed integers in range.
pub fn parse_offset(text: &str) -> Result<(i32, i32), VocabError> {
    let max = i64::from(MAX_OFFSET);
    let expected = format!("<dx>,<dy> with each from {} to {max}", -max);
    let (a, b) = pair("offset", text, &expected)?;
    let dx =
        bounded("offset", a, -max, max).map_err(|_| VocabError::new("offset", text, &expected))?;
    let dy =
        bounded("offset", b, -max, max).map_err(|_| VocabError::new("offset", text, &expected))?;
    Ok((dx as i32, dy as i32))
}

/// Parses `--probe <x>,<y>`: area coordinates (non-negative 32-bit integers).
///
/// # Errors
///
/// [`VocabError`] for anything but two non-negative integers.
pub fn parse_point(text: &str) -> Result<(u32, u32), VocabError> {
    let expected = "<x>,<y> with non-negative integers";
    let (a, b) = pair("point", text, expected)?;
    let max = i64::from(u32::MAX);
    let x = bounded("point", a, 0, max).map_err(|_| VocabError::new("point", text, expected))?;
    let y = bounded("point", b, 0, max).map_err(|_| VocabError::new("point", text, expected))?;
    Ok((x as u32, y as u32))
}

/// The smallest zoom of a view: 1/256 (0.390625 %).
pub const MIN_ZOOM: f64 = 1.0 / 256.0;

/// The largest zoom of a view: 64 (6,400 %).
pub const MAX_ZOOM: f64 = 64.0;

/// A picture view's zoom as a session keeps it (`[view] zoom`, M2 W13b, D-370): `fit`, `actual` (one picture pixel
/// per physical screen pixel) or a scale (screen pixels per picture pixel, written as a percentage).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zoom {
    /// The whole area fits every pane (the default).
    Fit,
    /// Scale 1 in physical pixels.
    Actual,
    /// A scale in [`MIN_ZOOM`]…[`MAX_ZOOM`] (1.0 is 100 %).
    Scale(f64),
}

impl Zoom {
    /// The session word: `fit`, `actual` or the percentage with at most 6 decimals (`150%`, `0.390625%`).
    pub fn word(&self) -> String {
        match self {
            Zoom::Fit => "fit".to_owned(),
            Zoom::Actual => "actual".to_owned(),
            Zoom::Scale(s) => {
                let text = format!("{:.6}", s * 100.0);
                let text = text.trim_end_matches('0').trim_end_matches('.');
                format!("{text}%")
            }
        }
    }
}

/// Parses a zoom word (`[view] zoom`): `fit`, `actual` (ASCII case ignored) or a percentage of
/// 0.390625 %…6,400 % — decimal digits with at most one `.`, an optional `%`.
///
/// # Errors
///
/// [`VocabError`] for anything else.
pub fn parse_zoom(text: &str) -> Result<Zoom, VocabError> {
    let expected = "fit, actual or a percentage from 0.390625% to 6400%";
    let word = text.trim();
    match word.to_ascii_lowercase().as_str() {
        "fit" => return Ok(Zoom::Fit),
        "actual" => return Ok(Zoom::Actual),
        _ => {}
    }
    let number = word.strip_suffix('%').unwrap_or(word).trim_end();
    let digits = number.bytes().filter(u8::is_ascii_digit).count();
    let dots = number.bytes().filter(|&b| b == b'.').count();
    if digits == 0 || dots > 1 || digits + dots != number.len() {
        return Err(VocabError::new("zoom", text, expected));
    }
    match number.parse::<f64>() {
        Ok(p) if (MIN_ZOOM * 100.0..=MAX_ZOOM * 100.0).contains(&p) => Ok(Zoom::Scale(p / 100.0)),
        _ => Err(VocabError::new("zoom", text, expected)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_numbers() {
        assert_eq!(ToneMap::parse("Reinhard"), Ok(ToneMap::Reinhard));
        assert_eq!(Pane::parse("diff"), Ok(Pane::Difference));
        assert!(Pane::parse("middle").is_err());
        assert_eq!(parse_tolerance("255"), Ok(255));
        assert!(parse_tolerance("256").is_err());
        assert!(parse_tolerance("-1").is_err());
        assert!(parse_tolerance("").is_err());
        assert_eq!(parse_blend("0"), Ok(0));
        assert!(parse_blend("101").is_err());
        assert_eq!(parse_max_megapixels("256"), Ok(268_435_456));
        assert!(parse_max_megapixels("0").is_err());
        assert_eq!(parse_offset("-3, 4"), Ok((-3, 4)));
        assert_eq!(parse_offset("1048576,-1048576"), Ok((1 << 20, -(1 << 20))));
        assert!(parse_offset("1048577,0").is_err());
        assert!(parse_offset("1,2,3").is_err());
        assert!(parse_offset("1").is_err());
        assert_eq!(parse_point("0,7"), Ok((0, 7)));
        assert!(parse_point("-1,7").is_err());
    }

    /// A13.11's core table: the zoom words and their round trip.
    #[test]
    fn zoom_words() {
        assert_eq!(parse_zoom("FIT"), Ok(Zoom::Fit));
        assert_eq!(parse_zoom(" actual "), Ok(Zoom::Actual));
        assert_eq!(parse_zoom("150%"), Ok(Zoom::Scale(1.5)));
        assert_eq!(parse_zoom("100"), Ok(Zoom::Scale(1.0)));
        assert_eq!(parse_zoom("0.390625%"), Ok(Zoom::Scale(MIN_ZOOM)));
        assert_eq!(parse_zoom("6400%"), Ok(Zoom::Scale(MAX_ZOOM)));
        for bad in [
            "", "%", "0.39%", "6400.1%", "6401", "1e2", "-50%", "+50%", "50%%", "1.2.3", "nan",
            "inf", "zoom", "50 %x",
        ] {
            assert!(parse_zoom(bad).is_err(), "{bad:?}");
        }
        assert_eq!(Zoom::Fit.word(), "fit");
        assert_eq!(Zoom::Actual.word(), "actual");
        assert_eq!(Zoom::Scale(1.5).word(), "150%");
        assert_eq!(Zoom::Scale(MIN_ZOOM).word(), "0.390625%");
        assert_eq!(Zoom::Scale(MAX_ZOOM).word(), "6400%");
        // Every zoom step of the view (§1.13.10) round trips within 1e-8 and its word is stable.
        let steps = [
            1. / 256.,
            1. / 128.,
            1. / 64.,
            1. / 32.,
            1. / 16.,
            1. / 12.,
            1. / 8.,
            1. / 6.,
            1. / 4.,
            1. / 3.,
            1. / 2.,
            2. / 3.,
            1.,
            1.5,
            2.,
            3.,
            4.,
            6.,
            8.,
            12.,
            16.,
            24.,
            32.,
            48.,
            64.,
        ];
        for s in steps {
            let word = Zoom::Scale(s).word();
            let Ok(Zoom::Scale(back)) = parse_zoom(&word) else {
                panic!("{word}");
            };
            assert!((back - s).abs() < 1e-8, "{word}: {back} vs {s}");
            assert_eq!(Zoom::Scale(back).word(), word);
        }
    }
}
