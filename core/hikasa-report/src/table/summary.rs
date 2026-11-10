//! The table summary (TB-025, P0): the verdict, the totals and one block of statistics per sheet pair, as text or
//! HTML. Display filters never change it; ignore-unimportant changes the verdict.

use std::io::Write;

use hikasa_core::ContentResult;
use hikasa_table::{SheetState, SheetStats};

use super::{
    TableReportOptions, TableSource, alignment_words, header, sheet_title, state_words,
    verdict_words,
};
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::style::{self, Sgr, Sheet};

/// One line of a block: label, value, numeric (HTML alignment), colour under ANSI.
type Line = (&'static str, String, bool, &'static str);

/// One block: its title (none for the verdict block) and its lines.
type Block = (Option<String>, Vec<Line>);

/// The lines of a sheet's (or the totals') statistics.
fn stats_lines(s: &SheetStats, out: &mut Vec<Line>) {
    let mut push = |label, value: String, numeric| out.push((label, value, numeric, ""));
    push(
        "Rows",
        format!(
            "left {}, right {}, pairs {}",
            s.left_rows, s.right_rows, s.rows
        ),
        false,
    );
    push("Same rows", s.same_rows.to_string(), true);
    push(
        "Changed rows",
        format!(
            "important {}, unimportant {}",
            s.changed_important, s.changed_unimportant
        ),
        false,
    );
    push(
        "Left-only rows",
        format!(
            "important {}, unimportant {}",
            s.left_only_important, s.left_only_unimportant
        ),
        false,
    );
    push(
        "Right-only rows",
        format!(
            "important {}, unimportant {}",
            s.right_only_important, s.right_only_unimportant
        ),
        false,
    );
    push(
        "Cells",
        format!(
            "compared {}, same {}, unimportant {}, important {}",
            s.cells_compared, s.cells_same, s.cells_unimportant, s.cells_important
        ),
        false,
    );
    push(
        "Columns",
        format!(
            "paired {}, left-only {}, right-only {}",
            s.columns_paired, s.columns_left_only, s.columns_right_only
        ),
        false,
    );
    push("Header differences", s.header_differences.to_string(), true);
    push(
        "Duplicate key rows",
        format!("left {}, right {}", s.duplicate_left, s.duplicate_right),
        false,
    );
}

/// The colour of a verdict under ANSI.
fn verdict_colour(result: &ContentResult) -> &'static str {
    match result {
        ContentResult::UnimportantOnly => Sgr::BLUE,
        ContentResult::ImportantDifferences | ContentResult::BinaryDifferent => Sgr::RED,
        ContentResult::Error { .. } => Sgr::ERROR,
        _ => "",
    }
}

/// The colour of a sheet state under ANSI.
const fn state_colour(state: SheetState, iu: bool) -> &'static str {
    match state {
        SheetState::Same => "",
        SheetState::Unimportant if iu => "",
        SheetState::Unimportant => Sgr::BLUE,
        SheetState::Important => Sgr::RED,
        SheetState::LeftOnly | SheetState::RightOnly => Sgr::MAGENTA,
    }
}

/// The blocks of the summary: the verdict, the totals, each sheet pair.
fn blocks(src: &TableSource<'_>, opts: &TableReportOptions) -> Vec<Block> {
    let iu = opts.ignore_unimportant;
    let cmp = src.comparison;
    let stats = cmp.stats();
    let verdict = src.verdict(iu);
    let ignore = if iu {
        "yes (unimportant differences count as same)"
    } else {
        "no"
    };
    let mut out: Vec<Block> = vec![(
        None,
        vec![
            (
                "Verdict",
                verdict_words(&verdict),
                false,
                verdict_colour(&verdict),
            ),
            ("Ignore unimportant", ignore.to_owned(), false, ""),
            (
                "Sheets",
                format!(
                    "paired {}, left-only {}, right-only {}, differing {}",
                    stats.sheets_paired,
                    stats.sheets_left_only,
                    stats.sheets_right_only,
                    stats.sheets_differing
                ),
                false,
                "",
            ),
        ],
    )];
    let mut totals = Vec::new();
    stats_lines(&stats.totals, &mut totals);
    out.push((Some("Totals".to_owned()), totals));
    for (i, sheet) in cmp.sheets().iter().enumerate() {
        let state = sheet.state();
        let shown = if iu && state == SheetState::Unimportant {
            "same (unimportant differences only)"
        } else {
            state_words(state)
        };
        let mut lines: Vec<Line> = vec![
            ("State", shown.to_owned(), false, state_colour(state, iu)),
            ("Alignment", alignment_words(sheet), false, ""),
        ];
        stats_lines(sheet.stats(), &mut lines);
        out.push((
            Some(format!("Sheet {}: {}", i + 1, sheet_title(src, i))),
            lines,
        ));
    }
    out
}

/// Writes the text summary.
pub(super) fn write_text(
    w: &mut dyn Write,
    src: &TableSource<'_>,
    opts: &TableReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    header(src, meta).write_text(w, "", ansi)?;
    let blocks = blocks(src, opts);
    let width = blocks
        .iter()
        .flat_map(|(_, lines)| lines.iter().map(|(l, ..)| l.len() + 1))
        .max()
        .unwrap_or(0);
    for (k, (title, lines)) in blocks.iter().enumerate() {
        let indent = if title.is_some() { "  " } else { "" };
        if let Some(title) = title {
            if k > 0 {
                writeln!(w)?;
            }
            let (bold, reset) = if ansi {
                (Sgr::BOLD, Sgr::RESET)
            } else {
                ("", "")
            };
            writeln!(w, "{bold}{}{reset}", escape::escape_controls(title))?;
        }
        for (label, value, _, colour) in lines {
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

/// Writes the HTML summary.
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
    for (title, lines) in blocks(src, opts) {
        if let Some(title) = title {
            writeln!(w, "<h2>{}</h2>", escape::html(&title))?;
        }
        writeln!(w, "<table class=\"summary\">")?;
        for (label, value, numeric, _) in lines {
            let class = if numeric { " class=\"num\"" } else { "" };
            writeln!(
                w,
                "<tr><th>{}</th><td{class}>{}</td></tr>",
                escape::html(label),
                escape::html(&value)
            )?;
        }
        writeln!(w, "</table>")?;
    }
    style::html_end(w)?;
    Ok(())
}
