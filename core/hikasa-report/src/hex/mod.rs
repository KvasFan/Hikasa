//! Hex reports (§1.12.25, D-348; RPT-001, HX-008): the summary (P0, plan A10: the statistics, the alignment, the first
//! `max_sections` sections with previews) and the side-by-side and interleaved layouts (HX-008's P1 layouts on the rows
//! model), as plain text (with ANSI colours on a terminal) and as self-contained HTML; the human output's section lines
//! ([`section_line`], [`hex_summary_line`]) and the JSON documents of `hex compare`, `hex dump` and `hex find`
//! ([`crate::json`]) read the same model.
//!
//! Every renderer reads the [`HexSource`] only (RPT-011) and writes to the caller's writer; the output is a
//! deterministic function of its inputs (INV-RPT-1). The bytes are read through the documents' page caches.

mod rows;
mod summary;

use std::fmt::Write as _;
use std::io::{BufWriter, Write};
use std::ops::Range;

use hikasa_core::{Classify, ContentKind, ContentResult, Pair, Side, Warning};
use hikasa_hex::{
    AlignMode, DisplayFilter, HexAlignOptions, HexAlignment, HexDocument, HexError, HexLayout,
    HexLoad, HexStats, Radix, Section, SectionKind,
};

use crate::error::ReportError;
use crate::json::FormatJson;
use crate::meta::{Header, ReportMeta};
use crate::options::{HtmlStyle, OutputFormat};

/// Default title of hex reports.
pub(crate) const HEX_TITLE: &str = "Hex Compare";

/// The layout words of hex reports (the `expected` of an unknown one).
pub(crate) const HEX_LAYOUTS: &str = "summary, side-by-side, interleaved";

/// Layouts of a hex report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HexReportLayout {
    /// The verdict, the statistics, the alignment and the first sections with previews.
    Summary,
    /// Each row: the left address, bytes and characters, then the right ones.
    SideBySide,
    /// A row without differences once; a differing row's left row, then its right row.
    Interleaved,
}

impl HexReportLayout {
    /// Parses `summary`, `side-by-side` (`sbs`), `interleaved`; ASCII case-insensitive, `_` equals `-`.
    ///
    /// # Errors
    ///
    /// [`ReportError::InvalidOption`].
    pub fn parse(word: &str) -> Result<HexReportLayout, ReportError> {
        Ok(
            match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "summary" => HexReportLayout::Summary,
                "side-by-side" | "sbs" => HexReportLayout::SideBySide,
                "interleaved" => HexReportLayout::Interleaved,
                _ => return Err(ReportError::invalid("hex report layout", word, HEX_LAYOUTS)),
            },
        )
    }

    /// `summary`, `side-by-side`, `interleaved`.
    pub const fn name(self) -> &'static str {
        match self {
            HexReportLayout::Summary => "summary",
            HexReportLayout::SideBySide => "side-by-side",
            HexReportLayout::Interleaved => "interleaved",
        }
    }
}

/// Options of a hex report (the report words; the byte layout comes from the [`HexSource`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexReportOptions {
    /// Layout.
    pub layout: HexReportLayout,
    /// The rows shown (`None`: `display-none`, the header and the totals only).
    pub display: Option<DisplayFilter>,
    /// Context rows around differing rows under `differences` (`context-<n>`, 0…64).
    pub context: u8,
    /// Byte addresses in side-by-side and interleaved (`line-numbers`, default on).
    pub addresses: bool,
    /// Output format (every layout has a text and an HTML form).
    pub format: OutputFormat,
}

impl Default for HexReportOptions {
    /// Side-by-side, every row, no context, addresses, plain text.
    fn default() -> HexReportOptions {
        HexReportOptions {
            layout: HexReportLayout::SideBySide,
            display: Some(DisplayFilter::All),
            context: 0,
            addresses: true,
            format: OutputFormat::Text { ansi: false },
        }
    }
}

/// The word of a hex display (`all`, `differences`, `same`, `none`).
pub(crate) const fn display_word(display: Option<DisplayFilter>) -> &'static str {
    match display {
        Some(DisplayFilter::All) => "all",
        Some(DisplayFilter::Differences) => "differences",
        Some(DisplayFilter::Same) => "same",
        None => "none",
    }
}

/// What the caller knows of one side beyond its document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexSide {
    /// The canonical spec, shown in headers.
    pub spec: String,
    /// Size in bytes (of the converted output when converted).
    pub size: u64,
    /// Modification time (JSON: RFC 3339 UTC).
    pub mtime: Option<jiff::Timestamp>,
    /// The Hikasa file format of the Hex kind chosen for the side, if any.
    pub format: Option<FormatJson>,
    /// The side was read through its format's load conversion.
    pub converted: bool,
    /// How the document holds its bytes.
    pub load: HexLoad,
}

/// The input of a hex report and of `hikasa.hex_compare.v1`.
#[derive(Clone, Copy)]
pub struct HexSource<'a> {
    /// The two documents.
    pub documents: Pair<&'a HexDocument>,
    /// Their alignment.
    pub alignment: &'a HexAlignment,
    /// The alignment options asked for.
    pub options: &'a HexAlignOptions,
    /// The two sides.
    pub sides: Pair<&'a HexSide>,
    /// The verdict (D-331).
    pub kind: ContentKind,
    /// An input changed during the job (`input_changed`): the result may be stale.
    pub stale: bool,
    /// The byte layout of the rows.
    pub layout: HexLayout,
    /// Sections listed (0: all of the stored ones).
    pub max_sections: u64,
    /// Preview bytes of each side of a section (0…4,096).
    pub preview: u32,
    /// The comparison's warnings.
    pub warnings: &'a [Warning],
}

impl HexSource<'_> {
    /// The verdict as a content result.
    pub fn verdict(&self) -> ContentResult {
        hex_verdict(self.kind)
    }

    /// The side `side`.
    pub(crate) fn side(&self, side: Side) -> &HexSide {
        match side {
            Side::Left => self.sides.left,
            _ => self.sides.right,
        }
    }

    /// The sections listed: the first `max_sections` stored ones (all with 0).
    pub fn listed_sections(&self) -> impl Iterator<Item = Section> + '_ {
        let n = match self.max_sections {
            0 => usize::MAX,
            n => usize::try_from(n).unwrap_or(usize::MAX),
        };
        self.alignment
            .sections
            .iter(&self.alignment.segments)
            .take(n)
    }

    /// Digits of the section addresses: the larger file's hex width (at least 8).
    pub fn address_width(&self) -> usize {
        let last = self
            .documents
            .left
            .len()
            .max(self.documents.right.len())
            .saturating_sub(1);
        ((64 - last.leading_zeros()).div_ceil(4) as usize).max(8)
    }
}

/// The content result of a hex verdict (D-331: never `unimportant_only`).
pub fn hex_verdict(kind: ContentKind) -> ContentResult {
    match kind {
        ContentKind::BinarySame => ContentResult::BinarySame,
        ContentKind::RulesSame => ContentResult::RulesSame,
        ContentKind::BinaryDifferent => ContentResult::BinaryDifferent,
        _ => ContentResult::ImportantDifferences,
    }
}

/// A hex read error as a report error (its kind and message kept).
pub(crate) fn input_error(e: &HexError) -> ReportError {
    ReportError::Input {
        kind: e.kind(),
        message: e.to_string(),
    }
}

/// Up to `n` bytes of `range` of `doc`.
pub(crate) fn preview_bytes(
    doc: &HexDocument,
    range: &Range<u64>,
    n: u32,
) -> Result<Vec<u8>, HexError> {
    let len = (range.end - range.start).min(u64::from(n)) as usize;
    let mut buf = vec![0u8; len];
    let mut at = 0;
    while at < len {
        let got = doc.read(range.start + at as u64, &mut buf[at..])?;
        if got == 0 {
            return Err(HexError::Truncated {
                spec: doc.display().to_owned(),
            });
        }
        at += got;
    }
    Ok(buf)
}

/// Lower-case hex, two digits a byte, no separator (the JSON's form).
pub fn hex_text(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Lower-case hex, two digits a byte, separated by spaces (the human output's form).
pub fn hex_spaced(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The word of a section kind in the human output (`changed`, `left-only`, `right-only`, `mixed`).
pub(crate) const fn kind_words(kind: SectionKind) -> &'static str {
    match kind {
        SectionKind::Changed => "changed",
        SectionKind::LeftOnly => "left-only",
        SectionKind::RightOnly => "right-only",
        SectionKind::Mixed => "mixed",
    }
}

/// One section as the human output and the summary write it: `0x00001F40 L 8  R 8  changed   de ad | 00 00`. The
/// address is the left offset (the right one when the section holds no left byte); when both sides hold bytes at
/// different offsets the right one follows after `/`. An empty side's length and preview are `—`; a preview longer
/// than `preview` bytes ends with `…`.
///
/// # Errors
///
/// A read error of a preview.
pub fn section_line(src: &HexSource<'_>, s: &Section) -> Result<String, HexError> {
    let w = src.address_width();
    let addr = |o: u64| format!("0x{o:0w$X}");
    let has = Pair::new(!s.left.is_empty(), !s.right.is_empty());
    let mut line = match (has.left, has.right) {
        (true, true) if s.left.start != s.right.start => {
            format!("{}/{}", addr(s.left.start), addr(s.right.start))
        }
        (true, _) => addr(s.left.start),
        (false, true) => addr(s.right.start),
        (false, false) => addr(s.left.start),
    };
    let len = |r: &Range<u64>| {
        if r.is_empty() {
            "\u{2014}".to_owned()
        } else {
            (r.end - r.start).to_string()
        }
    };
    let _ = write!(
        line,
        " L {}  R {}  {:<10}",
        len(&s.left),
        len(&s.right),
        kind_words(s.kind)
    );
    let preview = |doc: &HexDocument, r: &Range<u64>| -> Result<String, HexError> {
        if r.is_empty() {
            return Ok("\u{2014}".to_owned());
        }
        let bytes = preview_bytes(doc, r, src.preview)?;
        let mut text = hex_spaced(&bytes);
        if (r.end - r.start) > u64::from(src.preview) {
            text.push_str(" \u{2026}");
        }
        Ok(text)
    };
    if src.preview > 0 {
        let _ = write!(
            line,
            " {} | {}",
            preview(src.documents.left, &s.left)?,
            preview(src.documents.right, &s.right)?
        );
    }
    Ok(line.trim_end().to_owned())
}

/// `hex compare: N sections, X bytes differ, Y only left, Z only right (alignment fast)`.
pub fn hex_summary_line(stats: &HexStats, mode: AlignMode) -> String {
    format!(
        "hex compare: {}, {} {}, {} only left, {} only right (alignment {})",
        crate::options::plural(stats.sections, "section"),
        stats.differing_bytes,
        if stats.differing_bytes == 1 {
            "byte differs"
        } else {
            "bytes differ"
        },
        stats.left_only_bytes,
        stats.right_only_bytes,
        mode.as_str()
    )
}

/// `<spec> (<n> bytes, <load>[, converted])`.
fn side_description(src: &HexSource<'_>, side: Side) -> String {
    let s = src.side(side);
    format!(
        "{} ({}, {}{})",
        s.spec,
        crate::options::plural(s.size, "byte"),
        s.load.as_str(),
        if s.converted { ", converted" } else { "" }
    )
}

/// The `Mode:` line: the alignment, the display and the context.
fn mode_line(src: &HexSource<'_>, opts: &HexReportOptions) -> String {
    let mut out = format!("alignment: {}", src.alignment.mode.as_str());
    if src.alignment.degraded {
        out.push_str(" (degraded)");
    }
    if opts.layout != HexReportLayout::Summary {
        let _ = write!(out, "; display: {}", display_word(opts.display));
        if opts.display == Some(DisplayFilter::Differences) {
            let _ = write!(out, "; context: {}", opts.context);
        }
    }
    out
}

/// The `Rules:` line: the block, skew, window and budget of the alignment.
fn rules_line(src: &HexSource<'_>) -> String {
    let o = src.options;
    format!(
        "block {} bytes, skew {} bytes, window {} bytes, budget {} MiB",
        o.block, o.skew, o.window, o.budget_mib
    )
}

/// The RPT-004 header of a hex report.
pub(crate) fn header(src: &HexSource<'_>, opts: &HexReportOptions, meta: &ReportMeta) -> Header {
    let mut meta = meta.clone();
    if meta.mode.is_none() {
        meta.mode = Some(mode_line(src, opts));
    }
    if meta.filters.is_none() {
        meta.filters = Some(rules_line(src));
    }
    Header::new(
        &meta,
        HEX_TITLE,
        Pair::new(
            side_description(src, Side::Left),
            side_description(src, Side::Right),
        ),
        Vec::new(),
        src.warnings,
    )
}

/// The address text of a row: hex upper case zero-padded to `width`, decimal right-aligned.
pub(crate) fn address(offset: u64, radix: Radix, width: usize) -> String {
    match radix {
        Radix::Hex => format!("{offset:0width$X}"),
        Radix::Decimal => format!("{offset:>width$}"),
    }
}

/// Renders a hex report into `out`. Returns the warnings of the rendering itself (none today).
///
/// # Errors
///
/// A write error; [`ReportError::Input`] for a read error of a document.
pub fn render_hex_report(
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    let mut w = BufWriter::with_capacity(64 * 1024, out);
    match (opts.layout, opts.format) {
        (HexReportLayout::Summary, OutputFormat::Text { ansi }) => {
            summary::write_text(&mut w, src, opts, meta, ansi)?;
        }
        (HexReportLayout::Summary, OutputFormat::Html { mono }) => {
            summary::write_html(&mut w, src, opts, meta, mono, style)?;
        }
        (layout, OutputFormat::Text { ansi }) => {
            rows::write_text(&mut w, src, opts, meta, ansi, layout)?;
        }
        (layout, OutputFormat::Html { mono }) => {
            rows::write_html(&mut w, src, opts, meta, mono, style, layout)?;
        }
    }
    w.flush()?;
    Ok(Vec::new())
}
