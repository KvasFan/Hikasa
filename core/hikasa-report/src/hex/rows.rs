//! The hex side-by-side and interleaved layouts (HX-008's P1 layouts on the rows model, D-348): the rows of
//! [`HexView`] under the display filter, as text (a `^` marker line under each differing row) or HTML (RPT-010's
//! colours: differing bytes red, one-sided bytes purple).

use std::fmt::Write as _;
use std::io::Write;

use hikasa_hex::{ByteState, CharSet, HexCell, HexRow, HexView, RowState};

use super::{
    HexReportLayout, HexReportOptions, HexSource, address, header, hex_summary_line, input_error,
};
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::style::{self, Sgr, Sheet};

/// Rows read per call of [`HexView::rows`].
const BATCH: u32 = 4096;

/// The view of the report: the source's layout under the report's filter.
fn view<'a>(src: &HexSource<'a>, opts: &HexReportOptions) -> Option<HexView<'a>> {
    let filter = opts.display?;
    Some(
        HexView::new(
            src.documents.left,
            src.documents.right,
            src.alignment,
            src.layout,
        )
        .with_filter(filter, opts.context),
    )
}

/// Visits every row the view shows, in order.
fn each_row(
    view: &HexView<'_>,
    mut f: impl FnMut(&HexRow) -> Result<(), ReportError>,
) -> Result<Option<u64>, ReportError> {
    let count = view.row_count();
    let mut first = 0;
    let mut last = None;
    while first < count {
        let rows = view.rows(first, BATCH).map_err(|e| input_error(&e))?;
        if rows.is_empty() {
            break;
        }
        for row in &rows {
            f(row)?;
            if !row.cells.is_empty() {
                last = Some(row.index);
            }
        }
        first += rows.len() as u64;
    }
    Ok(last)
}

/// The rows hidden after the last shown one.
fn hidden_after(view: &HexView<'_>, last: Option<u64>) -> u64 {
    let full = view.full_row_count();
    match last {
        Some(index) => full.saturating_sub(index + 1),
        None => full,
    }
}

/// `1 row hidden`, `N rows hidden`.
fn hidden_text(n: u64) -> String {
    format!("{} hidden", crate::options::plural(n, "row"))
}

/// The text of a marker row (rows past a truncated section list).
fn marker_text(n: u64) -> String {
    format!(
        "{} past the truncated section list (shown under display-all)",
        crate::options::plural(n, "row")
    )
}

/// The separator before cell `i` of a row.
const fn cell_sep(i: usize, group: u8) -> &'static str {
    if i == 0 {
        ""
    } else if group > 1 && i.is_multiple_of(group as usize) {
        "  "
    } else {
        " "
    }
}

/// One side's byte of a cell.
const fn byte_of(cell: &HexCell, left: bool) -> Option<u8> {
    if left { cell.left } else { cell.right }
}

/// The ANSI colour of a side's byte.
const fn ansi_of(state: ByteState) -> &'static str {
    match state {
        ByteState::Same => "",
        ByteState::Different => Sgr::RED,
        ByteState::LeftOnly | ByteState::RightOnly => Sgr::MAGENTA,
    }
}

/// The HTML class of a side's byte.
const fn class_of(state: ByteState) -> &'static str {
    match state {
        ByteState::Same => "",
        ByteState::Different => "d",
        ByteState::LeftOnly | ByteState::RightOnly => "o",
    }
}

/// The text of one side of a row: `addr  hex  chars` (no address without addresses; no characters with
/// `CharSet::None`), padded to the full width.
struct SideText<'a> {
    src: &'a HexSource<'a>,
    opts: &'a HexReportOptions,
    width: usize,
}

impl SideText<'_> {
    fn bpl(&self) -> usize {
        usize::from(self.src.layout.bytes_per_line.clamp(1, 256))
    }

    /// The hex column's width.
    fn hex_width(&self) -> usize {
        let mut w = 0;
        for i in 0..self.bpl() {
            w += cell_sep(i, self.src.layout.group).len() + 2;
        }
        w
    }

    fn text(&self, row: &HexRow, left: bool, ansi: bool, marker: bool) -> String {
        let layout = &self.src.layout;
        let mut out = String::new();
        if self.opts.addresses {
            let addr = if left {
                row.left_address
            } else {
                row.right_address
            };
            match addr {
                Some(a) if !marker => out.push_str(&address(a, layout.address, self.width)),
                _ => out.push_str(&" ".repeat(self.width)),
            }
            out.push_str("  ");
        }
        for i in 0..self.bpl() {
            out.push_str(cell_sep(i, layout.group));
            let cell = row.cells.get(i);
            match cell.and_then(|c| byte_of(c, left).map(|b| (c, b))) {
                Some((c, _)) if marker => out.push_str(if c.state == ByteState::Same {
                    "  "
                } else {
                    "^^"
                }),
                Some((c, b)) => {
                    let colour = if ansi { ansi_of(c.state) } else { "" };
                    let reset = if colour.is_empty() { "" } else { Sgr::RESET };
                    let _ = write!(out, "{colour}{b:02x}{reset}");
                }
                None => out.push_str("  "),
            }
        }
        if layout.chars != CharSet::None && !marker {
            out.push_str("  ");
            for i in 0..self.bpl() {
                let cell = row.cells.get(i);
                match cell.and_then(|c| byte_of(c, left).map(|b| (c, b))) {
                    Some((c, b)) => {
                        let ch = layout.char_of(b).unwrap_or('.');
                        let colour = if ansi { ansi_of(c.state) } else { "" };
                        let reset = if colour.is_empty() { "" } else { Sgr::RESET };
                        let _ = write!(out, "{colour}{ch}{reset}");
                    }
                    None => out.push(' '),
                }
            }
        }
        out
    }

    /// The width of a side's text without colours.
    fn plain_width(&self) -> usize {
        let mut w = self.hex_width();
        if self.opts.addresses {
            w += self.width + 2;
        }
        if self.src.layout.chars != CharSet::None {
            w += 2 + self.bpl();
        }
        w
    }
}

/// Writes the side-by-side or interleaved text report.
pub(super) fn write_text(
    w: &mut dyn Write,
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    meta: &ReportMeta,
    ansi: bool,
    layout: HexReportLayout,
) -> Result<(), ReportError> {
    header(src, opts, meta).write_text(w, "", ansi)?;
    let grey = |s: &str| {
        if ansi {
            format!("{}{s}{}", Sgr::GREY, Sgr::RESET)
        } else {
            s.to_owned()
        }
    };
    if let Some(view) = view(src, opts) {
        let side = SideText {
            src,
            opts,
            width: usize::from(view.address_width()),
        };
        let pad = side.plain_width();
        let last = each_row(&view, |row| {
            if row.hidden_before > 0 && !row.cells.is_empty() {
                writeln!(
                    w,
                    "{}",
                    grey(&format!("... {} ...", hidden_text(row.hidden_before)))
                )?;
            }
            if row.cells.is_empty() {
                writeln!(
                    w,
                    "{}",
                    grey(&format!("... {} ...", marker_text(row.hidden_before)))
                )?;
                return Ok(());
            }
            let differs = row.state != RowState::Same;
            match layout {
                HexReportLayout::Interleaved if differs => {
                    writeln!(w, "< {}", side.text(row, true, ansi, false).trim_end())?;
                    writeln!(w, "> {}", side.text(row, false, ansi, false).trim_end())?;
                    let marker = side.text(row, true, false, true);
                    let marker_r = side.text(row, false, false, true);
                    writeln!(w, "  {}", merge_markers(&marker, &marker_r).trim_end())?;
                }
                HexReportLayout::Interleaved => {
                    writeln!(w, "  {}", side.text(row, true, ansi, false).trim_end())?;
                }
                _ => {
                    let l = side.text(row, true, ansi, false);
                    let r = side.text(row, false, ansi, false);
                    let fill =
                        pad.saturating_sub(side.text(row, true, false, false).chars().count());
                    writeln!(w, "{l}{} \u{2016} {}", " ".repeat(fill), r.trim_end())?;
                    if differs {
                        let ml = side.text(row, true, false, true);
                        let mr = side.text(row, false, false, true);
                        let fill = pad.saturating_sub(ml.chars().count());
                        let line = format!("{ml}{}   {mr}", " ".repeat(fill));
                        writeln!(w, "{}", line.trim_end())?;
                    }
                }
            }
            Ok(())
        })?;
        let after = hidden_after(&view, last);
        if after > 0 && view.row_count() > 0 {
            writeln!(w, "{}", grey(&format!("... {} ...", hidden_text(after))))?;
        }
        writeln!(w)?;
    }
    writeln!(
        w,
        "{}",
        hex_summary_line(&src.alignment.stats, src.alignment.mode)
    )?;
    Ok(())
}

/// Two marker lines of the same row merged: a `^` where either has one.
fn merge_markers(a: &str, b: &str) -> String {
    let mut out: Vec<char> = a.chars().collect();
    for (i, c) in b.chars().enumerate() {
        if c == '^' {
            if i < out.len() {
                out[i] = '^';
            } else {
                out.resize(i, ' ');
                out.push('^');
            }
        }
    }
    out.into_iter().collect()
}

/// The HTML of one side of a row: address, bytes and characters cells.
fn side_html(
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    row: &HexRow,
    left: bool,
    width: usize,
) -> String {
    let layout = &src.layout;
    let bpl = usize::from(layout.bytes_per_line.clamp(1, 256));
    let mut out = String::new();
    if opts.addresses {
        let addr = if left {
            row.left_address
        } else {
            row.right_address
        };
        let _ = write!(
            out,
            "<td class=\"n\">{}</td>",
            addr.map(|a| address(a, layout.address, width))
                .unwrap_or_default()
        );
    }
    out.push_str("<td class=\"h\">");
    for i in 0..bpl {
        out.push_str(cell_sep(i, layout.group));
        match row
            .cells
            .get(i)
            .and_then(|c| byte_of(c, left).map(|b| (c, b)))
        {
            Some((c, b)) => match class_of(c.state) {
                "" => {
                    let _ = write!(out, "{b:02x}");
                }
                class => {
                    let _ = write!(out, "<span class=\"{class}\">{b:02x}</span>");
                }
            },
            None => out.push_str("  "),
        }
    }
    out.push_str("</td>");
    if layout.chars != CharSet::None {
        out.push_str("<td class=\"c\">");
        for i in 0..bpl {
            match row
                .cells
                .get(i)
                .and_then(|c| byte_of(c, left).map(|b| (c, b)))
            {
                Some((c, b)) => {
                    let mut ch = String::new();
                    escape::push_html_char(&mut ch, layout.char_of(b).unwrap_or('.'));
                    match class_of(c.state) {
                        "" => out.push_str(&ch),
                        class => {
                            let _ = write!(out, "<span class=\"{class}\">{ch}</span>");
                        }
                    }
                }
                None => out.push(' '),
            }
        }
        out.push_str("</td>");
    }
    out
}

/// Writes the side-by-side or interleaved HTML report.
pub(super) fn write_html(
    w: &mut dyn Write,
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    meta: &ReportMeta,
    mono: bool,
    style: &HtmlStyle,
    layout: HexReportLayout,
) -> Result<(), ReportError> {
    let header = header(src, opts, meta);
    style::html_start(w, &header.title, &meta.generator, Sheet::Hex, mono, style)?;
    header.write_html(w)?;
    if let Some(view) = view(src, opts) {
        let width = usize::from(view.address_width());
        let side_cols =
            1 + usize::from(opts.addresses) + usize::from(src.layout.chars != CharSet::None);
        let cols = match layout {
            HexReportLayout::Interleaved => side_cols + 1,
            _ => 2 * side_cols + 1,
        };
        let class = match layout {
            HexReportLayout::Interleaved => "hil",
            _ => "hsbs",
        };
        writeln!(w, "<table class=\"{class}\">")?;
        let gap = |w: &mut dyn Write, text: &str| -> std::io::Result<()> {
            writeln!(
                w,
                "<tr class=\"gap\"><td colspan=\"{cols}\">{}</td></tr>",
                escape::html(text)
            )
        };
        let last = each_row(&view, |row| {
            if row.hidden_before > 0 && !row.cells.is_empty() {
                gap(w, &hidden_text(row.hidden_before))?;
            }
            if row.cells.is_empty() {
                gap(w, &marker_text(row.hidden_before))?;
                return Ok(());
            }
            let state = match row.state {
                RowState::Same => "s",
                RowState::Different => "d",
                RowState::Orphan => "o",
            };
            match layout {
                HexReportLayout::Interleaved if row.state != RowState::Same => {
                    writeln!(
                        w,
                        "<tr class=\"{state} l\"><td class=\"m\">&lt;</td>{}</tr>",
                        side_html(src, opts, row, true, width)
                    )?;
                    writeln!(
                        w,
                        "<tr class=\"{state} r\"><td class=\"m\">&gt;</td>{}</tr>",
                        side_html(src, opts, row, false, width)
                    )?;
                }
                HexReportLayout::Interleaved => {
                    writeln!(
                        w,
                        "<tr class=\"s\"><td class=\"m\"></td>{}</tr>",
                        side_html(src, opts, row, true, width)
                    )?;
                }
                _ => {
                    writeln!(
                        w,
                        "<tr class=\"{state}\">{}<td class=\"sep\"></td>{}</tr>",
                        side_html(src, opts, row, true, width),
                        side_html(src, opts, row, false, width)
                    )?;
                }
            }
            Ok(())
        })?;
        let after = hidden_after(&view, last);
        if after > 0 && view.row_count() > 0 {
            gap(w, &hidden_text(after))?;
        }
        writeln!(w, "</table>")?;
    }
    writeln!(
        w,
        "<p class=\"totals\">{}</p>",
        escape::html(&hex_summary_line(&src.alignment.stats, src.alignment.mode))
    )?;
    style::html_end(w)?;
    Ok(())
}
