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
    // Two entries used in turn: one copy each, whatever the order of the cells.
    let (a, b) = ("a".repeat(100_000), "b".repeat(100_000));
    let data: String = (1..=150)
        .map(|r| {
            format!(
                r#"<row r="{r}"><c r="A{r}" t="s"><v>{}</v></c></row>"#,
                r % 2
            )
        })
        .collect();
    let w = load(common::minimal_xlsx(&data, &[&a, &b], false, ""), "t.xlsx").unwrap();
    let s = &w.sheets()[0];
    assert_eq!(s.row(0).get(0).display().len(), 100_000);
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

// F10: the 1904 epoch's serial 0.

#[test]
fn serial_zero_of_the_1904_epoch_is_its_first_day() {
    let row = |v: &str| format!(r#"<row r="1"><c r="A1" s="1"><v>{v}</v></c></row>"#);
    let d1904 = la(common::minimal_xlsx(&row("0"), &[], true, ""), "a.xlsx");
    let d1900 = la(common::minimal_xlsx(&row("1462"), &[], false, ""), "b.xlsx");
    assert_eq!(d1904.sheets()[0].cell(0, 0).display(), "1904-01-01");
    assert_eq!(d1900.sheets()[0].cell(0, 0).display(), "1904-01-01");
    assert_eq!(
        verdict(&d1904, &d1900, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
    // A 1900 workbook's serial 0 stays the time 00:00 of day 0.
    let zero = la(common::minimal_xlsx(&row("0"), &[], false, ""), "z.xlsx");
    assert_eq!(zero.sheets()[0].cell(0, 0).display(), "1899-12-30");
}

// F11: detection reads the package directory.

#[test]
fn detection_reads_the_package_directory() {
    let good = xlsx(|b| {
        b.add_worksheet().write_number(0, 0, 1.0).unwrap();
    });
    // A truncated workbook is damaged, at any cut.
    for cut in [10usize, 100, 1000, good.len() / 2, good.len() - 10] {
        match load(good[..cut].to_vec(), "t.xlsx") {
            Err(TableError::Corrupt { message, .. }) => {
                assert!(message.contains("damaged or truncated"), "{cut}: {message}")
            }
            other => panic!("{cut}: {other:?}"),
        }
    }
    // Truncated and renamed .bin: still a workbook by its first part.
    match load(good[..good.len() / 2].to_vec(), "t.bin") {
        Err(TableError::Corrupt { message, .. }) => assert!(message.contains("truncated")),
        other => panic!("{other:?}"),
    }
    // A zip holding a workbook is an archive, not a workbook.
    let outer = common::zip_of(&[("inner.xlsx", "PK not really"), ("readme.txt", "hi")]);
    assert_eq!(
        hikasa_table::detect_workbook(&outer, "a.zip").unwrap(),
        None
    );
    match load(outer, "a.xlsx") {
        Err(TableError::Corrupt { message, .. }) => {
            assert!(message.contains("zip archive"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    // An OpenDocument text is no spreadsheet; an OpenDocument spreadsheet is.
    let odt = stored_zip(&[
        ("mimetype", "application/vnd.oasis.opendocument.text"),
        ("content.xml", "<x/>"),
    ]);
    assert_eq!(hikasa_table::detect_workbook(&odt, "a.odt").unwrap(), None);
    match load(odt.clone(), "a.ods") {
        Err(TableError::Corrupt { message, .. }) => {
            assert!(message.contains("opendocument.text"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    let ods = common::minimal_ods("S", &[vec![("float", "1")]]);
    assert_eq!(
        hikasa_table::detect_workbook(&ods, "renamed.bin").unwrap(),
        Some(hikasa_table::SourceKind::Ods)
    );
    // A CSV that starts like a package and names `content.xml` is still a CSV.
    let w = load(b"PK\x03\x04,content.xml\n1,2\n".to_vec(), "a.csv").unwrap();
    assert_eq!(w.source(), hikasa_table::SourceKind::Delimited);
}

/// A zip of stored members (the ODF `mimetype` member must be stored).
fn stored_zip(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn a_declared_string_table_over_the_budget_is_refused_before_reading() {
    let book = sst_book(100_000, 3);
    let opts = LoadOptions {
        spreadsheet: hikasa_table::SpreadsheetOptions {
            max_text_bytes: Some(50_000),
            ..hikasa_table::SpreadsheetOptions::default()
        },
        ..LoadOptions::default()
    };
    match load_bytes(
        book,
        "s.xlsx",
        &TableFormat::from_name("s.xlsx"),
        &opts,
        &CancelToken::new(),
    ) {
        Err(TableError::TooLarge {
            what: "bytes of shared strings",
            ..
        }) => {}
        other => panic!("{other:?}"),
    }
}

// F12: skipped hidden sheets warn; a trailing Ctrl+Z is no data.

#[test]
fn skipped_hidden_sheets_warn_and_a_trailing_ctrl_z_is_dropped() {
    let book = xlsx(|b| {
        b.add_worksheet()
            .set_name("Shown")
            .unwrap()
            .write_number(0, 0, 1.0)
            .unwrap();
        let h = b.add_worksheet();
        h.set_name("Secret")
            .unwrap()
            .write_number(0, 0, 2.0)
            .unwrap();
        h.set_hidden(true);
    });
    let opts = LoadOptions {
        spreadsheet: hikasa_table::SpreadsheetOptions {
            load_hidden: false,
            ..hikasa_table::SpreadsheetOptions::default()
        },
        ..LoadOptions::default()
    };
    let w = load_bytes(
        book,
        "h.xlsx",
        &TableFormat::from_name("h.xlsx"),
        &opts,
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(w.sheets().len(), 1);
    let warn = &w.warnings()[0];
    assert_eq!(warn.code, hikasa_table::TableWarningCode::SheetSkipped);
    assert_eq!(warn.sheet.as_deref(), Some("Secret"));
    // `a,b / 1,2` and a lone 0x1A: two rows, not three.
    let w = load(b"a,b\r\n1,2\r\n\x1a".to_vec(), "z.csv").unwrap();
    assert_eq!(w.sheets()[0].row_count(), 1);
    let plain = load(b"a,b\r\n1,2\r\n".to_vec(), "z.csv").unwrap();
    assert_eq!(
        verdict(
            &Arc::new(w),
            &Arc::new(plain),
            &TableCompareOptions::default()
        ),
        ContentKind::RulesSame
    );
    // Inside the data a 0x1A stays data unless ctrl_z_eof.
    let w = load(b"a\n\x1a\nb\n".to_vec(), "m.csv").unwrap();
    assert_eq!(
        w.sheets()[0].cell(1, 0),
        hikasa_table::Value::Text("\u{1a}")
    );
}

// F5: the header vote allows blank names and years.

#[test]
fn pandas_style_headers_with_a_blank_corner_align_by_name() {
    let book = |swap: bool| {
        xlsx(|b| {
            let s = b.add_worksheet();
            let (name_col, qty_col) = if swap { (2, 1) } else { (1, 2) };
            s.write_string(0, name_col, "name").unwrap();
            s.write_string(0, qty_col, "qty").unwrap();
            for i in 0..5u32 {
                s.write_number(i + 1, 0, f64::from(i)).unwrap();
                s.write_string(i + 1, name_col, format!("item{i}")).unwrap();
                s.write_number(i + 1, qty_col, f64::from(i * 10)).unwrap();
            }
        })
    };
    let (l, r) = (la(book(false), "l.xlsx"), la(book(true), "r.xlsx"));
    assert_eq!(
        l.sheets()[0].header(),
        Some(&["".to_owned(), "name".to_owned(), "qty".to_owned()][..])
    );
    assert_eq!(l.sheets()[0].column_name(0), "A");
    assert_eq!(
        verdict(&l, &r, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
    // The same frame written as CSV (index column without a name) against the xlsx.
    let csv = wb(
        "r.csv",
        ",qty,name\n0,0,item0\n1,10,item1\n2,20,item2\n3,30,item3\n4,40,item4\n",
    );
    assert!(csv.sheets()[0].header().is_some());
    assert_eq!(
        verdict(&l, &csv, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
}

#[test]
fn year_headers_align_by_name() {
    let y1 = wb("y1.csv", "region,2023,2024\nnorth,1,2\nsouth,3,4\n");
    let y2 = wb("y2.csv", "region,2024,2023\nnorth,2,1\nsouth,4,3\n");
    assert!(y1.sheets()[0].header().is_some());
    assert_eq!(
        verdict(&y1, &y2, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
    // Typed years in a workbook header too.
    let x = la(
        xlsx(|b| {
            let s = b.add_worksheet();
            s.write_string(0, 0, "region").unwrap();
            s.write_number(0, 1, 2024.0).unwrap();
            s.write_number(0, 2, 2023.0).unwrap();
            for (r, (n, a, b2)) in [("north", 2.0, 1.0), ("south", 4.0, 3.0)]
                .iter()
                .enumerate()
            {
                s.write_string(r as u32 + 1, 0, *n).unwrap();
                s.write_number(r as u32 + 1, 1, *a).unwrap();
                s.write_number(r as u32 + 1, 2, *b2).unwrap();
            }
        }),
        "y.xlsx",
    );
    assert_eq!(
        x.sheets()[0].header().map(|h| h.to_vec()),
        Some(vec!["region".into(), "2024".into(), "2023".into()])
    );
    assert_eq!(
        verdict(&y1, &x, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
    // Dates as column names over numbers are a header too.
    let d = wb(
        "d.csv",
        "region,2024-01-31,2024-02-29\nnorth,1,2\nsouth,3,4\n",
    );
    assert!(d.sheets()[0].header().is_some());
}

#[test]
fn data_rows_stay_data() {
    for text in [
        "1,north,5\n2,south,7\n3,east,9\n",
        "2023,north,5\n2024,south,7\n2025,east,9\n",
        "2024-01-31,5\n2024-02-01,6\n",
        "a,1\nbb,2\ncc,3\n",
    ] {
        let w = wb("d.csv", text);
        assert!(w.sheets()[0].header().is_none(), "{text:?}");
    }
    // Years over numbers alone decide nothing.
    assert!(
        wb("y.csv", "2023,2024\n1,2\n3,4\n").sheets()[0]
            .header()
            .is_none()
    );
}

// F7: the sniffer keeps timestamps whole.

#[test]
fn single_column_timestamp_files_stay_one_column() {
    let one_column = |text: &str| {
        let w = wb("t.txt", text);
        let s = &w.sheets()[0];
        assert_eq!(s.column_count(), 1, "{text:?} → {:?}", w.detected().kind);
        w
    };
    let mut times = String::from("time\n");
    let mut stamps = String::from("when\n");
    let mut bare = String::new();
    let mut us = String::new();
    for i in 0..20 {
        times.push_str(&format!("12:{i:02}:00\n"));
        stamps.push_str(&format!("2024-01-02 12:{i:02}:00\n"));
        bare.push_str(&format!("2024-01-02T12:{i:02}:00\n"));
        us.push_str(&format!("01/{:02}/2024 1:{i:02} PM\n", i + 1));
    }
    let w = one_column(&times);
    assert_eq!(w.sheets()[0].row_count(), 20);
    assert_eq!(
        w.sheets()[0].column_kind(0),
        hikasa_table::ColumnKind::DateTime
    );
    one_column(&stamps);
    one_column(&bare);
    one_column(&us);
    one_column("when\n2024-01-02 12:00:00\n");
    // Real delimiters still win, and a delimiter must occur in the first record.
    assert_eq!(
        wb("t.txt", "at,v\n12:00:00,1\n12:01:00,2\n").sheets()[0].column_count(),
        2
    );
    assert_eq!(wb("t.txt", "a b c\nx y z\n").sheets()[0].column_count(), 3);
}

// F9: low-entropy rows after Standard's fallback.

#[test]
fn a_shifted_low_entropy_table_aligns_nearly_all_rows() {
    let (mut a, mut b) = (String::from("k,v\n"), String::from("k,v\n"));
    for i in 0..200_000 {
        a.push_str(&format!("1,{}\n", i % 7));
        b.push_str(&format!("1,{}\n", (i + 1) % 7));
    }
    let (l, r) = (wb("a.csv", &a), wb("b.csv", &b));
    let c = cmp(&l, &r, &TableCompareOptions::default());
    let t = c.stats().totals;
    assert_eq!(t.same_rows, 199_999, "{t:?}");
    assert_eq!(
        t.changed_important + t.left_only_important + t.right_only_important,
        2,
        "{t:?}"
    );
    assert!(
        c.warnings()
            .iter()
            .any(|w| w.code == hikasa_table::TableWarningCode::AlignmentDegraded)
    );
    // Every row of each side appears once, in order.
    let rows = c.sheets()[0].rows();
    let left: Vec<u32> = rows.iter().filter_map(|x| x.left()).collect();
    let right: Vec<u32> = rows.iter().filter_map(|x| x.right()).collect();
    assert_eq!(left, (0..200_000).collect::<Vec<u32>>());
    assert_eq!(right, (0..200_000).collect::<Vec<u32>>());
    // Mirrored.
    let m = cmp(&r, &l, &TableCompareOptions::default()).stats().totals;
    assert_eq!(m.same_rows, 199_999);
}

// F13: hidden rows and columns.

fn hidden_book() -> Vec<u8> {
    xlsx(|b| {
        let s = b.add_worksheet();
        s.set_name("Data").unwrap();
        for (c, h) in ["id", "secret", "qty"].iter().enumerate() {
            s.write_string(0, c as u16, *h).unwrap();
        }
        for i in 1..=4u32 {
            s.write_number(i, 0, f64::from(i)).unwrap();
            s.write_string(i, 1, format!("s{i}")).unwrap();
            s.write_number(i, 2, f64::from(i * 10)).unwrap();
        }
        // Row 3 (id 3) and column B are hidden.
        s.set_row_hidden(3).unwrap();
        s.set_column_hidden(1).unwrap();
    })
}

#[test]
fn hidden_rows_and_columns_are_flags_and_can_be_ignored() {
    let x = la(hidden_book(), "h.xlsx");
    let s = &x.sheets()[0];
    assert_eq!(s.info().hidden_rows, vec![(3, 3)]);
    assert_eq!(s.info().hidden_columns, vec![(1, 1)]);
    assert!(s.row_hidden(2) && !s.row_hidden(1));
    assert!(s.column_hidden(1) && !s.column_hidden(2));
    // The visible table, as a CSV.
    let c = wb("v.csv", "id,qty\n1,10\n2,20\n4,40\n");
    let mut o = TableCompareOptions::default();
    assert_eq!(verdict(&x, &c, &o), ContentKind::ImportantDifferences);
    o.default_sheet.ignore_hidden_rows = true;
    o.default_sheet.ignore_hidden_columns = true;
    assert_eq!(verdict(&x, &c, &o), ContentKind::RulesSame);
    // Keyed too; rows keep their sheet row labels.
    o.default_sheet.keys = vec![ColumnRef::Auto("id".into())];
    let cmp_ = cmp(&x, &c, &o);
    assert_eq!(cmp_.kind(false, false), ContentKind::RulesSame);
    let labels: Vec<u64> = (0..cmp_.sheets()[0].rows().len() as u32)
        .filter_map(|i| cmp_.row_label(0, i, hikasa_core::Side::Left))
        .collect();
    assert_eq!(labels, vec![2, 3, 5]);
    // A value changed in the hidden row is ignored, in a visible row it is not.
    let changed = la(
        xlsx(|b| {
            let s = b.add_worksheet();
            s.set_name("Data").unwrap();
            for (c, h) in ["id", "secret", "qty"].iter().enumerate() {
                s.write_string(0, c as u16, *h).unwrap();
            }
            for i in 1..=4u32 {
                s.write_number(i, 0, f64::from(i)).unwrap();
                s.write_string(i, 1, "other").unwrap();
                let qty = if i == 3 { 999 } else { i * 10 };
                s.write_number(i, 2, f64::from(qty)).unwrap();
            }
            s.set_row_hidden(3).unwrap();
            s.set_column_hidden(1).unwrap();
        }),
        "c.xlsx",
    );
    o.default_sheet.keys.clear();
    assert_eq!(verdict(&x, &changed, &o), ContentKind::RulesSame);
    assert_eq!(
        verdict(&x, &changed, &TableCompareOptions::default()),
        ContentKind::ImportantDifferences
    );
    // Off by load option: no flags.
    let off = load_bytes(
        hidden_book(),
        "h.xlsx",
        &TableFormat::from_name("h.xlsx"),
        &LoadOptions {
            spreadsheet: hikasa_table::SpreadsheetOptions {
                read_hidden_rows_columns: false,
                ..hikasa_table::SpreadsheetOptions::default()
            },
            ..LoadOptions::default()
        },
        &CancelToken::new(),
    )
    .unwrap();
    assert!(off.sheets()[0].info().hidden_rows.is_empty());
}

#[test]
fn rows_hidden_by_default_are_the_unlisted_ones() {
    let book = xlsx(|b| {
        let s = b.add_worksheet();
        s.write_number(0, 0, 1.0).unwrap();
        s.write_number(1, 0, 2.0).unwrap();
        s.hide_unused_rows(true);
    });
    let w = la(book, "z.xlsx");
    assert_eq!(w.sheets()[0].info().hidden_rows, vec![(2, 1_048_575)]);
    assert!(!w.sheets()[0].row_hidden(1));
}

// The review's surviving mutations (contract §9.8): each dies with one of these.

/// R5: a column empty on one side takes the other side's kind (here Text: keys order as texts, `10` before `9`).
#[test]
fn a_column_empty_on_one_side_takes_the_other_sides_kind() {
    let l = wb("l.csv", "id,name\n10,xx\n9,yy\nx,zz\n");
    let r = wb("r.csv", "id,name\n,xx\n,yy\n,zz\n");
    assert_eq!(
        r.sheets()[0].column_kind(0),
        hikasa_table::ColumnKind::Empty
    );
    let c = cmp(&l, &r, &keyed("id"));
    let left: Vec<u32> = c.sheets()[0]
        .rows()
        .iter()
        .filter_map(|x| x.left())
        .collect();
    assert_eq!(left, vec![0, 1, 2]);
}

/// R7, R10, R12: a formula without a cached value is an empty cell that keeps its column and its formula, and a
/// single one is reported; an empty cached string is an empty cell.
#[test]
fn formulas_without_values_keep_their_cells() {
    let w = load(
        common::minimal_xlsx(
            r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>A1*2</f></c></row><row r="2"><c r="A2" t="str"><f>""</f><v></v></c></row>"#,
            &[],
            false,
            "",
        ),
        "f.xlsx",
    )
    .unwrap();
    let s = &w.sheets()[0];
    assert!(s.header().is_none());
    assert_eq!(s.row(0).len(), 2);
    assert_eq!(s.column_count(), 2);
    assert_eq!(s.cell(0, 1), hikasa_table::Value::Empty);
    assert_eq!(s.formula(0, 1), Some("A1*2"));
    assert_eq!(s.cell(1, 0), hikasa_table::Value::Empty);
    assert_eq!(s.formula(1, 0), Some("\"\""));
    let warned: Vec<_> = w
        .warnings()
        .iter()
        .filter(|x| x.code == hikasa_table::TableWarningCode::FormulaWithoutValue)
        .collect();
    assert_eq!(warned.len(), 1, "{:?}", w.warnings());
    assert!(warned[0].message.starts_with("2 formulas"));
    let one = load(
        common::minimal_xlsx(
            r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>A1*2</f></c></row>"#,
            &[],
            false,
            "",
        ),
        "f.xlsx",
    )
    .unwrap();
    assert!(
        one.warnings()
            .iter()
            .any(|x| x.code == hikasa_table::TableWarningCode::FormulaWithoutValue)
    );
}

/// R8, R13, R15: durations stay durations, a time fraction rounds to the millisecond, 9999-12-31 is a date.
#[test]
fn durations_time_fractions_and_the_last_date() {
    let book = xlsx(|b| {
        let dur = rust_xlsxwriter::Format::new().set_num_format("[h]:mm");
        let time = rust_xlsxwriter::Format::new().set_num_format("hh:mm:ss");
        let date = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        let s = b.add_worksheet();
        s.write_number_with_format(0, 0, 1.5, &dur).unwrap();
        // 11 s: 11/86400 × 86,400,000 is 10,999.999999999998 ms, which must round to 11,000.
        s.write_number_with_format(0, 1, 11.0 / 86_400.0, &time)
            .unwrap();
        s.write_number_with_format(0, 2, 2_958_465.0, &date)
            .unwrap();
    });
    let w = load(book, "d.xlsx").unwrap();
    let s = &w.sheets()[0];
    assert_eq!(
        s.cell(0, 0),
        hikasa_table::Value::Duration(jiff::SignedDuration::from_hours(36))
    );
    assert_eq!(s.cell(0, 1).display(), "1899-12-30 00:00:11");
    assert_eq!(s.cell(0, 2).display(), "9999-12-31");
}

/// R9: a very hidden sheet is hidden too.
#[test]
fn very_hidden_sheets_are_hidden() {
    let book = xlsx(|b| {
        b.add_worksheet().write_number(0, 0, 1.0).unwrap();
        let v = b.add_worksheet();
        v.set_name("Deep").unwrap().write_number(0, 0, 2.0).unwrap();
        v.set_very_hidden(true);
    });
    let w = load(book, "v.xlsx").unwrap();
    assert!(!w.sheets()[0].info().hidden);
    assert!(w.sheets()[1].info().hidden);
}

/// R14: a workbook's -0 is the key 0.
#[test]
fn negative_zero_is_zero_as_a_key() {
    let x = la(
        xlsx(|b| {
            let s = b.add_worksheet();
            s.write_string(0, 0, "k").unwrap();
            s.write_string(0, 1, "v").unwrap();
            s.write_number(1, 0, -0.0).unwrap();
            s.write_string(1, 1, "a").unwrap();
        }),
        "z.xlsx",
    );
    assert!(
        matches!(x.sheets()[0].cell(0, 0), hikasa_table::Value::Number(n) if n == 0.0 && n.is_sign_negative())
    );
    let c = wb("z.csv", "k,v\n0,a\n");
    assert_eq!(verdict(&x, &c, &keyed("k")), ContentKind::RulesSame);
}

/// R17: the header vote reads 50 body rows: a name that the column repeats within them is no name.
#[test]
fn the_header_vote_reads_fifty_body_rows() {
    let table = |at: u32| {
        let mut t = String::from("status,n\n");
        for i in 1..=80u32 {
            if i == at {
                t.push_str(&format!("status,{i}\n"));
            } else {
                t.push_str(&format!("open{i},{i}\n"));
            }
        }
        wb("s.csv", &t)
    };
    assert!(table(30).sheets()[0].header().is_none());
    assert!(table(70).sheets()[0].header().is_some());
}
