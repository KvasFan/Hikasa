//! The review fixes (contract §9.8): long numbers, leading zeros, workbook precision, memory bounds, sparse rows,
//! out-of-order cells, header votes, sniffing, low-entropy alignment, epochs, detection errors, hidden sheets,
//! rows and columns, and the review's probes as asserting tests.

mod common;

use std::sync::Arc;

use common::{cmp, wb, xlsx};
use hikasa_core::{CancelToken, ContentKind};
use hikasa_table::{
    ColumnRef, ColumnType, LoadOptions, RulePatch, TableCompareOptions, TableError, TableFormat,
    Workbook, load_bytes,
};

fn load(bytes: Vec<u8>, name: &str) -> Result<Workbook, TableError> {
    load_bytes(
        bytes,
        name,
        &TableFormat::from_name(name),
        &LoadOptions::default(),
        &CancelToken::new(),
    )
}

fn la(bytes: Vec<u8>, name: &str) -> Arc<Workbook> {
    Arc::new(load(bytes, name).unwrap())
}

/// The verdict, checked to be the same with the sides swapped.
fn verdict(l: &Arc<Workbook>, r: &Arc<Workbook>, o: &TableCompareOptions) -> ContentKind {
    let (a, b) = (
        cmp(l, r, o).kind(false, false),
        cmp(r, l, o).kind(false, false),
    );
    assert_eq!(a, b, "the verdict is symmetric");
    a
}

fn keyed(keys: &str) -> TableCompareOptions {
    let mut o = TableCompareOptions::default();
    o.default_sheet.keys = hikasa_table::parse_column_list(keys).unwrap();
    o
}

fn with_type(column: &str, kind: ColumnType) -> TableCompareOptions {
    let mut o = TableCompareOptions::default();
    o.default_sheet.column_rules = vec![(
        ColumnRef::parse(column).unwrap(),
        RulePatch {
            kind: Some(kind),
            ..RulePatch::default()
        },
    )];
    o
}

// F1: long numbers.

#[test]
fn long_ids_are_distinct_keys_and_values() {
    let l = wb(
        "l.csv",
        "id,owner\n12345678901234567,alice\n12345678901234568,bob\n",
    );
    let r = wb(
        "r.csv",
        "id,owner\n12345678901234568,alice\n12345678901234567,bob\n",
    );
    // Identical files: no duplicate keys, same.
    let c = cmp(&l, &l, &keyed("id"));
    assert_eq!(c.kind(false, false), ContentKind::RulesSame);
    assert_eq!(c.sheets()[0].stats().duplicate_left, 0);
    // Swapped owners: keyed, unkeyed and with a forced Number id column, important.
    assert_eq!(
        verdict(&l, &r, &keyed("id")),
        ContentKind::ImportantDifferences
    );
    let c = cmp(&l, &r, &keyed("id"));
    assert_eq!(c.sheets()[0].stats().changed_important, 2);
    assert_eq!(c.sheets()[0].stats().duplicate_left, 0);
    assert_eq!(
        verdict(&l, &r, &TableCompareOptions::default()),
        ContentKind::ImportantDifferences
    );
    let mut forced = keyed("id");
    forced.default_sheet.column_rules = with_type("id", ColumnType::Number)
        .default_sheet
        .column_rules;
    assert_eq!(verdict(&l, &r, &forced), ContentKind::ImportantDifferences);
    assert_eq!(cmp(&l, &l, &forced).sheets()[0].stats().duplicate_left, 0);
    // The same long number in two spellings is one key under the Number type.
    let s = wb(
        "s.csv",
        "id,owner\n1.2345678901234567e16,alice\n12345678901234568.0,bob\n",
    );
    assert_eq!(verdict(&l, &s, &forced), ContentKind::RulesSame);
}

#[test]
fn a_long_number_column_is_text() {
    let l = wb("l.csv", "id\n12345678901234567\n");
    assert_eq!(l.sheets()[0].column_kind(0), hikasa_table::ColumnKind::Text);
    let short = wb("s.csv", "id\n123456789012345\n");
    assert_eq!(
        short.sheets()[0].column_kind(0),
        hikasa_table::ColumnKind::Number
    );
}

// F2: leading zeros (T-6).

#[test]
fn leading_zeros_are_important_in_auto_and_equal_as_numbers() {
    let l = wb("l.csv", "zip,city\n01234,Boston\n02134,Allston\n");
    let r = wb("r.csv", "zip,city\n1234,Boston\n2134,Allston\n");
    assert_eq!(l.sheets()[0].column_kind(0), hikasa_table::ColumnKind::Text);
    assert_eq!(
        verdict(&l, &r, &TableCompareOptions::default()),
        ContentKind::ImportantDifferences
    );
    assert_eq!(
        verdict(&l, &r, &keyed("city")),
        ContentKind::ImportantDifferences
    );
    // The explicit Number type equates them (a contract decision, T-6).
    assert_eq!(
        verdict(&l, &r, &with_type("zip", ColumnType::Number)),
        ContentKind::RulesSame
    );
    let (a, b) = (wb("a.csv", "code\n007\n"), wb("b.csv", "code\n7\n"));
    assert_eq!(
        verdict(&a, &b, &TableCompareOptions::default()),
        ContentKind::ImportantDifferences
    );
    // An xlsx number against a CSV id with a leading zero: important too.
    let x = la(
        xlsx(|book| {
            let s = book.add_worksheet();
            s.write_string(0, 0, "zip").unwrap();
            s.write_number(1, 0, 1234.0).unwrap();
        }),
        "x.xlsx",
    );
    let c = wb("c.csv", "zip\n01234\n");
    assert_eq!(
        verdict(&x, &c, &TableCompareOptions::default()),
        ContentKind::ImportantDifferences
    );
}

/// T-6's rationale, pinned: per-column detection keeps `007` ≠ `7` in an id column of text, and the opportunistic
/// rule keeps one stray `n/a` from turning a numeric column into text on one side.
#[test]
fn t6_rationale_id_columns_and_stray_text() {
    let l = wb("l.csv", "n,id,qty\n1,007,1.0\n2,008,2.50\n3,009,n/a\n");
    let r = wb("r.csv", "n,id,qty\n1,7,1\n2,8,2.5\n3,9,7\n");
    let c = cmp(&l, &r, &keyed("n"));
    let s = &c.sheets()[0];
    let important: Vec<u32> = s
        .all_differing_cells()
        .iter()
        .filter(|d| d.state == hikasa_table::CellState::Important)
        .map(|d| d.column)
        .collect();
    // Every id differs; of the quantities only `n/a` against `7` (`1.0` = `1`, `2.50` = `2.5` although the left
    // column is text).
    assert_eq!(important, vec![1, 1, 1, 2]);
    assert_eq!(s.stats().cells_unimportant, 0);
    assert_eq!(
        (s.columns()[2].left_kind, s.columns()[2].right_kind),
        (
            hikasa_table::ColumnKind::Text,
            hikasa_table::ColumnKind::Number
        )
    );
}

// F6: workbook precision.

#[test]
fn workbook_floats_equal_their_csv_spelling_unless_exact() {
    let x = la(
        xlsx(|book| {
            let s = book.add_worksheet();
            s.write_string(0, 0, "n").unwrap();
            s.write_number(1, 0, 0.1 + 0.2).unwrap();
            s.write_number(2, 0, 1.0 / 3.0).unwrap();
        }),
        "x.xlsx",
    );
    let c = wb("c.csv", "n\n0.3\n0.333333333333333\n");
    assert_eq!(
        verdict(&x, &c, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
    let mut exact = TableCompareOptions::default();
    exact.default_sheet.default_rule.exact_numbers = true;
    assert_eq!(verdict(&x, &c, &exact), ContentKind::ImportantDifferences);
    // As keys too.
    assert_eq!(verdict(&x, &c, &keyed("n")), ContentKind::RulesSame);
}

// F3: shared strings are interned; the text budget refuses cleanly.

fn sst_book(text_len: usize, cells: u32) -> Vec<u8> {
    let big = "a".repeat(text_len);
    let data: String = (1..=cells)
        .map(|r| format!(r#"<row r="{r}"><c r="A{r}" t="s"><v>0</v></c></row>"#))
        .collect();
    common::minimal_xlsx(&data, &[&big], false, "")
}

#[test]
fn a_shared_string_is_stored_once_per_sheet() {
    let w = load(sst_book(200_000, 150), "s.xlsx").unwrap();
    let s = &w.sheets()[0];
    assert_eq!(s.row_count() + u32::from(s.header().is_some()), 150);
    assert_eq!(
        s.cell(10, 0),
        hikasa_table::Value::Text(&"a".repeat(200_000))
    );
    // 150 cells, one copy of the text.
    assert!(s.heap_bytes() < 300_000, "{}", s.heap_bytes());
}

#[test]
fn the_text_budget_refuses_with_a_typed_error() {
    let opts = |max: u64| LoadOptions {
        spreadsheet: hikasa_table::SpreadsheetOptions {
            max_text_bytes: Some(max),
            ..hikasa_table::SpreadsheetOptions::default()
        },
        ..LoadOptions::default()
    };
    let inline: String = (1..=20)
        .map(|r| {
            format!(
                r#"<row r="{r}"><c r="A{r}" t="inlineStr"><is><t>{}</t></is></c></row>"#,
                format!("{r:04}").repeat(250)
            )
        })
        .collect();
    let book = common::minimal_xlsx(&inline, &[], false, "");
    let run = |max| {
        load_bytes(
            book.clone(),
            "t.xlsx",
            &TableFormat::from_name("t.xlsx"),
            &opts(max),
            &CancelToken::new(),
        )
    };
    match run(10_000) {
        Err(TableError::TooLarge {
            what: "bytes of text",
            limit: 10_000,
            ..
        }) => {}
        other => panic!("{other:?}"),
    }
    assert!(run(20_000).is_ok());
    // The default budget: 64 × the input, at least 1 GiB.
    let d = hikasa_table::SpreadsheetOptions::default();
    assert_eq!(d.text_budget(1000), 1 << 30);
    assert_eq!(d.text_budget(1 << 30), 64 << 30);
}

// F4: a gap costs nothing.

fn wide_book(rows: u32) -> Vec<u8> {
    let data: String = (1..=rows)
        .map(|r| {
            format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="XFD{r}"><v>2</v></c></row>"#)
        })
        .collect();
    common::minimal_xlsx(&data, &[], false, "")
}

#[test]
fn rows_with_a_and_xfd_store_two_cells_and_compare_fast() {
    let w = la(wide_book(3000), "w.xlsx");
    let s = &w.sheets()[0];
    assert!(s.is_sparse());
    assert_eq!(s.column_count(), 16_384);
    assert!(s.heap_bytes() < 200_000, "{}", s.heap_bytes());
    let o = TableCompareOptions::default();
    assert_eq!(verdict(&w, &w, &o), ContentKind::RulesSame);
    let c = cmp(&w, &w, &o);
    assert_eq!(c.stats().totals.cells_compared, 3000 * 16_384);
    // One changed cell far right, one row more: found, keyed and unkeyed.
    let changed: String = (1..=3001)
        .map(|r| {
            let v = if r == 1500 { 3 } else { 2 };
            format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c><c r="XFD{r}"><v>{v}</v></c></row>"#)
        })
        .collect();
    let x = la(common::minimal_xlsx(&changed, &[], false, ""), "x.xlsx");
    for o in [TableCompareOptions::default(), keyed("A")] {
        let c = cmp(&w, &x, &o);
        let t = c.stats().totals;
        assert_eq!((t.cells_important, t.right_only_important), (1, 1), "{t:?}");
        let d = c.sheets()[0].all_differing_cells();
        assert_eq!(d[0].column, 16_383);
    }
}

#[test]
fn far_rows_and_columns_are_refused_not_allocated() {
    let far_row = r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="3000000"><c r="A3000000"><v>2</v></c></row>"#;
    match load(common::minimal_xlsx(far_row, &[], false, ""), "f.xlsx") {
        Err(TableError::TooLarge { what: "rows", .. }) => {}
        other => panic!("{other:?}"),
    }
}

// F8: cells and rows out of order.

#[test]
fn cells_and_rows_out_of_order_load_sorted() {
    let texts = |data: &str| -> Vec<Vec<String>> {
        let w = load(common::minimal_xlsx(data, &[], false, ""), "o.xlsx").unwrap();
        let s = &w.sheets()[0];
        (0..s.row_count())
            .map(|r| {
                s.row(r)
                    .values()
                    .map(|v| v.display().into_owned())
                    .collect()
            })
            .collect()
    };
    assert_eq!(
        texts(r#"<row r="1"><c r="B1"><v>2</v></c><c r="A1"><v>1</v></c></row>"#),
        vec![vec!["1", "2"]]
    );
    assert_eq!(
        texts(
            r#"<row r="2"><c r="A2"><v>2</v></c></row><row r="1"><c r="A1"><v>1</v></c><c r="C1"><v>3</v></c></row>"#
        ),
        vec![vec!["1", "", "3"], vec!["2"]]
    );
    // A duplicate position keeps its last cell; cells without references follow their row.
    assert_eq!(
        texts(r#"<row r="1"><c r="A1"><v>1</v></c><c r="A1"><v>5</v></c></row>"#),
        vec![vec!["5"]]
    );
    assert_eq!(
        texts(r#"<row><c><v>1</v></c><c><v>2</v></c></row><row><c><v>3</v></c></row>"#),
        vec![vec!["1", "2"], vec!["3"]]
    );
    // Rows out of order with shared strings and formulas: values and formulas follow their cells.
    let w = load(
        common::minimal_xlsx(
            r#"<row r="3"><c r="A3" t="s"><v>1</v></c><c r="B3"><f>A1*2</f><v>2</v></c></row><row r="1"><c r="A1" t="s"><v>0</v></c></row>"#,
            &["first", "third"],
            false,
            "",
        ),
        "o.xlsx",
    )
    .unwrap();
    let s = &w.sheets()[0];
    assert_eq!(s.cell(0, 0), hikasa_table::Value::Text("first"));
    assert_eq!(s.cell(2, 0), hikasa_table::Value::Text("third"));
    assert_eq!(s.formula(2, 1), Some("A1*2"));
}

/// Peak RSS of this process (`VmHWM`), in MiB.
fn peak_mib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

/// The review's F3 probe, by hand under a memory cap: a 22 KB xlsx whose one 20 MB shared string fills 150 cells
/// (5.8 GB before the fix).
#[test]
#[ignore = "memory probe, by hand under systemd-run -p MemoryMax"]
fn probe_sst_amplification() {
    let book = sst_book(20_000_000, 150);
    let t = std::time::Instant::now();
    let w = load(book.clone(), "s.xlsx").unwrap();
    let c = cmp(
        &Arc::new(w.clone()),
        &Arc::new(w),
        &TableCompareOptions::default(),
    );
    assert_eq!(c.kind(false, false), ContentKind::RulesSame);
    eprintln!(
        "probe F3: {} bytes in, load+compare {:?}, peak {} MiB",
        book.len(),
        t.elapsed(),
        peak_mib()
    );
}

/// The review's F4 probe, by hand under a memory cap: an 81 KB xlsx of 12,000 rows using A and XFD (2.26 GB and
/// 10 s before the fix).
#[test]
#[ignore = "memory probe, by hand under systemd-run -p MemoryMax"]
fn probe_wide_padding() {
    let book = wide_book(12_000);
    let t = std::time::Instant::now();
    let w = Arc::new(load(book.clone(), "w.xlsx").unwrap());
    let loaded = t.elapsed();
    let c = cmp(&w, &w, &TableCompareOptions::default());
    assert_eq!(c.kind(false, false), ContentKind::RulesSame);
    eprintln!(
        "probe F4: {} bytes in, load {:?}, load+compare {:?}, peak {} MiB",
        book.len(),
        loaded,
        t.elapsed(),
        peak_mib()
    );
}
