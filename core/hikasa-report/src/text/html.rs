//! Side-by-side HTML (RPT-010): one self-contained document with an embedded stylesheet (colour or mono, plus an
//! optional custom one), the RPT-004 header, a legend, and one table row per visible row; inline differences are
//! `<span class="imp|unimp">`, rows are classed `imp`/`unimp` (changed) or `lo`/`ro` (orphans), and every run of
//! hidden rows is one `gap` row "… N lines hidden …". Every text node is HTML-escaped; there are no scripts, no
//! external resources and no generated ids.

use std::io::Write;

use hikasa_core::Side;
use hikasa_text::{
    Importance, InlineRange, Row, RowIdx, RowState, TextComparison, TextDocument, ViewItem,
    ViewOptions,
};

use crate::error::ReportError;
use crate::escape;
use crate::meta::ReportMeta;
use crate::style::{self, Sheet};
use crate::text::sbs::{hidden_text, marker};
use crate::text::{TextReportOptions, spec_text};

/// The legend (RPT-010 colour semantics).
const LEGEND: &str = "<table class=\"legend\">\n<tr><td class=\"imp\">Important difference</td>\
<td class=\"unimp\">Unimportant difference</td><td class=\"lo\">Orphan (one side only)</td>\
<td class=\"repl\">Replacement</td><td class=\"gap\">Hidden lines</td></tr>\n</table>\n";

/// The class attribute of a row.
fn row_class(row: &Row, ignore_unimportant: bool) -> &'static str {
    let unimportant_as_same = ignore_unimportant && row.importance == Importance::Unimportant;
    match (row.state, row.importance) {
        (RowState::Same, _) => "",
        (_, _) if unimportant_as_same && row.state == RowState::Changed => "",
        (RowState::Changed, Importance::Important) => " class=\"imp\"",
        (RowState::Changed, _) => " class=\"unimp\"",
        (RowState::LeftOnly, _) if unimportant_as_same => " class=\"lo\"",
        (RowState::LeftOnly, Importance::Important) => " class=\"lo imp\"",
        (RowState::LeftOnly, _) => " class=\"lo unimp\"",
        (RowState::RightOnly, _) if unimportant_as_same => " class=\"ro\"",
        (RowState::RightOnly, Importance::Important) => " class=\"ro imp\"",
        (RowState::RightOnly, _) => " class=\"ro unimp\"",
    }
}

/// Appends `text` HTML-escaped, with `<span>`s around the inline ranges (sorted, disjoint byte ranges on char
/// boundaries).
pub(crate) fn push_spans(out: &mut String, text: &str, ranges: &[InlineRange]) {
    let mut next = 0usize;
    let mut open: Option<u32> = None;
    for (at, c) in text.char_indices() {
        let at = at as u32;
        if let Some(end) = open
            && at >= end
        {
            out.push_str("</span>");
            open = None;
        }
        if open.is_none() {
            while ranges.get(next).is_some_and(|r| r.range.end <= at) {
                next += 1;
            }
            if let Some(r) = ranges.get(next)
                && r.range.start <= at
            {
                out.push_str(if r.important {
                    "<span class=\"imp\">"
                } else {
                    "<span class=\"unimp\">"
                });
                open = Some(r.range.end);
                next += 1;
            }
        }
        escape::push_html_char(out, c);
    }
    if open.is_some() {
        out.push_str("</span>");
    }
}

/// Writes the document.
pub(super) fn write(
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
    w.write_all(LEGEND.as_bytes())?;
    let iu = opts.ignore_unimportant;
    let numbers = opts.line_numbers;
    let tab = cmp.format().text().tab_size;
    writeln!(w, "<table class=\"sbs\" style=\"tab-size:{tab}\">")?;
    let col_n = if numbers { "<col class=\"n\">" } else { "" };
    writeln!(
        w,
        "<colgroup>{col_n}<col class=\"t\"><col class=\"m\">{col_n}<col class=\"t\"></colgroup>"
    )?;
    let th_n = if numbers { "<th></th>" } else { "" };
    writeln!(
        w,
        "<thead><tr>{th_n}<th>{}</th><th></th>{th_n}<th>{}</th></tr></thead>",
        escape::html(&spec_text(cmp.document(Side::Left))),
        escape::html(&spec_text(cmp.document(Side::Right)))
    )?;
    writeln!(w, "<tbody>")?;
    let columns = if numbers { 5 } else { 3 };
    let view = cmp.view(&ViewOptions {
        filter: opts.display.text_filter(),
        ignore_unimportant: iu,
        expanded: Vec::new(),
    });
    let docs: [&TextDocument; 2] = [cmp.document(Side::Left), cmp.document(Side::Right)];
    let mut line = String::new();
    for item in view.iter() {
        line.clear();
        match item {
            ViewItem::Gap { rows } => {
                line.push_str(&format!(
                    "<tr class=\"gap\"><td colspan=\"{columns}\">{}</td></tr>",
                    hidden_text(&rows)
                ));
            }
            ViewItem::Row { row: idx } => {
                let Some(row) = cmp.row(idx) else { continue };
                push_row(&mut line, cmp, &docs, idx, &row, iu, numbers);
            }
        }
        line.push('\n');
        w.write_all(line.as_bytes())?;
    }
    writeln!(w, "</tbody>")?;
    writeln!(w, "</table>")?;
    style::html_end(w)?;
    Ok(())
}

/// Appends one `<tr>`.
fn push_row(
    out: &mut String,
    cmp: &TextComparison,
    docs: &[&TextDocument; 2],
    idx: RowIdx,
    row: &Row,
    iu: bool,
    numbers: bool,
) {
    let inline =
        if row.state == RowState::Changed && !(iu && row.importance == Importance::Unimportant) {
            cmp.inline(idx)
        } else {
            None
        };
    out.push_str("<tr");
    out.push_str(row_class(row, iu));
    out.push('>');
    for (i, line) in [row.left, row.right].into_iter().enumerate() {
        let side_class = if i == 0 { "l" } else { "r" };
        if numbers {
            match line {
                // The source line (pieces of a line broken by the characters-per-line limit share it).
                Some(l) => out.push_str(&format!(
                    "<td class=\"n\">{}</td>",
                    docs[i].physical_line(l)
                )),
                None => out.push_str("<td class=\"n\"></td>"),
            }
        }
        match line {
            Some(l) => {
                out.push_str("<td class=\"t ");
                out.push_str(side_class);
                out.push_str("\">");
                let text = docs[i].line(l);
                let ranges: Vec<InlineRange> = inline
                    .as_ref()
                    .map(|inl| if i == 0 { &inl.left } else { &inl.right })
                    .map(|r| r.iter().filter(|r| !iu || r.important).cloned().collect())
                    .unwrap_or_default();
                push_spans(out, &text, &ranges);
                out.push_str("</td>");
            }
            None => {
                out.push_str("<td class=\"t ");
                out.push_str(side_class);
                out.push_str(" x\"></td>");
            }
        }
        if i == 0 {
            out.push_str("<td class=\"m\">");
            escape::push_html_char(out, marker(row, iu));
            out.push_str("</td>");
        }
    }
    out.push_str("</tr>");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(start: u32, end: u32, important: bool) -> InlineRange {
        InlineRange {
            range: start..end,
            important,
        }
    }

    #[test]
    fn spans_wrap_ranges_and_escape() {
        let mut out = String::new();
        push_spans(&mut out, "a<b>c", &[range(1, 4, true)]);
        assert_eq!(out, "a<span class=\"imp\">&lt;b&gt;</span>c");
        let mut out = String::new();
        push_spans(&mut out, "x = 1;", &[range(0, 1, false), range(4, 5, true)]);
        assert_eq!(
            out,
            "<span class=\"unimp\">x</span> = <span class=\"imp\">1</span>;"
        );
        let mut out = String::new();
        push_spans(&mut out, "äöü", &[range(2, 6, true)]);
        assert_eq!(out, "ä<span class=\"imp\">öü</span>");
        let mut out = String::new();
        push_spans(&mut out, "", &[]);
        assert_eq!(out, "");
        // Adjacent ranges close and reopen.
        let mut out = String::new();
        push_spans(&mut out, "ab", &[range(0, 1, true), range(1, 2, false)]);
        assert_eq!(
            out,
            "<span class=\"imp\">a</span><span class=\"unimp\">b</span>"
        );
    }

    #[test]
    fn row_classes() {
        let row = |state, importance| Row {
            left: None,
            right: None,
            state,
            importance,
        };
        assert_eq!(row_class(&row(RowState::Same, Importance::None), false), "");
        assert_eq!(
            row_class(&row(RowState::Changed, Importance::Important), true),
            " class=\"imp\""
        );
        assert_eq!(
            row_class(&row(RowState::Changed, Importance::Unimportant), false),
            " class=\"unimp\""
        );
        assert_eq!(
            row_class(&row(RowState::Changed, Importance::Unimportant), true),
            ""
        );
        assert_eq!(
            row_class(&row(RowState::LeftOnly, Importance::Unimportant), true),
            " class=\"lo\""
        );
        assert_eq!(
            row_class(&row(RowState::RightOnly, Importance::Important), true),
            " class=\"ro imp\""
        );
    }
}
