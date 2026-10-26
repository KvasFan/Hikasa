//! Side-by-side folder text: the RPT-004 header, a title row, then per visible row
//! `indent(depth) name  columns…  marker [status]  name  columns…`. A folder's Size is the total of the files
//! below it on that side (FC-040). Column widths come from `unicode-width` over every visible row (two passes,
//! each reading the model in chunks).

use std::io::Write;

use hikasa_folder::FolderComparison;

use crate::cells::{self, text_width};
use crate::error::ReportError;
use crate::folder::{
    CHUNK_ROWS, FolderColumns, FolderReportOptions, RowCells, SIDE_COLUMNS, colour_sgr, marker_sgr,
    row_cells, status_words, visible,
};
use crate::meta::ReportMeta;
use crate::style::Sgr;

/// Column widths of one side: the indented name, then each enabled column.
#[derive(Debug, Clone, Default)]
struct Widths {
    name: usize,
    columns: Vec<usize>,
}

/// The enabled per-side columns: `(title, right-aligned)`.
fn enabled(columns: FolderColumns) -> Vec<(&'static str, bool)> {
    SIDE_COLUMNS
        .iter()
        .filter(|(c, _, _)| columns.contains(*c))
        .map(|(_, title, right)| (*title, *right))
        .collect()
}

/// Appends `text` padded to `width` columns (left- or right-aligned).
fn push_cell(out: &mut String, text: &str, width: usize, right: bool) {
    let pad = width.saturating_sub(text_width(text));
    if right {
        cells::pad(out, pad);
        out.push_str(text);
    } else {
        out.push_str(text);
        cells::pad(out, pad);
    }
}

/// Appends one side: the indented name and the columns, each padded.
fn push_side(
    out: &mut String,
    indent: usize,
    side: Option<(&str, &[String])>,
    widths: &Widths,
    cols: &[(&'static str, bool)],
) {
    let (name, values) = side.unwrap_or(("", &[]));
    let mut first = String::with_capacity(indent + name.len());
    if !name.is_empty() {
        cells::pad(&mut first, indent);
        first.push_str(name);
    }
    push_cell(out, &first, widths.name, false);
    for (k, (_, right)) in cols.iter().enumerate() {
        out.push_str("  ");
        let v = values.get(k).map_or("", String::as_str);
        push_cell(out, v, widths.columns.get(k).copied().unwrap_or(0), *right);
    }
}

/// Writes the report.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    super::header(cmp, meta).write_text(w, "", ansi)?;
    let iu = opts.display.ignore_unimportant;
    let cols = enabled(opts.columns);
    let status = opts.columns.contains(FolderColumns::STATUS);
    let rows = visible(cmp, &opts.display);
    // Pass 1: widths (titles included).
    let title_widths = Widths {
        name: "Name".len(),
        columns: cols.iter().map(|(t, _)| t.len()).collect(),
    };
    let mut widths = [title_widths.clone(), title_widths];
    let mut status_width = if status { "Status".len() } else { 0 };
    for chunk in rows.chunks(CHUNK_ROWS) {
        let read = cmp.read();
        for row in chunk {
            let Some(cells) = row_cells(&read, *row, opts.columns, iu) else {
                continue;
            };
            if status {
                status_width = status_width.max(status_words(cells.marker).len());
            }
            for (i, side) in cells.sides.iter().enumerate() {
                let Some((name, values)) = side else { continue };
                let wd = &mut widths[i];
                wd.name = wd.name.max(2 * cells.depth as usize + text_width(name));
                for (k, v) in values.iter().enumerate() {
                    if let Some(slot) = wd.columns.get_mut(k) {
                        *slot = (*slot).max(text_width(v));
                    }
                }
            }
        }
    }
    // Title row.
    let mut line = String::new();
    let titles: Vec<String> = cols.iter().map(|(t, _)| (*t).to_owned()).collect();
    push_side(&mut line, 0, Some(("Name", &titles)), &widths[0], &cols);
    line.push_str("   ");
    if status {
        line.push(' ');
        push_cell(&mut line, "Status", status_width, false);
    }
    line.push_str("  ");
    push_side(&mut line, 0, Some(("Name", &titles)), &widths[1], &cols);
    writeln!(w, "{}", line.trim_end_matches(' '))?;
    // Pass 2: rows.
    for chunk in rows.chunks(CHUNK_ROWS) {
        let read = cmp.read();
        for row in chunk {
            let Some(cells) = row_cells(&read, *row, opts.columns, iu) else {
                continue;
            };
            line.clear();
            write_row(
                &mut line,
                &cells,
                &widths,
                &cols,
                status,
                status_width,
                ansi,
            );
            writeln!(w, "{}", line.trim_end_matches(' '))?;
        }
    }
    Ok(())
}

/// Appends one row.
fn write_row(
    line: &mut String,
    cells: &RowCells,
    widths: &[Widths; 2],
    cols: &[(&'static str, bool)],
    status: bool,
    status_width: usize,
    ansi: bool,
) {
    let indent = 2 * cells.depth as usize;
    for (i, width) in widths.iter().enumerate() {
        let side = cells.sides[i]
            .as_ref()
            .map(|(n, v)| (n.as_str(), v.as_slice()));
        let colour = if ansi {
            colour_sgr(cells.colours[i])
        } else {
            ""
        };
        line.push_str(colour);
        push_side(line, indent, side, width, cols);
        if !colour.is_empty() {
            line.push_str(Sgr::RESET);
        }
        if i == 0 {
            line.push_str("  ");
            let colour = if ansi { marker_sgr(cells.marker) } else { "" };
            line.push_str(colour);
            line.push(cells.marker);
            if status {
                line.push(' ');
                push_cell(line, status_words(cells.marker), status_width, false);
            }
            if !colour.is_empty() {
                line.push_str(Sgr::RESET);
            }
            line.push_str("  ");
        }
    }
}
