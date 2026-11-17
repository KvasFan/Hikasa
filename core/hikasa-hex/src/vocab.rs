//! The words of hex (§1.12.23's flags, the session keys of W12c): [`VocabError`] and the parsers of
//! [`AlignMode`], [`DisplayFilter`], [`CharSet`], [`Radix`], [`TextEncoding`] and the numeric limits. Adapters
//! reach them through the Service (check 7); the engine's own types use them for their `parse` functions.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A word or number that names nothing (kind `invalid_input`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} {word:?}: expected {expected}")]
pub struct VocabError {
    /// What was parsed (`alignment`, `bytes per line`, …).
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
    /// Alignment of the two files (HX-002, D-327).
    AlignMode, "alignment" {
        /// Byte n with byte n.
        None = "none",
        /// Content-defined blocks, small gaps refined byte-level (default).
        #[default]
        Fast = "fast",
        /// Blocks, then every gap byte-level within the budget.
        Complete = "complete",
    }
}

words! {
    /// Display filter of the rows (HX-005).
    DisplayFilter, "display filter" {
        /// Every row.
        #[default]
        All = "all",
        /// Rows with a non-same cell (and `context` rows around them).
        Differences = "differences" | "diff",
        /// Rows without a non-same cell.
        Same = "same",
    }
}

words! {
    /// The character column (HX-001).
    CharSet, "character set" {
        /// 0x20…0x7E (default).
        #[default]
        Ascii = "ascii",
        /// Also 0xA0…0xFF.
        Latin1 = "latin1" | "latin-1" | "iso-8859-1",
        /// No character column.
        None = "none",
    }
}

words! {
    /// The radix of addresses (HX-001, HX-007).
    Radix, "address radix" {
        /// Hexadecimal (default).
        #[default]
        Hex = "hex" | "hexadecimal",
        /// Decimal.
        Decimal = "decimal" | "dec",
    }
}

words! {
    /// The encoding of a text pattern (HX-005, D-334).
    TextEncoding, "encoding" {
        /// UTF-8 (default).
        #[default]
        Utf8 = "utf-8" | "utf8",
        /// ISO-8859-1: one byte per character up to U+00FF.
        Latin1 = "latin1" | "latin-1" | "iso-8859-1",
        /// UTF-16, little endian.
        Utf16Le = "utf-16le" | "utf16le",
        /// UTF-16, big endian.
        Utf16Be = "utf-16be" | "utf16be",
    }
}

/// Parses a non-negative decimal integer (digits, `_` separators allowed).
fn number(what: &'static str, word: &str, expected: &str) -> Result<u64, VocabError> {
    let digits: String = word.trim().chars().filter(|&c| c != '_').collect();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(VocabError::new(what, word, expected));
    }
    digits
        .parse()
        .map_err(|_| VocabError::new(what, word, expected))
}

/// Parses a number in `min..=max`.
fn ranged(what: &'static str, word: &str, min: u64, max: u64) -> Result<u64, VocabError> {
    let expected = format!("a number from {min} to {max}");
    let n = number(what, word, &expected)?;
    if (min..=max).contains(&n) {
        Ok(n)
    } else {
        Err(VocabError::new(what, word, expected))
    }
}

/// Bytes per line: 1…256.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_bytes_per_line(word: &str) -> Result<u16, VocabError> {
    Ok(ranged("bytes per line", word, 1, 256)? as u16)
}

/// Group size: 1, 2, 4 or 8.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_group(word: &str) -> Result<u8, VocabError> {
    match number("group", word, "1, 2, 4 or 8")? {
        n @ (1 | 2 | 4 | 8) => Ok(n as u8),
        _ => Err(VocabError::new("group", word, "1, 2, 4 or 8")),
    }
}

/// Context rows of the `differences` filter: 0…64.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_context(word: &str) -> Result<u8, VocabError> {
    Ok(ranged("context", word, 0, 64)? as u8)
}

/// Fast's average block (`[rules.hex] block`, bytes): a power of two from 1 KiB to 1 MiB.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_block(word: &str) -> Result<u32, VocabError> {
    let expected = "a power of two from 1024 to 1048576";
    let n = number("block size", word, expected)?;
    if (1024..=1 << 20).contains(&n) && n.is_power_of_two() {
        Ok(n as u32)
    } else {
        Err(VocabError::new("block size", word, expected))
    }
}

/// Complete's byte window (`[rules.hex] window`, bytes): 4 KiB…16 MiB.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_window(word: &str) -> Result<u32, VocabError> {
    Ok(ranged("window", word, 4 << 10, 16 << 20)? as u32)
}

/// The skew limit (`[rules.hex] skew`, bytes): at least 1.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_skew(word: &str) -> Result<u64, VocabError> {
    ranged("skew", word, 1, u64::MAX)
}

/// Complete's byte budget (`[rules.hex] budget_mib`, MiB): 1…1,048,576.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_budget_mib(word: &str) -> Result<u64, VocabError> {
    ranged("budget", word, 1, 1 << 20)
}

/// The largest number of stored sections: 1…2^32.
///
/// # Errors
///
/// [`VocabError`].
pub fn parse_max_sections(word: &str) -> Result<u64, VocabError> {
    ranged("max sections", word, 1, 1 << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_round_trip_and_refuse() {
        for mode in AlignMode::ALL {
            assert_eq!(AlignMode::parse(mode.as_str()), Ok(*mode));
        }
        assert_eq!(AlignMode::parse(" FAST "), Ok(AlignMode::Fast));
        assert_eq!(
            AlignMode::parse("slow").unwrap_err().to_string(),
            "invalid alignment \"slow\": expected none, fast, complete"
        );
        assert_eq!(CharSet::parse("Latin-1"), Ok(CharSet::Latin1));
        assert_eq!(Radix::parse("dec"), Ok(Radix::Decimal));
        assert_eq!(TextEncoding::parse("UTF16BE"), Ok(TextEncoding::Utf16Be));
        assert_eq!(DisplayFilter::parse("diff"), Ok(DisplayFilter::Differences));
        assert_eq!(parse_bytes_per_line("256"), Ok(256));
        assert!(parse_bytes_per_line("0").is_err() && parse_bytes_per_line("257").is_err());
        assert_eq!(parse_group("8"), Ok(8));
        assert!(parse_group("3").is_err());
        assert_eq!(parse_block("8192"), Ok(8192));
        assert!(parse_block("8000").is_err() && parse_block("512").is_err());
        assert_eq!(parse_window("4096"), Ok(4096));
        assert!(parse_window("4095").is_err());
        assert_eq!(parse_context("64"), Ok(64));
        assert!(parse_context("65").is_err() && parse_context("-1").is_err());
        assert_eq!(parse_skew("16_777_216"), Ok(16 << 20));
        assert!(parse_budget_mib("0").is_err());
        assert!(parse_max_sections("x").is_err());
    }
}
