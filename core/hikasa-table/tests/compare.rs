//! AT.6, AT.9, AT.10: comparison goldens, column references, the folder comparator.

mod common;

use common::{cmp, dump, golden, wb, wb_with, xlsx};
use std::sync::Arc;

use hikasa_core::{CancelToken, ContentKind};
use hikasa_table::{
    ColumnAlignment, ColumnRef, RowAlignmentUsed, RulePatch, SheetAlignment, SheetPair, SheetRules,
    SheetSelector, SheetState, TableCompareOptions, TableError, TableFormat, TableWarningCode,
    Tolerance, compare_workbooks,
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
    let l = wb(
        "a.csv",
        "id,name,qty\n1,apple,3\n2,pear,5\n3,plum,7\n4,fig,1\n",
    );
    let r = wb(
        "b.csv",
        "id,name,qty\n1,apple,3\n3,plum,8\n4,fig,1\n5,kiwi,2\n",
    );
    golden(
        "unkeyed",
        &dump(&cmp(&l, &r, &TableCompareOptions::default())),
    );
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
    let l = wb(
        "a.csv",
        "k,price,when,name,note\n1,10.00,2024-01-01 10:00,Alpha,x\n2,5,2024-01-02,beta,y\n",
    );
    let r = wb(
        "b.csv",
        "k,price,when,name,note\n1,10.004,2024-01-01 10:00:20,alpha ,z\n2,5.5,2024-01-02,beta,y\n",
    );
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
    let csv = wb(
        "e.csv",
        "id,price,day,ok\n1,2.50,2024-01-31,TRUE\n2,\"1,000\",2024-02-01,false\n",
    );
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
        SheetAlignment::Custom(vec![
            SheetPair::parse("Three=Four").unwrap(),
            SheetPair::parse("1=").unwrap(),
        ]),
    ] {
        for cols in [
            ColumnAlignment::Auto,
            ColumnAlignment::Number,
            ColumnAlignment::RightName,
        ] {
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
    let err = compare_workbooks(
        l.clone(),
        l.clone(),
        &opts,
        &hikasa_core::CancelToken::new(),
    )
    .unwrap_err();
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
    assert_eq!(
        cmp(&l, &r, &TableCompareOptions::default()).kind(false, false),
        ContentKind::RulesSame
    );
    let opts = TableCompareOptions {
        compare_formulas: true,
        ..TableCompareOptions::default()
    };
    assert_eq!(
        cmp(&l, &r, &opts).kind(false, false),
        ContentKind::ImportantDifferences
    );
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
        fn read_all(
            &self,
            _: u64,
            _: &hikasa_core::CancelToken,
        ) -> Result<Vec<u8>, hikasa_core::ReadError> {
            Ok(self.1.clone())
        }
    }
    let c = hikasa_table::TableComparator::new();
    let run = |a: &[u8], b: &[u8]| {
        c.compare(
            &Mem("a.csv", a.to_vec()),
            &Mem("b.csv", b.to_vec()),
            &Ctl::detached(),
        )
        .unwrap()
        .result
    };
    assert_eq!(run(b"x,1.0\n", b"x,1\n"), ContentResult::RulesSame);
    assert_eq!(run(b"x,1\n", b"x,1\n"), ContentResult::BinarySame);
    assert_eq!(run(b"x,1\n", b"x,2\n"), ContentResult::ImportantDifferences);
    let bad = c
        .compare(
            &Mem("a.xlsx", vec![1; 100]),
            &Mem("b.xlsx", vec![1; 100]),
            &Ctl::detached(),
        )
        .unwrap()
        .result;
    assert!(matches!(bad, ContentResult::Error { .. }));
}

#[test]
fn duplicate_keys_pair_in_their_order_of_appearance() {
    let body: String = (0..300).map(|i| format!("1,{i}\n")).collect();
    let l = wb("a.csv", &format!("k,v\n{body}"));
    let r = wb("b.csv", &format!("k,v\n{body}"));
    let c = cmp(&l, &r, &keyed("k"));
    assert_eq!(c.kind(false, false), ContentKind::RulesSame);
    assert_eq!(c.sheets()[0].stats().duplicate_left, 300);
}

#[test]
fn values_of_a_one_sided_column_are_differences() {
    let l = wb("a.csv", "id,extra\n1,x\n2,\n");
    let r = wb("b.csv", "id\n1\n2\n");
    let c = cmp(&l, &r, &TableCompareOptions::default());
    let s = &c.sheets()[0];
    assert_eq!(s.stats().columns_left_only, 1);
    assert_eq!(s.stats().cells_important, 1, "{}", dump(&c));
}

#[test]
fn one_detected_header_is_used_on_both_sides() {
    // The right side's vote fails (`name` occurs in its body) but its first row is all text.
    let l = wb("a.csv", "id,name\n1,x\n2,y\n");
    let r = wb("b.csv", "id,name\nid2,name\n");
    assert!(l.sheets()[0].header().is_some());
    assert!(r.sheets()[0].header().is_none());
    let c = cmp(&l, &r, &TableCompareOptions::default());
    assert!(c.sheets()[0].has_header(hikasa_core::Side::Left));
    assert!(c.sheets()[0].has_header(hikasa_core::Side::Right));
    assert_eq!(c.sheets()[0].header_states().len(), 2);
}

#[test]
fn duplicates_keep_their_order_whatever_the_interleaving() {
    let mut rng = common::Rng(0xD0B1);
    let mut left = String::from("k,v\n");
    let mut by_key: Vec<Vec<String>> = vec![Vec::new(); 5];
    for i in 0..1000 {
        let k = rng.below(5) as usize;
        left.push_str(&format!("{k},{i}\n"));
        by_key[k].push(format!("{k},{i}\n"));
    }
    let right = format!("k,v\n{}", by_key.concat().concat());
    let c = cmp(&wb("a.csv", &left), &wb("b.csv", &right), &keyed("k"));
    assert_eq!(c.kind(false, false), ContentKind::RulesSame, "{}", dump(&c));
}

/// T-30: rules of every sheet with lenient references skip a sheet whose columns they do not name (the sheet is
/// compared unkeyed, the warning `rule_not_applied` names it); a reference that names nothing in every paired sheet
/// is still `InvalidInput`; strict rules refuse a missing column at once.
#[test]
fn lenient_references_skip_sheets_without_the_column() {
    let book = |b: &str| {
        xlsx(|w| {
            let s = w.add_worksheet();
            s.set_name("Prices").unwrap();
            for (r, row) in [["id", "price"], ["1", "2"], ["2", b]].iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    match v.parse::<f64>() {
                        Ok(n) if r > 0 => s.write_number(r as u32, c as u16, n).map(drop),
                        _ => s.write_string(r as u32, c as u16, *v).map(drop),
                    }
                    .unwrap();
                }
            }
            let n = w.add_worksheet();
            n.set_name("Notes").unwrap();
            n.write_string(0, 0, "note").unwrap();
            n.write_string(1, 0, "hello").unwrap();
        })
    };
    let fmt = TableFormat::from_name("x.xlsx");
    let l = wb_with("l.xlsx", book("3"), &fmt);
    let r = wb_with("r.xlsx", book("4"), &fmt);
    let mut opts = TableCompareOptions::default();
    opts.default_sheet.keys = vec![ColumnRef::parse("id").unwrap()];
    opts.default_sheet.column_rules.push((
        ColumnRef::parse("price").unwrap(),
        RulePatch {
            numeric_tolerance: Some(Some(Tolerance::Absolute(2.0))),
            ..RulePatch::default()
        },
    ));
    // Strict (the engine's default): `Notes` has no `id`.
    let strict = compare_workbooks(Arc::clone(&l), Arc::clone(&r), &opts, &CancelToken::new());
    assert!(
        matches!(strict, Err(TableError::InvalidInput(_))),
        "{strict:?}"
    );
    opts.default_sheet.lenient_references = true;
    let c = cmp(&l, &r, &opts);
    assert_eq!(
        c.sheets()[0].alignment(),
        &RowAlignmentUsed::Keyed { keys: vec![0] }
    );
    assert_eq!(
        c.sheets()[0].state(),
        SheetState::Unimportant,
        "the tolerance applied"
    );
    assert!(matches!(
        c.sheets()[1].alignment(),
        RowAlignmentUsed::Engine { .. }
    ));
    let skipped: Vec<_> = c
        .warnings()
        .iter()
        .filter(|w| w.code == TableWarningCode::RuleNotApplied)
        .map(|w| w.sheet.clone().unwrap_or_default())
        .collect();
    assert_eq!(skipped, ["Notes", "Notes"]);
    // A typo names nothing anywhere: refused.
    opts.default_sheet.keys = vec![ColumnRef::parse("idd").unwrap()];
    let typo = compare_workbooks(l, r, &opts, &CancelToken::new());
    assert!(
        matches!(&typo, Err(TableError::InvalidInput(m)) if m.contains("idd")),
        "{typo:?}"
    );
}

/// T-30: keys that do not all resolve in a sheet leave it unkeyed (a partial key would join by fewer columns than
/// asked); the sheets where they all resolve stay keyed.
#[test]
fn partial_keys_leave_a_sheet_unkeyed() {
    let book = |extra: f64| {
        xlsx(|w| {
            let s = w.add_worksheet();
            s.set_name("Both").unwrap();
            s.write_string(0, 0, "id").unwrap();
            s.write_string(0, 1, "code").unwrap();
            s.write_string(0, 2, "v").unwrap();
            for r in 1..4u32 {
                s.write_number(r, 0, f64::from(r)).unwrap();
                s.write_number(r, 1, f64::from(r * 10)).unwrap();
                s.write_number(r, 2, extra).unwrap();
            }
            let o = w.add_worksheet();
            o.set_name("IdOnly").unwrap();
            o.write_string(0, 0, "id").unwrap();
            o.write_string(0, 1, "v").unwrap();
            for r in 1..4u32 {
                o.write_number(r, 0, f64::from(r)).unwrap();
                o.write_number(r, 1, extra).unwrap();
            }
        })
    };
    let fmt = TableFormat::from_name("x.xlsx");
    let l = wb_with("l.xlsx", book(1.0), &fmt);
    let r = wb_with("r.xlsx", book(1.0), &fmt);
    let mut opts = TableCompareOptions::default();
    opts.default_sheet.keys = hikasa_table::parse_column_list("id,code").unwrap();
    opts.default_sheet.lenient_references = true;
    let c = cmp(&l, &r, &opts);
    assert_eq!(
        c.sheets()[0].alignment(),
        &RowAlignmentUsed::Keyed { keys: vec![0, 1] }
    );
    assert!(
        matches!(c.sheets()[1].alignment(), RowAlignmentUsed::Engine { .. }),
        "{:?}",
        c.sheets()[1].alignment()
    );
}

/// T-30: a reference is an error only when every lenient sheet lacks it, whatever the order of the sheets (the first
/// sheet lacking a column the second has is fine).
#[test]
fn a_reference_the_first_sheet_lacks_is_no_error() {
    let book = |v: f64| {
        xlsx(|w| {
            let n = w.add_worksheet();
            n.set_name("Notes").unwrap();
            n.write_string(0, 0, "note").unwrap();
            n.write_string(1, 0, "hello").unwrap();
            let s = w.add_worksheet();
            s.set_name("Prices").unwrap();
            s.write_string(0, 0, "id").unwrap();
            s.write_string(0, 1, "price").unwrap();
            s.write_number(1, 0, 1.0).unwrap();
            s.write_number(1, 1, v).unwrap();
        })
    };
    let fmt = TableFormat::from_name("x.xlsx");
    let l = wb_with("l.xlsx", book(1.0), &fmt);
    let r = wb_with("r.xlsx", book(2.0), &fmt);
    let mut opts = TableCompareOptions::default();
    opts.default_sheet.keys = vec![ColumnRef::parse("id").unwrap()];
    opts.default_sheet.lenient_references = true;
    let c = cmp(&l, &r, &opts);
    assert_eq!(c.sheets()[0].state(), SheetState::Same);
    assert_eq!(
        c.sheets()[1].alignment(),
        &RowAlignmentUsed::Keyed { keys: vec![0] }
    );
}

/// Phase 3 (TB-020 *Hide Same Columns*): a column differs by a differing cell or header name of a paired row, an
/// unimportant one only while unimportant differences count; orphan rows make no column differ.
#[test]
fn column_differs_follows_the_differing_cells() {
    let l = wb(
        "a.csv",
        "k,price,name,same,gone\n1,10.00,Alpha,x,a\n2,5,beta,y,b\n3,1,c,z,c\n",
    );
    let r = wb(
        "b.csv",
        "k,price,name,same,gone\n1,10.004,alpha,x,a\n2,5,beta,y,b\n4,1,c,z,c\n",
    );
    let mut opts = keyed("k");
    opts.default_sheet.column_rules = vec![(
        ColumnRef::parse("price").unwrap(),
        RulePatch {
            numeric_tolerance: Some(Some(Tolerance::Absolute(0.01))),
            ..RulePatch::default()
        },
    )];
    let c = cmp(&l, &r, &opts);
    let s = &c.sheets()[0];
    let differs = |col: u32, iu: bool| s.column_differs(col, iu);
    assert!(!differs(0, false), "keys");
    assert!(
        differs(1, false) && !differs(1, true),
        "within tolerance: unimportant"
    );
    assert!(
        differs(2, false) && differs(2, true),
        "a case change is important"
    );
    assert!(!differs(3, false), "same cells");
    assert!(
        !differs(4, false),
        "orphan rows (k 3 and 4) make no column differ"
    );
    assert!(!differs(99, false), "no such column");
    assert_eq!(
        s.differing_columns(false),
        vec![false, true, true, false, false]
    );
    assert_eq!(
        s.differing_columns(true),
        vec![false, false, true, false, false]
    );
    // A header difference (columns aligned by number) is a difference of its column.
    let l = wb("a.csv", "id,name\n1,x\n");
    let r = wb("b.csv", "id,title\n1,x\n");
    let mut by_number = TableCompareOptions::default();
    by_number.default_sheet.columns = ColumnAlignment::Number;
    let c = cmp(&l, &r, &by_number);
    assert_eq!(c.sheets()[0].differing_columns(false), vec![false, true]);
    assert!(c.sheets()[0].column_differs(1, true));
}

/// Phase 3: each preset has its word, which `RowFilter::parse` reads back; a mix of toggles has none.
#[test]
fn preset_words_read_back() {
    use hikasa_table::RowFilter;
    for preset in [
        RowFilter::ALL,
        RowFilter::DIFFERENCES,
        RowFilter::SAME,
        RowFilter::NONE,
    ] {
        let word = preset.preset_word().expect("a preset");
        assert_eq!(RowFilter::parse(word).unwrap(), preset);
        let ignored = RowFilter {
            ignore_unimportant: true,
            ..preset
        };
        assert_eq!(ignored.preset_word(), Some(word));
    }
    let mix = RowFilter {
        left_only: false,
        ..RowFilter::ALL
    };
    assert_eq!(mix.preset_word(), None);
}
