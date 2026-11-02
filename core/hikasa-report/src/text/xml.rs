//! The XML dump of a text comparison (M2 W7, D-198): the header, one `row` per visible row (a row with its `l` and
//! `r` on one line, inline differences as `d` and `u`), one `hidden` per run of hidden rows, and the summary with
//! [`summary_counts`](crate::summary_counts). Display modes, ignore-unimportant and the scope choose the rows; colours,
//! wrap, width and columns do not apply. A row's data is the comparison's: under ignore-unimportant an unimportant row
//! keeps its importance and its `u` elements (`mode/@ignore-unimportant` says how to read them).

use std::io::Write;

use hikasa_core::{ContentResult, Side};
use hikasa_text::{
    Importance, InlineRange, LineIdx, RowState, TextComparison, TextDocument, ViewItem,
};

use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::options::DisplayMode;
use crate::request::LineScope;
use crate::summary_counts::text_counts;
use crate::text::{Items, TEXT_TITLE, TextReportOptions, spec_text};
use crate::xml::{self, XmlHeader, XmlSide};

/// The `verdict` of a text comparison, as in `hikasa.text_compare.v1`.
pub(crate) const fn verdict_word(result: &ContentResult) -> &'static str {
    match result {
        ContentResult::BinarySame => "binary_same",
        ContentResult::RulesSame => "rules_same",
        ContentResult::UnimportantOnly => "unimportant_only",
        ContentResult::ImportantDifferences => "important_differences",
        ContentResult::BinaryDifferent => "binary_different",
        ContentResult::Error { .. } => "error",
    }
}

/// The `display` word of a mode (the canonical word without `display-`).
pub(crate) const fn display_word(mode: DisplayMode) -> &'static str {
    match mode {
        DisplayMode::All => "all",
        DisplayMode::Mismatches => "mismatches",
        DisplayMode::Context { .. } => "context",
        DisplayMode::Matches => "matches",
        DisplayMode::None => "none",
    }
}

/// The piece number of lines, side by side: 1 for a line that starts a physical line, 2, 3, … for the pieces after
/// artificial (characters-per-line) breaks. Remembers the last line asked, so rows in order cost O(1).
struct Pieces<'a> {
    doc: &'a TextDocument,
    last: Option<(u32, u32, u32)>,
}

impl Pieces<'_> {
    fn of(&mut self, line: LineIdx) -> u32 {
        let physical = self.doc.physical_line(line);
        let piece = match self.last {
            Some((at, p, piece)) if at + 1 == line.0 && p == physical => piece + 1,
            Some((at, p, piece)) if at == line.0 && p == physical => piece,
            _ => {
                let mut piece = 1;
                let mut at = line.0;
                while at > 0 && self.doc.physical_line(LineIdx(at - 1)) == physical {
                    piece += 1;
                    at -= 1;
                }
                piece
            }
        };
        self.last = Some((line.0, physical, piece));
        piece
    }
}

/// Appends the text of a line with its inline differences (`d` important, `u` unimportant).
fn push_line(out: &mut String, text: &str, ranges: &[InlineRange]) {
    let mut at = 0usize;
    for r in ranges {
        let (start, end) = (
            r.range.start as usize,
            (r.range.end as usize).min(text.len()),
        );
        if start >= end || start < at {
            continue;
        }
        xml::push_text(out, text.get(at..start).unwrap_or_default());
        let name = if r.important { "d" } else { "u" };
        out.push('<');
        out.push_str(name);
        out.push('>');
        xml::push_text(out, text.get(start..end).unwrap_or_default());
        out.push_str("</");
        out.push_str(name);
        out.push('>');
        at = end;
    }
    xml::push_text(out, text.get(at..).unwrap_or_default());
}

/// Writes the dump.
pub(super) fn write(
    w: &mut dyn Write,
    cmp: &TextComparison,
    opts: &TextReportOptions,
    meta: &ReportMeta,
    items: &Items,
    scope: Option<LineScope>,
) -> Result<(), ReportError> {
    let docs: [&TextDocument; 2] = [cmp.document(Side::Left), cmp.document(Side::Right)];
    let described: Vec<[String; 4]> = docs
        .iter()
        .map(|doc| {
            let enc = doc.encoding();
            [
                enc.encoding.name().to_owned(),
                xml::boolean(enc.bom_len > 0).to_owned(),
                doc.eol().style().as_str().to_owned(),
                xml::boolean(doc.binary().is_binary).to_owned(),
            ]
        })
        .collect();
    let side = |i: usize, id: &'static str| XmlSide {
        id,
        spec: spec_text(docs[i]),
        text: Some([
            ("encoding", described[i][0].as_str()),
            ("bom", described[i][1].as_str()),
            ("eol", described[i][2].as_str()),
            ("binary", described[i][3].as_str()),
        ]),
    };
    let mut mode = vec![("display", display_word(opts.display).to_owned())];
    if let DisplayMode::Context { lines } = opts.display {
        mode.push(("context", lines.to_string()));
    }
    mode.push((
        "ignore-unimportant",
        xml::boolean(opts.ignore_unimportant).to_owned(),
    ));
    if let Some(scope) = scope {
        mode.push((
            "scope",
            format!(
                "lines {}\u{2013}{} of the {}",
                scope.first,
                scope.last,
                scope.side.as_str()
            ),
        ));
    }
    xml::write_start(
        w,
        meta,
        &XmlHeader {
            kind: "text",
            default_title: TEXT_TITLE,
            sides: [side(0, "left"), side(1, "right")],
            mode,
            status: None,
            warnings: cmp.warnings(),
        },
    )?;
    let mut iter = items.iter().peekable();
    if iter.peek().is_none() {
        w.write_all(b"  <rows/>\n")?;
    } else {
        w.write_all(b"  <rows>\n")?;
        let mut pieces = [
            Pieces {
                doc: docs[0],
                last: None,
            },
            Pieces {
                doc: docs[1],
                last: None,
            },
        ];
        let mut out = String::new();
        for item in iter {
            out.clear();
            match item {
                ViewItem::Gap { rows } => {
                    let n = rows.end.saturating_sub(rows.start).to_string();
                    xml::empty_element(&mut out, 2, "hidden", &[("rows", &n)]);
                }
                ViewItem::Row { row: idx } => {
                    let Some(row) = cmp.row(idx) else { continue };
                    xml::open(&mut out, 2, "row");
                    let state = match row.state {
                        RowState::Same => "same",
                        RowState::Changed => "changed",
                        RowState::LeftOnly => "left-only",
                        RowState::RightOnly => "right-only",
                    };
                    xml::attr(&mut out, "state", state);
                    if row.state != RowState::Same {
                        match row.importance {
                            Importance::Important => xml::attr(&mut out, "importance", "important"),
                            Importance::Unimportant => {
                                xml::attr(&mut out, "importance", "unimportant");
                            }
                            Importance::None => {}
                        }
                    }
                    let mut piece = 1;
                    for (i, (name, line)) in [("left", row.left), ("right", row.right)]
                        .into_iter()
                        .enumerate()
                    {
                        if let Some(line) = line {
                            xml::attr(&mut out, name, &docs[i].physical_line(line).to_string());
                            piece = piece.max(pieces[i].of(line));
                        }
                    }
                    if piece > 1 {
                        xml::attr(&mut out, "piece", &piece.to_string());
                    }
                    out.push('>');
                    let inline = if row.state == RowState::Changed {
                        cmp.inline(idx)
                    } else {
                        None
                    };
                    for (i, (name, line)) in
                        [("l", row.left), ("r", row.right)].into_iter().enumerate()
                    {
                        let Some(line) = line else { continue };
                        out.push('<');
                        out.push_str(name);
                        out.push('>');
                        let ranges = inline
                            .as_ref()
                            .map_or(&[][..], |inl| if i == 0 { &inl.left } else { &inl.right });
                        push_line(&mut out, &docs[i].line(line), ranges);
                        out.push_str("</");
                        out.push_str(name);
                        out.push('>');
                    }
                    out.push_str("</row>\n");
                }
            }
            w.write_all(out.as_bytes())?;
        }
        w.write_all(b"  </rows>\n")?;
    }
    let verdict = cmp.verdict(opts.ignore_unimportant);
    xml::write_end(w, &[("verdict", verdict_word(&verdict))], &text_counts(cmp))?;
    Ok(())
}
