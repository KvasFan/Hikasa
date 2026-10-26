//! Text reports (13-reports, M1 subset): the annotated unified diff (CLI default), patches (normal, context,
//! unified; TP-003), side-by-side text and HTML (RPT-010), the summary and the statistics CSV (D-18).
//!
//! Every renderer reads the [`TextComparison`] only (RPT-011) and writes to the caller's writer; the output is a
//! deterministic function of `(cmp, opts, meta)` (INV-RPT-1).

mod context;
mod html;
mod normal;
mod patch;
mod sbs;
mod stats;
mod summary;
mod unified;

use std::io::{BufWriter, Write};

use hikasa_core::{Pair, Side, Warning};
use hikasa_text::{TextComparison, TextDocument};

use crate::error::ReportError;
use crate::meta::{Header, ReportMeta};
use crate::options::{DisplayMode, HtmlStyle, OutputFormat, Wrap};

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

    /// Stable name: `annotated-unified`, `side-by-side`, `summary`, `patch`, `statistics` (the `layout` of
    /// [`ReportError::Unsupported`]).
    pub const fn name(self) -> &'static str {
        match self {
            TextLayout::AnnotatedUnified { .. } => "annotated-unified",
            TextLayout::SideBySide => "side-by-side",
            TextLayout::Summary => "summary",
            TextLayout::Patch(_) => "patch",
            TextLayout::Statistics => "statistics",
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
    /// Output format (HTML allowed for SideBySide and Summary only).
    pub format: OutputFormat,
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
            sbs::write(&mut w, cmp, opts, meta, ansi)?;
            Vec::new()
        }
        (TextLayout::SideBySide, OutputFormat::Html { mono }) => {
            html::write(&mut w, cmp, opts, meta, mono, style.custom_css.as_deref())?;
            Vec::new()
        }
        (TextLayout::Summary, OutputFormat::Text { ansi }) => {
            summary::write_text(&mut w, cmp, opts, meta, ansi)?;
            Vec::new()
        }
        (TextLayout::Summary, OutputFormat::Html { mono }) => {
            summary::write_html(&mut w, cmp, opts, meta, mono, style.custom_css.as_deref())?;
            Vec::new()
        }
        (TextLayout::Statistics, OutputFormat::Text { .. }) => {
            stats::write(&mut w, cmp, meta)?;
            Vec::new()
        }
        (
            layout @ (TextLayout::AnnotatedUnified { .. }
            | TextLayout::Patch(_)
            | TextLayout::Statistics),
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
