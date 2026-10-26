//! The normal diff: one command per group of differing rows (rows that are not byte-identical, [`patch`]; for a
//! `lines_independent` format, one per row, like its sections, §2.17.2): `<l>a<r1>,<r2>`, `<l1>,<l2>d<r>` or
//! `<l1>,<l2>c<r1>,<r2>`, followed by the old lines (`< `), `---` for a change, and the new lines (`> `).
//! Ignore-unimportant leaves out the commands without an important row.

use std::io::Write;

use hikasa_core::Warning;
use hikasa_text::TextComparison;

use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::style::Sgr;
use crate::text::TextReportOptions;
use crate::text::patch::{self, Lossy, Sides};

/// Writes the header (as `# ` comment lines) and one command per group.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<Vec<Warning>, ReportError> {
    let sides = Sides::new(cmp);
    let split_rows = cmp.format().text().lines_independent;
    let commands = patch::plan(cmp, &sides, 0, opts.ignore_unimportant, split_rows);
    let warnings = Lossy::of_hunks(cmp, &sides, &commands).warnings(&sides);
    super::header(cmp, meta, &warnings).write_text(w, "# ", ansi)?;
    let (cyan, reset) = if ansi {
        (Sgr::CYAN, Sgr::RESET)
    } else {
        ("", "")
    };
    for c in &commands {
        let (l, r) = (c.left, c.right);
        let command = match (l.len, r.len) {
            (0, _) => format!("{}a{}", l.start, patch::line_range(r)),
            (_, 0) => format!("{}d{}", patch::line_range(l), r.start),
            _ => format!("{}c{}", patch::line_range(l), patch::line_range(r)),
        };
        writeln!(w, "{cyan}{command}{reset}")?;
        let colour = patch::tone(ansi, c.importance);
        for row in cmp.rows(c.rows.clone()) {
            if let Some(line) = row.left {
                patch::write_line(w, &sides, 0, "< ", line, ansi, colour)?;
            }
        }
        if l.len > 0 && r.len > 0 {
            writeln!(w, "---")?;
        }
        for row in cmp.rows(c.rows.clone()) {
            if let Some(line) = row.right {
                patch::write_line(w, &sides, 1, "> ", line, ansi, colour)?;
            }
        }
    }
    Ok(warnings)
}
