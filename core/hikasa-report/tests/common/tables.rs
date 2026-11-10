//! Table comparisons of the table report tests (Table Compare §6.3): a multi-sheet pair built in memory (a keyed sheet
//! with same, changed, unimportant (a tolerance) and orphan rows; a renamed pair matched by case; a left-only sheet) and a
//! CSV pair loaded by the real loader, plus their [`TableSide`]s.

use std::sync::Arc;

use hikasa_core::{CancelToken, Pair};
use hikasa_report::json::FormatJson;
use hikasa_report::{TableSide, TableSource};
use hikasa_table::{
    ColumnRef, FirstLine, LoadOptions, Regional, RulePatch, Sheet, SheetRules, SheetSelector,
    SourceKind, TableCompareOptions, TableComparison, TableFormat, Tolerance, Workbook,
    compare_workbooks, load_bytes,
};

/// The comparison and everything a [`TableSource`] borrows.
pub struct TableCase {
    /// The comparison.
    pub comparison: TableComparison,
    /// Its options.
    pub options: TableCompareOptions,
    /// The sides.
    pub sides: [TableSide; 2],
}

impl TableCase {
    /// The source of the case.
    pub fn source(&self) -> TableSource<'_> {
        TableSource {
            comparison: &self.comparison,
            options: &self.options,
            sides: Pair::new(&self.sides[0], &self.sides[1]),
            bytes_identical: false,
        }
    }
}

fn sheet(name: &str, rows: &[&[&str]]) -> Sheet {
    let rows: Vec<Vec<&str>> = rows.iter().map(|r| r.to_vec()).collect();
    Sheet::from_text_rows(name, &rows, FirstLine::Names, Regional::default())
}

/// On sheet `Data`: keys `id`, a tolerance of 0.05 on `price`.
pub fn keyed_options() -> TableCompareOptions {
    let mut options = TableCompareOptions::default();
    let mut data = SheetRules {
        keys: vec![ColumnRef::parse("id").unwrap()],
        ..SheetRules::default()
    };
    data.column_rules = vec![(
        ColumnRef::parse("price").unwrap(),
        RulePatch {
            numeric_tolerance: Some(Some(Tolerance::Absolute(0.05))),
            ..RulePatch::default()
        },
    )];
    options.sheet_rules = vec![(SheetSelector::parse("Data").unwrap(), data)];
    options
}

fn side(spec: &str, size: u64, id: &str, name: &str) -> TableSide {
    TableSide {
        spec: spec.to_owned(),
        size: Some(size),
        mtime: Some(jiff::Timestamp::from_second(1_700_000_000).unwrap()),
        format: Some(FormatJson {
            id: Some(id.to_owned()),
            name: name.to_owned(),
            kind: "table",
            reason: None,
        }),
        sniffed: false,
    }
}

/// Two workbooks: `Data` (keyed by `id`), `Notes`/`notes` (renamed by case), `Extra` (left only).
pub fn workbooks() -> TableCase {
    let left = Workbook::from_sheets(
        vec![
            sheet(
                "Data",
                &[
                    &["id", "name", "price", "date"],
                    &["1", "apple", "1.50", "2024-01-31"],
                    &["2", "pear", "2.00", "2024-02-01"],
                    &["3", "plum", "0.75", "2024-02-02"],
                    &["4", "fig", "3.00", "2024-02-03"],
                ],
            ),
            sheet(
                "Notes",
                &[
                    &["note", "by"],
                    &["first <draft> & \"q\"", "ann"],
                    &["second", "bob"],
                ],
            ),
            sheet("Extra", &[&["k"], &["x"]]),
        ],
        SourceKind::Xlsx,
    );
    let right = Workbook::from_sheets(
        vec![
            sheet(
                "Data",
                &[
                    &["id", "name", "price", "date"],
                    &["1", "apple", "1.5", "2024-01-31"],
                    &["2", "Pear", "2.50", "2024-02-01"],
                    &["3", "plum", "0.76", "2024-02-02"],
                    &["5", "kiwi", "1.00", "2024-02-05"],
                ],
            ),
            sheet(
                "notes",
                &[
                    &["note", "by"],
                    &["first <draft> & \"q\"", "ann"],
                    &["second", "bob"],
                ],
            ),
        ],
        SourceKind::Xlsx,
    );
    let options = keyed_options();
    let comparison = compare_workbooks(
        Arc::new(left),
        Arc::new(right),
        &options,
        &CancelToken::new(),
    )
    .unwrap();
    TableCase {
        comparison,
        options,
        sides: [
            side(
                "/data/left.xlsx",
                9000,
                "spreadsheets",
                "Spreadsheet Workbooks",
            ),
            side(
                "/data/right.xlsx",
                9100,
                "spreadsheets",
                "Spreadsheet Workbooks",
            ),
        ],
    }
}

/// Two CSV files loaded by the real loader (comma preset by name), compared without keys.
pub fn csv_pair() -> TableCase {
    let load = |text: &str, name: &str| {
        load_bytes(
            text.as_bytes().to_vec(),
            name,
            &TableFormat::from_name(name),
            &LoadOptions::default(),
            &CancelToken::new(),
        )
        .unwrap()
    };
    let left = load(
        "code,city,amount\nA1,Rome,10\nB2,Oslo,20\nC3,Lima,30\nD4,Pune,40\n",
        "left.csv",
    );
    let right = load(
        "code,city,amount\nA1,Rome,10\nB2,Oslo,25\nC3,Lima,30\nE5,Kyiv,50\nD4,Pune,40\n",
        "right.csv",
    );
    let options = TableCompareOptions::default();
    let comparison = compare_workbooks(
        Arc::new(left),
        Arc::new(right),
        &options,
        &CancelToken::new(),
    )
    .unwrap();
    let mut sides = [
        side("/data/left.csv", 52, "csv", "Comma Separated Values"),
        side("/data/right.csv", 62, "csv", "Comma Separated Values"),
    ];
    sides[1].mtime = None;
    TableCase {
        comparison,
        options,
        sides,
    }
}
