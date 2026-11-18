//! [`ReportRequest`] (M2 W7, D-196, D-209, D-210): the words of one report request, as a script, the dialog or the
//! CLI gives them, parsed once into the options of `hikasa-report` (T11: the one parser of the vocabulary), and
//! [`render_report`], the one entry point of a request.
//!
//! Words are ASCII case-insensitive and `_` equals `-`; a word fills one slot. The M1 parsers ([`DisplayMode::parse`],
//! [`Wrap::parse`], [`OutputFormat::parse`], [`PatchStyle::parse`], [`TextLayout::parse`], [`FolderLayout::parse`],
//! [`FolderColumns::parse_list`], [`FolderDisplayFilter::parse`]) read the values of their slots, so every M1 spelling
//! stays valid; the words W7 adds (`interleaved`, `xml`, `display-none`, …) are read here, and the M1 parsers keep
//! their M1 words (their tests are unchanged).

use std::collections::BTreeMap;
use std::io::Write;

use hikasa_core::{Side, Warning, WarningCode};
use hikasa_folder::select::Selection;
use hikasa_folder::{DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode};
use hikasa_hex::DisplayFilter as HexFilter;
use hikasa_picture::DiffMode;
use hikasa_table::RowFilter;
use hikasa_text::TextComparison;
use serde::{Deserialize, Serialize};

use crate::error::ReportError;
use crate::folder::{FolderColumns, FolderLayout, FolderReportOptions};
use crate::hex::{HexReportLayout, HexReportOptions, HexSource};
use crate::links::PairLinks;
use crate::meta::ReportMeta;
use crate::options::{DisplayMode, HtmlStyle, OutputFormat, Wrap};
use crate::picture::{PictureReportLayout, PictureReportOptions, PictureSource};
use crate::table::{TableCells, TableLayout, TableReportOptions, TableSource};
use crate::text::{PatchStyle, TextLayout, TextReportOptions};

/// What a report reports on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    /// A text comparison.
    Text,
    /// A folder comparison.
    Folder,
    /// A table comparison (Table Compare §6.3; additive).
    Table,
    /// A hex comparison (M2 W12, §1.12.25; additive).
    Hex,
    /// A picture comparison (M2 W12, §1.12.25; additive).
    Picture,
}

impl ReportKind {
    /// `text`, `folder`, `table`, `hex`, `picture`.
    pub const fn as_str(self) -> &'static str {
        match self {
            ReportKind::Text => "text",
            ReportKind::Folder => "folder",
            ReportKind::Table => "table",
            ReportKind::Hex => "hex",
            ReportKind::Picture => "picture",
        }
    }
}

/// The words of one request, as a script, the dialog or the CLI gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportWords {
    /// Script `layout:`, CLI `--layout`/`--report`.
    pub layout: Option<String>,
    /// `options:`: one word per element; an element holding commas is split as [`ReportWords::split_list`] does.
    pub options: Vec<String>,
    /// `output-options:`, the same way.
    pub output_options: Vec<String>,
    /// `title:`, `--title`.
    pub title: Option<String>,
}

impl ReportWords {
    /// Splits a comma-separated list of words (a script line's `options:`, the CLI's `--options`): items are trimmed,
    /// empty ones dropped. An item that starts with `html-custom:` takes the rest of the list, commas included, as its
    /// stylesheet spec, so put it last: [`ReportRequest::parse`] refuses such a spec when the rest holds `,` and a
    /// report word (`ReportError::Request`).
    pub fn split_list(list: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = list;
        loop {
            let trimmed = rest.trim_start();
            let lower = trimmed
                .get(..HTML_CUSTOM.len())
                .map(|head| head.to_ascii_lowercase().replace('_', "-"));
            if lower.as_deref() == Some(HTML_CUSTOM) {
                out.push(trimmed.trim_end().to_owned());
                return out;
            }
            match rest.split_once(',') {
                Some((item, after)) => {
                    let item = item.trim();
                    if !item.is_empty() {
                        out.push(item.to_owned());
                    }
                    rest = after;
                }
                None => {
                    let item = rest.trim();
                    if !item.is_empty() {
                        out.push(item.to_owned());
                    }
                    return out;
                }
            }
        }
    }
}

/// The prefix of a custom stylesheet word.
const HTML_CUSTOM: &str = "html-custom:";

/// What an adapter means by an absent word; the CLI's values are M1's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportDefaults {
    /// `report`: `side-by-side`; `text compare`: `unified`; `folder compare`: `list`.
    pub layout: &'static str,
    /// The display of text reports without a `display-*` word.
    pub text_display: DisplayMode,
    /// The display of folder reports without a `display-*` word (filter, folders, suppress filters, ignore
    /// unimportant).
    pub folder_display: DisplayOptions,
    /// Line numbers without `line-numbers`/`no-line-numbers`.
    pub line_numbers: bool,
    /// The lines of `display-context` and of the patch styles.
    pub context_lines: u32,
    /// The width of text side-by-side and interleaved.
    pub width: u16,
    /// The rows of table reports without a `display-*` word (additive, Table Compare §6.3).
    pub table_display: RowFilter,
    /// The rows of hex reports without a `display-*` word (`None`: `display-none`; additive, M2 W12).
    pub hex_display: Option<HexFilter>,
}

impl ReportDefaults {
    /// M1's defaults with `layout`: display all, the folder structure, line numbers, 3 context lines, width 160.
    pub fn new(layout: &'static str) -> ReportDefaults {
        ReportDefaults {
            layout,
            text_display: DisplayMode::All,
            folder_display: FolderReportOptions::default().display,
            line_numbers: true,
            context_lines: 3,
            width: 160,
            table_display: RowFilter::ALL,
            hex_display: Some(HexFilter::All),
        }
    }
}

bitflags::bitflags! {
    /// Strike-out of the differences of a side in the interleaved layout (`strikeout-left-diffs`,
    /// `strikeout-right-diffs`, D-197).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct StrikeOut: u8 {
        /// The left side's differences.
        const LEFT = 1;
        /// The right side's differences.
        const RIGHT = 2;
    }
}

/// The page orientation of a print block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// `print-portrait`.
    Portrait,
    /// `print-landscape`.
    Landscape,
}

/// The print block of an HTML report (D-210): the orientation and the print palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrintBlock {
    /// `@page { size: … }`.
    pub orientation: Orientation,
    /// The print palette is the monochrome one (`print-mono`).
    pub mono: bool,
}

/// Lines of one side (RPT-003 *Just selection* of a text report, D-209): 1-based, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineScope {
    /// The side whose line numbers these are.
    pub side: Side,
    /// The first line.
    pub first: u32,
    /// The last line.
    pub last: u32,
}

/// The part of a comparison a report shows (D-209). Set by the Service from `--lines`, `--select` and `--path`, never
/// from words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ReportScope {
    /// Everything.
    #[default]
    All,
    /// The rows from the first whose line on the side is at least `first` to the last whose line is at most `last`.
    TextLines(LineScope),
    /// The selected rows, everything below a selected folder, and their ancestors.
    FolderRows(Selection),
}

/// The text reports a folder report links to (D-200): their options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileLinks {
    /// A pair report's options: text side-by-side HTML, display all, line numbers, `wrap-character`, the folder
    /// report's palette.
    pub text: TextReportOptions,
}

/// The options of a request by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportBody {
    /// A text report.
    Text(TextReportOptions),
    /// A folder report, with its links to file reports (`include-file-links`).
    Folder {
        /// The folder report's options.
        options: FolderReportOptions,
        /// The pair reports, when linked.
        links: Option<FileLinks>,
    },
    /// A table report (additive, Table Compare §6.3).
    Table(TableReportOptions),
    /// A hex report (additive, M2 W12).
    Hex(HexReportOptions),
    /// A picture report (additive, M2 W12).
    Picture(PictureReportOptions),
}

/// One parsed report request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRequest {
    /// The kind.
    pub kind: ReportKind,
    /// The options.
    pub body: ReportBody,
    /// `html-custom:<css>` as written: a spec the Service reads (core does no I/O).
    pub css: Option<String>,
    /// `print-portrait`, `print-landscape`, or the printer target's.
    pub print: Option<PrintBlock>,
    /// `print-mono` (the print palette; colour without it). Kept apart from `print`, which only an orientation or the
    /// printer target makes (§1.7.20 W7a).
    pub print_mono: bool,
    /// `title:`, `--title`.
    pub title: Option<String>,
    /// Set by the Service, never from words.
    pub scope: ReportScope,
    /// Accepted words the layout does not use, canonical, in slot order: one warning `report_option_unused`.
    pub unused: Vec<String>,
}

/// The target a request is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Standard output; `ansi` on a terminal that takes colours.
    Stdout {
        /// Colours.
        ansi: bool,
    },
    /// A file.
    File,
    /// The clipboard.
    Clipboard,
    /// The printer (an HTML file opened for printing).
    Printer,
    /// The browser.
    Browser,
}

/// The input of [`render_report`].
#[derive(Clone, Copy)]
pub enum ReportInput<'a> {
    /// A text comparison.
    Text(&'a TextComparison),
    /// A folder comparison.
    Folder(&'a FolderComparison),
    /// A table comparison with its sides (additive, Table Compare §6.3).
    Table(&'a TableSource<'a>),
    /// A hex comparison with its sides (additive, M2 W12).
    Hex(&'a HexSource<'a>),
    /// A picture comparison with its sides (additive, M2 W12).
    Picture(&'a PictureSource<'a>),
}

impl ReportInput<'_> {
    /// The kind of report this input makes.
    pub const fn kind(&self) -> ReportKind {
        match self {
            ReportInput::Text(_) => ReportKind::Text,
            ReportInput::Folder(_) => ReportKind::Folder,
            ReportInput::Table(_) => ReportKind::Table,
            ReportInput::Hex(_) => ReportKind::Hex,
            ReportInput::Picture(_) => ReportKind::Picture,
        }
    }
}

// ---- slots -------------------------------------------------------------------------------------------------------

/// The slots of the vocabulary, in the table's order (§1.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Display,
    Ignore,
    Numbers,
    Strike,
    Patch,
    Columns,
    Links,
    Palette,
    PrintPalette,
    Orientation,
    Wrap,
    Cells,
    Context,
    FullSize,
    Mode,
    Checkerboard,
}

impl Slot {
    const ALL: [Slot; 16] = [
        Slot::Display,
        Slot::Ignore,
        Slot::Numbers,
        Slot::Strike,
        Slot::Patch,
        Slot::Columns,
        Slot::Links,
        Slot::Palette,
        Slot::PrintPalette,
        Slot::Orientation,
        Slot::Wrap,
        Slot::Cells,
        Slot::Context,
        Slot::FullSize,
        Slot::Mode,
        Slot::Checkerboard,
    ];

    const fn name(self) -> &'static str {
        match self {
            Slot::Display => "display",
            Slot::Ignore => "ignore",
            Slot::Numbers => "line numbers",
            Slot::Strike => "strike-out",
            Slot::Patch => "patch style",
            Slot::Columns => "columns",
            Slot::Links => "links",
            Slot::Palette => "palette",
            Slot::PrintPalette => "print palette",
            Slot::Orientation => "orientation",
            Slot::Wrap => "wrap",
            Slot::Cells => "cells",
            Slot::Context => "context",
            Slot::FullSize => "image size",
            Slot::Mode => "difference mode",
            Slot::Checkerboard => "checkerboard",
        }
    }

    /// An output word (`output-options:`), else an option word.
    const fn output(self) -> bool {
        matches!(
            self,
            Slot::Palette | Slot::PrintPalette | Slot::Orientation | Slot::Wrap
        )
    }
}

/// The screen palette.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Palette {
    Text,
    Html { mono: bool },
    Custom(String),
}

/// The value a word gives its slot.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    TextDisplay(DisplayMode),
    /// A filter, and `display-context` (which also sets the folder structure).
    FolderDisplay(FolderDisplayFilter, bool),
    Ignore,
    Numbers(bool),
    Strike(StrikeOut),
    Patch(PatchStyle),
    /// Empty: `column-none`.
    Columns(FolderColumns),
    Links,
    Palette(Palette),
    PrintMono(bool),
    Orientation(Orientation),
    Wrap(Wrap),
    /// A table display (Table Compare).
    TableDisplay(RowFilter),
    /// The cells of a table JSON report.
    Cells(TableCells),
    /// A hex display (`None`: `display-none`).
    HexDisplay(Option<HexFilter>),
    /// `context-<n>` of a hex report.
    Context(u8),
    /// `full-size` of a picture report.
    FullSize,
    /// `mode-*` of a picture report.
    PictureMode(DiffMode),
    /// `checkerboard` of a picture report.
    Checkerboard,
}

/// The words a report request takes (the `expected` of an unknown word).
const VOCABULARY: &str = "a layout word, display-*, ignore-unimportant, line-numbers, no-line-numbers, \
strikeout-left-diffs, strikeout-right-diffs, patch-*, column-*, include-file-links, html-color, html-mono, \
html-custom:<css>, text, print-color, print-mono, print-portrait, print-landscape, wrap-*, row-numbers, \
no-row-numbers, cells-all, cells-differing, context-<n>, full-size, mode-tolerance, mode-range, mode-blend, \
checkerboard";

/// A word normalised for matching: trimmed, ASCII lower case, `_` read as `-`.
fn normal(word: &str) -> String {
    word.trim().to_ascii_lowercase().replace('_', "-")
}

fn unknown(word: &str) -> ReportError {
    ReportError::invalid("report option", word, VOCABULARY)
}

fn wrong_kind(kind: ReportKind, word: &str, owner: ReportKind) -> ReportError {
    use ReportKind::{Folder, Hex, Picture, Table, Text};
    let what = match kind {
        Text => "text report option",
        Folder => "folder report option",
        Table => "table report option",
        Hex => "hex report option",
        Picture => "picture report option",
    };
    let expected = match (kind, owner) {
        (Hex | Picture, Text) => "a word of this kind of report (this one is for text reports)",
        (Hex | Picture, Folder) => "a word of this kind of report (this one is for folder reports)",
        (Hex | Picture, Table) => "a word of this kind of report (this one is for table reports)",
        (_, Hex) => "a word of this kind of report (this one is for hex reports)",
        (_, Picture) => "a word of this kind of report (this one is for picture reports)",
        (Text, Table) => "a word of text reports (this one is for table reports)",
        (Text, _) => "a word of text reports (this one is for folder reports)",
        (Folder, Table) => "a word of folder reports (this one is for table reports)",
        (Folder, _) => "a word of folder reports (this one is for text reports)",
        (Table, Folder) => "a word of table reports (this one is for folder reports)",
        (Table, _) => "a word of table reports (this one is for text reports)",
    };
    ReportError::invalid(what, word, expected)
}

/// The slot and value of one option or output word.
fn classify(word: &str, kind: ReportKind, lines: u32) -> Result<(Slot, Value), ReportError> {
    let w = normal(word);
    let trimmed = word.trim();
    if let Some(css) = w.strip_prefix(HTML_CUSTOM) {
        let css = trimmed.get(HTML_CUSTOM.len()..).unwrap_or(css).trim();
        if css.is_empty() {
            return Err(ReportError::invalid(
                "report option",
                word,
                "html-custom:<css> with the stylesheet's path",
            ));
        }
        // `html-custom:` takes the rest of a list (a path may hold commas), so a report word after it would be
        // swallowed into the path: refused, never guessed.
        if let Some(swallowed) = css.split(',').skip(1).map(str::trim).find(|w| {
            !w.is_empty()
                && [
                    ReportKind::Text,
                    ReportKind::Folder,
                    ReportKind::Table,
                    ReportKind::Hex,
                    ReportKind::Picture,
                ]
                .into_iter()
                .any(|k| classify(w, k, lines).is_ok())
        }) {
            return Err(ReportError::Request {
                message: format!(
                    "`{HTML_CUSTOM}{css}` takes the rest of the list, so the report word `{swallowed}` would be \
                     part of the stylesheet's path: put `{HTML_CUSTOM}` last in the list"
                ),
            });
        }
        return Ok((
            Slot::Palette,
            Value::Palette(Palette::Custom(css.to_owned())),
        ));
    }
    let text_only = |slot, value| match kind {
        ReportKind::Text => Ok((slot, value)),
        _ => Err(wrong_kind(kind, word, ReportKind::Text)),
    };
    let folder_only = |slot, value| match kind {
        ReportKind::Folder => Ok((slot, value)),
        _ => Err(wrong_kind(kind, word, ReportKind::Folder)),
    };
    let table_only = |slot, value| match kind {
        ReportKind::Table => Ok((slot, value)),
        _ => Err(wrong_kind(kind, word, ReportKind::Table)),
    };
    let hex_only = |slot, value| match kind {
        ReportKind::Hex => Ok((slot, value)),
        _ => Err(wrong_kind(kind, word, ReportKind::Hex)),
    };
    let picture_only = |slot, value| match kind {
        ReportKind::Picture => Ok((slot, value)),
        _ => Err(wrong_kind(kind, word, ReportKind::Picture)),
    };
    let rows = |slot, value| match kind {
        ReportKind::Text | ReportKind::Table | ReportKind::Hex => Ok((slot, value)),
        ReportKind::Folder | ReportKind::Picture => Err(wrong_kind(kind, word, ReportKind::Text)),
    };
    match w.as_str() {
        "ignore-unimportant" => return Ok((Slot::Ignore, Value::Ignore)),
        "line-numbers" | "row-numbers" => return rows(Slot::Numbers, Value::Numbers(true)),
        "no-line-numbers" | "no-row-numbers" => {
            return rows(Slot::Numbers, Value::Numbers(false));
        }
        "cells-all" => return table_only(Slot::Cells, Value::Cells(TableCells::All)),
        "cells-differing" | "cells-differences" => {
            return table_only(Slot::Cells, Value::Cells(TableCells::Differing));
        }
        "strikeout-left-diffs" => return text_only(Slot::Strike, Value::Strike(StrikeOut::LEFT)),
        "strikeout-right-diffs" => {
            return text_only(Slot::Strike, Value::Strike(StrikeOut::RIGHT));
        }
        "include-file-links" => return folder_only(Slot::Links, Value::Links),
        "full-size" => return picture_only(Slot::FullSize, Value::FullSize),
        "checkerboard" => return picture_only(Slot::Checkerboard, Value::Checkerboard),
        "print-color" | "print-colour" => return Ok((Slot::PrintPalette, Value::PrintMono(false))),
        "print-mono" => return Ok((Slot::PrintPalette, Value::PrintMono(true))),
        "print-portrait" => {
            return Ok((Slot::Orientation, Value::Orientation(Orientation::Portrait)));
        }
        "print-landscape" => {
            return Ok((
                Slot::Orientation,
                Value::Orientation(Orientation::Landscape),
            ));
        }
        // M1's spellings of the palette (`OutputFormat::parse`); `ansi` is plain text here: colours come from the
        // terminal and `--no-color`, never from a word.
        "text" | "plain" | "ansi" | "html" | "html-color" | "html-colour" | "html-mono" => {
            let palette = match OutputFormat::parse(&w)? {
                OutputFormat::Text { .. } => Palette::Text,
                OutputFormat::Html { mono } => Palette::Html { mono },
            };
            return Ok((Slot::Palette, Value::Palette(palette)));
        }
        _ => {}
    }
    if let Some(rest) = w.strip_prefix("mode-") {
        let mode = DiffMode::parse(rest).map_err(|_| unknown(word))?;
        return picture_only(Slot::Mode, Value::PictureMode(mode));
    }
    if let Some(rest) = w.strip_prefix("context-")
        && let Ok(n) = rest.parse::<u8>()
        && n <= 64
        && rest.bytes().all(|b| b.is_ascii_digit())
    {
        return hex_only(Slot::Context, Value::Context(n));
    }
    if let Some(rest) = w.strip_prefix("display-") {
        return match kind {
            ReportKind::Hex => match rest {
                "all" => Ok((Slot::Display, Value::HexDisplay(Some(HexFilter::All)))),
                "differences" | "mismatches" => Ok((
                    Slot::Display,
                    Value::HexDisplay(Some(HexFilter::Differences)),
                )),
                "same" | "matches" => Ok((Slot::Display, Value::HexDisplay(Some(HexFilter::Same)))),
                "none" => Ok((Slot::Display, Value::HexDisplay(None))),
                _ => Err(unknown(word)),
            },
            ReportKind::Picture => Err(wrong_kind(kind, word, ReportKind::Text)),
            ReportKind::Text => {
                if rest == "none" {
                    return Ok((Slot::Display, Value::TextDisplay(DisplayMode::None)));
                }
                match DisplayMode::parse(&w, lines) {
                    Ok(mode) => Ok((Slot::Display, Value::TextDisplay(mode))),
                    Err(_) if FolderDisplayFilter::parse(&w).is_ok() => {
                        Err(wrong_kind(kind, word, ReportKind::Folder))
                    }
                    Err(_) => Err(unknown(word)),
                }
            }
            ReportKind::Table => match rest {
                "all" => Ok((Slot::Display, Value::TableDisplay(RowFilter::ALL))),
                "mismatches" | "differences" => {
                    Ok((Slot::Display, Value::TableDisplay(RowFilter::DIFFERENCES)))
                }
                "matches" | "same" => Ok((Slot::Display, Value::TableDisplay(RowFilter::SAME))),
                "none" => Ok((Slot::Display, Value::TableDisplay(RowFilter::NONE))),
                "context" => Err(wrong_kind(kind, word, ReportKind::Text)),
                _ if FolderDisplayFilter::parse(&w).is_ok() => {
                    Err(wrong_kind(kind, word, ReportKind::Folder))
                }
                _ => Err(unknown(word)),
            },
            ReportKind::Folder => {
                if rest == "context" {
                    return Ok((
                        Slot::Display,
                        Value::FolderDisplay(FolderDisplayFilter::Differences, true),
                    ));
                }
                FolderDisplayFilter::parse(&w)
                    .map(|filter| (Slot::Display, Value::FolderDisplay(filter, false)))
                    .map_err(|_| unknown(word))
            }
        };
    }
    if w.starts_with("patch-") {
        let style = PatchStyle::parse(&w, lines).map_err(|_| unknown(word))?;
        return text_only(Slot::Patch, Value::Patch(style));
    }
    if let Some(rest) = w.strip_prefix("column-") {
        if matches!(rest, "vcs" | "revision" | "version") {
            return Err(ReportError::Refused {
                message: "the version control columns (column-vcs, column-revision, column-version) are not available",
            });
        }
        let columns = if rest == "none" {
            FolderColumns::empty()
        } else {
            let columns = FolderColumns::parse_list(&w).map_err(|_| unknown(word))?;
            if columns.is_empty() {
                return Err(unknown(word));
            }
            columns
        };
        return folder_only(Slot::Columns, Value::Columns(columns));
    }
    if w.starts_with("wrap-") {
        let wrap = Wrap::parse(&w).map_err(|_| unknown(word))?;
        return text_only(Slot::Wrap, Value::Wrap(wrap));
    }
    Err(unknown(word))
}

/// The canonical words of a value (the first spelling of the table of §1.7.4).
fn canonical(value: &Value) -> Vec<String> {
    let one = |w: &str| vec![w.to_owned()];
    match value {
        Value::TextDisplay(mode) => one(match mode {
            DisplayMode::All => "display-all",
            DisplayMode::Mismatches => "display-mismatches",
            DisplayMode::Context { .. } => "display-context",
            DisplayMode::Matches => "display-matches",
            DisplayMode::None => "display-none",
        }),
        Value::FolderDisplay(_, true) => one("display-context"),
        Value::FolderDisplay(filter, false) => folder_display_word(*filter)
            .map(|w| vec![w.to_owned()])
            .unwrap_or_default(),
        Value::Ignore => one("ignore-unimportant"),
        Value::Numbers(true) => one("line-numbers"),
        Value::Numbers(false) => one("no-line-numbers"),
        Value::Strike(strike) => {
            let mut out = Vec::new();
            if strike.contains(StrikeOut::LEFT) {
                out.push("strikeout-left-diffs".to_owned());
            }
            if strike.contains(StrikeOut::RIGHT) {
                out.push("strikeout-right-diffs".to_owned());
            }
            out
        }
        Value::Patch(style) => one(match style {
            PatchStyle::Normal => "patch-normal",
            PatchStyle::Context { .. } => "patch-context",
            PatchStyle::Unified { .. } => "patch-unified",
        }),
        Value::Columns(columns) if columns.is_empty() => one("column-none"),
        Value::Columns(columns) => COLUMN_WORDS
            .iter()
            .filter(|(c, _)| columns.contains(*c))
            .map(|(_, w)| (*w).to_owned())
            .collect(),
        Value::Links => one("include-file-links"),
        Value::Palette(Palette::Text) => one("text"),
        Value::Palette(Palette::Html { mono: false }) => one("html-color"),
        Value::Palette(Palette::Html { mono: true }) => one("html-mono"),
        Value::Palette(Palette::Custom(css)) => vec![format!("{HTML_CUSTOM}{css}")],
        Value::PrintMono(false) => one("print-color"),
        Value::PrintMono(true) => one("print-mono"),
        Value::Orientation(Orientation::Portrait) => one("print-portrait"),
        Value::Orientation(Orientation::Landscape) => one("print-landscape"),
        Value::Wrap(wrap) => vec![format!("wrap-{}", wrap.as_str())],
        Value::TableDisplay(filter) => match crate::table::display_word(*filter) {
            "custom" => Vec::new(),
            word => vec![format!("display-{word}")],
        },
        Value::Cells(cells) => vec![format!("cells-{}", cells.as_str())],
        Value::HexDisplay(display) => {
            vec![format!("display-{}", crate::hex::display_word(*display))]
        }
        Value::Context(n) => vec![format!("context-{n}")],
        Value::FullSize => one("full-size"),
        Value::PictureMode(mode) => vec![format!("mode-{}", mode.as_str())],
        Value::Checkerboard => one("checkerboard"),
    }
}

/// The columns in the table's order with their words.
const COLUMN_WORDS: [(FolderColumns, &str); 7] = [
    (FolderColumns::SIZE, "column-size"),
    (FolderColumns::MODIFIED, "column-timestamp"),
    (FolderColumns::ATTRIBUTES, "column-attributes"),
    (FolderColumns::CRC, "column-crc"),
    (FolderColumns::OWNER, "column-owner"),
    (FolderColumns::GROUP, "column-group"),
    (FolderColumns::STATUS, "column-status"),
];

/// The `display-*` word of a folder filter (`None` for a toggle set, which has no word).
fn folder_display_word(filter: FolderDisplayFilter) -> Option<&'static str> {
    use FolderDisplayFilter as F;
    Some(match filter {
        F::All => "display-all",
        F::Differences => "display-mismatches",
        F::Same => "display-matches",
        F::NoOrphans => "display-no-orphans",
        F::DifferencesNoOrphans => "display-mismatches-no-orphans",
        F::Orphans => "display-orphans",
        F::LeftNewer => "display-left-newer",
        F::RightNewer => "display-right-newer",
        F::LeftNewerAndLeftOrphans => "display-left-newer-orphans",
        F::RightNewerAndRightOrphans => "display-right-newer-orphans",
        F::LeftOrphans => "display-left-orphans",
        F::RightOrphans => "display-right-orphans",
        F::None => "display-none",
        F::Toggles { .. } => return None,
    })
}

/// True when a folder display reads back as `display-context` (the differences in the folder structure): the word
/// [`ReportRequest::words`] gives it, and the folder XML's `mode/@display` `context`.
pub(crate) fn folder_display_is_context(d: &DisplayOptions) -> bool {
    d.filter == FolderDisplayFilter::Differences && d.folders == FolderMode::CompareStructure
}

/// Adds `value` (given as `word`) to its slot: strike-outs and columns accumulate, anything else conflicts with a
/// different value.
fn merge(
    slots: &mut BTreeMap<Slot, (Value, String)>,
    slot: Slot,
    value: Value,
    word: &str,
) -> Result<(), ReportError> {
    let Some((old, first)) = slots.get_mut(&slot) else {
        slots.insert(slot, (value, word.trim().to_owned()));
        return Ok(());
    };
    let conflict = |first: &str| ReportError::Conflict {
        slot: slot.name(),
        first: first.to_owned(),
        second: word.trim().to_owned(),
    };
    match (&mut *old, value) {
        (Value::Strike(a), Value::Strike(b)) => *a |= b,
        (Value::Columns(a), Value::Columns(b)) => {
            if a.is_empty() != b.is_empty() {
                return Err(conflict(first));
            }
            *a |= b;
        }
        (a, b) if *a == b => {}
        _ => return Err(conflict(first)),
    }
    Ok(())
}

// ---- the parser's tables (M2 W13c, D-367) ------------------------------------------------------------------------

/// The canonical layout words of text reports.
const TEXT_LAYOUT_WORDS: [&str; 7] = [
    "unified",
    "side-by-side",
    "summary",
    "interleaved",
    "patch",
    "statistics",
    "xml",
];
/// The canonical layout words of folder reports.
const FOLDER_LAYOUT_WORDS: [&str; 5] = ["list", "side-by-side", "summary", "xml", "json"];
/// The canonical layout words of table reports.
const TABLE_LAYOUT_WORDS: [&str; 5] = ["summary", "side-by-side", "interleaved", "xml", "json"];
/// The canonical layout words of hex reports.
const HEX_LAYOUT_WORDS: [&str; 3] = ["summary", "side-by-side", "interleaved"];
/// The canonical layout words of picture reports.
const PICTURE_LAYOUT_WORDS: [&str; 2] = ["summary", "side-by-side"];

/// The canonical option and output words every kind takes (`html-custom:` is the prefix of a stylesheet word).
const COMMON_WORDS: [&str; 9] = [
    "ignore-unimportant",
    "text",
    "html-color",
    "html-mono",
    HTML_CUSTOM,
    "print-color",
    "print-mono",
    "print-portrait",
    "print-landscape",
];
/// The canonical option and output words of text reports beyond [`COMMON_WORDS`].
const TEXT_WORDS: [&str; 15] = [
    "display-all",
    "display-mismatches",
    "display-context",
    "display-matches",
    "display-none",
    "line-numbers",
    "no-line-numbers",
    "strikeout-left-diffs",
    "strikeout-right-diffs",
    "patch-normal",
    "patch-context",
    "patch-unified",
    "wrap-none",
    "wrap-character",
    "wrap-word",
];
/// The canonical option words of folder reports beyond [`COMMON_WORDS`].
const FOLDER_WORDS: [&str; 23] = [
    "display-all",
    "display-mismatches",
    "display-matches",
    "display-no-orphans",
    "display-mismatches-no-orphans",
    "display-orphans",
    "display-left-newer",
    "display-right-newer",
    "display-left-newer-orphans",
    "display-right-newer-orphans",
    "display-left-orphans",
    "display-right-orphans",
    "display-none",
    "display-context",
    "column-size",
    "column-timestamp",
    "column-attributes",
    "column-crc",
    "column-owner",
    "column-group",
    "column-status",
    "column-none",
    "include-file-links",
];
/// The canonical option words of table reports beyond [`COMMON_WORDS`].
const TABLE_WORDS: [&str; 8] = [
    "display-all",
    "display-mismatches",
    "display-matches",
    "display-none",
    "line-numbers",
    "no-line-numbers",
    "cells-all",
    "cells-differing",
];
/// The canonical option words of hex reports beyond [`COMMON_WORDS`] and [`CONTEXT_WORDS`].
const HEX_WORDS: [&str; 6] = [
    "display-all",
    "display-differences",
    "display-same",
    "display-none",
    "line-numbers",
    "no-line-numbers",
];
/// `context-<n>` of hex reports, 0…64.
pub(crate) const CONTEXT_WORDS: [&str; 65] = [
    "context-0",
    "context-1",
    "context-2",
    "context-3",
    "context-4",
    "context-5",
    "context-6",
    "context-7",
    "context-8",
    "context-9",
    "context-10",
    "context-11",
    "context-12",
    "context-13",
    "context-14",
    "context-15",
    "context-16",
    "context-17",
    "context-18",
    "context-19",
    "context-20",
    "context-21",
    "context-22",
    "context-23",
    "context-24",
    "context-25",
    "context-26",
    "context-27",
    "context-28",
    "context-29",
    "context-30",
    "context-31",
    "context-32",
    "context-33",
    "context-34",
    "context-35",
    "context-36",
    "context-37",
    "context-38",
    "context-39",
    "context-40",
    "context-41",
    "context-42",
    "context-43",
    "context-44",
    "context-45",
    "context-46",
    "context-47",
    "context-48",
    "context-49",
    "context-50",
    "context-51",
    "context-52",
    "context-53",
    "context-54",
    "context-55",
    "context-56",
    "context-57",
    "context-58",
    "context-59",
    "context-60",
    "context-61",
    "context-62",
    "context-63",
    "context-64",
];
/// The canonical option words of picture reports beyond [`COMMON_WORDS`].
const PICTURE_WORDS: [&str; 5] = [
    "full-size",
    "checkerboard",
    "mode-tolerance",
    "mode-range",
    "mode-blend",
];

/// The canonical layout words [`ReportRequest::parse`] accepts for `kind` (the parser's table: a layout the parser
/// reads and this table does not list is refused; D-367's completeness test reads it).
pub const fn layout_words(kind: ReportKind) -> &'static [&'static str] {
    match kind {
        ReportKind::Text => &TEXT_LAYOUT_WORDS,
        ReportKind::Folder => &FOLDER_LAYOUT_WORDS,
        ReportKind::Table => &TABLE_LAYOUT_WORDS,
        ReportKind::Hex => &HEX_LAYOUT_WORDS,
        ReportKind::Picture => &PICTURE_LAYOUT_WORDS,
    }
}

/// The canonical option and output words [`ReportRequest::parse`] accepts for `kind`, in the table's order (every
/// spelling of a word reads as one of these; `html-custom:` stands for a stylesheet word). A word the parser reads
/// and this table does not list is refused (D-367).
pub fn option_words(kind: ReportKind) -> Vec<&'static str> {
    let own: &[&str] = match kind {
        ReportKind::Text => &TEXT_WORDS,
        ReportKind::Folder => &FOLDER_WORDS,
        ReportKind::Table => &TABLE_WORDS,
        ReportKind::Hex => &HEX_WORDS,
        ReportKind::Picture => &PICTURE_WORDS,
    };
    let context: &[&str] = if kind == ReportKind::Hex {
        &CONTEXT_WORDS
    } else {
        &[]
    };
    COMMON_WORDS
        .iter()
        .chain(own)
        .chain(context)
        .copied()
        .collect()
}

/// True when the canonical word `word` is in `kind`'s table.
fn listed(kind: ReportKind, word: &str) -> bool {
    #[cfg(test)]
    if tests::PLANTED.with(|p| p.get() == Some(word)) {
        return false;
    }
    word.starts_with(HTML_CUSTOM) || option_words(kind).contains(&word)
}

// ---- layouts -----------------------------------------------------------------------------------------------------

/// A layout of either kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Text(TextLayout),
    Folder(FolderLayout),
    Table(TableLayout),
    Hex(HexReportLayout),
    Picture(PictureReportLayout),
}

const TEXT_LAYOUTS: &str = "side-by-side, summary, interleaved, patch, statistics, xml, unified";
const FOLDER_LAYOUTS: &str = "side-by-side, summary, xml, list, json";

/// Parses a layout word of `kind`.
fn parse_layout(kind: ReportKind, word: &str, lines: u32) -> Result<Layout, ReportError> {
    let w = normal(word);
    match kind {
        ReportKind::Text => {
            let layout = match w.as_str() {
                "interleaved" => TextLayout::Interleaved,
                "xml" => TextLayout::Xml,
                _ => match TextLayout::parse(&w, PatchStyle::Unified { lines }, lines) {
                    Ok(layout) => layout,
                    Err(_) if FolderLayout::parse(&w).is_ok() => {
                        return Err(ReportError::invalid(
                            "text report layout",
                            word,
                            "a text layout (this one is for folder reports)",
                        ));
                    }
                    Err(_) => {
                        return Err(ReportError::invalid(
                            "text report layout",
                            word,
                            TEXT_LAYOUTS,
                        ));
                    }
                },
            };
            Ok(Layout::Text(layout))
        }
        ReportKind::Folder => {
            let layout = match w.as_str() {
                "xml" => FolderLayout::Xml,
                _ => match FolderLayout::parse(&w) {
                    Ok(layout) => layout,
                    Err(_)
                        if w == "interleaved"
                            || TextLayout::parse(&w, PatchStyle::Normal, lines).is_ok() =>
                    {
                        return Err(ReportError::invalid(
                            "folder report layout",
                            word,
                            "a folder layout (this one is for text reports)",
                        ));
                    }
                    Err(_) => {
                        return Err(ReportError::invalid(
                            "folder report layout",
                            word,
                            FOLDER_LAYOUTS,
                        ));
                    }
                },
            };
            Ok(Layout::Folder(layout))
        }
        ReportKind::Table => match TableLayout::parse(&w) {
            Ok(layout) => Ok(Layout::Table(layout)),
            Err(_) => Err(
                if TextLayout::parse(&w, PatchStyle::Normal, lines).is_ok() {
                    ReportError::invalid(
                        "table report layout",
                        word,
                        "a table layout (this one is for text reports)",
                    )
                } else if FolderLayout::parse(&w).is_ok() {
                    ReportError::invalid(
                        "table report layout",
                        word,
                        "a table layout (this one is for folder reports)",
                    )
                } else {
                    ReportError::invalid("table report layout", word, crate::table::TABLE_LAYOUTS)
                },
            ),
        },
        ReportKind::Hex => HexReportLayout::parse(&w).map(Layout::Hex),
        ReportKind::Picture => PictureReportLayout::parse(&w).map(Layout::Picture),
    }
}

impl Layout {
    /// The canonical layout word.
    const fn word(self) -> &'static str {
        match self {
            Layout::Text(TextLayout::AnnotatedUnified { .. }) => "unified",
            Layout::Text(layout) => layout.name(),
            Layout::Folder(layout) => layout.name(),
            Layout::Table(layout) => layout.name(),
            Layout::Hex(layout) => layout.name(),
            Layout::Picture(layout) => layout.name(),
        }
    }

    /// The layout has an HTML form.
    const fn html(self) -> bool {
        matches!(
            self,
            Layout::Text(TextLayout::SideBySide | TextLayout::Summary | TextLayout::Interleaved)
                | Layout::Folder(FolderLayout::SideBySide | FolderLayout::Summary)
                | Layout::Table(
                    TableLayout::Summary | TableLayout::SideBySide | TableLayout::Interleaved
                )
                | Layout::Hex(_)
                | Layout::Picture(_)
        )
    }

    /// The layout has a plain-text form (the word `text`).
    const fn text(self) -> bool {
        !matches!(
            self,
            Layout::Text(TextLayout::Xml)
                | Layout::Folder(FolderLayout::Xml | FolderLayout::Json)
                | Layout::Table(TableLayout::Xml | TableLayout::Json)
                | Layout::Picture(PictureReportLayout::SideBySide)
        )
    }

    /// The layout has rows (a scope applies).
    const fn rows(self) -> bool {
        matches!(
            self,
            Layout::Text(TextLayout::SideBySide | TextLayout::Interleaved | TextLayout::Xml)
                | Layout::Folder(FolderLayout::List | FolderLayout::SideBySide | FolderLayout::Xml)
        )
    }

    /// The layout uses `slot` (the "Used by layouts" column of §1.7.4).
    const fn uses(self, slot: Slot) -> bool {
        use FolderLayout as F;
        use HexReportLayout as H;
        use PictureReportLayout as P;
        use TableLayout as B;
        use TextLayout as T;
        match slot {
            Slot::Ignore => !matches!(self, Layout::Hex(_)),
            Slot::Context => matches!(self, Layout::Hex(H::SideBySide | H::Interleaved)),
            Slot::FullSize | Slot::Mode | Slot::Checkerboard => {
                matches!(self, Layout::Picture(P::SideBySide))
            }
            Slot::Palette | Slot::PrintPalette | Slot::Orientation => self.html(),
            Slot::Display => matches!(
                self,
                Layout::Text(T::SideBySide | T::Interleaved | T::Xml)
                    | Layout::Folder(F::List | F::SideBySide | F::Xml | F::Json)
                    | Layout::Table(B::SideBySide | B::Interleaved | B::Xml | B::Json)
                    | Layout::Hex(H::SideBySide | H::Interleaved)
            ),
            Slot::Numbers => matches!(
                self,
                Layout::Text(T::SideBySide | T::Interleaved)
                    | Layout::Table(B::SideBySide | B::Interleaved)
                    | Layout::Hex(H::SideBySide | H::Interleaved)
            ),
            Slot::Wrap => matches!(self, Layout::Text(T::SideBySide | T::Interleaved)),
            Slot::Cells => matches!(self, Layout::Table(B::Json)),
            Slot::Strike => matches!(self, Layout::Text(T::Interleaved)),
            Slot::Patch => matches!(self, Layout::Text(T::Patch(_))),
            Slot::Columns | Slot::Links => matches!(self, Layout::Folder(F::SideBySide)),
        }
    }
}

// ---- the request -------------------------------------------------------------------------------------------------

impl ReportRequest {
    /// Parses the words of one request of `kind` (§1.7.4): unknown words and words of the other kind are
    /// `InvalidOption`, two different words of one slot `Conflict`, an HTML palette with a layout without an HTML form
    /// `Unsupported`; words the layout does not use go to [`ReportRequest::unused`]. Runs before anything is compared.
    pub fn parse(
        kind: ReportKind,
        words: &ReportWords,
        defaults: &ReportDefaults,
    ) -> Result<ReportRequest, ReportError> {
        let lines = defaults.context_lines;
        let layout_word = words.layout.as_deref().unwrap_or(defaults.layout);
        let layout = parse_layout(kind, layout_word, lines)?;
        if !layout_words(kind).contains(&layout.word()) {
            return Err(ReportError::invalid(
                "report layout",
                layout_word,
                match kind {
                    ReportKind::Text => TEXT_LAYOUTS,
                    ReportKind::Folder => FOLDER_LAYOUTS,
                    ReportKind::Table => crate::table::TABLE_LAYOUTS,
                    ReportKind::Hex => crate::hex::HEX_LAYOUTS,
                    ReportKind::Picture => crate::picture::PICTURE_LAYOUTS,
                },
            ));
        }
        let mut slots: BTreeMap<Slot, (Value, String)> = BTreeMap::new();
        for element in words.options.iter().chain(&words.output_options) {
            for word in ReportWords::split_list(element) {
                let (slot, value) = classify(&word, kind, lines)?;
                if canonical(&value).iter().any(|c| !listed(kind, c)) {
                    return Err(unknown(&word));
                }
                merge(&mut slots, slot, value, &word)?;
            }
        }
        // The palette: an HTML word needs an HTML form, `text` a text form.
        if let Some((Value::Palette(palette), _)) = slots.get(&Slot::Palette) {
            let refused = match palette {
                Palette::Text => (!layout.text()).then_some("text"),
                Palette::Html { .. } | Palette::Custom(_) => (!layout.html()).then_some("html"),
            };
            if let Some(format) = refused {
                return Err(ReportError::Unsupported {
                    layout: layout.word(),
                    format,
                });
            }
        }
        let mut unused = Vec::new();
        for (slot, (value, _)) in &slots {
            if !layout.uses(*slot) {
                unused.extend(canonical(value));
            }
        }
        let get = |slot: Slot| slots.get(&slot).map(|(value, _)| value);
        let ignore = get(Slot::Ignore).is_some();
        let (format, css) = match get(Slot::Palette) {
            Some(Value::Palette(Palette::Html { mono })) => {
                (OutputFormat::Html { mono: *mono }, None)
            }
            Some(Value::Palette(Palette::Custom(css))) => {
                (OutputFormat::Html { mono: false }, Some(css.clone()))
            }
            _ => (OutputFormat::Text { ansi: false }, None),
        };
        let print_mono = matches!(get(Slot::PrintPalette), Some(Value::PrintMono(true)));
        let print = match get(Slot::Orientation) {
            Some(Value::Orientation(orientation)) => Some(PrintBlock {
                orientation: *orientation,
                mono: print_mono,
            }),
            _ => None,
        };
        let wrap = match get(Slot::Wrap) {
            Some(Value::Wrap(wrap)) => *wrap,
            _ => Wrap::Character,
        };
        let body = match layout {
            Layout::Text(layout) => {
                let layout = match (layout, get(Slot::Patch)) {
                    (TextLayout::Patch(_), Some(Value::Patch(style))) => TextLayout::Patch(*style),
                    (layout, _) => layout,
                };
                ReportBody::Text(TextReportOptions {
                    layout,
                    display: match get(Slot::Display) {
                        Some(Value::TextDisplay(mode)) => *mode,
                        _ => defaults.text_display,
                    },
                    ignore_unimportant: ignore,
                    line_numbers: match get(Slot::Numbers) {
                        Some(Value::Numbers(on)) => *on,
                        _ => defaults.line_numbers,
                    },
                    width: defaults.width,
                    wrap,
                    format,
                    strike: match get(Slot::Strike) {
                        Some(Value::Strike(strike)) => *strike,
                        _ => StrikeOut::empty(),
                    },
                })
            }
            Layout::Folder(layout) => {
                let mut display = defaults.folder_display;
                display.ignore_unimportant |= ignore;
                if let Some(Value::FolderDisplay(filter, context)) = get(Slot::Display) {
                    display.filter = *filter;
                    if *context {
                        display.folders = FolderMode::CompareStructure;
                    }
                }
                let options = FolderReportOptions {
                    layout,
                    display,
                    columns: match get(Slot::Columns) {
                        Some(Value::Columns(columns)) => *columns,
                        _ => FolderColumns::default(),
                    },
                    format,
                };
                let links = (layout == FolderLayout::SideBySide && get(Slot::Links).is_some())
                    .then(|| FileLinks {
                        text: pair_options(&options, defaults.width),
                    });
                ReportBody::Folder { options, links }
            }
            Layout::Table(layout) => ReportBody::Table(TableReportOptions {
                layout,
                display: match get(Slot::Display) {
                    Some(Value::TableDisplay(filter)) => *filter,
                    _ => defaults.table_display,
                },
                ignore_unimportant: ignore,
                row_numbers: match get(Slot::Numbers) {
                    Some(Value::Numbers(on)) => *on,
                    _ => defaults.line_numbers,
                },
                width: defaults.width,
                format,
                cells: match get(Slot::Cells) {
                    Some(Value::Cells(cells)) => *cells,
                    _ => TableCells::Differing,
                },
            }),
            Layout::Hex(layout) => ReportBody::Hex(HexReportOptions {
                layout,
                display: match get(Slot::Display) {
                    Some(Value::HexDisplay(display)) => *display,
                    _ => defaults.hex_display,
                },
                context: match get(Slot::Context) {
                    Some(Value::Context(n)) => *n,
                    _ => 0,
                },
                addresses: match get(Slot::Numbers) {
                    Some(Value::Numbers(on)) => *on,
                    _ => defaults.line_numbers,
                },
                format,
            }),
            Layout::Picture(layout) => ReportBody::Picture(PictureReportOptions {
                layout,
                ignore_unimportant: ignore,
                full_size: get(Slot::FullSize).is_some(),
                mode: match get(Slot::Mode) {
                    Some(Value::PictureMode(mode)) => Some(*mode),
                    _ => None,
                },
                checkerboard: get(Slot::Checkerboard).is_some(),
                format,
            }),
        };
        Ok(ReportRequest {
            kind,
            body,
            css,
            print,
            print_mono,
            title: words.title.clone(),
            scope: ReportScope::All,
            unused,
        })
    }

    /// The layout of the request.
    fn layout(&self) -> Layout {
        match &self.body {
            ReportBody::Text(opts) => Layout::Text(opts.layout),
            ReportBody::Folder { options, .. } => Layout::Folder(options.layout),
            ReportBody::Table(opts) => Layout::Table(opts.layout),
            ReportBody::Hex(opts) => Layout::Hex(opts.layout),
            ReportBody::Picture(opts) => Layout::Picture(opts.layout),
        }
    }

    /// The canonical layout word (`side-by-side`, `interleaved`, `unified`, …).
    pub fn layout_word(&self) -> &'static str {
        self.layout().word()
    }

    /// The output format of the main report.
    pub fn format(&self) -> OutputFormat {
        match &self.body {
            ReportBody::Text(opts) => opts.format,
            ReportBody::Folder { options, .. } => options.format,
            ReportBody::Table(opts) => opts.format,
            ReportBody::Hex(opts) => opts.format,
            ReportBody::Picture(opts) => opts.format,
        }
    }

    fn format_mut(&mut self) -> &mut OutputFormat {
        match &mut self.body {
            ReportBody::Text(opts) => &mut opts.format,
            ReportBody::Folder { options, .. } => &mut options.format,
            ReportBody::Table(opts) => &mut opts.format,
            ReportBody::Hex(opts) => &mut opts.format,
            ReportBody::Picture(opts) => &mut opts.format,
        }
    }

    /// The value of a used slot as a [`Value`], `None` when it has no word.
    fn used_value(&self, slot: Slot) -> Option<Value> {
        match (&self.body, slot) {
            (ReportBody::Text(opts), Slot::Display) => Some(Value::TextDisplay(opts.display)),
            (ReportBody::Folder { options, .. }, Slot::Display) => {
                let d = options.display;
                folder_display_word(d.filter)
                    .map(|_| Value::FolderDisplay(d.filter, folder_display_is_context(&d)))
            }
            (ReportBody::Table(opts), Slot::Display) => (crate::table::display_word(opts.display)
                != "custom")
                .then_some(Value::TableDisplay(opts.display)),
            (_, Slot::Ignore) => self.ignore_unimportant().then_some(Value::Ignore),
            (ReportBody::Text(opts), Slot::Numbers) => Some(Value::Numbers(opts.line_numbers)),
            (ReportBody::Table(opts), Slot::Numbers) => Some(Value::Numbers(opts.row_numbers)),
            (ReportBody::Table(opts), Slot::Cells) => Some(Value::Cells(opts.cells)),
            (ReportBody::Hex(opts), Slot::Display) => Some(Value::HexDisplay(opts.display)),
            (ReportBody::Hex(opts), Slot::Numbers) => Some(Value::Numbers(opts.addresses)),
            (ReportBody::Hex(opts), Slot::Context) => {
                (opts.context != 0).then_some(Value::Context(opts.context))
            }
            (ReportBody::Picture(opts), Slot::FullSize) => {
                opts.full_size.then_some(Value::FullSize)
            }
            (ReportBody::Picture(opts), Slot::Mode) => opts.mode.map(Value::PictureMode),
            (ReportBody::Picture(opts), Slot::Checkerboard) => {
                opts.checkerboard.then_some(Value::Checkerboard)
            }
            (ReportBody::Text(opts), Slot::Strike) => {
                (!opts.strike.is_empty()).then_some(Value::Strike(opts.strike))
            }
            (ReportBody::Text(opts), Slot::Patch) => match opts.layout {
                TextLayout::Patch(style) => Some(Value::Patch(style)),
                _ => None,
            },
            (ReportBody::Text(opts), Slot::Wrap) => {
                (opts.wrap != Wrap::Character).then_some(Value::Wrap(opts.wrap))
            }
            (ReportBody::Folder { options, .. }, Slot::Columns) => {
                Some(Value::Columns(options.columns))
            }
            (ReportBody::Folder { links, .. }, Slot::Links) => links.map(|_| Value::Links),
            (_, Slot::Palette) => match (&self.css, self.format()) {
                (Some(css), _) => Some(Value::Palette(Palette::Custom(css.clone()))),
                (None, OutputFormat::Html { mono }) => Some(Value::Palette(Palette::Html { mono })),
                (None, OutputFormat::Text { .. }) => None,
            },
            (_, Slot::PrintPalette) => self.print_mono.then_some(Value::PrintMono(true)),
            (_, Slot::Orientation) => self.print.map(|p| Value::Orientation(p.orientation)),
            _ => None,
        }
    }

    /// The canonical words of this request: the layout word, then the option words in the table's slot order, each
    /// in its first spelling, then the output words; `parse(words())` with the same defaults equals the request
    /// (P-RPTREQ). Words of slots the layout does not use come from [`ReportRequest::unused`].
    pub fn words(&self) -> ReportWords {
        let layout = self.layout();
        let mut words = ReportWords {
            layout: Some(layout.word().to_owned()),
            title: self.title.clone(),
            ..ReportWords::default()
        };
        for slot in Slot::ALL {
            let list: Vec<String> = if layout.uses(slot) {
                self.used_value(slot)
                    .map(|value| canonical(&value))
                    .unwrap_or_default()
            } else {
                self.unused
                    .iter()
                    .filter(|word| classify(word, self.kind, 0).is_ok_and(|(of, _)| of == slot))
                    .cloned()
                    .collect()
            };
            if slot.output() {
                words.output_options.extend(list);
            } else {
                words.options.extend(list);
            }
        }
        words
    }

    /// The `ignore_unimportant` of the comparison's request (§1.7.4: the word sets the comparison's too).
    pub fn ignore_unimportant(&self) -> bool {
        match &self.body {
            ReportBody::Text(opts) => opts.ignore_unimportant,
            ReportBody::Folder { options, .. } => options.display.ignore_unimportant,
            ReportBody::Table(opts) => opts.ignore_unimportant,
            ReportBody::Hex(_) => false,
            ReportBody::Picture(opts) => opts.ignore_unimportant,
        }
    }

    /// The warning `report_option_unused` "`<layout> does not use: <words>`", when a word was unused.
    pub fn unused_warning(&self) -> Option<Warning> {
        (!self.unused.is_empty()).then(|| Warning {
            code: WarningCode::ReportOptionUnused,
            message: format!(
                "{} does not use: {}",
                self.layout_word(),
                self.unused.join(", ")
            ),
            spec: None,
        })
    }

    /// Checks the request against a target (the matrices of §1.7.4): the printer and the browser take the layouts with
    /// an HTML form; links need folder side-by-side HTML (or the printer's, the browser's) on a file, the printer or
    /// the browser; a scope needs a row layout. Runs before anything is compared.
    pub fn check_target(&self, target: TargetKind) -> Result<(), ReportError> {
        let layout = self.layout();
        match target {
            TargetKind::Printer | TargetKind::Browser if !layout.html() => {
                return Err(ReportError::Unsupported {
                    layout: layout.word(),
                    format: if target == TargetKind::Printer {
                        "printer"
                    } else {
                        "browser"
                    },
                });
            }
            _ => {}
        }
        if let ReportBody::Folder {
            links: Some(_),
            options,
        } = &self.body
        {
            if !matches!(
                target,
                TargetKind::File | TargetKind::Printer | TargetKind::Browser
            ) {
                return Err(ReportError::Refused {
                    message: "file reports need a folder on disk",
                });
            }
            let html = matches!(options.format, OutputFormat::Html { .. })
                || matches!(target, TargetKind::Printer | TargetKind::Browser);
            if !html {
                return Err(ReportError::Refused {
                    message: "file reports need side-by-side HTML (html-color, html-mono or html-custom)",
                });
            }
        }
        self.check_scope()
    }

    /// Checks the scope: `TextLines` for a text request, `FolderRows` for a folder one, each with a row layout (text
    /// side-by-side, interleaved, XML; folder list, side-by-side, XML).
    pub fn check_scope(&self) -> Result<(), ReportError> {
        let kind = match &self.scope {
            ReportScope::All => return Ok(()),
            ReportScope::TextLines(lines) => {
                if lines.first == 0 || lines.first > lines.last {
                    return Err(ReportError::Request {
                        message: format!(
                            "the line range {}-{} is empty or starts at 0",
                            lines.first, lines.last
                        ),
                    });
                }
                if !matches!(lines.side, Side::Left | Side::Right) {
                    return Err(ReportError::Request {
                        message: "a line range is on the left or the right".to_owned(),
                    });
                }
                ReportKind::Text
            }
            ReportScope::FolderRows(_) => ReportKind::Folder,
        };
        if kind != self.kind {
            return Err(ReportError::Request {
                message: format!(
                    "a {} scope does not apply to a {} report",
                    kind.as_str(),
                    self.kind.as_str()
                ),
            });
        }
        if !self.layout().rows() {
            return Err(ReportError::Request {
                message: format!(
                    "{} reports the whole comparison: a scope needs side-by-side, interleaved, list or xml",
                    self.layout_word()
                ),
            });
        }
        Ok(())
    }

    /// Adapts the request to its target, after [`ReportRequest::check_target`]: ANSI colours for a terminal's stdout
    /// (a layout with a plain-text form and no HTML palette); the print palette and the print block (portrait by
    /// default) for the printer; the print palette for the browser. `html-color` and `html-mono` are unused there.
    pub fn apply_target(&mut self, target: TargetKind) {
        match target {
            TargetKind::Stdout { ansi: true } => {
                let colours = matches!(
                    self.layout(),
                    Layout::Text(
                        TextLayout::AnnotatedUnified { .. }
                            | TextLayout::SideBySide
                            | TextLayout::Summary
                            | TextLayout::Interleaved
                            | TextLayout::Patch(_)
                    ) | Layout::Folder(
                        FolderLayout::List | FolderLayout::SideBySide | FolderLayout::Summary
                    ) | Layout::Table(
                        TableLayout::Summary | TableLayout::SideBySide | TableLayout::Interleaved
                    ) | Layout::Hex(_)
                        | Layout::Picture(PictureReportLayout::Summary)
                );
                let format = self.format_mut();
                if colours && *format == (OutputFormat::Text { ansi: false }) {
                    *format = OutputFormat::Text { ansi: true };
                }
            }
            TargetKind::Printer | TargetKind::Browser => {
                if self.css.is_none()
                    && let OutputFormat::Html { mono } = self.format()
                {
                    let word = if mono { "html-mono" } else { "html-color" };
                    let at = self
                        .unused
                        .iter()
                        .position(|w| {
                            classify(w, self.kind, 0).is_ok_and(|(slot, _)| slot > Slot::Palette)
                        })
                        .unwrap_or(self.unused.len());
                    self.unused.insert(at, word.to_owned());
                }
                let mono = self.print_mono;
                *self.format_mut() = OutputFormat::Html { mono };
                if target == TargetKind::Printer {
                    let orientation = self.print.map_or(Orientation::Portrait, |p| p.orientation);
                    self.print = Some(PrintBlock { orientation, mono });
                }
                if let ReportBody::Folder {
                    links: Some(links), ..
                } = &mut self.body
                {
                    links.text.format = OutputFormat::Html { mono };
                }
            }
            TargetKind::Stdout { ansi: false } | TargetKind::File | TargetKind::Clipboard => {}
        }
    }

    /// The `; scope: …` suffix of the header's `Mode:` line (empty without a scope).
    pub(crate) fn scope_suffix(&self) -> String {
        match &self.scope {
            ReportScope::All => String::new(),
            ReportScope::TextLines(lines) => format!(
                "; scope: lines {}\u{2013}{} of the {}",
                lines.first,
                lines.last,
                lines.side.as_str()
            ),
            ReportScope::FolderRows(selection) => {
                let n = selection.len();
                format!(
                    "; scope: {n} selected {}",
                    if n == 1 { "row" } else { "rows" }
                )
            }
        }
    }
}

/// A pair report's options (§1.7.8): text side-by-side HTML, display all, line numbers, `wrap-character`, the folder
/// report's palette and ignore-unimportant.
fn pair_options(folder: &FolderReportOptions, width: u16) -> TextReportOptions {
    TextReportOptions {
        layout: TextLayout::SideBySide,
        display: DisplayMode::All,
        ignore_unimportant: folder.display.ignore_unimportant,
        line_numbers: true,
        width,
        wrap: Wrap::Character,
        format: match folder.format {
            OutputFormat::Html { mono } => OutputFormat::Html { mono },
            OutputFormat::Text { .. } => OutputFormat::Html { mono: false },
        },
        strike: StrikeOut::empty(),
    }
}

/// Renders one request (the one entry point of a request; the M1 functions remain for their callers): checks the kind
/// and the scope, adds the scope to the header's `Mode:` line, takes the request's print block and HTML wrap into
/// `style`, and links the pair reports of a folder side-by-side HTML or XML report through `links` (§1.7.8). Returns
/// the warnings of the request (`report_option_unused`) and of the rendering.
pub fn render_report(
    input: ReportInput<'_>,
    req: &ReportRequest,
    meta: &ReportMeta,
    style: &HtmlStyle,
    links: Option<&dyn PairLinks>,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    if input.kind() != req.kind {
        return Err(ReportError::Request {
            message: format!(
                "a {} report of a {} comparison",
                req.kind.as_str(),
                input.kind().as_str()
            ),
        });
    }
    req.check_scope()?;
    let mut meta = meta.clone();
    let suffix = req.scope_suffix();
    if !suffix.is_empty()
        && let Some(mode) = &mut meta.mode
    {
        mode.push_str(&suffix);
    }
    let mut style = style.clone();
    if style.print.is_none() {
        style.print = req.print;
    }
    let mut warnings: Vec<Warning> = req.unused_warning().into_iter().collect();
    match (input, &req.body) {
        (ReportInput::Text(cmp), ReportBody::Text(opts)) => {
            // The wrap rule is the side-by-side and interleaved HTML's (D-210); elsewhere the word is unused and
            // changes no byte.
            if style.wrap.is_none()
                && matches!(
                    opts.layout,
                    TextLayout::SideBySide | TextLayout::Interleaved
                )
            {
                style.wrap = Some(opts.wrap);
            }
            let scope = match &req.scope {
                ReportScope::TextLines(lines) => Some(*lines),
                _ => None,
            };
            warnings.extend(crate::text::render(cmp, opts, &meta, &style, scope, out)?);
        }
        (ReportInput::Folder(cmp), ReportBody::Folder { options, .. }) => {
            let scope = match &req.scope {
                ReportScope::FolderRows(selection) => Some(selection),
                _ => None,
            };
            crate::folder::render(cmp, options, &meta, &style, scope, links, out)?;
        }
        (ReportInput::Table(src), ReportBody::Table(opts)) => {
            warnings.extend(crate::table::render_table_report(
                src, opts, &meta, &style, out,
            )?);
        }
        (ReportInput::Hex(src), ReportBody::Hex(opts)) => {
            warnings.extend(crate::hex::render_hex_report(
                src, opts, &meta, &style, out,
            )?);
        }
        (ReportInput::Picture(src), ReportBody::Picture(opts)) => {
            warnings.extend(crate::picture::render_picture_report(
                src, opts, &meta, &style, out,
            )?);
        }
        _ => {
            return Err(ReportError::Request {
                message: "the request's options do not match its kind".to_owned(),
            });
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// A word the tables leave out in this test thread (a parser word added without its table).
        pub(super) static PLANTED: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
    }

    /// The gate of the parser's tables (W13c review finding 14): a word the parser accepts but its kind's table leaves
    /// out is refused as unknown, every spelling of it; listed again, it parses.
    #[test]
    fn a_parsed_word_missing_from_its_table_is_refused() {
        let parse = |option: &str| {
            let words = ReportWords {
                options: vec![option.to_owned()],
                ..ReportWords::default()
            };
            ReportRequest::parse(
                ReportKind::Text,
                &words,
                &ReportDefaults::new("side-by-side"),
            )
        };
        for word in ["line-numbers", "display-all", "ignore-unimportant"] {
            assert!(parse(word).is_ok(), "{word}");
            PLANTED.with(|p| p.set(Some(word)));
            let refused = parse(word);
            PLANTED.with(|p| p.set(None));
            assert_eq!(
                refused.err().map(|e| e.to_string()),
                Some(unknown(word).to_string()),
                "{word}"
            );
        }
    }

    #[test]
    fn split_lists() {
        assert_eq!(
            ReportWords::split_list(" display-all , ,line-numbers,"),
            ["display-all", "line-numbers"]
        );
        assert_eq!(
            ReportWords::split_list("wrap-word,html-custom:/a/b,c.css"),
            ["wrap-word", "html-custom:/a/b,c.css"]
        );
        assert_eq!(
            ReportWords::split_list("HTML_CUSTOM:x.css"),
            ["HTML_CUSTOM:x.css"]
        );
        assert!(ReportWords::split_list("").is_empty());
    }

    #[test]
    fn canonical_words_classify_back() {
        for kind in [ReportKind::Text, ReportKind::Folder] {
            let words = [
                "display-all",
                "ignore-unimportant",
                "print-portrait",
                "html-color",
                "html-custom:a.css",
            ];
            for word in words {
                let (slot, value) = classify(word, kind, 3).unwrap();
                assert_eq!(canonical(&value), [word], "{word}");
                assert!(Slot::ALL.contains(&slot));
            }
        }
    }
}
