//! Side-by-side plain text: `line no | left text | marker | line no | right text` in `width` columns. Markers:
//! `' '` same, `'|'` changed, `'~'` unimportant change, `'<'` left only, `'>'` right only. Tabs are expanded to the
//! format's tab size, widths come from `unicode-width`, long lines wrap per [`Wrap`], and every run of rows hidden
//! by the display mode becomes one `… N lines hidden …` row.

use std::io::Write;
use std::ops::Range;

use hikasa_core::Side;
use hikasa_text::{
    Importance, Row, RowIdx, RowState, TextComparison, TextDocument, ViewItem, ViewOptions,
};

use crate::cells::{self, Atom, Atoms, Tone, Wrapper};
use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::options::Wrap;
use crate::style::Sgr;
use crate::text::{TextReportOptions, spec_text};

/// The marker of a row (`'~'` becomes `' '` when unimportant differences count as same, TC-024).
pub(crate) fn marker(row: &Row, ignore_unimportant: bool) -> char {
    match row.state {
        RowState::Same => ' ',
        RowState::Changed => match (row.importance, ignore_unimportant) {
            (Importance::Unimportant, true) => ' ',
            (Importance::Unimportant, false) => '~',
            _ => '|',
        },
        RowState::LeftOnly => '<',
        RowState::RightOnly => '>',
    }
}

/// The "N lines hidden" text of a gap.
pub(crate) fn hidden_text(rows: &Range<u32>) -> String {
    let n = rows.end.saturating_sub(rows.start);
    if n == 1 {
        "… 1 line hidden …".to_owned()
    } else {
        format!("… {n} lines hidden …")
    }
}

/// Number of decimal digits of `n` (1 for 0).
pub(crate) fn digits(n: u32) -> usize {
    n.checked_ilog10().map_or(1, |d| d as usize + 1)
}

/// The tones of both sides of a row for ANSI output (`[left, right]`, each the toned byte ranges and the base
/// tone): inline ranges on changed rows (unimportant ones dropped under ignore-unimportant), the whole line on
/// orphans, nothing on same rows and on unimportant rows under ignore-unimportant.
type SideTones = (Vec<(Range<u32>, Tone)>, Tone);

/// See [`SideTones`].
fn tones(cmp: &TextComparison, idx: RowIdx, row: &Row, ignore_unimportant: bool) -> [SideTones; 2] {
    let tone_of = |important: bool| {
        if important {
            Tone::Important
        } else {
            Tone::Unimportant
        }
    };
    let plain = || (Vec::new(), Tone::Plain);
    if row.state == RowState::Same
        || (ignore_unimportant && row.importance == Importance::Unimportant)
    {
        return [plain(), plain()];
    }
    if row.state != RowState::Changed {
        let base = tone_of(row.importance == Importance::Important);
        return [(Vec::new(), base), (Vec::new(), base)];
    }
    let Some(inline) = cmp.inline(idx) else {
        return [plain(), plain()];
    };
    let convert = |ranges: Vec<hikasa_text::InlineRange>| -> SideTones {
        (
            ranges
                .into_iter()
                .filter(|r| !ignore_unimportant || r.important)
                .map(|r| (r.range, tone_of(r.important)))
                .collect(),
            Tone::Plain,
        )
    };
    [convert(inline.left), convert(inline.right)]
}

/// Column geometry.
struct Geometry {
    /// Line-number width (0 without line numbers).
    numbers: usize,
    /// Width of each text column.
    text: usize,
}

impl Geometry {
    fn new(cmp: &TextComparison, opts: &TextReportOptions) -> Geometry {
        let numbers = if opts.line_numbers {
            let max = cmp
                .document(Side::Left)
                .line_count()
                .max(cmp.document(Side::Right).line_count());
            digits(max)
        } else {
            0
        };
        let fixed = if opts.line_numbers {
            2 * numbers + 5
        } else {
            3
        };
        Geometry {
            numbers,
            text: (usize::from(opts.width).saturating_sub(fixed) / 2).max(1),
        }
    }
}

/// One side of a row being wrapped.
struct SideText<'a> {
    wrapper: Option<Wrapper<'a>>,
    number: Option<u32>,
}

/// Writes the report.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    ansi: bool,
) -> Result<(), ReportError> {
    super::header(cmp, meta, &[]).write_text(w, "", ansi)?;
    let g = Geometry::new(cmp, opts);
    let iu = opts.ignore_unimportant;
    let tab = usize::from(cmp.format().text().tab_size);
    let docs: [&TextDocument; 2] = [cmp.document(Side::Left), cmp.document(Side::Right)];
    // Column titles: the two specs.
    let mut line = String::new();
    for (i, doc) in docs.iter().enumerate() {
        let title = spec_text(doc);
        let mut wrapper = Wrapper::new(
            Atoms::new(&title, tab, &[], Tone::Plain),
            g.text,
            Wrap::None,
        );
        let seg = wrapper.next_segment().unwrap_or_default();
        if g.numbers > 0 {
            cells::pad(&mut line, g.numbers + 1);
        }
        cells::push_segment(&mut line, &seg, false);
        if i == 0 {
            cells::pad(&mut line, g.text.saturating_sub(cells::segment_width(&seg)));
            line.push_str("   ");
        }
    }
    writeln!(w, "{}", line.trim_end())?;
    let view = cmp.view(&ViewOptions {
        filter: opts.display.text_filter(),
        ignore_unimportant: iu,
        expanded: Vec::new(),
    });
    let (grey, reset) = if ansi {
        (Sgr::GREY, Sgr::RESET)
    } else {
        ("", "")
    };
    for item in view.iter() {
        match item {
            ViewItem::Gap { rows } => {
                writeln!(w, "{grey}      {}{reset}", hidden_text(&rows))?;
            }
            ViewItem::Row { row: idx } => {
                let Some(row) = cmp.row(idx) else { continue };
                write_row(w, cmp, &docs, &g, opts, idx, &row, tab, ansi)?;
            }
        }
    }
    Ok(())
}

/// Writes the physical lines of one row.
#[allow(clippy::too_many_arguments)]
fn write_row(
    w: &mut dyn Write,
    cmp: &TextComparison,
    docs: &[&TextDocument; 2],
    g: &Geometry,
    opts: &TextReportOptions,
    idx: RowIdx,
    row: &Row,
    tab: usize,
    ansi: bool,
) -> Result<(), ReportError> {
    let iu = opts.ignore_unimportant;
    let m = marker(row, iu);
    let texts = [
        row.left.map(|l| docs[0].line(l)),
        row.right.map(|r| docs[1].line(r)),
    ];
    let tone_sets = if ansi {
        tones(cmp, idx, row, iu)
    } else {
        [(Vec::new(), Tone::Plain), (Vec::new(), Tone::Plain)]
    };
    let mut sides: Vec<SideText<'_>> = Vec::with_capacity(2);
    for (i, line) in [row.left, row.right].into_iter().enumerate() {
        let wrapper = texts[i].as_ref().map(|text| {
            let (ranges, base) = &tone_sets[i];
            Wrapper::new(Atoms::new(text, tab, ranges, *base), g.text, opts.wrap)
        });
        // The source line number: with a characters-per-line limit, the pieces of a broken line share the
        // number of the line they came from (TC-003).
        sides.push(SideText {
            wrapper,
            number: line.map(|l| docs[i].physical_line(l)),
        });
    }
    let marker_colour = match (ansi, m, row.importance) {
        (false, _, _) | (true, ' ', _) => "",
        (true, _, Importance::Unimportant) if iu => "",
        (true, _, Importance::Important) => Sgr::RED,
        (true, _, _) => Sgr::BLUE,
    };
    let mut first = true;
    loop {
        let segs: Vec<Option<Vec<Atom>>> = sides
            .iter_mut()
            .map(|s| s.wrapper.as_mut().and_then(Wrapper::next_segment))
            .collect();
        if !first && segs.iter().all(Option::is_none) {
            break;
        }
        let mut line = String::with_capacity(g.text * 2 + 16);
        for (i, seg) in segs.iter().enumerate() {
            if g.numbers > 0 {
                match (first, sides[i].number) {
                    (true, Some(n)) => line.push_str(&format!("{n:>width$}", width = g.numbers)),
                    _ => cells::pad(&mut line, g.numbers),
                }
                line.push(' ');
            }
            let seg = seg.as_deref().unwrap_or(&[]);
            cells::push_segment(&mut line, seg, ansi);
            if i == 0 {
                cells::pad(&mut line, g.text.saturating_sub(cells::segment_width(seg)));
                line.push(' ');
                if marker_colour.is_empty() {
                    line.push(m);
                } else {
                    line.push_str(marker_colour);
                    line.push(m);
                    line.push_str(Sgr::RESET);
                }
                line.push(' ');
            }
        }
        writeln!(w, "{}", line.trim_end_matches(' '))?;
        first = false;
        if opts.wrap == Wrap::None {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(digits(0), 1);
        assert_eq!(digits(9), 1);
        assert_eq!(digits(10), 2);
        assert_eq!(digits(1_000_000), 7);
        assert_eq!(hidden_text(&(3..4)), "… 1 line hidden …");
        assert_eq!(hidden_text(&(0..12)), "… 12 lines hidden …");
        let row = |state, importance| Row {
            left: None,
            right: None,
            state,
            importance,
        };
        assert_eq!(marker(&row(RowState::Same, Importance::None), false), ' ');
        assert_eq!(
            marker(&row(RowState::Changed, Importance::Important), true),
            '|'
        );
        assert_eq!(
            marker(&row(RowState::Changed, Importance::Unimportant), false),
            '~'
        );
        assert_eq!(
            marker(&row(RowState::Changed, Importance::Unimportant), true),
            ' '
        );
        assert_eq!(
            marker(&row(RowState::LeftOnly, Importance::Unimportant), true),
            '<'
        );
        assert_eq!(
            marker(&row(RowState::RightOnly, Importance::Important), false),
            '>'
        );
    }
}
