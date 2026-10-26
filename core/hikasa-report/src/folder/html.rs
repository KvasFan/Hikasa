//! Side-by-side folder HTML: a self-contained document with the RPT-004 header, a colour legend (FC-031) and one
//! table row per visible row; each side's cells carry its colour class, the marker cell the category's class,
//! and the name cell its depth as left padding.

use std::io::Write;

use hikasa_folder::FolderComparison;

use crate::error::ReportError;
use crate::escape;
use crate::folder::{
    CHUNK_ROWS, FolderColumns, FolderReportOptions, SIDE_COLUMNS, colour_class, marker_class,
    row_cells, status_words, visible,
};
use crate::meta::ReportMeta;
use crate::style::{self, Sheet};

/// The legend (FC-031 colours).
const LEGEND: &str = "<table class=\"legend\">\n<tr><td class=\"c-same\">Same</td>\
<td class=\"c-newer\">Newer or different</td><td class=\"c-unimp\">Unimportant differences only</td>\
<td class=\"c-orphan\">Orphan</td><td class=\"c-older\">Older or unknown</td>\
<td class=\"c-filtered\">Filtered</td><td class=\"c-error\">Error</td></tr>\n</table>\n";

/// Writes the document.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    mono: bool,
    custom_css: Option<&str>,
) -> Result<(), ReportError> {
    let header = super::header(cmp, meta);
    style::html_start(
        w,
        &header.title,
        &meta.generator,
        Sheet::Folder,
        mono,
        custom_css,
    )?;
    header.write_html(w)?;
    w.write_all(LEGEND.as_bytes())?;
    let iu = opts.display.ignore_unimportant;
    let status = opts.columns.contains(FolderColumns::STATUS);
    let cols: Vec<(&str, bool)> = SIDE_COLUMNS
        .iter()
        .filter(|(c, _, _)| opts.columns.contains(*c))
        .map(|(_, t, r)| (*t, *r))
        .collect();
    writeln!(w, "<table class=\"fsbs\">")?;
    let mut head = String::from("<thead><tr>");
    for i in 0..2 {
        head.push_str("<th>Name</th>");
        for (title, _) in &cols {
            head.push_str("<th>");
            head.push_str(title);
            head.push_str("</th>");
        }
        if i == 0 {
            head.push_str("<th class=\"m\"></th>");
            if status {
                head.push_str("<th>Status</th>");
            }
        }
    }
    head.push_str("</tr></thead>");
    writeln!(w, "{head}")?;
    writeln!(w, "<tbody>")?;
    let rows = visible(cmp, &opts.display);
    let mut line = String::new();
    for chunk in rows.chunks(CHUNK_ROWS) {
        let read = cmp.read();
        for row in chunk {
            let Some(cells) = row_cells(&read, *row, opts.columns, iu) else {
                continue;
            };
            line.clear();
            line.push_str("<tr>");
            for i in 0..2 {
                let class = colour_class(cells.colours[i]);
                match &cells.sides[i] {
                    Some((name, values)) => {
                        line.push_str(&format!(
                            "<td class=\"name {class}\" style=\"padding-left:{}ch\">{}</td>",
                            1 + 2 * u64::from(cells.depth),
                            escape::html(name)
                        ));
                        for ((_, right), v) in cols.iter().zip(values) {
                            let num = if *right { " num" } else { "" };
                            line.push_str(&format!(
                                "<td class=\"{class}{num}\">{}</td>",
                                escape::html(v)
                            ));
                        }
                    }
                    None => {
                        line.push_str("<td class=\"name c-absent\"></td>");
                        for _ in &cols {
                            line.push_str("<td class=\"c-absent\"></td>");
                        }
                    }
                }
                if i == 0 {
                    let mclass = marker_class(cells.marker);
                    line.push_str(&format!("<td class=\"m {mclass}\">"));
                    escape::push_html_char(&mut line, cells.marker);
                    line.push_str("</td>");
                    if status {
                        line.push_str(&format!(
                            "<td class=\"{mclass}\">{}</td>",
                            status_words(cells.marker)
                        ));
                    }
                }
            }
            line.push_str("</tr>\n");
            w.write_all(line.as_bytes())?;
        }
    }
    writeln!(w, "</tbody>")?;
    writeln!(w, "</table>")?;
    style::html_end(w)?;
    Ok(())
}
