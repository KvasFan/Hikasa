//! The context diff (`diff -c` layout): `***************` hunk separators, `*** a,b ****` with the old lines and
//! `--- c,d ----` with the new ones (an empty range after line `l` is `l+1,l`, [`patch::context_range`]); a
//! group of differing rows holding lines of both sides is marked `!`, a pure deletion `-`, a pure insertion `+`,
//! a byte-identical row `  ` (context, always written from the left file). A part without changes is written as
//! its range line only (patch programs then take its lines from the other part's context, which is the same
//! bytes).

use std::io::Write;

use hikasa_core::Warning;
use hikasa_text::TextComparison;

use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::style::Sgr;
use crate::text::TextReportOptions;
use crate::text::patch::{self, Hunk, Lossy, Sides};

/// Writes one part (`s` = 0 old, 1 new) of a hunk: context rows from the left file, the group rows of side `s`
/// with their group's marker.
fn write_part(
    w: &mut dyn Write,
    cmp: &TextComparison,
    sides: &Sides<'_>,
    h: &Hunk,
    s: usize,
    ansi: bool,
) -> std::io::Result<()> {
    let mut at = h.rows.start;
    let end = h.rows.end..h.rows.end;
    for g in h.groups.iter().map(Some).chain(std::iter::once(None)) {
        let rows = g.map_or(&end, |g| &g.rows);
        for row in cmp.rows(at..rows.start) {
            if let Some(l) = row.left {
                patch::write_line(w, sides, 0, "  ", l, ansi, "")?;
            }
        }
        if let Some(g) = g {
            let marker = match (g.left > 0, g.right > 0, s) {
                (true, true, _) => "! ",
                (_, _, 0) => "- ",
                _ => "+ ",
            };
            for row in cmp.rows(g.rows.clone()) {
                let line = if s == 0 { row.left } else { row.right };
                if let Some(line) = line {
                    let colour = patch::tone(ansi, patch::row_importance(&row));
                    patch::write_line(w, sides, s, marker, line, ansi, colour)?;
                }
            }
        }
        at = rows.end;
    }
    Ok(())
}

/// Writes the header (as `# ` comment lines), the file headers and every hunk.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    lines: u32,
    ansi: bool,
) -> Result<Vec<Warning>, ReportError> {
    let sides = Sides::new(cmp);
    let hunks = patch::plan(cmp, &sides, lines, opts.ignore_unimportant, false);
    let warnings = Lossy::of_hunks(cmp, &sides, &hunks).warnings(&sides);
    super::header(cmp, meta, &warnings).write_text(w, "# ", ansi)?;
    if hunks.is_empty() {
        return Ok(warnings);
    }
    patch::file_headers(w, &sides, "*** ", "--- ", ansi)?;
    let (cyan, reset) = if ansi {
        (Sgr::CYAN, Sgr::RESET)
    } else {
        ("", "")
    };
    for h in &hunks {
        writeln!(w, "{cyan}***************{reset}")?;
        // A part lists its lines only when some group holds lines of its side.
        let listed = [
            h.groups.iter().any(|g| g.left > 0),
            h.groups.iter().any(|g| g.right > 0),
        ];
        for (s, open, close, span) in [
            (0usize, "*** ", " ****", h.left),
            (1, "--- ", " ----", h.right),
        ] {
            writeln!(
                w,
                "{cyan}{open}{}{close}{reset}",
                patch::context_range(span)
            )?;
            if listed[s] {
                write_part(w, cmp, &sides, h, s, ansi)?;
            }
        }
    }
    Ok(warnings)
}
