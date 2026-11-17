//! The hex summary (P0, plan A10): the verdict, the alignment, the statistics and the first `max_sections` sections
//! with their previews, as text or HTML. The display words never change it.

use std::io::Write;

use hikasa_core::ContentResult;

use super::{HexReportOptions, HexSource, header, input_error, section_line};
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::style::{self, Sgr, Sheet};

/// One line: label, value, colour under ANSI.
type Line = (&'static str, String, &'static str);

/// The colour of a verdict under ANSI.
fn verdict_colour(result: &ContentResult) -> &'static str {
    match result {
        ContentResult::ImportantDifferences | ContentResult::BinaryDifferent => Sgr::RED,
        ContentResult::Error { .. } => Sgr::ERROR,
        _ => "",
    }
}

/// The statistics block.
fn lines(src: &HexSource<'_>) -> Vec<Line> {
    let a = src.alignment;
    let s = &a.stats;
    let verdict = src.verdict();
    let o = src.options;
    let mut out: Vec<Line> = vec![(
        "Verdict",
        crate::text::summary_verdict_words(&verdict),
        verdict_colour(&verdict),
    )];
    if src.stale {
        out.push((
            "Stale",
            "yes (an input changed during the comparison)".to_owned(),
            Sgr::RED,
        ));
    }
    out.push((
        "Alignment",
        format!(
            "{} (requested {}), block {}, skew {}, window {}, budget {} MiB, degraded {}",
            a.mode.as_str(),
            o.mode.as_str(),
            o.block,
            o.skew,
            o.window,
            o.budget_mib,
            if a.degraded { "yes" } else { "no" }
        ),
        "",
    ));
    out.push((
        "Bytes",
        format!("left {}, right {}", s.left_bytes, s.right_bytes),
        "",
    ));
    out.push(("Same bytes", s.same_bytes.to_string(), ""));
    out.push(("Differing bytes", s.differing_bytes.to_string(), ""));
    out.push(("Left-only bytes", s.left_only_bytes.to_string(), ""));
    out.push(("Right-only bytes", s.right_only_bytes.to_string(), ""));
    let stored = a.sections.len() as u64;
    let listed = match src.max_sections {
        0 => stored,
        n => stored.min(n),
    };
    let mut sections = format!("{} (listed {listed})", s.sections);
    if a.sections.truncated() {
        sections.push_str(", list truncated");
    }
    out.push(("Sections", sections, ""));
    out
}

/// The listed sections' lines.
fn section_lines(src: &HexSource<'_>) -> Result<Vec<String>, ReportError> {
    src.listed_sections()
        .map(|s| section_line(src, &s).map_err(|e| input_error(&e)))
        .collect()
}

/// Writes the text summary.
pub(super) fn write_text(
    w: &mut dyn Write,
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    header(src, opts, meta).write_text(w, "", ansi)?;
    let lines = lines(src);
    let width = lines.iter().map(|(l, ..)| l.len() + 1).max().unwrap_or(0);
    for (label, value, colour) in &lines {
        let colour = if ansi { *colour } else { "" };
        let reset = if colour.is_empty() { "" } else { Sgr::RESET };
        writeln!(
            w,
            "{:<width$}  {colour}{}{reset}",
            format!("{label}:"),
            escape::escape_controls(value)
        )?;
    }
    let sections = section_lines(src)?;
    if !sections.is_empty() {
        let (bold, reset) = if ansi {
            (Sgr::BOLD, Sgr::RESET)
        } else {
            ("", "")
        };
        writeln!(w)?;
        writeln!(w, "{bold}Sections{reset}")?;
        for line in sections {
            writeln!(w, "  {line}")?;
        }
    }
    Ok(())
}

/// Writes the HTML summary.
pub(super) fn write_html(
    w: &mut dyn Write,
    src: &HexSource<'_>,
    opts: &HexReportOptions,
    meta: &ReportMeta,
    mono: bool,
    style: &HtmlStyle,
) -> Result<(), ReportError> {
    let header = header(src, opts, meta);
    style::html_start(w, &header.title, &meta.generator, Sheet::Hex, mono, style)?;
    header.write_html(w)?;
    writeln!(w, "<table class=\"summary\">")?;
    for (label, value, _) in lines(src) {
        writeln!(
            w,
            "<tr><th>{}</th><td>{}</td></tr>",
            escape::html(label),
            escape::html(&value)
        )?;
    }
    writeln!(w, "</table>")?;
    let sections = section_lines(src)?;
    if !sections.is_empty() {
        writeln!(w, "<h2>Sections</h2>")?;
        writeln!(w, "<pre class=\"sections\">")?;
        for line in sections {
            writeln!(w, "{}", escape::html(&line))?;
        }
        writeln!(w, "</pre>")?;
    }
    style::html_end(w)?;
    Ok(())
}
