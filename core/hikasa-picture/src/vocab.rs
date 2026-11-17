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
}
