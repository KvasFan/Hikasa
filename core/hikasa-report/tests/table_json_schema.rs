//! `docs/schemas/table_compare.v1.json` (Table Compare §6.3): the schema is closed (every object lists and requires
//! its keys and refuses others); the JSON goldens of this crate, documents generated from random tables, and every
//! `*.json` golden of the CLI's `table` suite (its envelope's `result`) validate; the validator rejects planted
//! violations.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::fixtures;
use common::jsonschema::{closed, validate};
use common::tables::{csv_pair, workbooks};
use hikasa_core::{CancelToken, Pair};
use hikasa_report::json::{TableViewOptions, table_compare_json};
use hikasa_report::{TableCells, TableSide, TableSource};
use hikasa_table::{
    FirstLine, Regional, RowFilter, Sheet, SourceKind, TableCompareOptions, Workbook,
    compare_workbooks,
};
use proptest::prelude::*;
use serde_json::Value;

fn schema() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schemas/table_compare.v1.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn assert_valid(what: &str, doc: &Value) {
    let errors = validate(&schema(), doc);
    assert!(errors.is_empty(), "{what}:\n{}", errors.join("\n"));
}

#[test]
fn the_schema_is_closed() {
    let errors = closed(&schema(), &["schema"]);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn the_goldens_validate() {
    for name in ["table_compare.json", "table_compare_csv.json"] {
        let doc: Value =
            serde_json::from_slice(&std::fs::read(fixtures("json").join(name)).unwrap()).unwrap();
        assert_valid(name, &doc);
    }
    let report: Value =
        serde_json::from_slice(&std::fs::read(fixtures("table").join("report.json")).unwrap())
            .unwrap();
    assert_eq!(report["schema"], "hikasa.table_compare.v1");
    assert_valid("report.json", &report);
    for case in [workbooks(), csv_pair()] {
        for filter in [
            RowFilter::ALL,
            RowFilter::DIFFERENCES,
            RowFilter::SAME,
            RowFilter::NONE,
        ] {
            for cells in [TableCells::All, TableCells::Differing] {
                for iu in [false, true] {
                    let view = TableViewOptions {
                        filter,
                        cells,
                        ignore_unimportant: iu,
                    };
                    let doc =
                        serde_json::to_value(table_compare_json(&case.source(), &view)).unwrap();
                    assert_valid("generated", &doc);
                }
            }
        }
    }
}

/// The CLI's goldens (`cli/hikasa-cli/tests/goldens/table/*.json`): the `result` of an envelope, or the document. The
/// folder may not exist yet.
#[test]
fn the_cli_goldens_validate() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../cli/hikasa-cli/tests/goldens/table");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    for path in files {
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let doc = if doc.get("hikasa_version").is_some() {
            if doc["schema"] != "hikasa.table_compare.v1" || doc["result"].is_null() {
                continue;
            }
            doc["result"].clone()
        } else {
            doc
        };
        assert_valid(&path.display().to_string(), &doc);
    }
}

#[test]
fn the_validator_rejects_planted_violations() {
    let golden: Value = serde_json::from_slice(
        &std::fs::read(fixtures("json").join("table_compare.json")).unwrap(),
    )
    .unwrap();
    let mut extra = golden.clone();
    extra["sheets"][0]["colour"] = Value::from("red");
    let errors = validate(&schema(), &extra);
    assert!(
        errors.iter().any(|e| e.contains("unexpected key colour")),
        "{errors:?}"
    );
    let mut wrong = golden.clone();
    wrong["stats"]["sheets_paired"] = Value::from("two");
    let errors = validate(&schema(), &wrong);
    assert!(
        errors.iter().any(|e| e.contains("$.stats.sheets_paired")),
        "{errors:?}"
    );
    let mut missing = golden.clone();
    missing["left"].as_object_mut().unwrap().remove("source");
    let errors = validate(&schema(), &missing);
    assert!(
        errors.iter().any(|e| e.contains("missing source")),
        "{errors:?}"
    );
    let mut bad_enum = golden.clone();
    bad_enum["sheets"][0]["state"] = Value::from("moved");
    assert!(!validate(&schema(), &bad_enum).is_empty());
    let mut negative = golden;
    negative["sheets"][0]["hidden_rows"] = Value::from(-1);
    assert!(!validate(&schema(), &negative).is_empty());
    // An unknown keyword in a schema is an error, never a silent pass.
    let odd = serde_json::json!({"type": "object", "patternProperties": {}});
    assert!(!validate(&odd, &serde_json::json!({})).is_empty());
}

/// A random table of up to 6 rows × 4 columns of small values (texts, numbers, blanks).
fn table() -> impl Strategy<Value = Vec<Vec<String>>> {
    let cell = prop_oneof![
        Just(String::new()),
        "[a-c]{1,3}",
        (0u32..20).prop_map(|n| n.to_string()),
        Just("1.50".to_owned()),
        Just("x\ty\"<&>".to_owned()),
    ];
    proptest::collection::vec(proptest::collection::vec(cell, 0..4), 1..6)
}

fn workbook(rows: &[Vec<String>], two: bool) -> Workbook {
    let mut sheets = vec![Sheet::from_text_rows(
        "S",
        rows,
        FirstLine::Detect,
        Regional::default(),
    )];
    if two {
        sheets.push(Sheet::from_text_rows(
            "T",
            &rows[..1],
            FirstLine::Data,
            Regional::default(),
        ));
    }
    Workbook::from_sheets(sheets, SourceKind::Ods)
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/table_json_schema.txt"))]

    /// Every document of random tables, views and sheet counts validates.
    #[test]
    fn p_table_json_validates(
        left in table(),
        right in table(),
        two in any::<bool>(),
        filter in 0usize..4,
        all in any::<bool>(),
        iu in any::<bool>(),
    ) {
        let options = TableCompareOptions::default();
        let cmp = compare_workbooks(
            Arc::new(workbook(&left, two)),
            Arc::new(workbook(&right, !two)),
            &options,
            &CancelToken::new(),
        )
        .unwrap();
        let side = TableSide::default();
        let src = TableSource {
            comparison: &cmp,
            options: &options,
            sides: Pair::new(&side, &side),
            bytes_identical: false,
        };
        let view = TableViewOptions {
            filter: [RowFilter::ALL, RowFilter::DIFFERENCES, RowFilter::SAME, RowFilter::NONE][filter],
            cells: if all { TableCells::All } else { TableCells::Differing },
            ignore_unimportant: iu,
        };
        let doc = serde_json::to_value(table_compare_json(&src, &view)).unwrap();
        let errors = validate(&schema(), &doc);
        prop_assert!(errors.is_empty(), "{}", errors.join("\n"));
    }
}
