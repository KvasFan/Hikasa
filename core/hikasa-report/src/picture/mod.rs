//! Picture reports (§1.12.25, D-348; RPT-001, PX-010): the summary (P0, plan A10: File Info of both sides, the
//! options, the statistics and the verdict; text and HTML) and the side-by-side layout (PX-010's P1 layout; HTML only,
//! self-contained: the left, right and difference images embedded as PNG data URIs, scaled down by a box filter to at
//! most 1,024 pixels on the long side unless `full-size`).
//!
//! Every renderer reads the [`PictureSource`] only (RPT-011); the images are rendered by the engine's
//! [`PictureComparison::render`] and `render_tile` with the theme's colours, so a report shows the pixels the
//! window shows.

use std::fmt::Write as _;
use std::io::{BufWriter, Write};

use hikasa_core::{Classify, ContentKind, ContentResult, Ctl, Pair, Side, Warning};
use hikasa_picture::{
    DiffMode, Pane, PictureColors, PictureComparison, PictureError, PictureInfo, PixelClass,
    PixelProbe, PngLevel, RgbaImage, TileRect, ToneMap, composite_checkerboard, encode_png,
};

use crate::error::ReportError;
use crate::escape;
use crate::json::FormatJson;
use crate::meta::{Header, ReportMeta};
use crate::options::{HtmlStyle, OutputFormat};
use crate::style::{self, Sgr, Sheet};

/// Default title of picture reports.
pub(crate) const PICTURE_TITLE: &str = "Picture Compare";

/// The layout words of picture reports (the `expected` of an unknown one).
pub(crate) const PICTURE_LAYOUTS: &str = "summary, side-by-side";

/// The long side of a side-by-side image without `full-size`.
pub const SCALED_MAX: u32 = 1024;

/// Layouts of a picture report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureReportLayout {
    /// File Info, options, statistics, verdict.
    Summary,
    /// The left, right and difference images (HTML only).
    SideBySide,
}

impl PictureReportLayout {
    /// Parses `summary`, `side-by-side` (`sbs`); ASCII case-insensitive, `_` equals `-`.
    ///
    /// # Errors
    ///
    /// [`ReportError::InvalidOption`].
    pub fn parse(word: &str) -> Result<PictureReportLayout, ReportError> {
        Ok(
            match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
                "summary" => PictureReportLayout::Summary,
                "side-by-side" | "sbs" => PictureReportLayout::SideBySide,
                _ => {
                    return Err(ReportError::invalid(
                        "picture report layout",
                        word,
                        PICTURE_LAYOUTS,
                    ));
                }
            },
        )
    }

    /// `summary`, `side-by-side`.
    pub const fn name(self) -> &'static str {
        match self {
            PictureReportLayout::Summary => "summary",
            PictureReportLayout::SideBySide => "side-by-side",
        }
    }
}

/// Options of a picture report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureReportOptions {
    /// Layout.
    pub layout: PictureReportLayout,
    /// `ignore-unimportant`: the verdict's exit reading (similar counts as same).
    pub ignore_unimportant: bool,
    /// `full-size`: side-by-side images at full resolution.
    pub full_size: bool,
    /// `mode-*`: the difference image's mode (`None`: the comparison's).
    pub mode: Option<DiffMode>,
    /// `checkerboard`: transparent pixels over the checkerboard.
    pub checkerboard: bool,
    /// Output format (the summary has both forms; side-by-side is HTML).
    pub format: OutputFormat,
}

impl Default for PictureReportOptions {
    /// Summary, plain text, scaled images, the comparison's mode, no checkerboard.
    fn default() -> PictureReportOptions {
        PictureReportOptions {
            layout: PictureReportLayout::Summary,
            ignore_unimportant: false,
            full_size: false,
            mode: None,
            checkerboard: false,
            format: OutputFormat::Text { ansi: false },
        }
    }
}

/// What the caller knows of one side beyond its picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureSide {
    /// The canonical spec.
    pub spec: String,
    /// Size of the file in bytes.
    pub size: u64,
    /// Modification time (JSON: RFC 3339 UTC).
    pub mtime: Option<jiff::Timestamp>,
    /// The Hikasa file format of the Picture kind chosen for the side, if any.
    pub format: Option<FormatJson>,
}

/// Where `--diff-output` wrote the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffOutput {
    /// The target as given (`-`, `clipboard://`, a spec).
    pub target: String,
    /// The pane written.
    pub pane: Pane,
    /// The mode written.
    pub mode: DiffMode,
    /// Its width.
    pub width: u32,
    /// Its height.
    pub height: u32,
    /// Transparent pixels over the checkerboard.
    pub checkerboard: bool,
}

/// The input of a picture report and of `hikasa.picture_compare.v1`.
#[derive(Clone, Copy)]
pub struct PictureSource<'a> {
    /// The comparison (both pictures, the options, the statistics).
    pub comparison: &'a PictureComparison,
    /// The two sides.
    pub sides: Pair<&'a PictureSide>,
    /// The verdict (D-339).
    pub kind: ContentKind,
    /// `--ignore-unimportant` of the comparison.
    pub ignore_unimportant: bool,
    /// The display mode of the comparison (the difference image's default).
    pub mode: DiffMode,
    /// The tone map the pictures were decoded with.
    pub tone_map: ToneMap,
    /// The theme's `[picture]` colours.
    pub colors: &'a PictureColors,
    /// `--probe` results.
    pub probes: &'a [PixelProbe],
    /// The `--diff-output` image, when one was written.
    pub diff_output: Option<&'a DiffOutput>,
    /// The comparison's warnings.
    pub warnings: &'a [Warning],
}

impl PictureSource<'_> {
    /// The verdict as a content result.
    pub fn verdict(&self) -> ContentResult {
        picture_verdict(self.kind)
    }

    /// The side `side`.
    pub(crate) fn side(&self, side: Side) -> &PictureSide {
        match side {
            Side::Left => self.sides.left,
            _ => self.sides.right,
        }
    }

    /// The decoded picture's information of `side`.
    pub(crate) fn info(&self, side: Side) -> &PictureInfo {
        let pictures = self.comparison.pictures();
        match side {
            Side::Left => &pictures.left.info,
            _ => &pictures.right.info,
        }
    }
}

/// The content result of a picture verdict (D-339: never `binary_different`).
pub fn picture_verdict(kind: ContentKind) -> ContentResult {
    match kind {
        ContentKind::BinarySame => ContentResult::BinarySame,
        ContentKind::RulesSame => ContentResult::RulesSame,
        ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
        _ => ContentResult::ImportantDifferences,
    }
}

/// `6000×4000`.
pub fn size_text(width: u32, height: u32) -> String {
    format!("{width}\u{d7}{height}")
}

/// The human output's line of a side: `left: a.png 6000×4000 png rgb8`.
pub fn side_line(spec: &str, info: &PictureInfo, side: Side) -> String {
    format!(
        "{}: {} {} {} {}",
        side.as_str(),
        spec,
        size_text(info.width, info.height),
        info.format.as_str(),
        info.color.as_str()
    )
}

/// A count and its percentage: `12 (0.0050 %)`.
fn counted(stats: &hikasa_picture::PictureStats, n: u64) -> String {
    format!("{n} ({:.4} %)", stats.percent(n))
}

/// The statistics line of the human output: `pixels P  same S (s %)  similar …  different D (d %)  max distance M
/// bounds x,y w×h`.
pub fn stats_line(cmp: &PictureComparison) -> String {
    let s = cmp.stats();
    let mut line = format!(
        "pixels {}  same {}  similar {}  different {}  max distance {}",
        s.pixels,
        counted(s, s.same),
        counted(s, s.similar),
        counted(s, s.different),
        s.max_distance
    );
    match s.bounds {
        Some(b) => {
            let _ = write!(
                line,
                "  bounds {},{} {}",
                b.x,
                b.y,
                size_text(b.width, b.height)
            );
        }
        None => line.push_str("  bounds none"),
    }
    line
}

/// The word of a pixel class (`same`, `similar`, `different`, `outside`).
pub const fn class_word(class: PixelClass) -> &'static str {
    match class {
        PixelClass::Same => "same",
        PixelClass::Similar => "similar",
        PixelClass::Different => "different",
        PixelClass::Outside => "outside",
    }
}

/// `#rrggbbaa`.
fn rgba_text(c: hikasa_picture::Rgba8) -> String {
    format!("#{:02x}{:02x}{:02x}{:02x}", c.r, c.g, c.b, c.a)
}

/// The human output's line of a probe: `probe 10,20: left 10,20 #ff0000ff  right 10,20 #fe0000ff  distance 1
/// similar` (`—` for a side that does not cover the position).
pub fn probe_line(p: &PixelProbe) -> String {
    let side = |s: &Option<(u32, u32, hikasa_picture::Rgba8)>| match s {
        Some((x, y, c)) => format!("{x},{y} {}", rgba_text(*c)),
        None => "\u{2014}".to_owned(),
    };
    format!(
        "probe {},{}: left {}  right {}  distance {} {}",
        p.area_x,
        p.area_y,
        side(&p.left),
        side(&p.right),
        p.distance,
        class_word(p.class)
    )
}

/// The difference image's mode: the report's `mode-*` word, else the comparison's; `mode-blend` keeps the
/// comparison's percentage when it blends.
fn effective_mode(src: &PictureSource<'_>, opts: &PictureReportOptions) -> DiffMode {
    match (opts.mode, src.mode) {
        (Some(DiffMode::Blend { .. }), DiffMode::Blend { percent }) => DiffMode::Blend { percent },
        (Some(mode), _) => mode,
        (None, mode) => mode,
    }
}

/// The `Mode:` line.
fn mode_line(src: &PictureSource<'_>, opts: &PictureReportOptions) -> String {
    let mode = effective_mode(src, opts);
    let mut out = format!("difference image: {}", mode.as_str());
    if let DiffMode::Blend { percent } = mode {
        let _ = write!(out, " {percent} %");
    }
    let _ = write!(
        out,
        "; ignore unimportant: {}",
        if opts.ignore_unimportant || src.ignore_unimportant {
            "yes"
        } else {
            "no"
        }
    );
    out
}

/// The `Rules:` line: tolerance, Auto Scale, tone map, orientations, offset.
pub(crate) fn rules_line(src: &PictureSource<'_>) -> String {
    let o = src.comparison.options();
    let g = &o.geometry;
    format!(
        "tolerance {}; auto scale: {}; tone map: {}; orientation: left {}, right {}; offset {},{}",
        o.tolerance,
        if g.auto_scale { "yes" } else { "no" },
        src.tone_map.as_str(),
        g.orientation.left.as_str(),
        g.orientation.right.as_str(),
        g.offset.0,
        g.offset.1
    )
}

/// `<spec> (png 6000×4000 rgb8)`.
fn side_description(src: &PictureSource<'_>, side: Side) -> String {
    let info = src.info(side);
    format!(
        "{} ({} {} {})",
        src.side(side).spec,
        info.format.as_str(),
        size_text(info.width, info.height),
        info.color.as_str()
    )
}

/// The RPT-004 header of a picture report.
fn header(src: &PictureSource<'_>, opts: &PictureReportOptions, meta: &ReportMeta) -> Header {
    let mut meta = meta.clone();
    if meta.mode.is_none() {
        meta.mode = Some(mode_line(src, opts));
    }
    if meta.filters.is_none() {
        meta.filters = Some(rules_line(src));
    }
    Header::new(
        &meta,
        PICTURE_TITLE,
        Pair::new(
            side_description(src, Side::Left),
            side_description(src, Side::Right),
        ),
        Vec::new(),
        src.warnings,
    )
}

/// One line: label, value, colour under ANSI.
type Line = (&'static str, String, &'static str);

/// A block: its title (none for the first) and its lines.
type Block = (Option<&'static str>, Vec<Line>);

/// The File Info lines of a side.
fn info_lines(src: &PictureSource<'_>, side: Side) -> Vec<Line> {
    let info = src.info(side);
    let s = src.side(side);
    let mut out: Vec<Line> = vec![
        ("Spec", s.spec.clone(), ""),
        ("Format", info.format.as_str().to_owned(), ""),
        ("Size", size_text(info.width, info.height), ""),
        (
            "Colour",
            format!("{}, {} bits a channel", info.color.as_str(), info.bit_depth),
            "",
        ),
        ("Alpha", yes_no(info.has_alpha).to_owned(), ""),
        (
            "Frames",
            info.frames
                .count()
                .map_or_else(|| "unknown".to_owned(), |n| n.to_string()),
            "",
        ),
        ("File size", crate::options::plural(s.size, "byte"), ""),
    ];
    if let Some(o) = info.orientation {
        out.push(("EXIF orientation", o.as_str().to_owned(), ""));
    }
    if let Some(t) = info.hdr {
        out.push(("Tone map", t.as_str().to_owned(), ""));
    }
    if info.converted {
        out.push(("Converted", "yes".to_owned(), ""));
    }
    if info.icc {
        out.push(("ICC profile", "ignored (sRGB assumed)".to_owned(), ""));
    }
    out
}

const fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// The blocks of the summary.
fn blocks(src: &PictureSource<'_>, opts: &PictureReportOptions) -> Vec<Block> {
    let cmp = src.comparison;
    let s = cmp.stats();
    let verdict = src.verdict();
    let iu = opts.ignore_unimportant || src.ignore_unimportant;
    let colour = match (&verdict, iu) {
        (ContentResult::ImportantDifferences, _) => Sgr::RED,
        (ContentResult::UnimportantOnly, false) => Sgr::BLUE,
        _ => "",
    };
    let mut first: Vec<Line> = vec![
        (
            "Verdict",
            crate::text::summary_verdict_words(&verdict),
            colour,
        ),
        (
            "Ignore unimportant",
            if iu {
                "yes (similar pixels count as same)".to_owned()
            } else {
                "no".to_owned()
            },
            "",
        ),
    ];
    let area = cmp.area();
    first.push(("Area", size_text(area.width, area.height), ""));
    if let Some((side, (w, h))) = cmp.scaled() {
        first.push((
            "Auto Scale",
            format!("{} scaled to {}", side.as_str(), size_text(w, h)),
            "",
        ));
    }
    let stats: Vec<Line> = vec![
        ("Pixels", s.pixels.to_string(), ""),
        ("Same", counted(s, s.same), ""),
        ("Similar", counted(s, s.similar), ""),
        ("Different", counted(s, s.different), ""),
        ("Left only", s.left_only.to_string(), ""),
        ("Right only", s.right_only.to_string(), ""),
        ("Max distance", s.max_distance.to_string(), ""),
        ("Mean distance", format!("{:.4}", s.mean_distance()), ""),
        (
            "Bounds",
            s.bounds.map_or_else(
                || "none".to_owned(),
                |b| format!("{},{} {}", b.x, b.y, size_text(b.width, b.height)),
            ),
            "",
        ),
    ];
    let mut out: Vec<Block> = vec![
        (None, first),
        (Some("Statistics"), stats),
        (Some("Left"), info_lines(src, Side::Left)),
        (Some("Right"), info_lines(src, Side::Right)),
    ];
    if !src.probes.is_empty() {
        out.push((
            Some("Probes"),
            src.probes
                .iter()
                .map(|p| ("Probe", probe_line(p), ""))
                .collect(),
        ));
    }
    out
}

fn write_summary_text(
    w: &mut dyn Write,
    src: &PictureSource<'_>,
    opts: &PictureReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    header(src, opts, meta).write_text(w, "", ansi)?;
    let blocks = blocks(src, opts);
    let width = blocks
        .iter()
        .flat_map(|(_, lines)| lines.iter().map(|(l, ..)| l.len() + 1))
        .max()
        .unwrap_or(0);
    for (title, lines) in &blocks {
        let indent = if title.is_some() { "  " } else { "" };
        if let Some(title) = title {
            writeln!(w)?;
            let (bold, reset) = if ansi {
                (Sgr::BOLD, Sgr::RESET)
            } else {
                ("", "")
            };
            writeln!(w, "{bold}{title}{reset}")?;
        }
        for (label, value, colour) in lines {
            let colour = if ansi { *colour } else { "" };
            let reset = if colour.is_empty() { "" } else { Sgr::RESET };
            writeln!(
                w,
                "{indent}{:<width$}  {colour}{}{reset}",
                format!("{label}:"),
                escape::escape_controls(value)
            )?;
        }
    }
    Ok(())
}

fn write_summary_tables(w: &mut dyn Write, blocks: &[Block], verdict: &str) -> std::io::Result<()> {
    for (title, lines) in blocks {
        if let Some(title) = title {
            writeln!(w, "<h2>{}</h2>", escape::html(title))?;
        }
        writeln!(w, "<table class=\"summary\">")?;
        for (label, value, _) in lines {
            let class = if *label == "Verdict" { verdict } else { "" };
            writeln!(
                w,
                "<tr><th>{}</th><td{class}>{}</td></tr>",
                escape::html(label),
                escape::html(value)
            )?;
        }
        writeln!(w, "</table>")?;
    }
    Ok(())
}

/// The class of the verdict cell.
fn verdict_class(src: &PictureSource<'_>) -> &'static str {
    match src.verdict() {
        ContentResult::ImportantDifferences => " class=\"imp\"",
        ContentResult::UnimportantOnly => " class=\"unimp\"",
        _ => "",
    }
}

fn write_summary_html(
    w: &mut dyn Write,
    src: &PictureSource<'_>,
    opts: &PictureReportOptions,
    meta: &ReportMeta,
    mono: bool,
    style: &HtmlStyle,
) -> Result<(), ReportError> {
    let header = header(src, opts, meta);
    style::html_start(
        w,
        &header.title,
        &meta.generator,
        Sheet::Picture,
        mono,
        style,
    )?;
    header.write_html(w)?;
    write_summary_tables(w, &blocks(src, opts), verdict_class(src))?;
    style::html_end(w)?;
    Ok(())
}

/// Standard base64 with padding (RFC 4648 §4).
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The level whose area fits [`SCALED_MAX`] on its long side (0 when the area already does).
pub fn scaled_level(cmp: &PictureComparison) -> u8 {
    let mut level = 0;
    loop {
        let (w, h) = cmp.level_size(level);
        if w.max(h) <= SCALED_MAX || level >= hikasa_picture::render::MAX_LEVEL {
            return level;
        }
        level += 1;
    }
}

fn picture_error(e: &PictureError) -> ReportError {
    ReportError::Input {
        kind: e.kind(),
        message: e.to_string(),
    }
}

/// Renders `pane` as the report shows it: full size, or at the level that fits [`SCALED_MAX`]; with the checkerboard
/// when asked.
pub fn report_image(
    cmp: &PictureComparison,
    pane: Pane,
    mode: DiffMode,
    colors: &PictureColors,
    checkerboard: bool,
    full_size: bool,
) -> Result<RgbaImage, PictureError> {
    let level = if full_size { 0 } else { scaled_level(cmp) };
    if level == 0 {
        return cmp.render(pane, mode, colors, checkerboard, &Ctl::detached());
    }
    let (width, height) = cmp.level_size(level);
    let mut image = cmp.render_tile(
        pane,
        mode,
        colors,
        TileRect {
            x: 0,
            y: 0,
            width,
            height,
        },
        level,
    );
    if checkerboard {
        composite_checkerboard(
            &mut image,
            (0, 0),
            colors,
            hikasa_picture::render::CHECKER_CELL,
        );
    }
    Ok(image)
}

fn write_side_by_side(
    w: &mut dyn Write,
    src: &PictureSource<'_>,
    opts: &PictureReportOptions,
    meta: &ReportMeta,
    mono: bool,
    style: &HtmlStyle,
) -> Result<(), ReportError> {
    let header = header(src, opts, meta);
    style::html_start(
        w,
        &header.title,
        &meta.generator,
        Sheet::Picture,
        mono,
        style,
    )?;
    header.write_html(w)?;
    let mode = effective_mode(src, opts);
    writeln!(w, "<div class=\"pictures\">")?;
    for (pane, caption) in [
        (Pane::Left, "Left"),
        (Pane::Right, "Right"),
        (Pane::Difference, "Difference"),
    ] {
        let image = report_image(
            src.comparison,
            pane,
            mode,
            src.colors,
            opts.checkerboard,
            opts.full_size,
        )
        .map_err(|e| picture_error(&e))?;
        let png = encode_png(&image, PngLevel::Best).map_err(|e| picture_error(&e))?;
        let mut caption = caption.to_owned();
        if pane == Pane::Difference {
            let _ = write!(caption, " ({})", mode.as_str());
        }
        writeln!(
            w,
            "<figure><img alt=\"{alt}\" width=\"{}\" height=\"{}\" src=\"data:image/png;base64,{}\"><figcaption>{} \
             {}</figcaption></figure>",
            image.width,
            image.height,
            base64(&png),
            escape::html(&caption),
            size_text(image.width, image.height),
            alt = escape::html(&caption),
        )?;
    }
    writeln!(w, "</div>")?;
    write_summary_tables(w, &blocks(src, opts), verdict_class(src))?;
    style::html_end(w)?;
    Ok(())
}

/// Renders a picture report into `out`. Returns the warnings of the rendering itself (none today).
///
/// # Errors
///
/// A write error; [`ReportError::Unsupported`] for a side-by-side report as text; [`ReportError::Input`] when an
/// image cannot be rendered (an area above the renderer's limit at full size).
pub fn render_picture_report(
    src: &PictureSource<'_>,
    opts: &PictureReportOptions,
    meta: &ReportMeta,
    style: &HtmlStyle,
    out: &mut dyn Write,
) -> Result<Vec<Warning>, ReportError> {
    let mut w = BufWriter::with_capacity(64 * 1024, out);
    match (opts.layout, opts.format) {
        (PictureReportLayout::Summary, OutputFormat::Text { ansi }) => {
            write_summary_text(&mut w, src, opts, meta, ansi)?;
        }
        (PictureReportLayout::Summary, OutputFormat::Html { mono }) => {
            write_summary_html(&mut w, src, opts, meta, mono, style)?;
        }
        (PictureReportLayout::SideBySide, OutputFormat::Html { mono }) => {
            write_side_by_side(&mut w, src, opts, meta, mono, style)?;
        }
        (PictureReportLayout::SideBySide, format @ OutputFormat::Text { .. }) => {
            return Err(ReportError::Unsupported {
                layout: "side-by-side",
                format: format.name(),
            });
        }
    }
    w.flush()?;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_of_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
}
