//! The review's probes (`zz_review.rs` of the 2026-11-10 review, print-only there) as asserting tests: cross-type
//! equalities, dates, keys with duplicates, sheet pairing by case, tolerances, sniffing shapes, shared formulas,
//! xlsx against CSV, alignment quality and the pathological sizes (scaled down to run in every suite).

mod common;

use std::sync::Arc;

use common::{cmp, minimal_xlsx, wb, xlsx, zip_of};
use hikasa_core::{CancelToken, ContentKind};
use hikasa_table::{
    CellState, ColumnAlignment, ColumnRef, LoadOptions, TableCompareOptions, TableError,
    TableFormat, Tolerance, Value, Workbook, load_bytes, parse_algorithm,
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

/// The verdict, the same with the sides swapped.
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

/// p_types: text `1` = number 1, bool = `TRUE`, error = `#N/A` text, shared = inline rich text, a formula without
/// value = one with an empty value; the 1900 serial 1462 equals the 1904 serial 0 (F10).
#[test]
fn cross_type_equalities_of_a_hand_built_workbook() {
    let a = minimal_xlsx(
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1" s="1"><v>59</v></c><c r="C1" s="1"><v>60</v></c><c r="D1" s="1"><v>1462</v></c><c r="E1" t="b"><v>1</v></c><c r="F1" t="e"><v>#N/A</v></c><c r="G1" t="s"><v>0</v></c><c r="H1"><f>A1+1</f></c><c r="I1" t="inlineStr"><is><r><t>ri</t></r><r><t>ch</t></r></is></c><c r="J1" s="1"><v>0.5</v></c></row>"#,
        &["x"],
        false,
        "",
    );
    let b = minimal_xlsx(
        r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" s="1"><v>59</v></c><c r="C1" s="1"><v>59</v></c><c r="D1" s="1"><v>0</v></c><c r="E1" t="str"><v>TRUE</v></c><c r="F1" t="str"><v>#N/A</v></c><c r="G1" t="s"><v>1</v></c><c r="H1"><f>A1+1</f><v></v></c><c r="I1" t="s"><v>2</v></c><c r="J1" s="1"><v>0.5</v></c></row>"#,
        &["1", "x", "rich"],
        true,
        "",
    );
    let (wa, wbk) = (la(a, "a.xlsx"), la(b, "b.xlsx"));
    let mut o = TableCompareOptions::default();
    o.default_sheet.columns = ColumnAlignment::Number;
    let c = cmp(&wa, &wbk, &o);
    let d: Vec<(u32, CellState)> = c.sheets()[0]
        .all_differing_cells()
        .iter()
        .map(|d| (d.column, d.state))
        .collect();
    // The right workbook is of the 1904 epoch: its serial 59 is 1904-02-29, not the left's 1900-02-28 (B, C); its
    // serial 0 is 1904-01-01, the left's 1462 (D: same since F10). Everything else is same across types.
    assert_eq!(
        d,
        vec![(1, CellState::Important), (2, CellState::Important)],
        "{d:?}"
    );
    assert_eq!(verdict(&wa, &wbk, &o), ContentKind::ImportantDifferences);
}

/// p_dates_xlsxwriter: a date, a time, a `[h]:mm` duration and a raw serial with a date format.
#[test]
fn generated_dates_times_and_durations() {
    let book = xlsx(|b| {
        let f = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        let t = rust_xlsxwriter::Format::new().set_num_format("hh:mm:ss");
        let dur = rust_xlsxwriter::Format::new().set_num_format("[h]:mm");
        let s = b.add_worksheet();
        let dt = rust_xlsxwriter::ExcelDateTime::from_ymd(2024, 2, 29).unwrap();
        s.write_datetime_with_format(0, 0, &dt, &f).unwrap();
        let tm = rust_xlsxwriter::ExcelDateTime::from_hms(13, 30, 0).unwrap();
        s.write_datetime_with_format(0, 1, &tm, &t).unwrap();
        s.write_number_with_format(0, 2, 1.5, &dur).unwrap();
        s.write_number_with_format(0, 3, 45351.0, &f).unwrap();
    });
    let w = load(book, "d.xlsx").unwrap();
    let texts: Vec<String> = w.sheets()[0]
        .row(0)
        .values()
        .map(|v| v.display().into_owned())
        .collect();
    assert_eq!(
        texts,
        vec![
            "2024-02-29",
            "1899-12-30 13:30:00",
            "36:00:00",
            "2024-02-29"
        ]
    );
}

/// p_keys: duplicates, an empty key and a leading-zero key (`01` is not `1`, T-24).
#[test]
fn keys_with_duplicates_empties_and_leading_zeros() {
    let l = wb("l.csv", "id,v\n1,a\n1,b\n,c\n01,d\n2,e\n");
    let r = wb("r.csv", "id,v\n1.0,a\n,c\n1,b\n2,e\n1,z\n");
    let c = cmp(&l, &r, &keyed("id"));
    let s = c.sheets()[0].stats();
    assert_eq!((s.duplicate_left, s.duplicate_right), (2, 3));
    // 1/a, 1/b, empty/c, 2/e pair same; 01/d is left only, the third 1 (z) right only.
    assert_eq!(s.same_rows, 4, "{}", common::dump(&c));
    assert_eq!(
        (
            s.left_only_important,
            s.right_only_important,
            s.changed_important
        ),
        (1, 1, 0)
    );
    assert_eq!(c.kind(false, false), ContentKind::ImportantDifferences);
}

/// p_case_sheets and p_case_dup_sheets: names pair exactly, then case-insensitively; a third, case-duplicate sheet
/// is an orphan.
#[test]
fn sheets_pair_by_case_and_a_case_duplicate_is_an_orphan() {
    // rust_xlsxwriter refuses case-duplicate names: the workbook is hand-built.
    let mk = |names: &[&str]| {
        let (mut sheets, mut rels) = (String::new(), String::new());
        let mut parts: Vec<(String, String)> = Vec::new();
        for (i, n) in names.iter().enumerate() {
            let i = i + 1;
            let v = if *n == "data" { 3 } else { i - 1 };
            sheets.push_str(&format!(
                r#"<sheet name="{n}" sheetId="{i}" r:id="rId{i}"/>"#
            ));
            rels.push_str(&format!(r#"<Relationship Id="rId{i}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{i}.xml"/>"#));
            parts.push((
                format!("xl/worksheets/sheet{i}.xml"),
                format!(r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1"><v>{v}</v></c></row></sheetData></worksheet>"#),
            ));
        }
        let workbook = format!(
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{sheets}</sheets></workbook>"#
        );
        let wrels = format!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
        );
        let mut all: Vec<(&str, &str)> = vec![
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            ),
            ("xl/workbook.xml", &workbook),
            ("xl/_rels/workbook.xml.rels", &wrels),
        ];
        for (n, b) in &parts {
            all.push((n, b));
        }
        la(zip_of(&all), "c.xlsx")
    };
    let a = la(
        xlsx(|b| {
            b.add_worksheet()
                .set_name("Data")
                .unwrap()
                .write_number(0, 0, 1.0)
                .unwrap();
            b.add_worksheet()
                .set_name("Ünï")
                .unwrap()
                .write_number(0, 0, 0.0)
                .unwrap();
        }),
        "a.xlsx",
    );
    let b = mk(&["ünï", "DATA", "data"]);
    let c = cmp(&a, &b, &TableCompareOptions::default());
    let pairs: Vec<(Option<u32>, Option<u32>)> =
        c.sheets().iter().map(|s| (s.left(), s.right())).collect();
    // Data ↔ DATA (the first case-insensitive match), Ünï ↔ ünï, `data` right only.
    assert!(pairs.contains(&(Some(0), Some(1))), "{pairs:?}");
    assert!(pairs.contains(&(Some(1), Some(0))), "{pairs:?}");
    assert!(pairs.contains(&(None, Some(2))), "{pairs:?}");
    assert_eq!(c.kind(false, false), ContentKind::ImportantDifferences);
}

/// p_tolerance_mirror: a relative tolerance is symmetric; `-0` is `0`.
#[test]
fn relative_tolerance_mirrors() {
    let l = wb("l.csv", "a\n100\n1e300\n-0\n");
    let r = wb("r.csv", "a\n101\n-1e300\n0\n");
    let mut o = keyed("a");
    o.default_sheet.keys.clear();
    o.default_sheet.default_rule.numeric_tolerance = Some(Tolerance::parse("1%").unwrap());
    let c = cmp(&l, &r, &o);
    let s = c.sheets()[0].stats();
    // 100/101 within 1 %, -0/0 same; 1e300 and -1e300 are rows of their own.
    assert_eq!((s.cells_same, s.cells_unimportant), (1, 1));
    assert_eq!((s.left_only_important, s.right_only_important), (1, 1));
    assert_eq!(verdict(&l, &r, &o), ContentKind::ImportantDifferences);
}

/// p_leading_zero: spellings of one value are same, identifiers and kinds differ.
#[test]
fn spellings_identifiers_and_kinds() {
    for (a, b, same) in [
        ("007", "7", false),
        ("1e3", "1000", true),
        ("\"1,000\"", "1000", true),
        ("NaN", "nan", false),
        ("TRUE", "1", false),
        ("2024-01-02", "45293", false),
        ("0012345678901234567890", "12345678901234567890", false),
        ("12345678901234567", "12345678901234568", false),
        // Columns of long numbers are text (T-24): another spelling is another text.
        ("12345678901234567", "12345678901234567.0", false),
    ] {
        let l = wb("l.csv", &format!("id,v\n{a},x\n"));
        let r = wb("r.csv", &format!("id,v\n{b},x\n"));
        let v = verdict(&l, &r, &TableCompareOptions::default());
        let expect = if same {
            ContentKind::RulesSame
        } else {
            ContentKind::ImportantDifferences
        };
        assert_eq!(v, expect, "{a} vs {b}");
    }
}

/// p_sniff: sniffed shapes.
#[test]
fn sniffed_shapes() {
    let shape = |name: &str, text: &str| -> (u32, u32, Vec<String>) {
        let w = la(text.as_bytes().to_vec(), name);
        let s = &w.sheets()[0];
        (
            s.row_count(),
            s.column_count(),
            s.row(0)
                .values()
                .map(|v| v.display().into_owned())
                .collect(),
        )
    };
    // A stray quote inside a field is data.
    let (rows, cols, first) = shape(
        "i.csv",
        "id,desc,qty\n1,5\" screen,3\n2,ok,4\n3,x,5\n4,y,6\n",
    );
    assert_eq!((rows, cols), (4, 3));
    assert_eq!(first[1], "5\" screen");
    // A qualified field holds a CRLF.
    let (rows, _, first) = shape("e.csv", "a,b\r\n\"x\r\ny\",2\r\n3,4\r\n");
    assert_eq!(rows, 2);
    assert_eq!(first[0], "x\r\ny");
    // A BOM and a trailing Ctrl+Z (T-28).
    let (rows, cols, _) = shape("b.csv", "\u{feff}a,b\r\n1,2\r\n\u{1a}");
    assert_eq!((rows, cols), (1, 2));
    // CJK text in fixed columns is cut by characters.
    let w = la(
        "名前  数量\n東京  10\nab    3\n".as_bytes().to_vec(),
        "f.txt",
    );
    assert!(matches!(
        w.detected().kind,
        hikasa_table::TableKind::Fixed(_)
    ));
    assert_eq!(w.sheets()[0].cell(0, 0), Value::Text("東京"));
}

/// p_shared_formula: shared formulas expand like explicit ones.
#[test]
fn shared_formulas_expand() {
    let shared = minimal_xlsx(
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f t="shared" ref="B1:B3" si="0">A1*2</f><v>2</v></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><f t="shared" si="0"/><v>4</v></c></row><row r="3"><c r="A3"><v>3</v></c><c r="B3"><f t="shared" si="0"/><v>6</v></c></row>"#,
        &[],
        false,
        "",
    );
    let explicit = minimal_xlsx(
        r#"<row r="1"><c r="A1"><v>1</v></c><c r="B1"><f>A1*2</f><v>2</v></c></row><row r="2"><c r="A2"><v>2</v></c><c r="B2"><f>A2*2</f><v>4</v></c></row><row r="3"><c r="A3"><v>3</v></c><c r="B3"><f>A3*2</f><v>6</v></c></row>"#,
        &[],
        false,
        "",
    );
    let (l, r) = (la(shared, "s.xlsx"), la(explicit, "e.xlsx"));
    let f: Vec<Option<String>> = (0..3)
        .map(|i| l.sheets()[0].formula(i, 1).map(str::to_owned))
        .collect();
    assert_eq!(
        f,
        vec![
            Some("A1*2".to_owned()),
            Some("A2*2".to_owned()),
            Some("A3*2".to_owned())
        ]
    );
    let o = TableCompareOptions {
        compare_formulas: true,
        ..TableCompareOptions::default()
    };
    assert_eq!(verdict(&l, &r, &o), ContentKind::RulesSame);
}

/// p_xlsx_vs_csv: dates, times, floats (F6), percentages, bools and texts of an xlsx equal their CSV export.
#[test]
fn an_xlsx_equals_its_csv_export() {
    let x = la(
        xlsx(|b| {
            let f = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
            let t = rust_xlsxwriter::Format::new().set_num_format("hh:mm");
            let s = b.add_worksheet();
            s.set_name("Data").unwrap();
            for (c, h) in ["d", "t", "n", "p", "b", "s"].iter().enumerate() {
                s.write_string(0, c as u16, *h).unwrap();
            }
            let ymd = |y, m, d| rust_xlsxwriter::ExcelDateTime::from_ymd(y, m, d).unwrap();
            let hms = |h, m| rust_xlsxwriter::ExcelDateTime::from_hms(h, m, 0).unwrap();
            s.write_datetime_with_format(1, 0, ymd(2024, 2, 29), &f)
                .unwrap();
            s.write_datetime_with_format(1, 1, hms(13, 30), &t).unwrap();
            s.write_number(1, 2, 0.1 + 0.2).unwrap();
            s.write_number(1, 3, 0.125).unwrap();
            s.write_boolean(1, 4, false).unwrap();
            s.write_string(1, 5, "x").unwrap();
            s.write_datetime_with_format(2, 0, ymd(2024, 3, 1), &f)
                .unwrap();
            s.write_datetime_with_format(2, 1, hms(0, 0), &t).unwrap();
            s.write_number(2, 2, 1.0).unwrap();
            s.write_number(2, 3, 1.0).unwrap();
            s.write_boolean(2, 4, true).unwrap();
            s.write_string(2, 5, "y").unwrap();
        }),
        "x.xlsx",
    );
    let c = wb(
        "x.csv",
        "d,t,n,p,b,s\n2024-02-29,13:30,0.3,12.5%,FALSE,x\n2024-03-01 00:00,00:00,1,100%,true,y\n",
    );
    assert_eq!(
        verdict(&x, &c, &TableCompareOptions::default()),
        ContentKind::RulesSame
    );
}

/// p_align_quality: one inserted, one changed and one deleted row among 20,000, both ways.
#[test]
fn insert_change_delete_among_twenty_thousand_rows() {
    let mut a = String::from("id,name,qty\n");
    for i in 0..20_000 {
        a.push_str(&format!("{i},n{i},{}\n", i * 3));
    }
    let b = a
        .replacen("id,name,qty\n", "id,name,qty\n-1,new,0\n", 1)
        .replace("500,n500,1500\n", "500,n500,1501\n")
        .replace("700,n700,2100\n", "");
    let (l, r) = (wb("a.csv", &a), wb("b.csv", &b));
    for (x, y, lo, ro) in [(&l, &r, 1, 1), (&r, &l, 1, 1)] {
        let t = cmp(x, y, &TableCompareOptions::default()).stats().totals;
        assert_eq!(t.same_rows, 19_998, "{t:?}");
        assert_eq!(t.changed_important, 1, "{t:?}");
        assert_eq!(
            (t.left_only_important, t.right_only_important),
            (lo, ro),
            "{t:?}"
        );
    }
}

/// p_swapped_ids (second half) and p_cyclic_warn: the exact engines and Standard on a cyclic table shifted by one,
/// and one row appended or prepended.
#[test]
fn cyclic_tables_align() {
    let (mut a, mut b) = (String::from("k,v\n"), String::from("k,v\n"));
    for i in 0..20_000 {
        a.push_str(&format!("1,{}\n", i % 7));
        b.push_str(&format!("1,{}\n", (i + 1) % 7));
    }
    let (l, r) = (wb("a.csv", &a), wb("b.csv", &b));
    for alg in ["myers", "patience", "histogram", "standard"] {
        let mut o = TableCompareOptions::default();
        o.default_sheet.rows.algorithm = parse_algorithm(alg).unwrap();
        let t = cmp(&l, &r, &o).stats().totals;
        assert_eq!(t.same_rows, 19_999, "{alg}: {t:?}");
    }
    let appended = wb("c.csv", &format!("{a}1,9\n"));
    let t = cmp(&l, &appended, &TableCompareOptions::default())
        .stats()
        .totals;
    assert_eq!((t.same_rows, t.right_only_important), (20_000, 1), "{t:?}");
    let prepended = wb("d.csv", &a.replacen("k,v\n", "k,v\n1,9\n", 1));
    let t = cmp(&l, &prepended, &TableCompareOptions::default())
        .stats()
        .totals;
    assert_eq!((t.same_rows, t.right_only_important), (20_000, 1), "{t:?}");
}

/// p_patho, scaled: a 100,000-column row, 20,000 rows of one key, a 10 MB cell.
#[test]
fn pathological_shapes_stay_correct() {
    let mut a = String::new();
    for i in 0..100_000 {
        if i > 0 {
            a.push(',');
        }
        a.push('x');
    }
    a.push('\n');
    let mut b = a.clone();
    b.replace_range(10..11, "y");
    let (l, r) = (wb("a.csv", &a), wb("b.csv", &b));
    let c = cmp(&l, &r, &TableCompareOptions::default());
    assert_eq!(c.stats().totals.cells_important, 1);
    let (mut a, mut b) = (String::from("k,v\n"), String::from("k,v\n"));
    for i in 0..20_000 {
        a.push_str(&format!("1,{}\n", i % 7));
        b.push_str(&format!("1,{}\n", (i + 1) % 7));
    }
    let c = cmp(&wb("a.csv", &a), &wb("b.csv", &b), &keyed("k"));
    let s = c.sheets()[0].stats();
    assert_eq!((s.duplicate_left, s.duplicate_right), (20_000, 20_000));
    assert_eq!(c.sheets()[0].duplicates().left_examples.len(), 1);
    let big = "z".repeat(10_000_000);
    let (l, r) = (
        wb("a.csv", &format!("h\n{big}\n")),
        wb("b.csv", &format!("h\n{big}y\n")),
    );
    assert_eq!(
        cmp(&l, &r, &TableCompareOptions::default()).kind(false, false),
        ContentKind::ImportantDifferences
    );
    let _ = ColumnRef::Position(1);
}
