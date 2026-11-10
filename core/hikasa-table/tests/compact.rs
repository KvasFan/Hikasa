//! F7: a compact text sheet reads exactly like the sheet it was (P-COMPACT), its kept cells, checkpoints and
//! implicit lines, and the comparison compacting the workbooks it owns.

use std::sync::Arc;

use hikasa_core::CancelToken;
use hikasa_table::{
    Cell, DelimitedOptions, FirstLine, FixedOptions, LoadOptions, Sheet, TableCompareOptions,
    TableFormat, TableKind, Value, Workbook, compare_workbooks, load_bytes,
};
use proptest::prelude::*;

fn load(text: &[u8], kind: TableKind, first: FirstLine, ctrl_z: bool) -> Workbook {
    let format = TableFormat {
        kind,
        first_line: first,
        ctrl_z_eof: ctrl_z,
        ..TableFormat::default()
    };
    load_bytes(
        text.to_vec(),
        "t.txt",
        &format,
        &LoadOptions::default(),
        &CancelToken::new(),
    )
    .unwrap()
}

/// Everything a reader sees of a sheet: per stored row (header first) its length, stored length, emptiness, values,
/// stored cells, packed cells up to two columns past its end and its label.
fn view(sheet: &Sheet) -> Vec<String> {
    let mut out = vec![format!(
        "rows {} cols {} header {:?} kinds {:?}",
        sheet.row_count(),
        sheet.column_count(),
        sheet.header(),
        (0..sheet.column_count())
            .map(|c| sheet.column_kind(c))
            .collect::<Vec<_>>()
    )];
    let width = sheet.column_count() + 2;
    let mut rows: Vec<(String, hikasa_table::RowView<'_>)> = Vec::new();
    if let Some(h) = sheet.header_row() {
        rows.push(("header".to_owned(), h));
    }
    for r in 0..sheet.row_count() {
        rows.push((format!("row {r} line {}", sheet.row_label(r)), sheet.row(r)));
    }
    for (name, row) in rows {
        let values: Vec<Value<'_>> = row.values().collect();
        let stored: Vec<(u32, Value<'_>)> = row.stored().collect();
        let cells: Vec<Cell> = (0..width.max(row.len() + 2)).map(|c| row.cell(c)).collect();
        let gets: Vec<Value<'_>> = (0..width.max(row.len() + 2)).map(|c| row.get(c)).collect();
        out.push(format!(
            "{name}: len {} stored {} empty {} sparse {} values {values:?} stored {stored:?} cells {cells:?} get {gets:?}",
            row.len(),
            row.stored_len(),
            row.is_empty(),
            row.is_sparse(),
        ));
    }
    for r in 0..sheet.row_count() {
        let cells: Vec<Value<'_>> = (0..width).map(|c| sheet.cell(r, c)).collect();
        out.push(format!("cell {r}: {cells:?}"));
    }
    out
}

/// Loads `text`, compacts a copy and checks that both read the same.
fn same_after_compaction(text: &[u8], kind: TableKind, first: FirstLine, ctrl_z: bool) -> bool {
    let full = load(text, kind, first, ctrl_z);
    let mut compact = full.clone();
    compact.compact();
    let sheet = &compact.sheets()[0];
    assert!(sheet.is_compact());
    assert_eq!(
        view(&full.sheets()[0]),
        view(sheet),
        "text {:?}",
        String::from_utf8_lossy(text)
    );
    // The packed cells and their row starts are gone.
    sheet
        .heap_parts()
        .iter()
        .all(|p| !matches!(p.0, "cells" | "rows") || p.1 == 0)
}

fn token() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "a",
        "b",
        "Z",
        "1",
        "2.5",
        "é",
        " ",
        " ",
        "\t",
        ",",
        ",",
        ";",
        "§",
        "|",
        "\"",
        "\"\"",
        "'",
        "«",
        "»",
        "\n",
        "\r",
        "\r\n",
        "\u{1A}",
        "2024-01-02",
        "x y",
    ])
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::collection::vec(token(), 0..60).prop_map(|t| t.concat()),
        // Wide records (checkpoints every 64 fields).
        1 => prop::collection::vec(
            prop_oneof![4 => Just(","), 1 => Just(",,"), 2 => token()],
            100..400
        )
        .prop_map(|t| t.concat()),
    ]
}

fn delimited() -> impl Strategy<Value = DelimitedOptions> {
    (
        prop::sample::subsequence(vec![',', ';', '\t', ' ', '§', '|'], 1..3),
        prop::sample::select(vec![None, Some('"'), Some('\''), Some('«')]),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(
            |(delimiters, qualifier, consecutive_as_one, trim_whitespace)| DelimitedOptions {
                delimiters,
                qualifier,
                consecutive_as_one,
                trim_whitespace,
            },
        )
}

fn fixed() -> impl Strategy<Value = FixedOptions> {
    prop_oneof![
        prop::collection::vec(0u32..5, 0..6),
        prop::collection::vec(0u32..3, 60..140),
    ]
    .prop_map(|widths| FixedOptions { widths })
}

fn first_line() -> impl Strategy<Value = FirstLine> {
    prop::sample::select(vec![FirstLine::Detect, FirstLine::Names, FirstLine::Data])
}

proptest! {
    #![proptest_config(ProptestConfig { cases: ProptestConfig::default().cases.max(512), ..ProptestConfig::default() })]

    /// P-COMPACT (F7): every row, value, stored cell, packed cell and label of a delimited sheet is the same after
    /// compaction.
    #[test]
    fn p_compact_delimited(text in text(), opts in delimited(), first in first_line(), ctrl_z in any::<bool>()) {
        prop_assert!(same_after_compaction(text.as_bytes(), TableKind::Delimited(opts), first, ctrl_z));
    }

    /// P-COMPACT (F7) for fixed-width sheets.
    #[test]
    fn p_compact_fixed(text in text(), opts in fixed(), first in first_line(), ctrl_z in any::<bool>()) {
        prop_assert!(same_after_compaction(text.as_bytes(), TableKind::Fixed(opts), first, ctrl_z));
    }
}

fn csv(text: &str) -> Workbook {
    load(
        text.as_bytes(),
        TableKind::Delimited(DelimitedOptions::with(',')),
        FirstLine::Data,
        false,
    )
}

#[test]
fn doubled_qualifiers_and_stray_characters_keep_their_cells() {
    let text = "a,b\n\"x\"\"y\",2\nc,d\n\"q\"r,3\n\"line\nbreak\",4\nlast,5\n";
    let full = csv(text);
    let mut w = full.clone();
    w.compact();
    let s = &w.sheets()[0];
    assert!(s.is_compact());
    assert_eq!(view(&full.sheets()[0]), view(s));
    assert_eq!(s.cell(1, 0), Value::Text("x\"y"));
    assert_eq!(s.cell(3, 0), Value::Text("qr"));
    assert_eq!(s.cell(4, 0), Value::Text("line\nbreak"));
    // Two records keep their cells (2 × 2 cells of 12 bytes): the side table, and the record starts.
    let records = s.heap_parts().iter().find(|p| p.0 == "records").unwrap().1;
    assert!(records >= 2 * 2 * 12 + 5 * 4, "{records}");
    // A record over two lines: the lines are explicit (the next record is line 7).
    assert_eq!((s.row_label(4), s.row_label(5)), (5, 7));
    assert!(s.heap_parts().iter().any(|p| p.0 == "lines" && p.1 > 0));
}

#[test]
fn one_line_records_have_implicit_lines() {
    let w = csv("a,b\n\nc\r\nd,e\n");
    let s = &w.sheets()[0];
    assert_eq!(s.heap_parts().iter().find(|p| p.0 == "lines").unwrap().1, 0);
    assert_eq!(
        (0..s.row_count())
            .map(|r| s.row_label(r))
            .collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
}

#[test]
fn wide_records_resume_at_checkpoints() {
    let row: Vec<String> = (0..300).map(|i| format!("v{i}")).collect();
    let mut text = row.join(",");
    text.push('\n');
    text.push_str(&row[..70].join(","));
    text.push_str(",,,\nshort\n");
    let full = csv(&text);
    let mut w = full.clone();
    w.compact();
    let s = &w.sheets()[0];
    assert_eq!(view(&full.sheets()[0]), view(s));
    for c in [0, 63, 64, 65, 127, 128, 299] {
        assert_eq!(s.cell(0, c), Value::Text(&row[c as usize]), "{c}");
    }
    assert_eq!(s.cell(0, 300), Value::Empty);
    assert_eq!(s.row(1).len(), 70);
    assert_eq!(s.cell(1, 69), Value::Text("v69"));
    assert_eq!(s.cell(1, 72), Value::Empty);
    assert_eq!(s.cell(2, 64), Value::Empty);
    // Checkpoints: 4 for the first record (fields 64, 128, 192, 256), 1 for the second (field 64), 8 bytes each.
    let records = s.heap_parts().iter().find(|p| p.0 == "records").unwrap().1;
    assert!(records >= 5 * 8 + 3 * 4, "{records}");
}

#[test]
fn sheets_built_in_memory_and_workbooks_stay_as_they_are() {
    let mut s = Sheet::from_text_rows(
        "S",
        &[vec!["a", "b"]],
        FirstLine::Data,
        hikasa_table::Regional::default(),
    );
    assert!(!s.compact());
    assert!(!s.is_compact());
    assert_eq!(s.cell(0, 1), Value::Text("b"));
}

#[test]
fn the_comparison_compacts_the_workbooks_it_owns_alone() {
    let opts = TableCompareOptions::default();
    let cancel = CancelToken::new();
    let (l, r) = (csv("k,v\n1,a\n2,b\n"), csv("k,v\n1,a\n2,c\n"));
    let c = compare_workbooks(Arc::new(l.clone()), Arc::new(r.clone()), &opts, &cancel).unwrap();
    assert!(c.left().sheets()[0].is_compact() && c.right().sheets()[0].is_compact());
    assert_eq!(c.cell(0, 2, 1, hikasa_core::Side::Right), Value::Text("c"));
    // A workbook the caller still holds is left alone.
    let (l, r) = (Arc::new(l), Arc::new(r));
    let c = compare_workbooks(Arc::clone(&l), Arc::clone(&r), &opts, &cancel).unwrap();
    assert!(!c.left().sheets()[0].is_compact());
}

#[test]
fn threads_reading_one_compact_sheet_read_the_same() {
    let mut text = String::new();
    for r in 0..2000 {
        let row: Vec<String> = (0..(r % 150)).map(|c| format!("{r}:{c}")).collect();
        text.push_str(&row.join(";"));
        text.push('\n');
    }
    let full = load(
        text.as_bytes(),
        TableKind::Delimited(DelimitedOptions::with(';')),
        FirstLine::Data,
        false,
    );
    let mut compact = full.clone();
    compact.compact();
    let (a, b) = (&full.sheets()[0], &compact.sheets()[0]);
    assert!(b.is_compact());
    std::thread::scope(|scope| {
        for t in 0..8u32 {
            scope.spawn(move || {
                for i in 0..20_000u32 {
                    let r = (i * 7 + t * 131) % a.row_count();
                    let c = (i * 13 + t) % 160;
                    assert_eq!(a.cell(r, c), b.cell(r, c), "row {r} column {c}");
                }
            });
        }
    });
}
