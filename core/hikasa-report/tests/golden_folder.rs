//! Folder report goldens (§6.2.7): every `tests/fixtures/folder/<case>/case.json` names a hikasa-folder tree case
//! (`tree.toml`, `rules.toml`), which is materialised into `MemoryVfs` and scanned by the real folder engine, and
//! the display options and columns of its reports. Each case produces `expected.list.txt`, `expected.sbs.txt`,
//! `expected.sbs.html`, `expected.summary.txt`, `expected.summary.html` and `expected.json` (the JSON layout's
//! output: the `hikasa.folder_compare.v1` result with its `"schema"` first); cases with `"ansi": true` also
//! produce the coloured `expected.list.ansi.txt` and `expected.sbs.ansi.txt`. Every output is rendered twice
//! (P-RPT-2) and every HTML document is checked (P-RPT-3). `HIKASA_BLESS=1` rewrites the expected files. The
//! goldens hold the inputs' Unix host paths, so the test is ignored (visibly) elsewhere.

mod common;

use common::{assert_golden, cases, check_html, fixtures, folder_case, sibling_fixtures};
use hikasa_folder::{DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode};
use hikasa_report::json::{FOLDER_COMPARE_SCHEMA, FolderReportJson};
use hikasa_report::{
    FolderColumns, FolderLayout, FolderReportOptions, OutputFormat, ReportMeta,
    folder_compare_json, render_folder_report,
};
use serde::Deserialize;

/// The options of a case (`case.json`).
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Case {
    /// What the case shows.
    description: String,
    /// The hikasa-folder tree case (`hikasa-folder/<case>`).
    source: String,
    /// Display filter (`FolderDisplayFilter::parse`).
    filter: String,
    /// Folder mode.
    folders: FolderMode,
    /// Ignore unimportant differences.
    ignore_unimportant: bool,
    /// Suppress filters.
    suppress_filters: bool,
    /// Columns (`FolderColumns::parse_list`).
    columns: String,
    /// Monochrome HTML.
    mono: bool,
    /// Also render the ANSI goldens `expected.list.ansi.txt` and `expected.sbs.ansi.txt`.
    ansi: bool,
}

impl Default for Case {
    fn default() -> Case {
        Case {
            description: String::new(),
            source: String::new(),
            filter: "all".into(),
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
            columns: "size,modified".into(),
            mono: false,
            ansi: false,
        }
    }
}

/// Renders with `opts` (twice: the bytes must be identical, P-RPT-2).
fn render(cmp: &FolderComparison, opts: &FolderReportOptions) -> Vec<u8> {
    let meta = ReportMeta::for_folder(cmp, opts, None, "hikasa test");
    let mut first = Vec::new();
    render_folder_report(cmp, opts, &meta, &mut first).unwrap();
    let mut second = Vec::new();
    render_folder_report(cmp, opts, &meta, &mut second).unwrap();
    assert_eq!(first, second, "rendering is deterministic (INV-RPT-1)");
    first
}

#[test]
#[cfg_attr(not(unix), ignore = "the goldens hold Unix host paths")]
fn folder_report_goldens() {
    let dirs = cases(&fixtures("folder"));
    assert!(dirs.len() >= 20, "folder report cases");
    for dir in &dirs {
        let case: Case =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        assert!(!case.description.is_empty(), "{}", dir.display());
        let source = case
            .source
            .strip_prefix("hikasa-folder/")
            .unwrap_or_else(|| panic!("{}: source", dir.display()));
        let (_, cmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join(source));
        let base = FolderReportOptions {
            layout: FolderLayout::List,
            display: DisplayOptions {
                filter: FolderDisplayFilter::parse(&case.filter).unwrap(),
                folders: case.folders,
                ignore_unimportant: case.ignore_unimportant,
                suppress_filters: case.suppress_filters,
            },
            columns: FolderColumns::parse_list(&case.columns).unwrap(),
            format: OutputFormat::Text { ansi: false },
        };
        let text = OutputFormat::Text { ansi: false };
        let html = OutputFormat::Html { mono: case.mono };
        let outputs = [
            ("expected.list.txt", FolderLayout::List, text),
            ("expected.sbs.txt", FolderLayout::SideBySide, text),
            ("expected.sbs.html", FolderLayout::SideBySide, html),
            ("expected.summary.txt", FolderLayout::Summary, text),
            ("expected.summary.html", FolderLayout::Summary, html),
            ("expected.json", FolderLayout::Json, text),
        ];
        let ansi = OutputFormat::Text { ansi: true };
        let ansi_outputs = [
            ("expected.list.ansi.txt", FolderLayout::List, ansi),
            ("expected.sbs.ansi.txt", FolderLayout::SideBySide, ansi),
        ];
        let ansi_outputs = if case.ansi { &ansi_outputs[..] } else { &[] };
        for &(file, layout, format) in outputs.iter().chain(ansi_outputs) {
            let opts = FolderReportOptions {
                layout,
                format,
                ..base
            };
            let bytes = render(&cmp, &opts);
            if matches!(format, OutputFormat::Html { .. }) {
                check_html(std::str::from_utf8(&bytes).unwrap())
                    .unwrap_or_else(|e| panic!("{}/{file}: {e}", dir.display()));
            }
            if layout == FolderLayout::Json {
                // The JSON layout is the DTO, pretty-printed (§3.4) with `"schema"` as its first key (§4.1 rule
                // 10), plus a newline (§4.1 rule 1).
                let dto = folder_compare_json(&cmp, &base.display);
                let mut doc = serde_json::to_string_pretty(&FolderReportJson::new(&dto)).unwrap();
                doc.push('\n');
                assert_eq!(std::str::from_utf8(&bytes).unwrap(), doc);
                assert!(
                    doc.starts_with(
                        "{\n  \"schema\": \"hikasa.folder_compare.v1\",\n  \"left\": {"
                    )
                );
                let value: serde_json::Value = serde_json::from_str(&doc).unwrap();
                let mut bare = serde_json::to_value(&dto).unwrap();
                bare.as_object_mut().unwrap().insert(
                    "schema".to_owned(),
                    serde_json::Value::from(FOLDER_COMPARE_SCHEMA),
                );
                assert_eq!(value, bare);
            }
            assert_golden(&dir.join(file), &bytes);
        }
    }
}
