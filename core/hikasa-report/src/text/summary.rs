//! The text summary (13-reports): the verdict and the counts of [`hikasa_text::TextStats`] (rows by state and
//! importance, sections, and the D-18 line counts), as text or HTML. Display filters never change it (D-03);
//! ignore-unimportant changes the verdict (TC-024).

use std::io::Write;

use hikasa_core::ContentResult;
use hikasa_text::{ChangeCounts, TextComparison};

use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::style::{self, Sgr, Sheet};
use crate::text::TextReportOptions;

/// Words of a verdict.
pub(crate) fn verdict_words(result: &ContentResult) -> String {
    match result {
        ContentResult::BinarySame => "identical (same bytes)".to_owned(),
        ContentResult::RulesSame => "same under the rules".to_owned(),
        ContentResult::UnimportantOnly => "unimportant differences only".to_owned(),
        ContentResult::ImportantDifferences => "important differences".to_owned(),
        ContentResult::BinaryDifferent => "binary content differs".to_owned(),
        ContentResult::Error { error } => format!("error: {}", error.message),
    }
}

/// `added A, deleted D, changed C, original O`.
fn counts(c: &ChangeCounts) -> String {
    format!(
        "added {}, deleted {}, changed {}, original {}",
        c.added, c.deleted, c.changed, c.original
    )
}

/// The summary lines: `(label, value, numeric)`.
fn lines(cmp: &TextComparison, opts: &TextReportOptions) -> Vec<(&'static str, String, bool)> {
    let s = cmp.stats();
    let iu = opts.ignore_unimportant;
    let ignore = if iu {
        "yes (unimportant differences count as same)"
    } else {
        "no"
    };
    vec![
        ("Verdict", verdict_words(&cmp.verdict(iu)), false),
        ("Ignore unimportant", ignore.to_owned(), false),
        ("Rows", s.rows.to_string(), true),
        ("Same rows", s.same_rows.to_string(), true),
        ("Changed rows", s.changed_rows.to_string(), true),
        ("Left-only rows", s.left_only_rows.to_string(), true),
        ("Right-only rows", s.right_only_rows.to_string(), true),
        ("Important rows", s.important_rows.to_string(), true),
        ("Unimportant rows", s.unimportant_rows.to_string(), true),
        ("Sections", s.sections.to_string(), true),
        ("Important sections", s.important_sections.to_string(), true),
        (
            "Unimportant sections",
            s.sections.saturating_sub(s.important_sections).to_string(),
            true,
        ),
        ("Important lines", counts(&s.important), false),
        ("Unimportant lines", counts(&s.unimportant), false),
        ("Left lines", s.left_lines.to_string(), true),
        ("Right lines", s.right_lines.to_string(), true),
    ]
}

/// The colour of the verdict under ANSI.
fn verdict_colour(result: &ContentResult) -> &'static str {
    match result {
        ContentResult::BinarySame | ContentResult::RulesSame => "",
        ContentResult::UnimportantOnly => Sgr::BLUE,
        ContentResult::ImportantDifferences | ContentResult::BinaryDifferent => Sgr::RED,
        ContentResult::Error { .. } => Sgr::ERROR,
    }
}

/// Writes the text summary.
pub(super) fn write_text(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    super::header(cmp, meta, &[]).write_text(w, "", ansi)?;
    let lines = lines(cmp, opts);
    let width = lines.iter().map(|(l, _, _)| l.len() + 1).max().unwrap_or(0);
    for (i, (label, value, _)) in lines.iter().enumerate() {
        let colour = if ansi && i == 0 {
            verdict_colour(&cmp.verdict(opts.ignore_unimportant))
        } else {
            ""
        };
        let reset = if colour.is_empty() { "" } else { Sgr::RESET };
        writeln!(
            w,
            "{:<width$}  {colour}{}{reset}",
            format!("{label}:"),
            escape::escape_controls(value)
        )?;
    }
    Ok(())
}

/// Writes the HTML summary.
pub(super) fn write_html(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    mono: bool,
    custom_css: Option<&str>,
) -> Result<(), ReportError> {
    let header = super::header(cmp, meta, &[]);
    style::html_start(
        w,
        &header.title,
        &meta.generator,
        Sheet::Text,
        mono,
        custom_css,
    )?;
    header.write_html(w)?;
    writeln!(w, "<table class=\"summary\">")?;
    for (label, value, numeric) in lines(cmp, opts) {
        let class = if numeric { " class=\"num\"" } else { "" };
        writeln!(
            w,
            "<tr><th>{}</th><td{class}>{}</td></tr>",
            escape::html(label),
            escape::html(&value)
        )?;
    }
    writeln!(w, "</table>")?;
    style::html_end(w)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_core::{ErrorKind, ErrorReport};

    #[test]
    fn verdict_wording() {
        assert_eq!(
            verdict_words(&ContentResult::BinarySame),
            "identical (same bytes)"
        );
        assert_eq!(
            verdict_words(&ContentResult::UnimportantOnly),
            "unimportant differences only"
        );
        let e = ContentResult::Error {
            error: ErrorReport {
                kind: ErrorKind::Io,
                message: "boom".into(),
                spec: None,
                causes: vec![],
            },
        };
        assert_eq!(verdict_words(&e), "error: boom");
        assert_eq!(verdict_colour(&e), Sgr::ERROR);
        assert_eq!(verdict_colour(&ContentResult::RulesSame), "");
    }
}
