//! The unified diff: the CLI's default annotated form (hunk headers end in ` important` / ` unimportant`, which
//! patch readers take as the optional section text, so it stays a valid patch) and the plain patch. Context lines
//! are byte-identical rows; every other row of a hunk is written as a deletion and an insertion ([`patch`]).

use std::io::Write;

use hikasa_core::Warning;
use hikasa_text::TextComparison;

use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::style::Sgr;
use crate::text::TextReportOptions;
use crate::text::patch::{self, Lossy, Sides};

/// Writes the header (as `# ` comment lines), the file headers and every hunk.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    lines: u32,
    annotated: bool,
    ansi: bool,
) -> Result<Vec<Warning>, ReportError> {
    let sides = Sides::new(cmp);
    let hunks = patch::plan(cmp, &sides, lines, opts.ignore_unimportant, false);
    let warnings = Lossy::of_hunks(cmp, &sides, &hunks).warnings(&sides);
    super::header(cmp, meta, &warnings).write_text(w, "# ", ansi)?;
    if hunks.is_empty() {
        return Ok(warnings);
    }
    patch::file_headers(w, &sides, "--- ", "+++ ", ansi)?;
    let (cyan, reset) = if ansi {
        (Sgr::CYAN, Sgr::RESET)
    } else {
        ("", "")
    };
    for h in &hunks {
        let note = match (annotated, h.important()) {
            (false, _) => "",
            (true, true) => " important",
            (true, false) => " unimportant",
        };
        writeln!(
            w,
            "{cyan}@@ -{} +{} @@{note}{reset}",
            patch::unified_range(h.left),
            patch::unified_range(h.right)
        )?;
        let mut at = h.rows.start;
        for g in h
            .groups
            .iter()
            .map(|g| &g.rows)
            .chain(std::iter::once(&(h.rows.end..h.rows.end)))
        {
            // The identical rows before the group: context, from the left file (the bytes are the same).
            for row in cmp.rows(at..g.start) {
                if let Some(l) = row.left {
                    patch::write_line(w, &sides, 0, " ", l, ansi, "")?;
                }
            }
            for row in cmp.rows(g.clone()) {
                if let Some(l) = row.left {
                    let colour = patch::tone(ansi, patch::row_importance(&row));
                    patch::write_line(w, &sides, 0, "-", l, ansi, colour)?;
                }
            }
            for row in cmp.rows(g.clone()) {
                if let Some(r) = row.right {
                    let colour = patch::tone(ansi, patch::row_importance(&row));
                    patch::write_line(w, &sides, 1, "+", r, ansi, colour)?;
                }
            }
            at = g.end;
        }
    }
    Ok(warnings)
}
