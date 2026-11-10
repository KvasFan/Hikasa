//! Table interleaved (TB-025): per sheet pair one grid of the comparison's columns; a changed pair is its left row,
//! then its right row; an orphan is its own side's row; a same row is written once; a run of hidden rows is one
//! `… N rows hidden …` line. Plain text (columns fitted into the width, ANSI colours per cell) or HTML.

use std::io::Write;

use hikasa_core::Side;
use hikasa_table::{RowKind, RowPair};

use super::sbs::{
    Grid, LEGEND, cell_state, label_width, natural_widths, push_html_cells, push_html_th,
    row_class, same_label, section_title, sgr, side_cell, side_name, write_html_section,
};
use super::{
    Item, TableReportOptions, TableSource, fit, header, hidden_text, marker, share_widths,
};
use crate::cells::pad;
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::style::{self, Sgr, Sheet};

/// The lines of a shown row: `(side tag, side)` for each line it takes.
fn lines_of(r: &RowPair) -> &'static [(char, Side)] {
    match r.kind() {
        RowKind::Same => &[(' ', Side::Left)],
        RowKind::Changed => &[('L', Side::Left), ('R', Side::Right)],
        RowKind::LeftOnly => &[('L', Side::Left)],
        RowKind::RightOnly => &[('R', Side::Right)],
    }
}

/// The name of a column in the interleaved header: the left name, the right name when they differ (`a/b`).
fn header_name(src: &TableSource<'_>, grid: &Grid<'_>, col: &hikasa_table::ColumnPair) -> String {
    let l = side_name(src, grid, col, Side::Left);
    let r = side_name(src, grid, col, Side::Right);
    match (l, r) {
        (Some(l), Some(r)) if l != r => format!("{l}/{r}"),
        (Some(l), _) => l,
        (None, Some(r)) => r,
        (None, None) => String::new(),
    }
}

/// Writes the plain-text interleaved report.
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
        let nw = label_width(src, &grid, opts.row_numbers, true);
        let n = grid.columns.len();
        let numbers = if nw > 0 { nw + 1 } else { 0 };
        let fixed = 4 + numbers + n.saturating_sub(1);
        let room = usize::from(opts.width).saturating_sub(fixed).max(n);
        let mut natural = natural_widths(src, &grid);
        for (k, (_, col)) in grid.columns.iter().enumerate() {
            natural[k] = natural[k].max(super::width(&header_name(src, &grid, col)));
        }
        let widths = share_widths(&natural, room);
        line.clear();
        line.push_str("    ");
        pad(&mut line, numbers);
        for (c, (_, col)) in grid.columns.iter().enumerate() {
            if c > 0 {
                line.push(' ');
            }
            line.push_str(&fit(&header_name(src, &grid, col), widths[c]));
        }
        writeln!(w, "{bold}{}{reset}", line.trim_end())?;
        if grid.items.is_empty() {
            writeln!(w, "    {grey}0 rows{reset}")?;
        }
        for item in &grid.items {
            match item {
                Item::Gap(n) => {
                    writeln!(w, "    {grey}{}{reset}", hidden_text(*n))?;
                }
                Item::Row(i) => {
                    let Some(r) = grid.sheet.rows().get(*i as usize) else {
                        continue;
                    };
                    let m = marker(r, iu);
                    let mcolour = match (ansi, m) {
                        (true, '|') => Sgr::RED,
                        (true, '~') => Sgr::BLUE,
                        (true, '<' | '>') => Sgr::MAGENTA,
                        _ => "",
                    };
                    for (tag, side) in lines_of(r) {
                        line.clear();
                        line.push_str(mcolour);
                        line.push(m);
                        if !mcolour.is_empty() {
                            line.push_str(Sgr::RESET);
                        }
                        line.push(' ');
                        line.push(*tag);
                        line.push(' ');
                        if nw > 0 {
                            let label = same_label(src, pair, *i, r).or_else(|| {
                                src.comparison
                                    .row_label(pair, *i, *side)
                                    .map(|l| l.to_string())
                            });
                            match label {
                                Some(l) => line.push_str(&format!("{l:>nw$} ")),
                                None => pad(&mut line, nw + 1),
                            }
                        }
                        for (k, (column, col)) in grid.columns.iter().enumerate() {
                            if k > 0 {
                                line.push(' ');
                            }
                            match side_cell(src, &grid, *i, r, *column, col, *side) {
                                Some(text) => {
                                    let colour = if ansi {
                                        sgr(cell_state(&grid, *i, r, *column, iu))
                                    } else {
                                        ""
                                    };
                                    line.push_str(colour);
                                    line.push_str(&fit(&text, widths[k]));
                                    if !colour.is_empty() {
                                        line.push_str(Sgr::RESET);
                                    }
                                }
                                None => pad(&mut line, widths[k]),
                            }
                        }
                        let trimmed = line.trim_end_matches(' ').len();
                        line.truncate(trimmed);
                        line.push('\n');
                        w.write_all(line.as_bytes())?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Writes the HTML interleaved report.
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
        writeln!(w, "<table class=\"til\">")?;
        line.clear();
        line.push_str("<thead><tr><th class=\"m\"></th><th class=\"s\"></th>");
        if numbers {
            line.push_str("<th class=\"n\"></th>");
        }
        for (_, col) in &grid.columns {
            let side = if col.left.is_some() {
                Side::Left
            } else {
                Side::Right
            };
            push_html_th(&mut line, src, &grid, col, side);
        }
        line.push_str("</tr></thead>\n<tbody>\n");
        w.write_all(line.as_bytes())?;
        let span = 2 + usize::from(numbers) + grid.columns.len();
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
                        "<tr class=\"gap\"><td colspan=\"{span}\">{}</td></tr>\n",
                        hidden_text(*n)
                    ));
                }
                Item::Row(i) => {
                    let Some(r) = grid.sheet.rows().get(*i as usize) else {
                        continue;
                    };
                    let m = escape::html(&marker(r, iu).to_string()).into_owned();
                    for (tag, side) in lines_of(r) {
                        line.push_str("<tr");
                        line.push_str(row_class(r, iu));
                        line.push_str("><td class=\"m\">");
                        line.push_str(&m);
                        line.push_str("</td><td class=\"s\">");
                        if *tag != ' ' {
                            line.push(*tag);
                        }
                        line.push_str("</td>");
                        if numbers {
                            line.push_str("<td class=\"n\">");
                            let label = same_label(src, pair, *i, r).or_else(|| {
                                src.comparison
                                    .row_label(pair, *i, *side)
                                    .map(|l| l.to_string())
                            });
                            if let Some(l) = label {
                                line.push_str(&l);
                            }
                            line.push_str("</td>");
                        }
                        push_html_cells(&mut line, src, &grid, Some((*i, r)), *side, iu);
                        line.push_str("</tr>\n");
                    }
                }
            }
            w.write_all(line.as_bytes())?;
        }
        writeln!(w, "</tbody>")?;
        writeln!(w, "</table>")?;
    }
    style::html_end(w)?;
    Ok(())
}
