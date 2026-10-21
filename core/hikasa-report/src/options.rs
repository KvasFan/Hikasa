//! Report options shared by text and folder reports: [`DisplayMode`], [`OutputFormat`], [`Wrap`], [`HtmlStyle`],
//! and the parsers of their words (SCR-001 vocabulary; T11: adapters map flags, core owns the words).

use hikasa_folder::{FolderDisplayFilter, FolderMode};
use hikasa_text::TextDisplayFilter;
use serde::{Deserialize, Serialize};

use crate::error::ReportError;

/// A vocabulary word normalised for matching: trimmed, ASCII lower case, `_` read as `-`, without `prefix`.
fn normalise<'a>(word: &str, prefix: &str, buf: &'a mut String) -> &'a str {
    *buf = word.trim().to_ascii_lowercase().replace('_', "-");
    let len = buf.len();
    let start = if buf.starts_with(prefix) && len > prefix.len() {
        prefix.len()
    } else {
        0
    };
    &buf[start..]
}

/// Report display modes (RPT-003, SCR-001 `display-*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum DisplayMode {
    /// display-all.
    All,
    /// display-mismatches.
    Mismatches,
    /// display-context (text: ± lines; folders: differences with their folder structure).
    Context {
        /// Context lines around every difference (text reports).
        lines: u32,
    },
    /// display-matches.
    Matches,
}

impl Default for DisplayMode {
    /// [`DisplayMode::All`], the view's default filter (RPT-003).
    fn default() -> DisplayMode {
        DisplayMode::All
    }
}

impl DisplayMode {
    /// Text filter: All→All, Mismatches→Differences, Context→Context, Matches→Same.
    pub fn text_filter(self) -> TextDisplayFilter {
        match self {
            DisplayMode::All => TextDisplayFilter::All,
            DisplayMode::Mismatches => TextDisplayFilter::Differences,
            DisplayMode::Context { lines } => TextDisplayFilter::Context { lines },
            DisplayMode::Matches => TextDisplayFilter::Same,
        }
    }

    /// Folder filter: All→(All, CompareStructure), Mismatches→(Differences, OnlyFiles),
    /// Context→(Differences, CompareStructure), Matches→(Same, OnlyFiles). The Context row is the default
    /// decision of Appendix C #12 ("differences with their folder structure").
    pub fn folder_filter(self) -> (FolderDisplayFilter, FolderMode) {
        match self {
            DisplayMode::All => (FolderDisplayFilter::All, FolderMode::CompareStructure),
            DisplayMode::Mismatches => (FolderDisplayFilter::Differences, FolderMode::OnlyFiles),
            DisplayMode::Context { .. } => (
                FolderDisplayFilter::Differences,
                FolderMode::CompareStructure,
            ),
            DisplayMode::Matches => (FolderDisplayFilter::Same, FolderMode::OnlyFiles),
        }
    }

    /// Parses a display word (additive): `all`, `mismatches` (or `differences`), `context`, `matches` (or
    /// `same`), each also with the script prefix `display-` (SCR-001); ASCII case-insensitive, `_` equals `-`.
    /// `context_lines` is the line count of `context` (the CLI's `--context`, default 3).
    pub fn parse(word: &str, context_lines: u32) -> Result<DisplayMode, ReportError> {
        let mut buf = String::new();
        Ok(match normalise(word, "display-", &mut buf) {
            "all" => DisplayMode::All,
            "mismatches" | "differences" => DisplayMode::Mismatches,
            "context" => DisplayMode::Context {
                lines: context_lines,
            },
            "matches" | "same" => DisplayMode::Matches,
            _ => {
                return Err(ReportError::invalid(
                    "display mode",
                    word,
                    "all, mismatches, context, matches",
                ));
            }
        })
    }

    /// Fixed English wording used in report headers: `all`, `differences`, `context (3 lines)`, `same`.
    pub fn describe(self) -> String {
        match self {
            DisplayMode::All => "all".to_owned(),
            DisplayMode::Mismatches => "differences".to_owned(),
            DisplayMode::Context { lines } => format!("context ({})", plural(lines, "line")),
            DisplayMode::Matches => "same".to_owned(),
        }
    }
}

/// `"1 line"`, `"3 lines"`.
pub(crate) fn plural(n: impl Into<u64>, noun: &str) -> String {
    let n = n.into();
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Output encoding of a layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputFormat {
    /// Plain text; `ansi` adds SGR colours (red important, blue unimportant, …).
    Text {
        /// Add ANSI SGR colour sequences.
        ansi: bool,
    },
    /// Self-contained HTML; `mono` uses the monochrome stylesheet (RPT-002 html-mono).
    Html {
        /// Use the monochrome stylesheet.
        mono: bool,
    },
}

impl Default for OutputFormat {
    /// Plain text without colours.
    fn default() -> OutputFormat {
        OutputFormat::Text { ansi: false }
    }
}

impl OutputFormat {
    /// Parses an output word (additive): `text`, `ansi`, `html` (or `html-color`, `html-colour`), `html-mono`
    /// (SCR-001 `html-color|html-mono`); ASCII case-insensitive. `html-custom:<css>` is not a format: the caller
    /// reads the stylesheet and passes it in [`HtmlStyle`].
    pub fn parse(word: &str) -> Result<OutputFormat, ReportError> {
        let mut buf = String::new();
        Ok(match normalise(word, "", &mut buf) {
            "text" | "plain" => OutputFormat::Text { ansi: false },
            "ansi" => OutputFormat::Text { ansi: true },
            "html" | "html-color" | "html-colour" => OutputFormat::Html { mono: false },
            "html-mono" => OutputFormat::Html { mono: true },
            _ => {
                return Err(ReportError::invalid(
                    "output format",
                    word,
                    "text, ansi, html, html-color, html-mono",
                ));
            }
        })
    }

    /// `"text"` or `"html"` (the `format` of [`ReportError::Unsupported`]).
    pub const fn name(self) -> &'static str {
        match self {
            OutputFormat::Text { .. } => "text",
            OutputFormat::Html { .. } => "html",
        }
    }
}

/// Line wrapping in side-by-side text (RPT-002 wrap none/character/word). HTML always wraps by CSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Wrap {
    /// Lines longer than their column are cut.
    None,
    /// Lines are broken at the column width.
    #[default]
    Character,
    /// Lines are broken after the last space that fits; a longer word is broken at the column width.
    Word,
}

impl Wrap {
    /// Parses `none`, `character` (or `char`), `word`, also with the script prefix `wrap-` (additive).
    pub fn parse(word: &str) -> Result<Wrap, ReportError> {
        let mut buf = String::new();
        Ok(match normalise(word, "wrap-", &mut buf) {
            "none" => Wrap::None,
            "character" | "char" => Wrap::Character,
            "word" => Wrap::Word,
            _ => return Err(ReportError::invalid("wrap", word, "none, character, word")),
        })
    }

    /// `none`, `character`, `word`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Wrap::None => "none",
            Wrap::Character => "character",
            Wrap::Word => "word",
        }
    }
}

/// Parses a `--folders` word (additive, wave 4; §1.9.2, T11): `always` (or `always-show-folders`), `structure`
/// (or `compare-structure`), `files-only` (or `only-files`); ASCII case-insensitive, `_` equals `-`.
pub fn parse_folder_mode(word: &str) -> Result<FolderMode, ReportError> {
    let mut buf = String::new();
    Ok(match normalise(word, "", &mut buf) {
        "always" | "always-show-folders" => FolderMode::AlwaysShowFolders,
        "structure" | "compare-structure" => FolderMode::CompareStructure,
        "files-only" | "only-files" => FolderMode::OnlyFiles,
        _ => {
            return Err(ReportError::invalid(
                "folder mode",
                word,
                "always, structure, files-only",
            ));
        }
    })
}

/// The `--folders` word of a folder mode (additive, wave 4): `always`, `structure`, `files-only`.
pub const fn folder_mode_word(mode: FolderMode) -> &'static str {
    match mode {
        FolderMode::AlwaysShowFolders => "always",
        FolderMode::CompareStructure => "structure",
        FolderMode::OnlyFiles => "files-only",
    }
}

/// Styling inputs that do not fit the `Copy` option structs (additive, wave 4): RPT-002 / SCR-001
/// `html-custom:<css>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct HtmlStyle {
    /// A custom stylesheet, embedded after the built-in one (so its rules win) in every HTML report. The caller
    /// reads the file (core does no I/O of its own). `None` = the built-in colour or mono stylesheet only.
    pub custom_css: Option<String>,
}

impl HtmlStyle {
    /// A style with a custom stylesheet.
    pub fn custom(css: impl Into<String>) -> HtmlStyle {
        HtmlStyle {
            custom_css: Some(css.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_mappings() {
        use DisplayMode as D;
        assert_eq!(D::All.text_filter(), TextDisplayFilter::All);
        assert_eq!(D::Mismatches.text_filter(), TextDisplayFilter::Differences);
        assert_eq!(
            D::Context { lines: 2 }.text_filter(),
            TextDisplayFilter::Context { lines: 2 }
        );
        assert_eq!(D::Matches.text_filter(), TextDisplayFilter::Same);
        assert_eq!(
            D::All.folder_filter(),
            (FolderDisplayFilter::All, FolderMode::CompareStructure)
        );
        assert_eq!(
            D::Mismatches.folder_filter(),
            (FolderDisplayFilter::Differences, FolderMode::OnlyFiles)
        );
        assert_eq!(
            D::Context { lines: 9 }.folder_filter(),
            (
                FolderDisplayFilter::Differences,
                FolderMode::CompareStructure
            )
        );
        assert_eq!(
            D::Matches.folder_filter(),
            (FolderDisplayFilter::Same, FolderMode::OnlyFiles)
        );
        assert_eq!(D::default(), D::All);
    }

    #[test]
    fn display_mode_words() {
        use DisplayMode as D;
        let cases = [
            ("all", D::All),
            ("display-all", D::All),
            ("Mismatches", D::Mismatches),
            ("display-mismatches", D::Mismatches),
            ("differences", D::Mismatches),
            ("context", D::Context { lines: 4 }),
            ("DISPLAY_CONTEXT", D::Context { lines: 4 }),
            ("matches", D::Matches),
            ("same", D::Matches),
            (" display-matches ", D::Matches),
        ];
        for (word, want) in cases {
            assert_eq!(D::parse(word, 4).unwrap(), want, "{word}");
        }
        for bad in ["", "display-", "none", "context:3", "orphans"] {
            assert!(D::parse(bad, 3).is_err(), "{bad}");
        }
        assert_eq!(D::All.describe(), "all");
        assert_eq!(D::Mismatches.describe(), "differences");
        assert_eq!(D::Context { lines: 1 }.describe(), "context (1 line)");
        assert_eq!(D::Context { lines: 3 }.describe(), "context (3 lines)");
        assert_eq!(D::Matches.describe(), "same");
    }

    #[test]
    fn display_mode_serde() {
        assert_eq!(
            serde_json::to_string(&DisplayMode::Context { lines: 3 }).unwrap(),
            r#"{"mode":"context","lines":3}"#
        );
        assert_eq!(
            serde_json::to_string(&DisplayMode::Mismatches).unwrap(),
            r#"{"mode":"mismatches"}"#
        );
        let back: DisplayMode = serde_json::from_str(r#"{"mode":"matches"}"#).unwrap();
        assert_eq!(back, DisplayMode::Matches);
    }

    #[test]
    fn wrap_and_format_words() {
        assert_eq!(Wrap::parse("none").unwrap(), Wrap::None);
        assert_eq!(Wrap::parse("wrap-word").unwrap(), Wrap::Word);
        assert_eq!(Wrap::parse("Character").unwrap(), Wrap::Character);
        assert_eq!(Wrap::parse("char").unwrap(), Wrap::Character);
        assert!(Wrap::parse("line").is_err());
        assert_eq!(Wrap::default(), Wrap::Character);
        assert_eq!(Wrap::Word.as_str(), "word");
        assert_eq!(
            OutputFormat::parse("html-color").unwrap(),
            OutputFormat::Html { mono: false }
        );
        assert_eq!(
            OutputFormat::parse("HTML_MONO").unwrap(),
            OutputFormat::Html { mono: true }
        );
        assert_eq!(
            OutputFormat::parse("ansi").unwrap(),
            OutputFormat::Text { ansi: true }
        );
        assert_eq!(
            OutputFormat::parse("text").unwrap(),
            OutputFormat::Text { ansi: false }
        );
        assert!(OutputFormat::parse("html-custom:x.css").is_err());
        assert_eq!(OutputFormat::Html { mono: true }.name(), "html");
        assert_eq!(HtmlStyle::custom("a{}").custom_css.as_deref(), Some("a{}"));
        for (word, mode) in [
            ("always", FolderMode::AlwaysShowFolders),
            ("Always_Show_Folders", FolderMode::AlwaysShowFolders),
            ("structure", FolderMode::CompareStructure),
            ("compare-structure", FolderMode::CompareStructure),
            ("files-only", FolderMode::OnlyFiles),
            ("only_files", FolderMode::OnlyFiles),
        ] {
            assert_eq!(parse_folder_mode(word).unwrap(), mode, "{word}");
        }
        assert!(parse_folder_mode("files").is_err());
        for mode in FolderMode::ALL {
            assert_eq!(parse_folder_mode(folder_mode_word(mode)).unwrap(), mode);
        }
        assert_eq!(plural(0u32, "line"), "0 lines");
    }
}
