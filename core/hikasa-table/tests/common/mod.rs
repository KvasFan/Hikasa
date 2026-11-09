//! Shared helpers of the hikasa-table integration tests: loading from text, generated workbooks, the golden
//! dump of a comparison.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Arc;

use hikasa_core::{CancelToken, Side};
use hikasa_table::{
    LoadOptions, TableCompareOptions, TableComparison, TableFormat, Workbook, compare_workbooks,
    load_bytes,
};

/// Loads `text` as the file `name` with its preset format.
pub fn wb(name: &str, text: &str) -> Arc<Workbook> {
    wb_with(name, text.as_bytes().to_vec(), &TableFormat::from_name(name))
}

/// Loads bytes with a format.
pub fn wb_with(name: &str, bytes: Vec<u8>, format: &TableFormat) -> Arc<Workbook> {
    Arc::new(load_bytes(bytes, name, format, &LoadOptions::default(), &CancelToken::new()).unwrap())
}

/// Compares two workbooks.
pub fn cmp(l: &Arc<Workbook>, r: &Arc<Workbook>, opts: &TableCompareOptions) -> TableComparison {
    compare_workbooks(Arc::clone(l), Arc::clone(r), opts, &CancelToken::new()).unwrap()
}

/// A deterministic xorshift generator (§6.1 style).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// A compact text dump of a comparison (the goldens): verdict, sheets, columns, stats and every non-same row
/// with its differing cells.
pub fn dump(c: &TableComparison) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "verdict {:?} / ignore-unimportant {:?}",
        c.kind(false, false),
        c.kind(false, true)
    );
    let st = c.stats();
    let _ = writeln!(
        out,
        "sheets paired {} left-only {} right-only {} differing {}",
        st.sheets_paired, st.sheets_left_only, st.sheets_right_only, st.sheets_differing
    );
    for (p, s) in c.sheets().iter().enumerate() {
        let _ = writeln!(
            out,
            "== sheet {:?} left {:?} right {:?} state {} renamed {} alignment {:?}",
            s.name(),
            s.left(),
            s.right(),
            s.state().as_str(),
            s.renamed(),
            s.alignment()
        );
        let cols: Vec<String> = s
            .columns()
            .iter()
            .map(|col| {
                format!(
                    "{}[{}|{}]{}",
                    col.name,
                    col.left.map_or("-".into(), |x| x.to_string()),
                    col.right.map_or("-".into(), |x| x.to_string()),
                    col.key.map_or(String::new(), |k| format!("*{k}"))
                )
            })
            .collect();
        let _ = writeln!(out, "columns {}", cols.join(" "));
        if !s.header_states().is_empty() {
            let h: Vec<&str> = s.header_states().iter().map(|x| x.as_str()).collect();
            let _ = writeln!(out, "header {}", h.join(" "));
        }
        let t = s.stats();
        let _ = writeln!(
            out,
            "rows L{} R{} total {} same {} chg {}/{} lo {}/{} ro {}/{} cells {} same {} unimp {} imp {} dup {}/{}",
            t.left_rows,
            t.right_rows,
            t.rows,
            t.same_rows,
            t.changed_important,
            t.changed_unimportant,
            t.left_only_important,
            t.left_only_unimportant,
            t.right_only_important,
            t.right_only_unimportant,
            t.cells_compared,
            t.cells_same,
            t.cells_unimportant,
            t.cells_important,
            t.duplicate_left,
            t.duplicate_right
        );
        for (i, row) in s.rows().iter().enumerate() {
            let i = i as u32;
            if row.kind() == hikasa_table::RowKind::Same {
                continue;
            }
            let label = |side| {
                c.row_label(p, i, side)
                    .map_or("-".to_owned(), |l| l.to_string())
            };
            let mut line = format!(
                "  {} {} L{} R{}",
                row.kind().as_str(),
                row.importance().as_str(),
                label(Side::Left),
                label(Side::Right)
            );
            match row.kind() {
                hikasa_table::RowKind::Changed => {
                    for d in s.differing_cells(i) {
                        let _ = write!(
                            line,
                            " | {}:{} {:?}->{:?}",
                            s.columns()[d.column as usize].name,
                            d.state.as_str(),
                            c.cell(p, i, d.column, Side::Left).display(),
                            c.cell(p, i, d.column, Side::Right).display()
                        );
                    }
                }
                _ => {
                    let side = if row.kind() == hikasa_table::RowKind::LeftOnly {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    let vals: Vec<String> = (0..s.columns().len() as u32)
                        .map(|col| c.cell(p, i, col, side).display().into_owned())
                        .collect();
                    let _ = write!(line, " | {}", vals.join(","));
                }
            }
            let _ = writeln!(out, "{line}");
        }
    }
    for w in c.warnings() {
        let _ = writeln!(out, "warning {} {:?}", w.code.as_str(), w.side);
    }
    out
}

/// Compares `actual` with `tests/fixtures/golden/<name>.txt` (`HIKASA_UPDATE_GOLDENS=1` rewrites it).
pub fn golden(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("HIKASA_UPDATE_GOLDENS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing golden {}; run with HIKASA_UPDATE_GOLDENS=1", path.display()));
    pretty_assertions::assert_eq!(expected, actual, "golden {name}");
}

/// A stored (uncompressed) zip with `files` in order.
pub fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in files {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
    }
    cursor.into_inner()
}

/// A minimal hand-built xlsx: one sheet given as `<sheetData>` XML, shared strings, optional 1904 epoch,
/// optional extra worksheet XML (merge cells).
pub fn minimal_xlsx(sheet_data: &str, shared: &[&str], date_1904: bool, extra: &str) -> Vec<u8> {
    let sst: String = shared
        .iter()
        .map(|s| format!("<si><t>{s}</t></si>"))
        .collect();
    let workbook = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr{}/><sheets><sheet name="Hand" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        if date_1904 { r#" date1904="1""# } else { "" }
    );
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy-mm-dd"/></numFmts><fonts count="1"><font/></fonts><fills count="1"><fill/></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0"/><xf numFmtId="164" applyNumberFormat="1"/></cellXfs></styleSheet>"#;
    let sheet = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet_data}</sheetData>{extra}</worksheet>"#
    );
    let sst = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{n}" uniqueCount="{n}">{sst}</sst>"#,
        n = shared.len()
    );
    zip_of(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        ("xl/workbook.xml", &workbook),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        ),
        ("xl/worksheets/sheet1.xml", &sheet),
        ("xl/sharedStrings.xml", &sst),
        ("xl/styles.xml", styles),
    ])
}

/// A minimal hand-built ods with one sheet of rows (each cell `(type, value, text)`; type `float`, `string`,
/// `date`, `boolean`, or `""` for an empty cell).
pub fn minimal_ods(sheet: &str, rows: &[Vec<(&str, &str)>]) -> Vec<u8> {
    let mut body = String::new();
    for row in rows {
        body.push_str("<table:table-row>");
        for (ty, v) in row {
            match *ty {
                "" => body.push_str("<table:table-cell/>"),
                "string" => {
                    let _ = write!(
                        body,
                        r#"<table:table-cell office:value-type="string"><text:p>{v}</text:p></table:table-cell>"#
                    );
                }
                "date" => {
                    let _ = write!(
                        body,
                        r#"<table:table-cell office:value-type="date" office:date-value="{v}"><text:p>{v}</text:p></table:table-cell>"#
                    );
                }
                "boolean" => {
                    let _ = write!(
                        body,
                        r#"<table:table-cell office:value-type="boolean" office:boolean-value="{v}"><text:p>{v}</text:p></table:table-cell>"#
                    );
                }
                _ => {
                    let _ = write!(
                        body,
                        r#"<table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell>"#
                    );
                }
            }
        }
        body.push_str("</table:table-row>");
    }
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="{sheet}">{body}</table:table></office:spreadsheet></office:body></office:document-content>"#
    );
    // The mimetype must be the first entry, stored.
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut cursor);
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        z.start_file("mimetype", stored).unwrap();
        z.write_all(b"application/vnd.oasis.opendocument.spreadsheet").unwrap();
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        z.start_file("content.xml", deflated).unwrap();
        z.write_all(content.as_bytes()).unwrap();
        z.start_file(
            "META-INF/manifest.xml",
            deflated,
        )
        .unwrap();
        z.write_all(br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#).unwrap();
        z.finish().unwrap();
    }
    cursor.into_inner()
}

/// An xlsx written with rust_xlsxwriter from a closure.
pub fn xlsx(build: impl FnOnce(&mut rust_xlsxwriter::Workbook)) -> Vec<u8> {
    let mut book = rust_xlsxwriter::Workbook::new();
    build(&mut book);
    book.save_to_buffer().unwrap()
}
