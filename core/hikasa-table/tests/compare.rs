//! AT.6, AT.9, AT.10: comparison goldens, column references, the folder comparator.

mod common;

use common::{cmp, dump, golden, wb, wb_with, xlsx};
use hikasa_core::ContentKind;
use hikasa_table::{
    ColumnAlignment, ColumnRef, RulePatch, SheetAlignment, SheetPair, SheetRules, SheetSelector,
    TableCompareOptions, TableError, TableFormat, Tolerance, compare_workbooks,
};

fn keyed(keys: &str) -> TableCompareOptions {
    TableCompareOptions {
        default_sheet: SheetRules {
            keys: hikasa_table::parse_column_list(keys).unwrap(),
            ..SheetRules::default()
        },
        ..TableCompareOptions::default()
    }
}

#[test]
fn unkeyed_inserted_deleted_and_changed_rows() {
    let l = wb("a.csv", "id,name,qty\n1,apple,3\n2,pear,5\n3,plum,7\n4,fig,1\n");
    let r = wb("b.csv", "id,name,qty\n1,apple,3\n3,plum,8\n4,fig,1\n5,kiwi,2\n");
    golden("unkeyed", &dump(&cmp(&l, &r, &TableCompareOptions::default())));
}

#[test]
fn keyed_join_with_duplicates_in_any_order() {
    let l = wb("a.csv", "id,v\n3,c\n1,a\n2,b\n2,b2\n");
    let r = wb("b.csv", "id,v\n2,B\n1,a\n4,d\n");
    let c = cmp(&l, &r, &keyed("id"));
    assert_eq!(c.sheets()[0].stats().duplicate_left, 2);
    golden("keyed", &dump(&c));
}

#[test]
fn tolerances_exceptions_and_unimportant_columns() {
    let l = wb("a.csv", "k,price,when,name,note\n1,10.00,2024-01-01 10:00,Alpha,x\n2,5,2024-01-02,beta,y\n");
    let r = wb("b.csv", "k,price,when,name,note\n1,10.004,2024-01-01 10:00:20,alpha ,z\n2,5.5,2024-01-02,beta,y\n");
    let mut opts = keyed("k");
    let rules = &mut opts.default_sheet;
    rules.column_rules = vec![
        (
            ColumnRef::parse("price").unwrap(),
            RulePatch {
                numeric_tolerance: Some(Some(Tolerance::Absolute(0.01))),
                ..RulePatch::default()
            },
        ),
        (
            ColumnRef::parse("when").unwrap(),
            RulePatch {
                date_tolerance_ms: Some(Some(30_000)),
                ..RulePatch::default()
            },
        ),
        (
            ColumnRef::parse("name").unwrap(),
            RulePatch {
                ignore_case: Some(true),
                ignore_whitespace: Some(true),
                ..RulePatch::default()
            },
        ),
        (
            ColumnRef::parse("E").unwrap(),
            RulePatch {
                unimportant: Some(true),
                ..RulePatch::default()
            },
        ),
    ];
    let c = cmp(&l, &r, &opts);
    golden("rules", &dump(&c));
    // Row 1 is unimportant only, row 2's price is important.
    assert_eq!(c.kind(false, false), ContentKind::ImportantDifferences);
    let l2 = wb("a.csv", "k,price\n1,10.00\n");
    let r2 = wb("b.csv", "k,price\n1,10.004\n");
    let mut o2 = keyed("k");
    o2.default_sheet.default_rule.numeric_tolerance = Some(Tolerance::Relative(0.1));
    let c2 = cmp(&l2, &r2, &o2);
    assert_eq!(c2.kind(false, false), ContentKind::UnimportantOnly);
    assert_eq!(c2.kind(false, true), ContentKind::RulesSame);
    assert_eq!(c2.kind(true, false), ContentKind::BinarySame);
}

#[test]
fn csv_against_xlsx_of_the_same_data_is_same() {
    let book = xlsx(|b| {
        let date = rust_xlsxwriter::Format::new().set_num_format("yyyy-mm-dd");
        let s = b.add_worksheet();
        s.set_name("Export").unwrap();
        for (c, h) in ["id", "price", "day", "ok"].iter().enumerate() {
            s.write_string(0, c as u16, *h).unwrap();
        }
        s.write_number(1, 0, 1.0).unwrap();
        s.write_number(1, 1, 2.5).unwrap();
        s.write_number_with_format(1, 2, 45_322.0, &date).unwrap();
        s.write_boolean(1, 3, true).unwrap();
        s.write_number(2, 0, 2.0).unwrap();
        s.write_number(2, 1, 1000.0).unwrap();
        s.write_number_with_format(2, 2, 45_323.0, &date).unwrap();
        s.write_boolean(2, 3, false).unwrap();
    });
    let x = wb_with("e.xlsx", book, &TableFormat::from_name("e.xlsx"));
    let csv = wb("e.csv", "id,price,day,ok\n1,2.50,2024-01-31,TRUE\n2,\"1,000\",2024-02-01,false\n");
    let c = cmp(&csv, &x, &TableCompareOptions::default());
    assert!(c.sheets()[0].renamed());
    assert_eq!(c.kind(false, false), ContentKind::RulesSame, "{}", dump(&c));
}

#[test]
fn sheet_and_column_alignment_modes() {
    let book = |names: &[&str], cols: &[&str]| {
        let names: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
        let cols: Vec<String> = cols.iter().map(|s| (*s).to_owned()).collect();
        let bytes = xlsx(move |b| {
            for n in &names {
                let s = b.add_worksheet();
                s.set_name(n).unwrap();
                for (c, h) in cols.iter().enumerate() {
                    s.write_string(0, c as u16, h).unwrap();
                    s.write_number(1, c as u16, c as f64).unwrap();
                }
            }
        });
        wb_with("x.xlsx", bytes, &TableFormat::from_name("x.xlsx"))
    };
    let l = book(&["One", "Two", "Three"], &["a", "b", "c"]);
    let r = book(&["two", "Four", "One"], &["c", "a", "x"]);
    let mut out = String::new();
    for how in [
        SheetAlignment::ByName,
        SheetAlignment::LeftName,
        SheetAlignment::RightName,
        SheetAlignment::Position,
        SheetAlignment::Custom(vec![SheetPair::parse("Three=Four").unwrap(), SheetPair::parse("1=").unwrap()]),
    ] {
        for cols in [ColumnAlignment::Auto, ColumnAlignment::Number, ColumnAlignment::RightName] {
            let opts = TableCompareOptions {
                sheets: how.clone(),
                default_sheet: SheetRules {
                    columns: cols.clone(),
                    ..SheetRules::default()
                },
                ..TableCompareOptions::default()
            };
            out.push_str(&format!("### {how:?} / {cols:?}\n"));
            out.push_str(&dump(&cmp(&l, &r, &opts)));
        }
    }
    golden("alignment", &out);
    // Per-sheet rules by selector.
    let opts = TableCompareOptions {
        sheet_rules: vec![(
            SheetSelector::Name("one".into()),
            SheetRules {
                columns: ColumnAlignment::Number,
                ..SheetRules::default()
            },
        )],
        ..TableCompareOptions::default()
    };
    let c = cmp(&l, &r, &opts);
    let one = c.sheets().iter().find(|s| s.name() == "One").unwrap();
    assert_eq!(one.stats().header_differences, 3);
}

#[test]
fn references_that_name_nothing_are_invalid_input() {
    let l = wb("a.csv", "id,v\n1,2\n");
    let opts = keyed("nope");
    let err = compare_workbooks(l.clone(), l.clone(), &opts, &hikasa_core::CancelToken::new()).unwrap_err();
    match err {
        TableError::InvalidInput(m) => assert!(m.contains("nope") && m.contains("id"), "{m}"),
        other => panic!("{other:?}"),
    }
    let opts = TableCompareOptions {
        sheets: SheetAlignment::Custom(vec![SheetPair::parse("Missing=").unwrap()]),
        ..TableCompareOptions::default()
    };
    assert!(compare_workbooks(l.clone(), l, &opts, &hikasa_core::CancelToken::new()).is_err());
}

#[test]
fn formulas_count_only_when_asked() {
    let book = |f: &str| {
        let f = f.to_owned();
        let bytes = xlsx(move |b| {
            let s = b.add_worksheet();
            s.write_number(0, 0, 2.0).unwrap();
            s.write_formula(0, 1, f.as_str()).unwrap();
            s.set_formula_result(0, 1, "4");
        });
        wb_with("f.xlsx", bytes, &TableFormat::from_name("f.xlsx"))
    };
    let (l, r) = (book("=A1*2"), book("=A1+A1"));
    assert_eq!(cmp(&l, &r, &TableCompareOptions::default()).kind(false, false), ContentKind::RulesSame);
    let opts = TableCompareOptions {
        compare_formulas: true,
        ..TableCompareOptions::default()
    };
    assert_eq!(cmp(&l, &r, &opts).kind(false, false), ContentKind::ImportantDifferences);
}

#[test]
fn the_folder_comparator_maps_verdicts() {
    use hikasa_core::{ContentComparator, ContentResult, Ctl};
    struct Mem(&'static str, Vec<u8>);
    impl hikasa_core::ContentSource for Mem {
        fn name(&self) -> &str {
            self.0
        }
        fn display(&self) -> String {
            self.0.to_owned()
        }
        fn len(&self) -> Option<u64> {
            Some(self.1.len() as u64)
        }
        fn open(&self) -> std::io::Result<Box<dyn std::io::Read + Send + '_>> {
            Ok(Box::new(&self.1[..]))
        }
        fn read_all(&self, _: u64, _: &hikasa_core::CancelToken) -> Result<Vec<u8>, hikasa_core::ReadError> {
            Ok(self.1.clone())
        }
    }
    let c = hikasa_table::TableComparator::new();
    let run = |a: &[u8], b: &[u8]| {
        c.compare(&Mem("a.csv", a.to_vec()), &Mem("b.csv", b.to_vec()), &Ctl::detached())
            .unwrap()
            .result
    };
    assert_eq!(run(b"x,1.0\n", b"x,1\n"), ContentResult::RulesSame);
    assert_eq!(run(b"x,1\n", b"x,1\n"), ContentResult::BinarySame);
    assert_eq!(run(b"x,1\n", b"x,2\n"), ContentResult::ImportantDifferences);
    let bad = c
        .compare(&Mem("a.xlsx", vec![1; 100]), &Mem("b.xlsx", vec![1; 100]), &Ctl::detached())
        .unwrap()
        .result;
    assert!(matches!(bad, ContentResult::Error { .. }));
}
