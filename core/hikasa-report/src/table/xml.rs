//! The XML dump of a table comparison (`type="table"`, schema `docs/schemas/hikasa-report-1.xsd`): the header, one
//! `sheet` per sheet pair with its `column`s, one `record` per shown row (its cells with the display texts of each
//! side, `l` and `r`; cells empty on both sides are left out) and one `hidden` per run of hidden rows, then the summary
//! with the counts of the summary layout. A record's data is the comparison's: under ignore-unimportant an unimportant
//! row keeps its importance (`mode/@ignore-unimportant` says how to read it).

use std::io::Write;

use hikasa_core::Side;
use hikasa_table::{CellState, Importance, RowAlignmentUsed, RowKind, SheetComparison};

use super::{
    Item, TABLE_TITLE, TableReportOptions, TableSource, display_word, filter_of, items, state_word,
    table_counts, warnings,
};
use crate::error::ReportError;
use crate::meta::ReportMeta;
use crate::text::verdict_word;
use crate::xml::{self, XmlHeader, XmlSide};

/// The word of a row kind (`left-only`, …).
const fn kind_word(kind: RowKind) -> &'static str {
    match kind {
        RowKind::Same => "same",
        RowKind::Changed => "changed",
        RowKind::LeftOnly => "left-only",
        RowKind::RightOnly => "right-only",
    }
}

/// Appends the `sheet` start tag with its attributes.
fn sheet_start(out: &mut String, src: &TableSource<'_>, pair: usize, sheet: &SheetComparison) {
    xml::open(out, 2, "sheet");
    xml::attr(out, "name", sheet.name());
    xml::attr(out, "state", state_word(sheet.state()));
    for (side, index, name) in [
        (Side::Left, "left-index", "left-name"),
        (Side::Right, "right-index", "right-name"),
    ] {
        let at = match side {
            Side::Left => sheet.left(),
            _ => sheet.right(),
        };
        if let (Some(i), Some(s)) = (at, src.comparison.sheet(pair, side)) {
            xml::attr(out, index, &(i + 1).to_string());
            xml::attr(out, name, s.name());
        }
    }
    if sheet.renamed() {
        xml::attr(out, "renamed", "true");
    }
    match sheet.alignment() {
        RowAlignmentUsed::Keyed { .. } => xml::attr(out, "alignment", "keyed"),
        RowAlignmentUsed::Engine { algorithm, sorted } => {
            xml::attr(out, "alignment", "engine");
            xml::attr(out, "algorithm", algorithm.as_str());
            if *sorted {
                xml::attr(out, "sorted", "true");
            }
        }
        RowAlignmentUsed::Orphan => xml::attr(out, "alignment", "orphan"),
    }
    out.push_str(">\n");
}

/// Writes the dump.
pub(super) fn write(
    w: &mut dyn Write,
    src: &TableSource<'_>,
    opts: &TableReportOptions,
    meta: &ReportMeta,
) -> Result<(), ReportError> {
    let iu = opts.ignore_unimportant;
    let filter = filter_of(opts);
    let side = |s: Side, id: &'static str| {
        let wb = src.comparison.workbook(s);
        let mut extra = vec![
            ("source", wb.source().as_str().to_owned()),
            ("sheets", wb.sheets().len().to_string()),
        ];
        if let Some(enc) = wb.encoding() {
            extra.push(("encoding", enc.to_owned()));
        }
        XmlSide {
            id,
            spec: src.side(s).spec.clone(),
            text: None,
            extra,
        }
    };
    let warnings = warnings(src);
    xml::write_start(
        w,
        meta,
        &XmlHeader {
            kind: "table",
            default_title: TABLE_TITLE,
            sides: [side(Side::Left, "left"), side(Side::Right, "right")],
            mode: vec![
                ("display", display_word(filter).to_owned()),
                ("ignore-unimportant", xml::boolean(iu).to_owned()),
            ],
            status: None,
            warnings: &warnings,
        },
    )?;
    let cmp = src.comparison;
    if cmp.sheets().is_empty() {
        w.write_all(b"  <sheets/>\n")?;
    } else {
        w.write_all(b"  <sheets>\n")?;
        let mut out = String::new();
        for (pair, sheet) in cmp.sheets().iter().enumerate() {
            out.clear();
            sheet_start(&mut out, src, pair, sheet);
            let header = sheet.header_states();
            for (k, col) in sheet.columns().iter().enumerate() {
                xml::open(&mut out, 3, "column");
                xml::attr(&mut out, "index", &(k + 1).to_string());
                xml::attr(&mut out, "name", &col.name);
                for (s, name) in [(Side::Left, "left"), (Side::Right, "right")] {
                    if let Some(letters) = super::column_letter(src, pair, col, s) {
                        xml::attr(&mut out, name, &letters);
                    }
                }
                if let Some(key) = col.key {
                    xml::attr(&mut out, "key", &(u32::from(key) + 1).to_string());
                }
                if col.hidden {
                    xml::attr(&mut out, "hidden", "true");
                }
                if let Some(state) = header.get(k) {
                    xml::attr(&mut out, "header", state.as_str());
                }
                out.push_str("/>\n");
            }
            w.write_all(out.as_bytes())?;
            for item in items(sheet, &filter) {
                out.clear();
                match item {
                    Item::Gap(n) => {
                        xml::empty_element(&mut out, 3, "hidden", &[("rows", &n.to_string())]);
                    }
                    Item::Row(i) => {
                        let Some(r) = sheet.rows().get(i as usize) else {
                            continue;
                        };
                        xml::open(&mut out, 3, "record");
                        xml::attr(&mut out, "state", kind_word(r.kind()));
                        match r.importance() {
                            Importance::Important => {
                                xml::attr(&mut out, "importance", "important");
                            }
                            Importance::Unimportant => {
                                xml::attr(&mut out, "importance", "unimportant");
                            }
                            Importance::None => {}
                        }
                        for (s, name) in [(Side::Left, "left"), (Side::Right, "right")] {
                            if let Some(l) = cmp.row_label(pair, i, s) {
                                xml::attr(&mut out, name, &l.to_string());
                            }
                        }
                        let mut cells = String::new();
                        for (k, col) in sheet.columns().iter().enumerate() {
                            let k = k as u32;
                            let text = |s: Side| -> Option<String> {
                                r.side(s)?;
                                match s {
                                    Side::Left => col.left?,
                                    _ => col.right?,
                                };
                                Some(cmp.cell(pair, i, k, s).display().into_owned())
                            };
                            let (l, rt) = (text(Side::Left), text(Side::Right));
                            let state = match r.kind() {
                                RowKind::Same => CellState::Same,
                                _ => sheet.cell_state(i, k),
                            };
                            let blank = |t: &Option<String>| t.as_deref().is_none_or(str::is_empty);
                            if state == CellState::Same && blank(&l) && blank(&rt) {
                                continue;
                            }
                            xml::open(&mut cells, 4, "cell");
                            xml::attr(&mut cells, "column", &(k + 1).to_string());
                            xml::attr(&mut cells, "state", state.as_str());
                            cells.push('>');
                            for (name, t) in [("l", &l), ("r", &rt)] {
                                if let Some(t) = t {
                                    cells.push('<');
                                    cells.push_str(name);
                                    cells.push('>');
                                    xml::push_text(&mut cells, t);
                                    cells.push_str("</");
                                    cells.push_str(name);
                                    cells.push('>');
                                }
                            }
                            cells.push_str("</cell>\n");
                        }
                        if cells.is_empty() {
                            out.push_str("/>\n");
                        } else {
                            out.push_str(">\n");
                            out.push_str(&cells);
                            out.push_str("      </record>\n");
                        }
                    }
                }
                w.write_all(out.as_bytes())?;
            }
            w.write_all(b"    </sheet>\n")?;
        }
        w.write_all(b"  </sheets>\n")?;
    }
    let verdict = src.verdict(iu);
    xml::write_end(
        w,
        &[("verdict", verdict_word(&verdict))],
        &table_counts(cmp),
    )?;
    Ok(())
}
