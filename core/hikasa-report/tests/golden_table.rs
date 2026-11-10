//! Table Compare phase 2 (contract §6.3, TB-025): a golden file for every table layout and format (summary,
//! side-by-side and interleaved as text, ANSI text and HTML; the XML dump; the JSON report), the JSON document of
//! `table_compare_json`, and the rules every rendering keeps (well-formed HTML, deterministic bytes, words refused).
//! `HIKASA_BLESS=1` rewrites the goldens.

mod common;

use common::tables::{TableCase, csv_pair, workbooks};
use common::{assert_golden, check_html, fixtures, xmlcheck};
use hikasa_report::json::{TableViewOptions, table_compare_json};
use hikasa_report::{
    HtmlStyle, ReportBody, ReportDefaults, ReportError, ReportInput, ReportKind, ReportMeta,
    ReportRequest, ReportWords, TableCells, TargetKind, render_report,
};
use hikasa_table::RowFilter;

/// Renders `case` with the request `words` (layout first, then option words).
fn render(case: &TableCase, layout: &str, options: &[&str], target: TargetKind) -> Vec<u8> {
    let words = ReportWords {
        layout: Some(layout.to_owned()),
        options: options.iter().map(|w| (*w).to_owned()).collect(),
        output_options: Vec::new(),
        title: None,
    };
    let mut req = ReportRequest::parse(
        ReportKind::Table,
        &words,
        &ReportDefaults::new("side-by-side"),
    )
    .unwrap();
    req.check_target(target).unwrap();
    req.apply_target(target);
    let src = case.source();
    let ReportBody::Table(opts) = &req.body else {
        panic!("a table body")
    };
    let meta = ReportMeta::for_table(&src, opts, None, "hikasa test");
    let mut out = Vec::new();
    render_report(
        ReportInput::Table(&src),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut out,
    )
    .unwrap();
    out
}

const FILE: TargetKind = TargetKind::File;
const TERMINAL: TargetKind = TargetKind::Stdout { ansi: true };

#[test]
fn every_layout_and_format_has_a_golden() {
    let case = workbooks();
    let dir = fixtures("table");
    let cases: &[(&str, &str, &[&str], TargetKind)] = &[
        ("summary.txt", "summary", &[], FILE),
        ("summary.html", "summary", &["html-color"], FILE),
        ("summary_ansi.txt", "summary", &[], TERMINAL),
        ("sbs.txt", "side-by-side", &[], FILE),
        ("sbs.html", "side-by-side", &["html-color"], FILE),
        ("sbs_mono.html", "side-by-side", &["html-mono"], FILE),
        ("sbs_ansi.txt", "side-by-side", &[], TERMINAL),
        (
            "sbs_differences_iu.txt",
            "side-by-side",
            &["display-mismatches", "ignore-unimportant", "no-row-numbers"],
            FILE,
        ),
        ("interleaved.txt", "interleaved", &[], FILE),
        ("interleaved.html", "interleaved", &["html-color"], FILE),
        ("interleaved_ansi.txt", "interleaved", &[], TERMINAL),
        ("report.xml", "xml", &[], FILE),
        (
            "report_differences.xml",
            "xml",
            &["display-differences"],
            FILE,
        ),
        ("report.json", "json", &["display-all", "cells-all"], FILE),
    ];
    for (name, layout, options, target) in cases {
        let bytes = render(&case, layout, options, *target);
        let text = String::from_utf8(bytes.clone()).unwrap();
        if name.ends_with(".html") {
            check_html(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        if name.ends_with(".xml") {
            xmlcheck::check(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        if name.ends_with(".json") {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["schema"], "hikasa.table_compare.v1");
        }
        if !name.contains("ansi") {
            assert!(!text.contains('\u{1b}'), "{name}: an escape without ANSI");
        }
        assert!(!text.contains('\r'), "{name}");
        assert_eq!(
            bytes,
            render(&case, layout, options, *target),
            "{name}: twice"
        );
        assert_golden(&dir.join(name), &bytes);
    }
    let csv = csv_pair();
    for (name, layout) in [
        ("csv_sbs.txt", "side-by-side"),
        ("csv_interleaved.txt", "interleaved"),
        ("csv_summary.txt", "summary"),
        ("csv_report.xml", "xml"),
    ] {
        let bytes = render(&csv, layout, &[], FILE);
        assert_golden(&dir.join(name), &bytes);
    }
}

/// The JSON document of `--json` (the DTO without the envelope): the default view (differences, differing cells), and
/// every row with every cell.
#[test]
fn table_compare_json_goldens() {
    let case = workbooks();
    let src = case.source();
    let view = TableViewOptions::default();
    let doc = serde_json::to_string_pretty(&table_compare_json(&src, &view)).unwrap() + "\n";
    assert_golden(&fixtures("json").join("table_compare.json"), doc.as_bytes());
    let csv = csv_pair();
    let all = TableViewOptions {
        filter: RowFilter::ALL,
        cells: TableCells::All,
        ignore_unimportant: true,
    };
    let doc =
        serde_json::to_string_pretty(&table_compare_json(&csv.source(), &all)).unwrap() + "\n";
    assert_golden(
        &fixtures("json").join("table_compare_csv.json"),
        doc.as_bytes(),
    );
}

/// Facts the goldens show, asserted so that a blessed regression cannot hide them.
#[test]
fn the_reports_say_what_the_comparison_found() {
    let case = workbooks();
    let summary = String::from_utf8(render(&case, "summary", &[], FILE)).unwrap();
    assert!(summary.contains("Verdict:"), "{summary}");
    assert!(summary.contains("important differences"));
    assert!(
        summary.contains("Sheet 2: Notes \u{2192} notes"),
        "{summary}"
    );
    assert!(summary.contains("Sheet 3: Extra"));
    assert!(summary.contains("keyed by id"));
    let iu = String::from_utf8(render(&case, "summary", &["ignore-unimportant"], FILE)).unwrap();
    assert!(iu.contains("Ignore unimportant:  yes"), "{iu}");
    let sbs = String::from_utf8(render(&case, "side-by-side", &[], FILE)).unwrap();
    // The unimportant price (tolerance), the important name, the orphans.
    let line = |needle: &str| {
        sbs.lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} in\n{sbs}"))
            .to_owned()
    };
    assert!(line("plum").starts_with('~'), "{}", line("plum"));
    assert!(line("Pear").starts_with('|'));
    assert!(line("fig").starts_with('<'));
    assert!(line("kiwi").starts_with('>'));
    assert!(line("apple").starts_with(' '));
    // Display filters hide rows behind one gap line.
    let differences = String::from_utf8(render(
        &case,
        "side-by-side",
        &["display-differences", "ignore-unimportant"],
        FILE,
    ))
    .unwrap();
    assert!(!differences.contains("apple") && !differences.contains("plum"));
    assert!(differences.contains("rows hidden"), "{differences}");
    let none = String::from_utf8(render(&case, "interleaved", &["display-none"], FILE)).unwrap();
    assert!(!none.contains("kiwi"));
    // Row numbers are the sheets' own labels (the header is row 1).
    let numbered = String::from_utf8(render(&case, "interleaved", &[], FILE)).unwrap();
    assert!(
        numbered
            .lines()
            .any(|l| l.starts_with("| L 3 ") && l.contains("pear")),
        "{numbered}"
    );
    let html = String::from_utf8(render(&case, "side-by-side", &["html-color"], FILE)).unwrap();
    assert!(html.contains("<td class=\"c imp\">Pear</td>"), "{html}");
    assert!(html.contains("<td class=\"c unimp\">0.76</td>"));
    assert!(html.contains("first &lt;draft&gt; &amp; &quot;q&quot;"));
    // The width bounds every plain-text line.
    let narrow = {
        let words = ReportWords {
            layout: Some("side-by-side".into()),
            ..ReportWords::default()
        };
        let mut defaults = ReportDefaults::new("side-by-side");
        defaults.width = 40;
        let req = ReportRequest::parse(ReportKind::Table, &words, &defaults).unwrap();
        let src = case.source();
        let ReportBody::Table(opts) = &req.body else {
            unreachable!()
        };
        let meta = ReportMeta::for_table(&src, opts, None, "t");
        let mut out = Vec::new();
        render_report(
            ReportInput::Table(&src),
            &req,
            &meta,
            &HtmlStyle::default(),
            None,
            &mut out,
        )
        .unwrap();
        String::from_utf8(out).unwrap()
    };
    let body = narrow.split("\n\n").skip(1).collect::<Vec<_>>().join("\n");
    for l in body.lines().filter(|l| !l.starts_with("Sheet ")) {
        assert!(
            unicode_width::UnicodeWidthStr::width(l) <= 40,
            "{l:?} wider than 40"
        );
    }
}

/// A table report refuses the words of other kinds, a scope, and HTML for XML/JSON.
#[test]
fn table_words_are_checked() {
    let defaults = ReportDefaults::new("side-by-side");
    let parse = |layout: &str, options: &[&str]| {
        ReportRequest::parse(
            ReportKind::Table,
            &ReportWords {
                layout: Some(layout.into()),
                options: options.iter().map(|w| (*w).to_owned()).collect(),
                ..ReportWords::default()
            },
            &defaults,
        )
    };
    for word in [
        "patch-unified",
        "strikeout-left-diffs",
        "wrap-word",
        "display-context",
    ] {
        let e = parse("side-by-side", &[word]).unwrap_err().to_string();
        assert!(e.contains("this one is for text reports"), "{word}: {e}");
    }
    for word in ["column-size", "include-file-links", "display-left-newer"] {
        let e = parse("side-by-side", &[word]).unwrap_err().to_string();
        assert!(e.contains("this one is for folder reports"), "{word}: {e}");
    }
    let e = parse("unified", &[]).unwrap_err().to_string();
    assert!(e.contains("this one is for text reports"), "{e}");
    let e = parse("list", &[]).unwrap_err().to_string();
    assert!(e.contains("this one is for folder reports"), "{e}");
    assert!(parse("pivot", &[]).is_err());
    // A text report refuses the table words.
    let e = ReportRequest::parse(
        ReportKind::Text,
        &ReportWords {
            options: vec!["cells-all".into()],
            ..ReportWords::default()
        },
        &defaults,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("this one is for table reports"), "{e}");
    // Conflicts, unused words, HTML refused for xml and json.
    assert!(matches!(
        parse("side-by-side", &["display-all", "display-none"]),
        Err(ReportError::Conflict { .. })
    ));
    let req = parse(
        "summary",
        &["display-mismatches", "row-numbers", "cells-all"],
    )
    .unwrap();
    assert_eq!(
        req.unused,
        ["display-mismatches", "line-numbers", "cells-all"]
    );
    for layout in ["xml", "json"] {
        assert!(matches!(
            parse(layout, &["html-color"]),
            Err(ReportError::Unsupported { .. })
        ));
    }
    let req = parse("json", &[]).unwrap();
    assert!(req.check_target(TargetKind::Browser).is_err());
    // Round trip of the canonical words.
    for (layout, options) in [
        (
            "side-by-side",
            vec![
                "display-differences",
                "no-row-numbers",
                "ignore-unimportant",
            ],
        ),
        (
            "interleaved",
            vec!["display-same", "html-mono", "print-landscape"],
        ),
        ("json", vec!["cells-all", "display-none"]),
        ("summary", vec!["html-color", "print-mono"]),
        ("xml", vec![]),
    ] {
        let req = parse(layout, &options).unwrap();
        let again = ReportRequest::parse(ReportKind::Table, &req.words(), &defaults).unwrap();
        assert_eq!(again, req, "{layout} {options:?}");
    }
}

mod props {
    use std::sync::Arc;

    use hikasa_core::{CancelToken, Pair};
    use hikasa_report::{
        HtmlStyle, ReportBody, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest,
        ReportWords, TableSide, TableSource, render_report,
    };
    use hikasa_table::{
        FirstLine, Regional, Sheet, SourceKind, TableCompareOptions, Workbook, compare_workbooks,
    };
    use proptest::prelude::*;

    use super::common::{check_html, config, xmlcheck};

    fn table() -> impl Strategy<Value = Vec<Vec<String>>> {
        let cell = prop_oneof![
            Just(String::new()),
            "[a-c]{1,3}",
            (0u32..20).prop_map(|n| n.to_string()),
            Just("x\ty\"<&>\u{1}\u{ffff}".to_owned()),
            Just("日本語のテキスト".to_owned()),
        ];
        proptest::collection::vec(proptest::collection::vec(cell, 0..5), 1..7)
    }

    proptest! {
        #![proptest_config(config("tests/proptest-regressions/golden_table.txt"))]

        /// Every table layout renders random tables: HTML well-formed, XML by the structure checks, plain text within
        /// its width (40 to 200 columns), deterministic.
        #[test]
        fn p_table_layouts_render(
            left in table(),
            right in table(),
            layout in 0usize..5,
            display in 0usize..4,
            width in 40u16..200,
            html in any::<bool>(),
        ) {
            let sheet = |rows: &[Vec<String>]| {
                Workbook::from_sheets(
                    vec![Sheet::from_text_rows("S", rows, FirstLine::Detect, Regional::default())],
                    SourceKind::Xlsx,
                )
            };
            let options = TableCompareOptions::default();
            let cmp = compare_workbooks(
                Arc::new(sheet(&left)),
                Arc::new(sheet(&right)),
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
            let layouts = ["side-by-side", "interleaved", "summary", "xml", "json"];
            let displays = ["display-all", "display-mismatches", "display-matches", "display-none"];
            let mut output_options = Vec::new();
            let html = html && layout < 3;
            if html {
                output_options.push("html-color".to_owned());
            }
            let words = ReportWords {
                layout: Some(layouts[layout].to_owned()),
                options: vec![displays[display].to_owned()],
                output_options,
                title: None,
            };
            let mut defaults = ReportDefaults::new("side-by-side");
            defaults.width = width;
            let req = ReportRequest::parse(ReportKind::Table, &words, &defaults).unwrap();
            let ReportBody::Table(opts) = &req.body else { unreachable!() };
            let meta = ReportMeta::for_table(&src, opts, None, "t");
            let render = || {
                let mut out = Vec::new();
                render_report(ReportInput::Table(&src), &req, &meta, &HtmlStyle::default(), None, &mut out).unwrap();
                out
            };
            let bytes = render();
            prop_assert_eq!(&bytes, &render());
            let text = String::from_utf8(bytes.clone()).unwrap();
            if html {
                prop_assert!(check_html(&text).is_ok(), "{:?}\n{}", check_html(&text), text);
            } else if layout == 3 {
                prop_assert!(xmlcheck::check(&bytes).is_ok(), "{:?}\n{}", xmlcheck::check(&bytes), text);
            } else if layout == 4 {
                prop_assert!(serde_json::from_slice::<serde_json::Value>(&bytes).is_ok());
            } else if layout < 2 {
                for line in text.split("\n\n").skip(1).flat_map(str::lines) {
                    if line.starts_with("Sheet ") {
                        continue;
                    }
                    prop_assert!(
                        unicode_width::UnicodeWidthStr::width(line) <= usize::from(width),
                        "{:?} wider than {}", line, width
                    );
                }
            }
        }
    }
}

/// T-38 (mutation R18 of the phase-2 review): an orphan row carries its non-empty cells only (columns are 1-based).
#[test]
fn orphan_rows_carry_their_non_empty_cells() {
    let sheet = |rows: &[&[&str]]| {
        let rows: Vec<Vec<&str>> = rows.iter().map(|r| r.to_vec()).collect();
        hikasa_table::Sheet::from_text_rows(
            "S",
            &rows,
            hikasa_table::FirstLine::Names,
            hikasa_table::Regional::default(),
        )
    };
    let left = hikasa_table::Workbook::from_sheets(
        vec![sheet(&[&["id", "a", "b"], &["1", "x", "y"]])],
        hikasa_table::SourceKind::Delimited,
    );
    let right = hikasa_table::Workbook::from_sheets(
        vec![sheet(&[
            &["id", "a", "b"],
            &["1", "x", "y"],
            &["2", "", "z"],
        ])],
        hikasa_table::SourceKind::Delimited,
    );
    let options = hikasa_table::TableCompareOptions::default();
    let comparison = hikasa_table::compare_workbooks(
        std::sync::Arc::new(left),
        std::sync::Arc::new(right),
        &options,
        &hikasa_core::CancelToken::new(),
    )
    .unwrap();
    let side = hikasa_report::TableSide {
        spec: "/t.csv".into(),
        size: None,
        mtime: None,
        format: None,
        sniffed: false,
    };
    let src = hikasa_report::TableSource {
        comparison: &comparison,
        options: &options,
        sides: hikasa_core::Pair::new(&side, &side),
        bytes_identical: false,
    };
    let doc = serde_json::to_value(table_compare_json(&src, &TableViewOptions::default())).unwrap();
    let rows = doc["sheets"][0]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0]["state"], "right_only");
    let columns: Vec<_> = rows[0]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["column"].as_u64().unwrap())
        .collect();
    assert_eq!(columns, [1, 3], "the empty cell of column 2 is left out");
}
