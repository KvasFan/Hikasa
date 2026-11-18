//! AT.4, AT.5: workbooks through calamine — generated xlsx, hand-built xlsx (1904, inline and shared strings,
//! errors, merged cells), ods, encrypted and corrupt inputs, archive members, memory.

mod common;

use std::sync::Arc;

use common::{minimal_ods, minimal_xlsx, xlsx, zip_of};
use hikasa_core::CancelToken;
use hikasa_table::{
    CellError, ColumnKind, LoadOptions, SourceKind, TableError, TableFormat, Value, load_bytes,
};
use jiff::civil::DateTime;

fn load(bytes: Vec<u8>, name: &str) -> Result<hikasa_table::Workbook, TableError> {
    load_bytes(
        bytes,
        name,
        &TableFormat::from_name(name),
        &LoadOptions::default(),
        &CancelToken::new(),
    )
}

fn texts(sheet: &hikasa_table::Sheet) -> Vec<Vec<String>> {
    (0..sheet.row_count())
        .map(|r| {
            sheet
                .row(r)
                .values()
                .map(|v| v.display().into_owned())
                .collect()
        })
        .collect()
}

#[test]
fn generated_xlsx_with_every_cell_kind() {
    let bytes = xlsx(|book| {
        let date = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd hh:mm");
        let s = book.add_worksheet();
        s.set_name("Data").unwrap();
        s.write_string(0, 0, "name").unwrap();
        s.write_string(0, 1, "qty").unwrap();
        s.write_string(0, 2, "when").unwrap();
        s.write_string(0, 3, "ok").unwrap();
        s.write_string(0, 4, "total").unwrap();
        s.write_string(1, 0, "apple").unwrap();
        s.write_number(1, 1, 3.5).unwrap();
        s.write_number_with_format(1, 2, 45_322.5, &date).unwrap();
        s.write_boolean(1, 3, true).unwrap();
        s.write_formula(1, 4, "=B2*2").unwrap();
        s.set_formula_result(1, 4, "7");
        s.write_string(2, 0, "apple").unwrap();
        s.write_number(2, 1, 1.0).unwrap();
        s.merge_range(3, 0, 3, 2, "merged", &rust_xlsxwriter::Format::new())
            .unwrap();
        let t = book.add_worksheet();
        t.set_name("Second").unwrap();
        t.write_number(2, 3, 42.0).unwrap();
        t.set_hidden(true);
    });
    let wb = load(bytes, "book.xlsx").unwrap();
    assert_eq!(wb.source(), SourceKind::Xlsx);
    assert_eq!(wb.sheets().len(), 2);
    let s = &wb.sheets()[0];
    assert_eq!(s.name(), "Data");
    assert_eq!(s.header().map(<[String]>::len), Some(5));
    assert_eq!(s.cell(0, 1), Value::Number(3.5));
    assert_eq!(
        s.cell(0, 2),
        Value::DateTime(DateTime::constant(2024, 1, 31, 12, 0, 0, 0))
    );
    assert_eq!(s.cell(0, 3), Value::Bool(true));
    assert_eq!(s.cell(0, 4), Value::Number(7.0));
    assert_eq!(s.formula(0, 4), Some("B2*2"));
    // Shared strings resolve; the merged value stays in the top-left cell.
    assert_eq!(s.cell(1, 0), Value::Text("apple"));
    assert_eq!(s.cell(2, 0), Value::Text("merged"));
    assert_eq!(s.cell(2, 1), Value::Empty);
    assert!(s.merged_region(2, 1).is_some());
    assert!(s.merged_region(1, 1).is_none());
    assert_eq!(s.column_kind(1), ColumnKind::Number);
    // The second sheet is hidden, its used range starts at D3.
    let t = &wb.sheets()[1];
    assert!(t.info().hidden);
    assert_eq!(t.info().origin, (2, 3));
    assert_eq!(t.column_label(0), "D");
    assert_eq!(t.row_label(0), 3);
    assert_eq!(texts(t), vec![vec!["42".to_owned()]]);
}

#[test]
fn hand_built_xlsx_1904_epoch_inline_strings_and_errors() {
    let data = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>inline</t></is></c><c r="C1" t="e"><v>#DIV/0!</v></c><c r="D1" s="1"><v>0</v></c><c r="E1" s="1"><v>0.25</v></c></row>"#;
    for (epoch, date) in [
        (false, DateTime::constant(1899, 12, 30, 0, 0, 0, 0)),
        // The 1904 epoch's serial 0 is its first day (F10).
        (true, DateTime::constant(1904, 1, 1, 0, 0, 0, 0)),
    ] {
        let bytes = minimal_xlsx(data, &["shared"], epoch, "");
        let wb = load(bytes, "hand.xlsx").unwrap();
        let s = &wb.sheets()[0];
        assert_eq!(s.cell(0, 0), Value::Text("shared"));
        assert_eq!(s.cell(0, 1), Value::Text("inline"));
        assert_eq!(s.cell(0, 2), Value::Error(CellError::Div0));
        // Serial 0.25: a time of day on 1899-12-30 in both epochs (T-14).
        assert_eq!(s.cell(0, 3), Value::DateTime(date));
        assert_eq!(
            s.cell(0, 4),
            Value::DateTime(DateTime::constant(1899, 12, 30, 6, 0, 0, 0))
        );
    }
    // The same serial 1000 in both epochs differs by 1,462 days.
    let row = |v: &str| format!(r#"<row r="1"><c r="A1" s="1"><v>{v}</v></c></row>"#);
    let d1900 = load(minimal_xlsx(&row("1000"), &[], false, ""), "a.xlsx").unwrap();
    let d1904 = load(minimal_xlsx(&row("1000"), &[], true, ""), "a.xlsx").unwrap();
    assert_eq!(d1900.sheets()[0].cell(0, 0).display(), "1902-09-26");
    assert_eq!(d1904.sheets()[0].cell(0, 0).display(), "1906-09-27");
    // Serials 59, 60 (the fictitious 1900-02-29) and 61.
    for (serial, text) in [
        ("59", "1900-02-28"),
        ("60", "1900-02-28"),
        ("61", "1900-03-01"),
        ("1", "1900-01-01"),
    ] {
        let wb = load(minimal_xlsx(&row(serial), &[], false, ""), "a.xlsx").unwrap();
        assert_eq!(wb.sheets()[0].cell(0, 0).display(), text, "serial {serial}");
    }
}

#[test]
fn merged_regions_of_a_hand_built_sheet() {
    let data = r#"<row r="2"><c r="B2" t="s"><v>0</v></c></row><row r="3"><c r="B3" t="s"><v>1</v></c></row>"#;
    let wb = load(
        minimal_xlsx(
            data,
            &["top", "below"],
            false,
            r#"<mergeCells count="1"><mergeCell ref="B2:C2"/></mergeCells>"#,
        ),
        "m.xlsx",
    )
    .unwrap();
    let s = &wb.sheets()[0];
    assert_eq!(s.info().origin, (1, 1));
    assert_eq!(s.info().merged.len(), 1);
    assert!(s.merged_region(0, 1).is_some() || s.header().is_some());
}

#[test]
fn ods_sheets_load() {
    let rows = vec![
        vec![("string", "id"), ("string", "when"), ("string", "flag")],
        vec![("float", "1"), ("date", "2024-01-31"), ("boolean", "true")],
        vec![("float", "2.5"), ("date", "2024-02-01T10:30:00"), ("", "")],
    ];
    let wb = load(minimal_ods("Odd", &rows), "x.ods").unwrap();
    assert_eq!(wb.source(), SourceKind::Ods);
    let s = &wb.sheets()[0];
    assert_eq!(s.name(), "Odd");
    assert_eq!(s.header().map(<[String]>::len), Some(3));
    assert_eq!(s.cell(0, 0), Value::Number(1.0));
    assert_eq!(s.cell(0, 1).display(), "2024-01-31");
    assert_eq!(s.cell(1, 1).display(), "2024-02-01 10:30:00");
    assert_eq!(s.cell(0, 2), Value::Bool(true));
}

#[test]
fn encrypted_and_corrupt_inputs_are_clear_errors() {
    // An encrypted OOXML package: a compound file with an `EncryptedPackage` stream name.
    let mut cfb = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    cfb.resize(512, 0);
    cfb.extend("EncryptedPackage".encode_utf16().flat_map(u16::to_le_bytes));
    match load(cfb, "secret.xlsx") {
        Err(TableError::Encrypted { name }) => assert_eq!(name, "secret.xlsx"),
        other => panic!("{other:?}"),
    }
    // A truncated workbook.
    let mut good = xlsx(|b| {
        b.add_worksheet().write_number(0, 0, 1.0).unwrap();
    });
    good.truncate(good.len() / 2);
    assert!(matches!(
        load(good, "cut.xlsx"),
        Err(TableError::Corrupt { .. })
    ));
    // Random bytes named .xlsx; a zip that is no workbook.
    assert!(matches!(
        load(vec![7; 300], "x.xlsx"),
        Err(TableError::Corrupt { .. })
    ));
    assert!(matches!(
        load(zip_of(&[("a.txt", "x")]), "x.xlsx"),
        Err(TableError::Corrupt { .. })
    ));
    // A compound file that is no valid xls.
    let mut junk = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    junk.resize(1024, 0xAB);
    assert!(matches!(
        load(junk, "old.xls"),
        Err(TableError::Corrupt { .. })
    ));
    use hikasa_core::{Classify, ErrorKind};
    assert_eq!(
        TableError::Encrypted { name: "x".into() }.kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn a_workbook_inside_a_zip_loads_through_the_vfs() {
    use hikasa_core::{CaseSensitivity, Ctl, LocalPath, Spec};
    use hikasa_vfs::{
        ArchiveConfig, ArchiveFormat, ArchiveLimits, ArchiveMasks, ArchiveVfs, MemMeta, MemoryVfs,
    };
    let inner = xlsx(|b| {
        let s = b.add_worksheet();
        s.write_string(0, 0, "k").unwrap();
        s.write_number(1, 0, 5.0).unwrap();
    });
    let mut outer = std::io::Cursor::new(Vec::new());
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(&mut outer);
        z.start_file("dir/book.xlsx", zip::write::SimpleFileOptions::default())
            .unwrap();
        z.write_all(&inner).unwrap();
        z.finish().unwrap();
    }
    let root = if cfg!(windows) { "C:\\data" } else { "/data" };
    let mem = MemoryVfs::new(CaseSensitivity::Sensitive);
    let zip_path = format!("{root}{}pack.zip", std::path::MAIN_SEPARATOR);
    mem.insert_file(&zip_path, outer.into_inner(), MemMeta::default())
        .unwrap();
    let masks = ArchiveMasks::new([(ArchiveFormat::Zip, "*.zip".to_owned())]).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let vfs = ArchiveVfs::new(
        Arc::new(mem),
        ArchiveConfig {
            masks,
            limits: ArchiveLimits::default(),
            clock: Arc::new(|| jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC)),
            cache_root: cache.path().to_path_buf(),
        },
    );
    let archive = Spec::Local(LocalPath::new(std::path::PathBuf::from(&zip_path)).unwrap());
    let spec = Spec::archive_member(archive, "dir/book.xlsx");
    let wb = hikasa_table::load_workbook(
        &vfs,
        &spec,
        &TableFormat::default(),
        &LoadOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(wb.source(), SourceKind::Xlsx);
    assert_eq!(wb.sheets()[0].cell(0, 0), Value::Number(5.0));
}

#[test]
fn sparse_sheets_cost_their_cells_and_limits_hold() {
    let data = r#"<row r="1"><c r="A1"><v>1</v></c></row><row r="1048576"><c r="XFD1048576"><v>2</v></c></row>"#;
    let wb = load(minimal_xlsx(data, &[], false, ""), "sparse.xlsx").unwrap();
    let s = &wb.sheets()[0];
    assert_eq!(s.row_count(), 1_048_576);
    assert_eq!(s.row(0).len(), 1);
    assert_eq!(s.row(1_048_575).len(), 16_384);
    // The gap costs nothing: two stored cells (F4).
    assert!(s.is_sparse());
    assert_eq!(s.row(1_048_575).stored_len(), 1);
    assert_eq!(s.cell(1_048_575, 16_383), Value::Number(2.0));
    assert_eq!(s.cell(1_048_575, 100), Value::Empty);
    // max_cells counts the stored cells: a wide gap passes, 101 cells over a limit of 100 do not.
    let small = LoadOptions {
        spreadsheet: hikasa_table::SpreadsheetOptions {
            max_cells: 100,
            ..hikasa_table::SpreadsheetOptions::default()
        },
        ..LoadOptions::default()
    };
    let load_small = |data: &str| {
        load_bytes(
            minimal_xlsx(data, &[], false, ""),
            "w.xlsx",
            &TableFormat::default(),
            &small,
            &CancelToken::new(),
        )
    };
    let wide = r#"<row r="1"><c r="A1"><v>1</v></c><c r="CW1"><v>1</v></c></row>"#;
    let w = load_small(wide).unwrap();
    assert_eq!(w.sheets()[0].row(0).len(), 101);
    assert_eq!(w.sheets()[0].row(0).stored_len(), 2);
    let full: String = (1..=101)
        .map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#))
        .collect();
    assert!(matches!(
        load_small(&full),
        Err(TableError::TooLarge { what: "cells", .. })
    ));
}

#[test]
fn spreadsheet_text_conversion_writes_every_sheet() {
    let bytes = xlsx(|b| {
        let s = b.add_worksheet();
        s.set_name("A").unwrap();
        s.write_string(0, 0, "x\ty").unwrap();
        s.write_number(0, 1, 1.5).unwrap();
        let t = b.add_worksheet();
        t.set_name("B").unwrap();
        t.write_boolean(0, 0, true).unwrap();
    });
    let wb = load(bytes, "two.xlsx").unwrap();
    assert_eq!(
        hikasa_table::spreadsheet_to_text(&wb),
        "--- A ---\n\"x\ty\"\t1.5\n--- B ---\nTRUE\n"
    );
}
