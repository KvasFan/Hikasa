//! The interleaved layout (M2 W7, D-197): one column of lines in row order. A same row is one line with both numbers;
//! a changed row is its left line, then its right line; an orphan is its own side's line; a run of hidden rows is one
//! `… N lines hidden …` row. Pieces of a line broken by the characters-per-line limit keep the line's number.
//!
//! Text: `<left number> <right number> <mark> <text>`, the numbers right-aligned to the widest number of the
//! comparison and blank when absent (left out without line numbers); the mark is the side (` ` both, `-` left, `+`
//! right) and the importance (` ` same, `!` important, `~` unimportant, ` ` for an unimportant difference under
//! ignore-unimportant). The text wraps in `width − gutter` columns with the side-by-side functions. Strike-out writes a
//! struck span `[-…-]` in plain text and SGR 9 with ANSI. HTML: one self-contained document with a table `il` (left
//! number, right number, text), rows classed `imp`/`unimp`, `l`/`r`, `lo`/`ro`, `gap`, inline differences
//! `<span class="imp|unimp">`, struck ones `<del class="imp|unimp">`.

use std::fmt::Write as _;
use std::io::Write;
use std::ops::Range;

use hikasa_core::Side;
use hikasa_text::{
    Importance, InlineRange, Row, RowIdx, RowState, TextComparison, TextDocument, ViewItem,
};

use crate::cells::{self, Atoms, Tone, Wrapper};
use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::options::HtmlStyle;
use crate::request::StrikeOut;
use crate::style::{self, Sgr, Sheet};
use crate::text::sbs::{digits, hidden_text, tones};
use crate::text::{Items, NO_ROWS, TextReportOptions};

/// The legend of the HTML form.
const LEGEND: &str = "<table class=\"legend\">\n<tr><td class=\"imp\">Important difference</td>\
<td class=\"unimp\">Unimportant difference</td><td class=\"lo\">Orphan (one side only)</td>\
<td class=\"gap\">Hidden lines</td>";

/// One line of the column: which side, which document line, the row it belongs to.
#[derive(Debug, Clone, Copy)]
struct Line {
    /// `None`: the same row's one line (both numbers); else the side of the line.
    side: Option<Side>,
    left: Option<u32>,
    right: Option<u32>,
    /// The line shown.
    line: hikasa_text::LineIdx,
    /// Index of its side (0 left, 1 right).
    at: usize,
}

/// The lines of one row, in order.
fn lines_of(row: &Row, docs: &[&TextDocument; 2]) -> Vec<Line> {
    let number = |i: usize, l: Option<hikasa_text::LineIdx>| l.map(|l| docs[i].physical_line(l));
    match (row.state, row.left, row.right) {
        (RowState::Same, Some(l), _) => vec![Line {
            side: None,
            left: number(0, row.left),
            right: number(1, row.right),
            line: l,
            at: 0,
        }],
        _ => {
            let mut out = Vec::with_capacity(2);
            if let Some(l) = row.left {
                out.push(Line {
                    side: Some(Side::Left),
                    left: number(0, Some(l)),
                    right: None,
                    line: l,
                    at: 0,
                });
            }
            if let Some(r) = row.right {
                out.push(Line {
                    side: Some(Side::Right),
                    left: None,
                    right: number(1, Some(r)),
                    line: r,
                    at: 1,
                });
            }
            out
        }
    }
}

/// The importance character of a row's mark.
fn importance_char(row: &Row, ignore_unimportant: bool) -> char {
    match (row.state, row.importance) {
        (RowState::Same, _) => ' ',
        (_, Importance::Unimportant) if ignore_unimportant => ' ',
        (_, Importance::Important) => '!',
        (_, _) => '~',
    }
}

/// The two-character mark of a line.
fn mark(line: &Line, row: &Row, ignore_unimportant: bool) -> [char; 2] {
    let side = match line.side {
        None => ' ',
        Some(Side::Right) => '+',
        Some(_) => '-',
    };
    [side, importance_char(row, ignore_unimportant)]
}

/// The bit of a side in a strike-out.
fn strike_bit(at: usize) -> StrikeOut {
    if at == 0 {
        StrikeOut::LEFT
    } else {
        StrikeOut::RIGHT
    }
}

/// `text` with `[-` and `-]` around the struck ranges (sorted, disjoint byte ranges on char boundaries).
fn bracket(text: &str, struck: &[Range<u32>]) -> String {
    let mut out = String::with_capacity(text.len() + 4 * struck.len());
    let mut at = 0usize;
    for r in struck {
        let (start, end) = (r.start as usize, (r.end as usize).min(text.len()));
        if start >= end || start < at {
            continue;
        }
        out.push_str(text.get(at..start).unwrap_or_default());
        out.push_str("[-");
        out.push_str(text.get(start..end).unwrap_or_default());
        out.push_str("-]");
        at = end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    out
}

/// The struck byte ranges of side `at` of a row: its toned ranges (the inline differences that count), or the whole
/// line of a toned orphan; nothing for a side that is not struck.
fn struck_ranges(tone: &(Vec<(Range<u32>, Tone)>, Tone), text: &str) -> Vec<Range<u32>> {
    let (ranges, base) = tone;
    if *base != Tone::Plain {
        return if text.is_empty() {
            Vec::new()
        } else {
            std::iter::once(0..text.len() as u32).collect()
        };
    }
    ranges.iter().map(|(r, _)| r.clone()).collect()
}

/// Writes the text form.
pub(super) fn write_text(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    items: &Items,
    ansi: bool,
) -> Result<(), ReportError> {
    super::header(cmp, meta, &[]).write_text(w, "", ansi)?;
    let docs: [&TextDocument; 2] = [cmp.document(Side::Left), cmp.document(Side::Right)];
    let numbers = if opts.line_numbers {
        digits(docs[0].line_count().max(docs[1].line_count()))
    } else {
        0
    };
    let gutter = if opts.line_numbers {
        2 * numbers + 5
    } else {
        3
    };
    let width = usize::from(opts.width).saturating_sub(gutter).max(1);
    let tab = usize::from(cmp.format().text().tab_size);
    let iu = opts.ignore_unimportant;
    let (grey, reset) = if ansi {
        (Sgr::GREY, Sgr::RESET)
    } else {
        ("", "")
    };
    let mut out = String::new();
    if items.nothing() {
        writeln!(w, "{grey}{:gutter$}{NO_ROWS}{reset}", "")?;
    }
    for item in items.iter() {
        match item {
            ViewItem::Gap { rows } => {
                writeln!(w, "{grey}{:gutter$}{}{reset}", "", hidden_text(&rows))?;
            }
            ViewItem::Row { row: idx } => {
                let Some(row) = cmp.row(idx) else { continue };
                let row_tones = tones(cmp, idx, &row, iu);
                for line in lines_of(&row, &docs) {
                    let text = docs[line.at].line(line.line);
                    let struck = opts.strike.contains(strike_bit(line.at)) && line.side.is_some();
                    let (ranges, base) = &row_tones[line.at];
                    let shown;
                    let (text_ref, tone_list, base): (&str, Vec<(Range<u32>, Tone)>, Tone) =
                        match (ansi, struck) {
                            (false, true) => {
                                let cut = struck_ranges(&row_tones[line.at], &text);
                                shown = bracket(&text, &cut);
                                (&shown, Vec::new(), Tone::Plain)
                            }
                            (false, false) => (&text, Vec::new(), Tone::Plain),
                            (true, true) => (
                                &text,
                                ranges
                                    .iter()
                                    .map(|(r, t)| (r.clone(), t.struck()))
                                    .collect(),
                                base.struck(),
                            ),
                            (true, false) => (&text, ranges.clone(), *base),
                        };
                    let [side_char, imp] = mark(&line, &row, iu);
                    let colour = match (ansi, imp) {
                        (false, _) | (true, ' ') => "",
                        (true, '!') => Sgr::RED,
                        (true, _) => Sgr::BLUE,
                    };
                    let mut wrapper = Wrapper::new(
                        Atoms::new(text_ref, tab, &tone_list, base),
                        width,
                        opts.wrap,
                    );
                    let mut first = true;
                    while let Some(seg) = wrapper.next_segment() {
                        out.clear();
                        if opts.line_numbers {
                            let num = |n: Option<u32>| match (first, n) {
                                (true, Some(n)) => n.to_string(),
                                _ => String::new(),
                            };
                            let _ = write!(
                                out,
                                "{:>numbers$} {:>numbers$} ",
                                num(line.left),
                                num(line.right)
                            );
                        }
                        if colour.is_empty() {
                            out.push(side_char);
                            out.push(imp);
                        } else {
                            out.push_str(colour);
                            out.push(side_char);
                            out.push(imp);
                            out.push_str(Sgr::RESET);
                        }
                        out.push(' ');
                        if seg.is_empty() {
                            out.truncate(out.trim_end_matches(' ').len());
                        } else {
                            cells::push_segment(&mut out, &seg, ansi);
                        }
                        out.push('\n');
                        w.write_all(out.as_bytes())?;
                        first = false;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Appends `text` HTML-escaped with its marked ranges: `<span>` or, when struck, `<del>`, classed by importance.
fn push_marked(out: &mut String, text: &str, ranges: &[InlineRange], struck: bool) {
    let element = if struck { "del" } else { "span" };
    let mut at = 0usize;
    for r in ranges {
        let (start, end) = (
            r.range.start as usize,
            (r.range.end as usize).min(text.len()),
        );
        if start >= end || start < at {
            continue;
        }
        for c in text.get(at..start).unwrap_or_default().chars() {
            escape::push_html_char(out, c);
        }
        let class = if r.important { "imp" } else { "unimp" };
        let _ = write!(out, "<{element} class=\"{class}\">");
        for c in text.get(start..end).unwrap_or_default().chars() {
            escape::push_html_char(out, c);
        }
        let _ = write!(out, "</{element}>");
        at = end;
    }
    for c in text.get(at..).unwrap_or_default().chars() {
        escape::push_html_char(out, c);
    }
}

/// The class attribute of a line's row.
fn line_class(line: &Line, row: &Row, iu: bool) -> String {
    let mut classes: Vec<&str> = Vec::with_capacity(3);
    match row.state {
        RowState::LeftOnly => classes.push("lo"),
        RowState::RightOnly => classes.push("ro"),
        _ => {}
    }
    let unimportant_as_same = iu && row.importance == Importance::Unimportant;
    if row.state != RowState::Same && !unimportant_as_same {
        classes.push(if row.importance == Importance::Important {
            "imp"
        } else {
            "unimp"
        });
    }
    match line.side {
        Some(Side::Right) => classes.push("r"),
        Some(_) => classes.push("l"),
        None => {}
    }
    if classes.is_empty() {
        String::new()
    } else {
        format!(" class=\"{}\"", classes.join(" "))
    }
}

/// Writes the HTML form.
pub(super) fn write_html(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    items: &Items,
    mono: bool,
    style: &HtmlStyle,
) -> Result<(), ReportError> {
    let header = super::header(cmp, meta, &[]);
    style::html_start(
        w,
        &header.title,
        &meta.generator,
        Sheet::Interleaved,
        mono,
        style,
    )?;
    header.write_html(w)?;
    w.write_all(LEGEND.as_bytes())?;
    if !opts.strike.is_empty() {
        w.write_all(b"<td><del class=\"imp\">Struck-out difference</del></td>")?;
    }
    w.write_all(b"</tr>\n</table>\n")?;
    let numbers = opts.line_numbers;
    let tab = cmp.format().text().tab_size;
    writeln!(w, "<table class=\"il\" style=\"tab-size:{tab}\">")?;
    let col_n = if numbers {
        "<col class=\"n\"><col class=\"n\">"
    } else {
        ""
    };
    writeln!(w, "<colgroup>{col_n}<col class=\"t\"></colgroup>")?;
    let th_n = if numbers {
        "<th>Left</th><th>Right</th>"
    } else {
        ""
    };
    writeln!(w, "<thead><tr>{th_n}<th>Text</th></tr></thead>")?;
    writeln!(w, "<tbody>")?;
    let columns = if numbers { 3 } else { 1 };
    let docs: [&TextDocument; 2] = [cmp.document(Side::Left), cmp.document(Side::Right)];
    let mut out = String::new();
    if items.nothing() {
        writeln!(
            w,
            "<tr class=\"gap\"><td colspan=\"{columns}\">{NO_ROWS}</td></tr>"
        )?;
    }
    for item in items.iter() {
        match item {
            ViewItem::Gap { rows } => {
                writeln!(
                    w,
                    "<tr class=\"gap\"><td colspan=\"{columns}\">{}</td></tr>",
                    hidden_text(&rows)
                )?;
            }
            ViewItem::Row { row: idx } => {
                let Some(row) = cmp.row(idx) else { continue };
                write_html_row(&mut out, cmp, &docs, idx, &row, opts)?;
                w.write_all(out.as_bytes())?;
            }
        }
    }
    writeln!(w, "</tbody>")?;
    writeln!(w, "</table>")?;
    style::html_end(w)?;
    Ok(())
}

/// The `<tr>`s of one row into `out` (cleared first).
fn write_html_row(
    out: &mut String,
    cmp: &TextComparison,
    docs: &[&TextDocument; 2],
    idx: RowIdx,
    row: &Row,
    opts: &TextReportOptions,
) -> Result<(), ReportError> {
    out.clear();
    let iu = opts.ignore_unimportant;
    let unimportant_as_same = iu && row.importance == Importance::Unimportant;
    let inline = if row.state == RowState::Changed && !unimportant_as_same {
        cmp.inline(idx)
    } else {
        None
    };
    for line in lines_of(row, docs) {
        out.push_str("<tr");
        out.push_str(&line_class(&line, row, iu));
        out.push('>');
        if opts.line_numbers {
            for n in [line.left, line.right] {
                match n {
                    Some(n) => {
                        let _ = write!(out, "<td class=\"n\">{n}</td>");
                    }
                    None => out.push_str("<td class=\"n\"></td>"),
                }
            }
        }
        out.push_str("<td class=\"t\">");
        let text = docs[line.at].line(line.line);
        let struck = opts.strike.contains(strike_bit(line.at)) && line.side.is_some();
        let orphan = matches!(row.state, RowState::LeftOnly | RowState::RightOnly);
        if orphan && struck && !unimportant_as_same && !text.is_empty() {
            let whole = [InlineRange {
                range: 0..text.len() as u32,
                important: row.importance == Importance::Important,
            }];
            push_marked(out, &text, &whole, true);
        } else {
            let ranges: Vec<InlineRange> = inline
                .as_ref()
                .filter(|_| line.side.is_some())
                .map(|inl| if line.at == 0 { &inl.left } else { &inl.right })
                .map(|r| r.iter().filter(|r| !iu || r.important).cloned().collect())
                .unwrap_or_default();
            push_marked(out, &text, &ranges, struck);
        }
        out.push_str("</td></tr>\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_and_marks() {
        assert_eq!(bracket("abcdef", &[1..2, 4..6]), "a[-b-]cd[-ef-]");
        assert_eq!(bracket("abc", &[]), "abc");
        assert_eq!(bracket("äb", &[0..2, 3..3]), "[-ä-]b");
        let row = |state, importance| Row {
            left: None,
            right: None,
            state,
            importance,
        };
        let line = |side| Line {
            side,
            left: None,
            right: None,
            line: hikasa_text::LineIdx(0),
            at: 0,
        };
        let changed = row(RowState::Changed, Importance::Important);
        assert_eq!(mark(&line(Some(Side::Left)), &changed, false), ['-', '!']);
        assert_eq!(mark(&line(Some(Side::Right)), &changed, false), ['+', '!']);
        let unimportant = row(RowState::Changed, Importance::Unimportant);
        assert_eq!(
            mark(&line(Some(Side::Left)), &unimportant, false),
            ['-', '~']
        );
        assert_eq!(
            mark(&line(Some(Side::Left)), &unimportant, true),
            ['-', ' ']
        );
        let same = row(RowState::Same, Importance::None);
        assert_eq!(mark(&line(None), &same, false), [' ', ' ']);
        let orphan = row(RowState::RightOnly, Importance::Unimportant);
        assert_eq!(mark(&line(Some(Side::Right)), &orphan, false), ['+', '~']);
        assert_eq!(
            line_class(&line(Some(Side::Right)), &orphan, false),
            " class=\"ro unimp r\""
        );
        assert_eq!(
            line_class(&line(Some(Side::Right)), &orphan, true),
            " class=\"ro r\""
        );
        assert_eq!(line_class(&line(None), &same, false), "");
        let mut out = String::new();
        push_marked(
            &mut out,
            "a<b",
            &[InlineRange {
                range: 1..2,
                important: false,
            }],
            true,
        );
        assert_eq!(out, "a<del class=\"unimp\">&lt;</del>b");
    }
}
