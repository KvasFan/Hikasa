//! Text reports (13-reports, M1 subset): the annotated unified diff (CLI default), patches (normal, context,
//! unified; TP-003), side-by-side text and HTML (RPT-010), the summary and the statistics CSV (D-18).
//!
//! Every renderer reads the [`TextComparison`] only (RPT-011) and writes to the caller's writer; the output is a
//! deterministic function of `(cmp, opts, meta)` (INV-RPT-1).

mod context;
mod html;
mod interleaved;
mod normal;
mod patch;
mod sbs;
mod stats;
mod summary;
mod unified;
mod xml;

use std::io::{BufWriter, Write};
use std::ops::Range;

use hikasa_core::{Pair, Side, Warning};
use hikasa_text::{FilteredView, TextComparison, TextDocument, ViewItem, ViewOptions};

use crate::error::ReportError;
use crate::meta::{Header, ReportMeta};
use crate::options::{DisplayMode, HtmlStyle, OutputFormat, Wrap};
use crate::request::{LineScope, StrikeOut};

/// Default title of text reports.
pub(crate) const TEXT_TITLE: &str = "Text Compare";

/// Patch styles (TP-003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchStyle {
    /// Normal (`a`/`d`/`c` commands).
    Normal,
    /// Context diff with N context lines.
    Context {
        /// Context lines around every change.
        lines: u32,
    },
    /// Unified diff with N context lines.
    Unified {
        /// Context lines around every change.
        lines: u32,
    },
}

impl PatchStyle {
    /// Parses `normal`, `context`, `unified` (also with the script prefix `patch-`, SCR-001); `lines` is the
    /// context of the context and unified styles (additive).
    pub fn parse(word: &str, lines: u32) -> Result<PatchStyle, ReportError> {
        let w = word.trim().to_ascii_lowercase().replace('_', "-");
        let w = w.strip_prefix("patch-").unwrap_or(&w);
        Ok(match w {
            "normal" => PatchStyle::Normal,
            "context" => PatchStyle::Context { lines },
            "unified" => PatchStyle::Unified { lines },
            _ => {
                return Err(ReportError::invalid(
                    "patch style",
                    word,
                    "normal, context, unified",
                ));
            }
        })
    }

    /// `normal`, `context`, `unified`.
    pub const fn as_str(self) -> &'static str {
        match self {
            PatchStyle::Normal => "normal",
            PatchStyle::Context { .. } => "context",
            PatchStyle::Unified { .. } => "unified",
        }
    }
}

/// Text report layouts (13-reports, M1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextLayout {
    /// CLI default: unified diff whose hunk headers carry ` important` / ` unimportant` (still a valid patch).
    AnnotatedUnified {
        /// Context lines around every change.
        lines: u32,
    },
    /// Side-by-side.
    SideBySide,
    /// Summary counts.
    Summary,
    /// Plain patch.
    Patch(PatchStyle),
    /// Statistics CSV (one header row + one data row).
    Statistics,
    /// One column of lines in row order (M2 W7, D-197): a changed row's left line, then its right line.
    Interleaved,
    /// The `hikasa-report` XML dump (M2 W7, D-198).
    Xml,
}

impl TextLayout {
    /// Parses a `--report` word (additive): `unified` (the annotated unified diff), `side-by-side`, `summary`,
    /// `patch` (in `patch` style), `statistics`; ASCII case-insensitive, `_` equals `-`. `lines` is the context
    /// of `unified`.
    pub fn parse(word: &str, patch: PatchStyle, lines: u32) -> Result<TextLayout, ReportError> {
        Ok(
            match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "unified" | "annotated-unified" | "annotated" => {
                    TextLayout::AnnotatedUnified { lines }
                }
                "side-by-side" | "sbs" => TextLayout::SideBySide,
                "summary" => TextLayout::Summary,
                "patch" => TextLayout::Patch(patch),
                "statistics" | "stats" => TextLayout::Statistics,
                _ => {
                    return Err(ReportError::invalid(
                        "text report layout",
                        word,
                        "unified, side-by-side, summary, patch, statistics",
                    ));
                }
            },
        )
    }

    /// Stable name: `annotated-unified`, `side-by-side`, `summary`, `patch`, `statistics`, `interleaved`, `xml` (the
    /// `layout` of [`ReportError::Unsupported`]). [`TextLayout::parse`] keeps M1's words: `interleaved` and `xml` are
    /// read by [`crate::ReportRequest::parse`].
    pub const fn name(self) -> &'static str {
        match self {
            TextLayout::AnnotatedUnified { .. } => "annotated-unified",
            TextLayout::SideBySide => "side-by-side",
            TextLayout::Summary => "summary",
            TextLayout::Patch(_) => "patch",
            TextLayout::Statistics => "statistics",
            TextLayout::Interleaved => "interleaved",
            TextLayout::Xml => "xml",
        }
    }
}

/// Text report options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextReportOptions {
    /// Layout.
    pub layout: TextLayout,
    /// Display mode (side-by-side and summary; patches always use their own context).
    pub display: DisplayMode,
    /// Treat unimportant as same (TC-024).
    pub ignore_unimportant: bool,
    /// Include line numbers (side-by-side). Default true.
    pub line_numbers: bool,
    /// Total width of side-by-side text (columns). Default 160.
    pub width: u16,
    /// Wrapping (side-by-side text).
    pub wrap: Wrap,
    /// Output format (HTML allowed for SideBySide, Summary and Interleaved only).
    pub format: OutputFormat,
    /// Strike-out of a side's differences (Interleaved; M2 W7, D-197).
    pub strike: StrikeOut,
}

impl Default for TextReportOptions {
    /// The annotated unified diff with 3 context lines; display All; ignore-unimportant off; line numbers;
    /// width 160; character wrapping; plain text.
    fn default() -> TextReportOptions {
        TextReportOptions {
            layout: TextLayout::AnnotatedUnified { lines: 3 },
            display: DisplayMode::All,
            ignore_unimportant: false,
            line_numbers: true,
            width: 160,
            wrap: Wrap::Character,
            format: OutputFormat::Text { ansi: false },
            strike: StrikeOut::empty(),
        }
    }
}

/// Renders a text report into `out`.
pub fn render_text_report(
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    out: &mut dyn Write,
) -> Result<(), ReportError> {
    render_text_report_with(cmp, opts, meta, &HtmlStyle::default(), out).map(|_| ())
}

/// [`render_text_report`] with an [`HtmlStyle`] (custom CSS), returning the warnings of the rendering itself
/// (additive): `patch_lossy_eol` when a patch writes lone-CR endings or artificial (characters-per-line) breaks
/// as LF. The same warnings appear in the report header.
pub fn render_text_report_with(
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    render(cmp, opts, meta, style, None, out)
}

/// Renders a text report with a scope (`None`: every row).
pub(crate) fn render(
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    scope: Option<LineScope>,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    let mut w = BufWriter::with_capacity(64 * 1024, out);
    let warnings = match (opts.layout, opts.format) {
        (TextLayout::AnnotatedUnified { lines }, OutputFormat::Text { ansi }) => {
            unified::write(&mut w, cmp, opts, meta, lines, true, ansi)?
        }
        (TextLayout::Patch(PatchStyle::Unified { lines }), OutputFormat::Text { ansi }) => {
            unified::write(&mut w, cmp, opts, meta, lines, false, ansi)?
        }
        (TextLayout::Patch(PatchStyle::Context { lines }), OutputFormat::Text { ansi }) => {
            context::write(&mut w, cmp, opts, meta, lines, ansi)?
        }
        (TextLayout::Patch(PatchStyle::Normal), OutputFormat::Text { ansi }) => {
            normal::write(&mut w, cmp, opts, meta, ansi)?
        }
        (TextLayout::SideBySide, OutputFormat::Text { ansi }) => {
            let items = Items::new(cmp, opts, scope);
            sbs::write(&mut w, cmp, opts, meta, &items, ansi)?;
            Vec::new()
        }
        (TextLayout::SideBySide, OutputFormat::Html { mono }) => {
            let items = Items::new(cmp, opts, scope);
            html::write(&mut w, cmp, opts, meta, &items, mono, style)?;
            Vec::new()
        }
        (TextLayout::Interleaved, OutputFormat::Text { ansi }) => {
            let items = Items::new(cmp, opts, scope);
            interleaved::write_text(&mut w, cmp, opts, meta, &items, ansi)?;
            Vec::new()
        }
        (TextLayout::Interleaved, OutputFormat::Html { mono }) => {
            let items = Items::new(cmp, opts, scope);
            interleaved::write_html(&mut w, cmp, opts, meta, &items, mono, style)?;
            Vec::new()
        }
        (TextLayout::Xml, OutputFormat::Text { .. }) => {
            let items = Items::new(cmp, opts, scope);
            xml::write(&mut w, cmp, opts, meta, &items, scope)?;
            Vec::new()
        }
        (TextLayout::Summary, OutputFormat::Text { ansi }) => {
            summary::write_text(&mut w, cmp, opts, meta, ansi)?;
            Vec::new()
        }
        (TextLayout::Summary, OutputFormat::Html { mono }) => {
            summary::write_html(&mut w, cmp, opts, meta, mono, style)?;
            Vec::new()
        }
        (TextLayout::Statistics, OutputFormat::Text { .. }) => {
            stats::write(&mut w, cmp, meta)?;
            Vec::new()
        }
        (
            layout @ (TextLayout::AnnotatedUnified { .. }
            | TextLayout::Patch(_)
            | TextLayout::Statistics
            | TextLayout::Xml),
            format @ OutputFormat::Html { .. },
        ) => {
            return Err(ReportError::Unsupported {
                layout: layout.name(),
                format: format.name(),
            });
        }
    };
    w.flush()?;
    Ok(warnings)
}

/// The rows a row layout shows: the display mode's view, cut to the scope's rows. Streams: the view holds runs of
/// rows, never one entry per row.
pub(crate) struct Items {
    view: FilteredView,
    /// The scope's rows; `None` = every row.
    range: Option<Range<u32>>,
}

impl Items {
    /// The items of `opts.display` within `scope`.
    pub(crate) fn new(
        cmp: &TextComparison,
        opts: &TextReportOptions,
        scope: Option<LineScope>,
    ) -> Items {
        let view = cmp.view(&ViewOptions {
            filter: opts.display.text_filter(),
            ignore_unimportant: opts.ignore_unimportant,
            expanded: Vec::new(),
        });
        Items {
            view,
            range: scope.map(|scope| scope_rows(cmp, scope)),
        }
    }

    /// The items in order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = ViewItem> + '_ {
        let range = self.range.clone();
        self.view
            .iter()
            .filter_map(move |item| match (&range, item) {
                (None, item) => Some(item),
                (Some(range), ViewItem::Row { row }) => {
                    range.contains(&row.0).then_some(ViewItem::Row { row })
                }
                (Some(range), ViewItem::Gap { rows }) => {
                    let rows = rows.start.max(range.start)..rows.end.min(range.end);
                    (rows.start < rows.end).then_some(ViewItem::Gap { rows })
                }
            })
    }

    /// True when a scope holds no row: the report is the header and `0 rows`.
    pub(crate) fn nothing(&self) -> bool {
        self.range.as_ref().is_some_and(|r| r.start >= r.end)
    }
}

/// The text of an empty scope.
pub(crate) const NO_ROWS: &str = "0 rows";

/// The rows of a line scope (D-209): from the first row whose line on the side is at least `first` to the last whose
/// line is at most `last`, every row between them included. Empty when no line of the side is in range.
pub(crate) fn scope_rows(cmp: &TextComparison, scope: LineScope) -> Range<u32> {
    let doc = cmp.document(scope.side);
    let mut start = None;
    let mut end = 0u32;
    for (i, row) in cmp.rows(0..cmp.row_count()).enumerate() {
        let line = match scope.side {
            Side::Right => row.right,
            _ => row.left,
        };
        let Some(line) = line else { continue };
        let n = doc.physical_line(line);
        let i = i as u32;
        if n >= scope.first && start.is_none() {
            start = Some(i);
        }
        if n <= scope.last {
            end = i + 1;
        }
    }
    match start {
        Some(start) if start < end => start..end,
        _ => 0..0,
    }
}

/// The spec text of a document (`<memory>` for in-memory documents).
pub(crate) fn spec_text(doc: &TextDocument) -> String {
    doc.origin().display()
}

/// `"<spec> (<encoding>[ BOM], <eol>)"`: the `Left:`/`Right:` value of the header (§3.4).
pub(crate) fn side_description(doc: &TextDocument) -> String {
    let enc = doc.encoding();
    let bom = if enc.bom_len > 0 { " BOM" } else { "" };
    let binary = if doc.binary().is_binary {
        ", binary"
    } else {
        ""
    };
    format!(
        "{} ({}{bom}, {}{binary})",
        spec_text(doc),
        enc.encoding.name(),
        doc.eol().style().as_str()
    )
}

/// The RPT-004 header of a text report: the comparison's warnings plus `extra` (rendering) warnings.
pub(crate) fn header(cmp: &TextComparison, meta: &ReportMeta, extra: &[Warning]) -> Header {
    let mut warnings: Vec<Warning> = cmp.warnings().to_vec();
    warnings.extend_from_slice(extra);
    Header::new(
        meta,
        TEXT_TITLE,
        Pair::new(
            side_description(cmp.document(Side::Left)),
            side_description(cmp.document(Side::Right)),
        ),
        Vec::new(),
        &warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_and_style_words() {
        assert_eq!(
            PatchStyle::parse("patch-normal", 3).unwrap(),
            PatchStyle::Normal
        );
        assert_eq!(
            PatchStyle::parse("Context", 5).unwrap(),
            PatchStyle::Context { lines: 5 }
        );
        assert_eq!(
            PatchStyle::parse("unified", 0).unwrap(),
            PatchStyle::Unified { lines: 0 }
        );
        assert!(PatchStyle::parse("ed", 3).is_err());
        assert_eq!(PatchStyle::Unified { lines: 1 }.as_str(), "unified");
        let p = PatchStyle::Context { lines: 2 };
        assert_eq!(
            TextLayout::parse("unified", p, 4).unwrap(),
            TextLayout::AnnotatedUnified { lines: 4 }
        );
        assert_eq!(
            TextLayout::parse("side_by_side", p, 4).unwrap(),
            TextLayout::SideBySide
        );
        assert_eq!(
            TextLayout::parse("summary", p, 4).unwrap(),
            TextLayout::Summary
        );
        assert_eq!(
            TextLayout::parse("patch", p, 4).unwrap(),
            TextLayout::Patch(p)
        );
        assert_eq!(
            TextLayout::parse("STATISTICS", p, 4).unwrap(),
            TextLayout::Statistics
        );
        assert!(TextLayout::parse("xml", p, 4).is_err());
        assert_eq!(TextLayout::Patch(p).name(), "patch");
        let d = TextReportOptions::default();
        assert_eq!(d.width, 160);
        assert!(d.line_numbers);
        assert_eq!(d.wrap, Wrap::Character);
        assert_eq!(d.layout, TextLayout::AnnotatedUnified { lines: 3 });
    }
}
