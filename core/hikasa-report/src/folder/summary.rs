//! The folder summary (13-reports, FC-064): the verdict, completeness, file and folder pairs per category, the
//! totals of each side and the content-test results, as text or HTML. Display filters never change it (D-03);
//! ignore-unimportant does.

use std::io::Write;

use hikasa_core::{CompareVerdict, ContentKind};
use hikasa_folder::{Category, FolderComparison, FolderStats};

use crate::error::ReportError;
use crate::escape;
use crate::folder::FolderReportOptions;
use crate::meta::ReportMeta;
use crate::style::{self, Sgr, Sheet};

/// Words of a folder verdict.
pub(crate) fn verdict_words(v: CompareVerdict) -> &'static str {
    match v {
        CompareVerdict::NoDifferences => "no differences",
        CompareVerdict::UnimportantOnly => "unimportant differences only",
        CompareVerdict::Differences => "differences",
        CompareVerdict::Incomplete => "incomplete",
    }
}

/// Row titles of the categories.
fn category_title(c: Category) -> &'static str {
    match c {
        Category::Same => "Same",
        Category::LeftNewer => "Left newer",
        Category::RightNewer => "Right newer",
        Category::Different => "Different",
        Category::LeftOrphan => "Left only",
        Category::RightOrphan => "Right only",
        Category::Unknown => "Unknown",
        Category::Error => "Errors",
        Category::Filtered => "Filtered",
    }
}

/// Row titles of the content results.
fn content_title(k: ContentKind) -> &'static str {
    match k {
        ContentKind::BinarySame => "Binary same",
        ContentKind::RulesSame => "Rules same",
        ContentKind::UnimportantOnly => "Unimportant differences only",
        ContentKind::ImportantDifferences => "Important differences",
        ContentKind::BinaryDifferent => "Binary different",
        ContentKind::Error => "Errors",
    }
}

/// A titled table: column titles and rows of `(title, cells)`.
struct Table {
    caption: &'static str,
    columns: Vec<&'static str>,
    rows: Vec<(&'static str, Vec<String>)>,
}

/// The three tables of the summary.
fn tables(stats: &FolderStats) -> [Table; 3] {
    let categories = Table {
        caption: "Items",
        columns: vec!["Category", "Files", "Folders"],
        rows: Category::ALL
            .iter()
            .map(|&c| {
                (
                    category_title(c),
                    vec![
                        stats.files.get(c).to_string(),
                        stats.folders.get(c).to_string(),
                    ],
                )
            })
            .collect(),
    };
    let totals = Table {
        caption: "Totals",
        columns: vec!["", "Left", "Right"],
        rows: vec![
            (
                "Files",
                vec![stats.left.files.to_string(), stats.right.files.to_string()],
            ),
            (
                "Folders",
                vec![
                    stats.left.folders.to_string(),
                    stats.right.folders.to_string(),
                ],
            ),
            (
                "Bytes",
                vec![stats.left.bytes.to_string(), stats.right.bytes.to_string()],
            ),
        ],
    };
    let mut content_rows: Vec<(&'static str, Vec<String>)> = ContentKind::ALL
        .iter()
        .map(|&k| (content_title(k), vec![stats.content.get(k).to_string()]))
        .collect();
    content_rows.push(("Not compared", vec![stats.content.not_compared.to_string()]));
    let content = Table {
        caption: "Content comparisons",
        columns: vec!["Result", "Pairs"],
        rows: content_rows,
    };
    [categories, totals, content]
}

/// The verdict and completeness lines.
fn verdict_lines(
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
) -> Vec<(&'static str, String)> {
    let iu = opts.display.ignore_unimportant;
    vec![
        ("Verdict", verdict_words(cmp.verdict(iu)).to_owned()),
        (
            "Ignore unimportant",
            if iu {
                "yes (unimportant differences count as same)"
            } else {
                "no"
            }
            .to_owned(),
        ),
        (
            "Complete",
            if cmp.is_complete() { "yes" } else { "no" }.to_owned(),
        ),
    ]
}

/// Writes the text summary.
pub(super) fn write_text(
    w: &mut dyn Write,
    cmp: &FolderComparison,
    opts: &FolderReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    super::header(cmp, meta).write_text(w, "", ansi)?;
    let iu = opts.display.ignore_unimportant;
    for (label, value) in verdict_lines(cmp, opts) {
        let colour = match (ansi, label, cmp.verdict(iu)) {
            (true, "Verdict", CompareVerdict::Differences) => Sgr::RED,
            (true, "Verdict", CompareVerdict::UnimportantOnly) => Sgr::BLUE,
            (true, "Verdict", CompareVerdict::Incomplete) => Sgr::GREY,
            _ => "",
        };
        let reset = if colour.is_empty() { "" } else { Sgr::RESET };
        writeln!(w, "{:<20}{colour}{value}{reset}", format!("{label}:"))?;
    }
    for table in tables(&cmp.stats(iu)) {
        writeln!(w)?;
        let first = table
            .rows
            .iter()
            .map(|(t, _)| t.len())
            .chain(std::iter::once(
                table.columns.first().map_or(0, |c| c.len()),
            ))
            .max()
            .unwrap_or(0);
        let widths: Vec<usize> = (1..table.columns.len())
            .map(|k| {
                table
                    .rows
                    .iter()
                    .map(|(_, cells)| cells.get(k - 1).map_or(0, String::len))
                    .chain(std::iter::once(table.columns[k].len()))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        writeln!(w, "{}:", table.caption)?;
        let mut line = format!("  {:<first$}", table.columns.first().copied().unwrap_or(""));
        for (k, width) in widths.iter().enumerate() {
            line.push_str(&format!("  {:>width$}", table.columns[k + 1]));
        }
        writeln!(w, "{}", line.trim_end())?;
        for (title, cells) in &table.rows {
            let mut line = format!("  {title:<first$}");
            for (cell, width) in cells.iter().zip(&widths) {
                line.push_str(&format!("  {cell:>width$}"));
            }
            writeln!(w, "{}", line.trim_end())?;
        }
    }
    Ok(())
}

/// Writes the HTML summary.
pub(super) fn write_html(
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
    writeln!(w, "<table class=\"summary\">")?;
    for (label, value) in verdict_lines(cmp, opts) {
        writeln!(
            w,
            "<tr><th>{}</th><td>{}</td></tr>",
            escape::html(label),
            escape::html(&value)
        )?;
    }
    writeln!(w, "</table>")?;
    for table in tables(&cmp.stats(opts.display.ignore_unimportant)) {
        writeln!(w, "<h2>{}</h2>", escape::html(table.caption))?;
        writeln!(w, "<table class=\"summary\">")?;
        let mut head = String::from("<tr>");
        for c in &table.columns {
            head.push_str(&format!("<th>{}</th>", escape::html(c)));
        }
        head.push_str("</tr>");
        writeln!(w, "{head}")?;
        for (title, cells) in &table.rows {
            let mut row = format!("<tr><th>{}</th>", escape::html(title));
            for cell in cells {
                row.push_str(&format!("<td class=\"num\">{}</td>", escape::html(cell)));
            }
            row.push_str("</tr>");
            writeln!(w, "{row}")?;
        }
        writeln!(w, "</table>")?;
    }
    style::html_end(w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wording() {
        assert_eq!(verdict_words(CompareVerdict::Incomplete), "incomplete");
        assert_eq!(category_title(Category::LeftOrphan), "Left only");
        assert_eq!(content_title(ContentKind::RulesSame), "Rules same");
        let t = tables(&FolderStats::default());
        assert_eq!(t[0].rows.len(), 9);
        assert_eq!(t[1].rows.len(), 3);
        assert_eq!(t[2].rows.len(), 7);
    }
}
