//! Table side-by-side (TB-025): per sheet pair a section with one grid, the left cells then the right cells of each
//! shown row, the row labels when asked, one `… N rows hidden …` line per run of hidden rows; plain text (columns fitted
//! into the width, cut with `…`, ANSI colours per cell on a terminal) or HTML (RPT-010 colours per cell).

use std::io::Write;

use hikasa_core::Side;
use hikasa_table::{CellState, ColumnPair, Importance, RowKind, RowPair, SheetComparison};

use super::{
    Item, TableReportOptions, TableSource, alignment_words, cell_text, column_letter, filter_of,
    fit, header, hidden_text, items, marker, sheet_title, shown_columns, shown_state, state_words,
    width,
};
use crate::cells::pad;
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::style::{self, Sgr, Sheet};

/// The grid of one sheet pair: the shown columns and the rows to write.
pub(super) struct Grid<'a> {
    /// The pair's index.
    pub pair: usize,
    /// The sheet pair.
    pub sheet: &'a SheetComparison,
    /// Shown columns: comparison index and column.
    pub columns: Vec<(u32, &'a ColumnPair)>,
    /// Rows and gaps.
    pub items: Vec<Item>,
}

impl<'a> Grid<'a> {
    /// The grid of pair `pair` under the report's filter.
    pub fn new(src: &TableSource<'a>, opts: &TableReportOptions, pair: usize) -> Grid<'a> {
        let sheet = &src.comparison.sheets()[pair];
        Grid {
            pair,
            sheet,
            columns: shown_columns(sheet),
            items: items(sheet, &filter_of(opts)),
        }
    }

    /// The shown rows.
    pub fn rows(&self) -> impl Iterator<Item = (u32, &'a RowPair)> + '_ {
        self.items.iter().filter_map(|item| match item {
            Item::Row(i) => self.sheet.rows().get(*i as usize).map(|r| (*i, r)),
            Item::Gap(_) => None,
        })
    }
}

/// The header name of a column on a side: the side's header name when the comparison read one, else its letters;
/// `None` when the column is not on that side.
pub(super) fn side_name(
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    column: &ColumnPair,
    side: Side,
) -> Option<String> {
    let c = match side {
        Side::Left => column.left,
        _ => column.right,
    }?;
    let sheet = src.comparison.sheet(grid.pair, side)?;
    Some(if grid.sheet.has_header(side) {
        escape::escape_controls(&sheet.column_name(c)).into_owned()
    } else {
        sheet.column_label(c)
    })
}

/// The text of a cell of `side` (`None` when the side has no row or no column there).
pub(super) fn side_cell(
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    row: u32,
    r: &RowPair,
    column: u32,
    col: &ColumnPair,
    side: Side,
) -> Option<String> {
    r.side(side)?;
    match side {
        Side::Left => col.left?,
        _ => col.right?,
    };
    Some(cell_text(
        &src.comparison.cell(grid.pair, row, column, side),
    ))
}

/// The state a cell shows: a changed row's cell state, an orphan row's importance, same otherwise.
pub(super) fn cell_state(
    grid: &Grid<'_>,
    row: u32,
    r: &RowPair,
    column: u32,
    iu: bool,
) -> CellState {
    let state = match r.kind() {
        RowKind::Same => CellState::Same,
        _ => grid.sheet.cell_state(row, column),
    };
    shown_state(state, iu)
}

/// The SGR colour of a cell state.
pub(super) const fn sgr(state: CellState) -> &'static str {
    match state {
        CellState::Same => "",
        CellState::Unimportant => Sgr::BLUE,
        CellState::Important => Sgr::RED,
    }
}

/// The width of the row labels of a grid (both sides), 0 without row numbers.
pub(super) fn label_width(
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    numbers: bool,
    interleaved: bool,
) -> usize {
    if !numbers {
        return 0;
    }
    let mut w = 1;
    for (i, r) in grid.rows() {
        for side in [Side::Left, Side::Right] {
            if let Some(l) = src.comparison.row_label(grid.pair, i, side) {
                w = w.max(l.to_string().len());
            }
        }
        if let Some(both) = same_label(src, grid.pair, i, r).filter(|_| interleaved) {
            w = w.max(both.len());
        }
    }
    w
}

/// The label of a same row written once (interleaved): `5`, or `5/6` when the sides' labels differ.
pub(super) fn same_label(
    src: &TableSource<'_>,
    pair: usize,
    row: u32,
    r: &RowPair,
) -> Option<String> {
    if r.kind() != RowKind::Same {
        return None;
    }
    let l = src.comparison.row_label(pair, row, Side::Left)?;
    let rl = src.comparison.row_label(pair, row, Side::Right)?;
    Some(if l == rl {
        l.to_string()
    } else {
        format!("{l}/{rl}")
    })
}

/// The natural widths of the shown columns: the widest header name or cell text of either side.
pub(super) fn natural_widths(src: &TableSource<'_>, grid: &Grid<'_>) -> Vec<usize> {
    let mut out: Vec<usize> = grid
        .columns
        .iter()
        .map(|(_, c)| {
            [Side::Left, Side::Right]
                .into_iter()
                .filter_map(|side| side_name(src, grid, c, side))
                .map(|n| width(&n))
                .max()
                .unwrap_or(0)
        })
        .collect();
    for (i, r) in grid.rows() {
        for (k, (column, col)) in grid.columns.iter().enumerate() {
            for side in [Side::Left, Side::Right] {
                if let Some(t) = side_cell(src, grid, i, r, *column, col, side) {
                    out[k] = out[k].max(width(&t));
                }
            }
        }
    }
    out
}

/// The section title of a sheet pair.
pub(super) fn section_title(src: &TableSource<'_>, pair: usize, iu: bool) -> String {
    let sheet = &src.comparison.sheets()[pair];
    let state = if iu && sheet.state() == hikasa_table::SheetState::Unimportant {
        "same (unimportant differences only)"
    } else {
        state_words(sheet.state())
    };
    match sheet.alignment() {
        hikasa_table::RowAlignmentUsed::Orphan => {
            format!("Sheet {}: {} ({state})", pair + 1, sheet_title(src, pair))
        }
        _ => format!(
            "Sheet {}: {} ({state}; {})",
            pair + 1,
            sheet_title(src, pair),
            alignment_words(sheet)
        ),
    }
}

/// Appends one side's cells of a row: each fitted to its width, separated by a space, coloured under ANSI.
#[allow(clippy::too_many_arguments)]
fn push_side(
    out: &mut String,
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    widths: &[usize],
    row: Option<(u32, &RowPair)>,
    side: Side,
    iu: bool,
    ansi: bool,
) {
    for (k, (column, col)) in grid.columns.iter().enumerate() {
        if k > 0 {
            out.push(' ');
        }
        let Some((i, r)) = row else {
            pad(out, widths[k]);
            continue;
        };
        match side_cell(src, grid, i, r, *column, col, side) {
            Some(text) => {
                let colour = if ansi {
                    sgr(cell_state(grid, i, r, *column, iu))
                } else {
                    ""
                };
                out.push_str(colour);
                out.push_str(&fit(&text, widths[k]));
                if !colour.is_empty() {
                    out.push_str(Sgr::RESET);
                }
            }
            None => pad(out, widths[k]),
        }
    }
}

/// Appends a row label fitted right to `w` (blank without one), then a space.
fn push_label(out: &mut String, label: Option<u64>, w: usize) {
    if w == 0 {
        return;
    }
    match label {
        Some(l) => out.push_str(&format!("{l:>w$} ")),
        None => pad(out, w + 1),
    }
}

/// Writes the plain-text side-by-side report.
pub(super) fn write_text(
    w: &mut dyn Write,
    src: &TableSource<'_>,
    opts: &TableReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    header(src, meta).write_text(w, "", ansi)?;
    let iu = opts.ignore_unimportant;
    let (bold, grey, reset) = if ansi {
        (Sgr::BOLD, Sgr::GREY, Sgr::RESET)
    } else {
        ("", "", "")
    };
    let mut line = String::new();
    for pair in 0..src.comparison.sheets().len() {
        if pair > 0 {
            writeln!(w)?;
        }
        let grid = Grid::new(src, opts, pair);
        writeln!(
            w,
            "{bold}{}{reset}",
            escape::escape_controls(&section_title(src, pair, iu))
        )?;
        let nw = label_width(src, &grid, opts.row_numbers, false);
        let n = grid.columns.len();
        let numbers = if nw > 0 { nw + 1 } else { 0 };
        let fixed = 2 + 3 + 2 * numbers + 2 * n.saturating_sub(1);
        let side_room = (usize::from(opts.width).saturating_sub(fixed) / 2).max(n);
        let widths = super::share_widths(&natural_widths(src, &grid), side_room);
        // The column names of each side.
        line.clear();
        line.push_str("  ");
        for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            if k == 1 {
                line.push_str(" | ");
            }
            pad(&mut line, numbers);
            for (c, (_, col)) in grid.columns.iter().enumerate() {
                if c > 0 {
                    line.push(' ');
                }
                let name = side_name(src, &grid, col, side).unwrap_or_default();
                line.push_str(&fit(&name, widths[c]));
            }
        }
        writeln!(w, "{bold}{}{reset}", line.trim_end())?;
        if grid.items.is_empty() {
            writeln!(w, "  {grey}0 rows{reset}")?;
        }
        for item in &grid.items {
            line.clear();
            match item {
                Item::Gap(n) => {
                    line.push_str("  ");
                    line.push_str(grey);
                    line.push_str(&hidden_text(*n));
                    line.push_str(reset);
                }
                Item::Row(i) => {
                    let Some(r) = grid.sheet.rows().get(*i as usize) else {
                        continue;
                    };
                    let m = marker(r, iu);
                    let colour = match (ansi, m) {
                        (true, '|') => Sgr::RED,
                        (true, '~') => Sgr::BLUE,
                        (true, '<' | '>') => Sgr::MAGENTA,
                        _ => "",
                    };
                    line.push_str(colour);
                    line.push(m);
                    if !colour.is_empty() {
                        line.push_str(Sgr::RESET);
                    }
                    line.push(' ');
                    for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
                        if k == 1 {
                            line.push_str(" | ");
                        }
                        let row = r.side(side).map(|_| (*i, r));
                        push_label(
                            &mut line,
                            row.and_then(|_| src.comparison.row_label(pair, *i, side)),
                            nw,
                        );
                        push_side(&mut line, src, &grid, &widths, row, side, iu, ansi);
                    }
                    let trimmed = line.trim_end_matches(' ').len();
                    line.truncate(trimmed);
                }
            }
            line.push('\n');
            w.write_all(line.as_bytes())?;
        }
    }
    Ok(())
}

/// The legend of the HTML table reports (RPT-010).
pub(super) const LEGEND: &str = "<table class=\"legend\">\n<tr><td class=\"imp\">Important difference</td>\
<td class=\"unimp\">Unimportant difference</td><td class=\"lo\">Orphan (one side only)</td>\
<td class=\"x\">No cell</td><td class=\"gap\">Hidden rows</td></tr>\n</table>\n";

/// The class attribute of a row.
pub(super) fn row_class(r: &RowPair, iu: bool) -> &'static str {
    let importance = if iu && r.importance() == Importance::Unimportant {
        Importance::None
    } else {
        r.importance()
    };
    match (r.kind(), importance) {
        (RowKind::LeftOnly, Importance::Important) => " class=\"lo imp\"",
        (RowKind::LeftOnly, Importance::Unimportant) => " class=\"lo unimp\"",
        (RowKind::LeftOnly, Importance::None) => " class=\"lo\"",
        (RowKind::RightOnly, Importance::Important) => " class=\"ro imp\"",
        (RowKind::RightOnly, Importance::Unimportant) => " class=\"ro unimp\"",
        (RowKind::RightOnly, Importance::None) => " class=\"ro\"",
        (_, Importance::Important) => " class=\"imp\"",
        (_, Importance::Unimportant) => " class=\"unimp\"",
        (_, Importance::None) => "",
    }
}

/// Appends one side's `<td>`s of a row.
pub(super) fn push_html_cells(
    out: &mut String,
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    row: Option<(u32, &RowPair)>,
    side: Side,
    iu: bool,
) {
    for (column, col) in &grid.columns {
        let text = row.and_then(|(i, r)| side_cell(src, grid, i, r, *column, col, side));
        match (row, text) {
            (Some((i, r)), Some(text)) => {
                let class = if r.kind() == RowKind::Changed {
                    match cell_state(grid, i, r, *column, iu) {
                        CellState::Important => "c imp",
                        CellState::Unimportant => "c unimp",
                        CellState::Same => "c",
                    }
                } else {
                    "c"
                };
                out.push_str("<td class=\"");
                out.push_str(class);
                out.push_str("\">");
                out.push_str(&escape::html(&text));
                out.push_str("</td>");
            }
            _ => out.push_str("<td class=\"x\"></td>"),
        }
    }
}

/// Appends the `<th>` of a column on a side: its name and its letters.
pub(super) fn push_html_th(
    out: &mut String,
    src: &TableSource<'_>,
    grid: &Grid<'_>,
    col: &ColumnPair,
    side: Side,
) {
    let name = side_name(src, grid, col, side).unwrap_or_default();
    out.push_str("<th>");
    out.push_str(&escape::html(&name));
    if let Some(letters) = column_letter(src, grid.pair, col, side) {
        out.push_str("<br><span class=\"ltr\">");
        out.push_str(&escape::html(&letters));
        out.push_str("</span>");
    }
    out.push_str("</th>");
}

/// Writes the section heading of a sheet pair in HTML.
pub(super) fn write_html_section(
    w: &mut dyn Write,
    src: &TableSource<'_>,
    pair: usize,
    iu: bool,
) -> std::io::Result<()> {
    let sheet = &src.comparison.sheets()[pair];
    writeln!(
        w,
        "<h2 class=\"sheet\">Sheet {}: {}</h2>",
        pair + 1,
        escape::html(&sheet_title(src, pair))
    )?;
    let state = if iu && sheet.state() == hikasa_table::SheetState::Unimportant {
        "same (unimportant differences only)"
    } else {
        state_words(sheet.state())
    };
    match sheet.alignment() {
        hikasa_table::RowAlignmentUsed::Orphan => {
            writeln!(w, "<p class=\"sheet\">{}</p>", escape::html(state))
        }
        _ => writeln!(
            w,
            "<p class=\"sheet\">{}; {}</p>",
            escape::html(state),
            escape::html(&alignment_words(sheet))
        ),
    }
}

/// Writes the HTML side-by-side report.
pub(super) fn write_html(
    w: &mut dyn Write,
    src: &TableSource<'_>,
    opts: &TableReportOptions,
    meta: &ReportMeta,
    mono: bool,
    style: &HtmlStyle,
) -> Result<(), ReportError> {
    let header = header(src, meta);
    style::html_start(w, &header.title, &meta.generator, Sheet::Table, mono, style)?;
    header.write_html(w)?;
    w.write_all(LEGEND.as_bytes())?;
    let iu = opts.ignore_unimportant;
    let numbers = opts.row_numbers;
    let mut line = String::new();
    for pair in 0..src.comparison.sheets().len() {
        let grid = Grid::new(src, opts, pair);
        write_html_section(w, src, pair, iu)?;
        writeln!(w, "<table class=\"tsbs\">")?;
        line.clear();
        line.push_str("<thead><tr><th class=\"m\"></th>");
        for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
            if k == 1 {
                line.push_str("<th class=\"sep\"></th>");
            }
            if numbers {
                line.push_str("<th class=\"n\"></th>");
            }
            for (_, col) in &grid.columns {
                push_html_th(&mut line, src, &grid, col, side);
            }
        }
        line.push_str("</tr></thead>\n<tbody>\n");
        w.write_all(line.as_bytes())?;
        let span = 2 + 2 * (usize::from(numbers) + grid.columns.len());
        if grid.items.is_empty() {
            writeln!(
                w,
                "<tr class=\"gap\"><td colspan=\"{span}\">0 rows</td></tr>"
            )?;
        }
        for item in &grid.items {
            line.clear();
            match item {
                Item::Gap(n) => {
                    line.push_str(&format!(
                        "<tr class=\"gap\"><td colspan=\"{span}\">{}</td></tr>",
                        hidden_text(*n)
                    ));
                }
                Item::Row(i) => {
                    let Some(r) = grid.sheet.rows().get(*i as usize) else {
                        continue;
                    };
                    line.push_str("<tr");
                    line.push_str(row_class(r, iu));
                    line.push_str("><td class=\"m\">");
                    line.push_str(&escape::html(&marker(r, iu).to_string()));
                    line.push_str("</td>");
                    for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
                        if k == 1 {
                            line.push_str("<td class=\"sep\"></td>");
                        }
                        let row = r.side(side).map(|_| (*i, r));
                        if numbers {
                            line.push_str("<td class=\"n\">");
                            if let Some(l) =
                                row.and_then(|_| src.comparison.row_label(pair, *i, side))
                            {
                                line.push_str(&l.to_string());
                            }
                            line.push_str("</td>");
                        }
                        push_html_cells(&mut line, src, &grid, row, side, iu);
                    }
                    line.push_str("</tr>");
                }
            }
            line.push('\n');
            w.write_all(line.as_bytes())?;
        }
        writeln!(w, "</tbody>")?;
        writeln!(w, "</table>")?;
    }
    style::html_end(w)?;
    Ok(())
}
