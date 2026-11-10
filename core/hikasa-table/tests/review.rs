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
