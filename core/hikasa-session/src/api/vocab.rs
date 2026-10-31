//! Vocabulary parsers reachable from the service API (T11: one parser for the CLI and the GUI).
//!
//! Every vocabulary lives with the crate that owns its meaning: `--display` (folders:
//! `FolderDisplayFilter::parse`; text: `hikasa_report::DisplayMode::parse`), `--folders`
//! (`hikasa_report::parse_folder_mode`), `--criteria` (`Criteria::parse_list`), `--unimportant`
//! (`UnimportantItem::parse_list`), the report words (`TextLayout::parse`, `PatchStyle::parse`, `Wrap::parse`,
//! `FolderLayout::parse`, `FolderColumns::parse_list`, `OutputFormat::parse`) and the legacy `gui open` switches
//! ([`super::gui::normalize_legacy_args`], [`super::gui::parse_open_args`]). This module adapts their error
//! types to [`ApiError`] and adds the few words no core crate parses (wave-4 review, §1.8.7):
//!
//! | Flag | Parser |
//! |---|---|
//! | `--folders` | [`parse_folder_mode`] |
//! | `--archives files\|opened\|folders` | [`parse_archive_handling`] |
//! | `--content none\|crc\|binary\|rules` | [`parse_content_method`] |
//! | `--algorithm …` | [`parse_algorithm`] |
//! | `--lines <a>-<b>` | [`parse_line_range`] |
//! | `--display`/`--context` defaults (text) | [`resolve_text_display`] |
//! | `--display`/`--folders`/`--suppress-filters` defaults (folders) | [`resolve_folder_display`] |

use std::ops::RangeInclusive;

use hikasa_core::ContentMethod;
use hikasa_folder::{ArchiveHandling, DisplayOptions, FolderDisplayFilter, FolderMode};
use hikasa_report::DisplayMode;
pub use hikasa_report::folder_mode_word;
use hikasa_text::AlgorithmChoice;

use super::ApiError;

/// The default folder display filter of `folder compare` and of a folder view (§1.9.2 amendment): show the
/// differences (the §4.3.3 examples).
pub const DEFAULT_FOLDER_FILTER: FolderDisplayFilter = FolderDisplayFilter::Differences;

/// The default folder mode: compare files and folder structure (the §4.3.3 examples).
pub const DEFAULT_FOLDER_MODE: FolderMode = FolderMode::CompareStructure;

/// Parses `--folders` (the folder mode of the display filters, FC-055): `always` (Always Show Folders),
/// `structure` (Compare Files and Folder Structure), `files-only` (Only Compare Files), and the long forms
/// hikasa-report accepts. Delegates to `hikasa_report::parse_folder_mode`; an unknown word is
/// `ApiError::Report(ReportError::InvalidOption)` (kind `invalid_input`, exit 100).
pub fn parse_folder_mode(word: &str) -> Result<FolderMode, ApiError> {
    Ok(hikasa_report::parse_folder_mode(word)?)
}

/// The words of `--archives` (D-152).
const ARCHIVE_WORDS: [(&str, ArchiveHandling); 3] = [
    ("files", ArchiveHandling::AsFiles),
    ("opened", ArchiveHandling::OnceOpened),
    ("folders", ArchiveHandling::Always),
];

/// Parses `--archives files|opened|folders` (ASCII case-insensitive). An unknown word is `InvalidArgument` naming
/// the valid words.
pub fn parse_archive_handling(word: &str) -> Result<ArchiveHandling, ApiError> {
    let trimmed = word.trim();
    ARCHIVE_WORDS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
        .map(|(_, mode)| *mode)
        .ok_or_else(|| {
            ApiError::InvalidArgument(format!(
                "unknown archive handling {word:?} (valid: files, opened, folders)"
            ))
        })
}

/// The words of `--content` (§1.9.2), in `ContentMethod` order.
const CONTENT_WORDS: [(&str, ContentMethod); 4] = [
    ("none", ContentMethod::None),
    ("crc", ContentMethod::Crc),
    ("binary", ContentMethod::Binary),
    ("rules", ContentMethod::Rules),
];

/// Parses `--content none|crc|binary|rules` (ASCII case-insensitive; `rules-based` is accepted as in the
/// `--criteria` vocabulary). An unknown word is `InvalidArgument` naming the valid words. Additive.
pub fn parse_content_method(word: &str) -> Result<ContentMethod, ApiError> {
    let trimmed = word.trim();
    if trimmed.eq_ignore_ascii_case("rules-based") {
        return Ok(ContentMethod::Rules);
    }
    CONTENT_WORDS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(trimmed))
        .map(|(_, method)| *method)
        .ok_or_else(|| {
            ApiError::InvalidArgument(format!(
                "unknown content method {word:?} (valid: none, crc, binary, rules)"
            ))
        })
}

/// Parses `--algorithm` (`AlgorithmChoice::parse`, ASCII case-insensitive); an unknown word is
/// `InvalidArgument` naming the valid words. Additive.
pub fn parse_algorithm(word: &str) -> Result<AlgorithmChoice, ApiError> {
    AlgorithmChoice::parse(word).ok_or_else(|| {
        let valid: Vec<&str> = AlgorithmChoice::ALL.iter().map(|a| a.as_str()).collect();
        ApiError::InvalidArgument(format!(
            "unknown algorithm {word:?} (valid: {})",
            valid.join(", ")
        ))
    })
}

/// Parses `--lines <a>-<b>` (`format tokens`): 1-based inclusive lines with `1 ≤ a ≤ b`; a single number `n`
/// means `n-n`. Spaces around the numbers are ignored. `5-`, `-3`, `0-1`, `9-2` and non-numbers are
/// `InvalidArgument`. Additive.
pub fn parse_line_range(text: &str) -> Result<RangeInclusive<u32>, ApiError> {
    let invalid = || {
        ApiError::InvalidArgument(format!(
            "invalid line range {text:?} (expected <first>-<last> with 1 <= first <= last, or one line number)"
        ))
    };
    let number = |s: &str| -> Result<u32, ApiError> {
        let s = s.trim();
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        s.parse::<u32>().map_err(|_| invalid())
    };
    let (first, last) = match text.split_once('-') {
        Some((a, b)) => (number(a)?, number(b)?),
        None => {
            let n = number(text)?;
            (n, n)
        }
    };
    if first == 0 || first > last {
        return Err(invalid());
    }
    Ok(first..=last)
}

/// The text display mode of `--display` and `--context` (§4.3.2, §1.9.2 amendment): a `--display` word
/// (`DisplayMode::parse`, its `context` taking `--context` or `default_context` lines); else `--context n`
/// alone means `context` with `n` lines; else `mismatches` for JSON output and `all` otherwise (the human
/// report layouts and GUI views). An unknown word is `ApiError::Report` (exit 100). Additive.
pub fn resolve_text_display(
    display: Option<&str>,
    context: Option<u32>,
    default_context: u32,
    json: bool,
) -> Result<DisplayMode, ApiError> {
    Ok(match (display, context) {
        (Some(word), _) => DisplayMode::parse(word, context.unwrap_or(default_context))?,
        (None, Some(lines)) => DisplayMode::Context { lines },
        (None, None) if json => DisplayMode::Mismatches,
        (None, None) => DisplayMode::All,
    })
}

/// The folder display options of `--display`, `--folders`, `--suppress-filters` and the effective
/// ignore-unimportant (§4.3.3, §1.9.2 amendment): defaults [`DEFAULT_FOLDER_FILTER`] and
/// [`DEFAULT_FOLDER_MODE`]; unknown words are errors (exit 100). Additive.
pub fn resolve_folder_display(
    display: Option<&str>,
    folders: Option<&str>,
    suppress_filters: bool,
    ignore_unimportant: bool,
) -> Result<DisplayOptions, ApiError> {
    let filter = match display {
        Some(word) => FolderDisplayFilter::parse(word).map_err(ApiError::Folder)?,
        None => DEFAULT_FOLDER_FILTER,
    };
    let folders = match folders {
        Some(word) => parse_folder_mode(word)?,
        None => DEFAULT_FOLDER_MODE,
    };
    Ok(DisplayOptions {
        filter,
        folders,
        ignore_unimportant,
        suppress_filters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_mode_words() {
        for mode in FolderMode::ALL {
            assert_eq!(parse_folder_mode(folder_mode_word(mode)).unwrap(), mode);
        }
        assert_eq!(
            parse_folder_mode(" Always_Show_Folders ").unwrap(),
            FolderMode::AlwaysShowFolders
        );
        assert_eq!(
            parse_folder_mode("ONLY-FILES").unwrap(),
            FolderMode::OnlyFiles
        );
        let err = parse_folder_mode("tree").unwrap_err();
        assert_eq!(err.kind(), hikasa_core::ErrorKind::InvalidInput);
    }

    #[test]
    fn content_words() {
        for (word, method) in CONTENT_WORDS {
            assert_eq!(parse_content_method(word).unwrap(), method);
            assert_eq!(method.as_str(), word);
            assert_eq!(parse_content_method(&word.to_uppercase()).unwrap(), method);
        }
        assert_eq!(
            parse_content_method("rules-based").unwrap(),
            ContentMethod::Rules
        );
        let err = parse_content_method("md5").unwrap_err();
        assert!(
            err.to_string().contains("none, crc, binary, rules"),
            "{err}"
        );
    }

    #[test]
    fn algorithm_words() {
        for choice in AlgorithmChoice::ALL {
            assert_eq!(parse_algorithm(choice.as_str()).unwrap(), choice);
        }
        let err = parse_algorithm("diff3").unwrap_err();
        assert!(err.to_string().contains("histogram"), "{err}");
    }

    #[test]
    fn line_ranges() {
        assert_eq!(parse_line_range("2-5").unwrap(), 2..=5);
        assert_eq!(parse_line_range(" 3 - 3 ").unwrap(), 3..=3);
        assert_eq!(parse_line_range("7").unwrap(), 7..=7);
        for bad in [
            "",
            "5-",
            "-3",
            "0-1",
            "0",
            "9-2",
            "a-b",
            "1-2-3",
            "+1-2",
            "4294967296",
        ] {
            assert!(parse_line_range(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn text_display_defaults() {
        assert_eq!(
            resolve_text_display(None, None, 3, true).unwrap(),
            DisplayMode::Mismatches
        );
        assert_eq!(
            resolve_text_display(None, None, 3, false).unwrap(),
            DisplayMode::All
        );
        assert_eq!(
            resolve_text_display(None, Some(2), 3, true).unwrap(),
            DisplayMode::Context { lines: 2 }
        );
        assert_eq!(
            resolve_text_display(Some("context"), None, 4, true).unwrap(),
            DisplayMode::Context { lines: 4 }
        );
        assert_eq!(
            resolve_text_display(Some("all"), Some(9), 3, true).unwrap(),
            DisplayMode::All
        );
        assert!(resolve_text_display(Some("some"), None, 3, false).is_err());
    }

    #[test]
    fn folder_display_defaults() {
        let d = resolve_folder_display(None, None, false, true).unwrap();
        assert_eq!(d.filter, DEFAULT_FOLDER_FILTER);
        assert_eq!(d.folders, DEFAULT_FOLDER_MODE);
        assert!(d.ignore_unimportant && !d.suppress_filters);
        let d = resolve_folder_display(Some("all"), Some("files-only"), true, false).unwrap();
        assert_eq!(d.filter, FolderDisplayFilter::All);
        assert_eq!(d.folders, FolderMode::OnlyFiles);
        assert!(d.suppress_filters);
        assert!(resolve_folder_display(Some("bogus"), None, false, false).is_err());
        assert!(resolve_folder_display(None, Some("tree"), false, false).is_err());
    }
}
