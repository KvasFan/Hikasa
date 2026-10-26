//! Text report goldens (§6.2.7): every `tests/fixtures/text/<case>/case.json` names its inputs (a hikasa-text
//! compare case, `hikasa-text/<case>`, whose `left.*`, `right.*` and `rules.toml` are reused, or files in the case
//! folder itself) and its report options. The inputs are loaded through `MemoryVfs` and compared by the real text
//! engine with the Everything Else format; each case produces `expected.annotated.diff`,
//! `expected.unified.patch`, `expected.context.patch`, `expected.normal.patch`, `expected.sbs.txt`,
//! `expected.sbs.html`, `expected.summary.txt`, `expected.summary.html`, `expected.stats.csv` and
//! `expected.json` (the `hikasa.text_compare.v1` result); cases with `"ansi": true` also produce the coloured
//! `expected.annotated.ansi`, `expected.context.ansi` and `expected.sbs.ansi.txt`. The goldens hold the inputs'
//! Unix host paths, so the test is ignored (visibly) elsewhere. Every output is also rendered twice (P-RPT-2) and every
//! HTML document is checked (P-RPT-3). `HIKASA_BLESS=1` rewrites the expected files.

mod common;

use std::path::Path;

use common::{assert_golden, cases, check_html, compare_case, fixtures, sibling_fixtures};
use hikasa_report::{
    DisplayMode, OutputFormat, PatchStyle, ReportMeta, TextLayout, TextReportOptions, Wrap,
    render_text_report, text_compare_json,
};
use hikasa_text::{TextComparison, ViewOptions};
use serde::Deserialize;

/// The options of a case (`case.json`).
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Case {
    /// What the case shows.
    description: String,
    /// `hikasa-text/<case>` (a hikasa-text compare case) or `text/<case>` (another case of this suite); absent:
    /// the inputs are in the case folder.
    source: Option<String>,
    /// Context lines of the patches (and of display `context`).
    context: u32,
    /// Display mode of the side-by-side and summary layouts (`DisplayMode::parse`).
    display: String,
    /// Ignore unimportant differences.
    ignore_unimportant: bool,
    /// Side-by-side width.
    width: u16,
    /// Side-by-side wrapping (`Wrap::parse`).
    wrap: String,
    /// Line numbers.
    line_numbers: bool,
    /// Monochrome HTML.
    mono: bool,
    /// Also render the ANSI goldens: `expected.annotated.ansi`, `expected.context.ansi` and
    /// `expected.sbs.ansi.txt`.
    ansi: bool,
}

impl Default for Case {
    fn default() -> Case {
        Case {
            description: String::new(),
            source: None,
            context: 3,
            display: "all".into(),
            ignore_unimportant: false,
            width: 80,
            wrap: "character".into(),
            line_numbers: true,
            mono: false,
            ansi: false,
        }
    }
}

/// The folder holding a case's inputs.
fn inputs(dir: &Path, case: &Case) -> std::path::PathBuf {
    match case.source.as_deref() {
        None => dir.to_path_buf(),
        Some(source) => match source.split_once('/') {
            Some(("hikasa-text", name)) => sibling_fixtures("hikasa-text", "compare").join(name),
            Some(("text", name)) => fixtures("text").join(name),
            _ => panic!("{}: unknown source {source}", dir.display()),
        },
    }
}

/// Renders with `opts` (twice: the bytes must be identical, P-RPT-2).
fn render(cmp: &TextComparison, opts: &TextReportOptions) -> Vec<u8> {
    let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    let mut first = Vec::new();
    render_text_report(cmp, opts, &meta, &mut first).unwrap();
    let mut second = Vec::new();
    render_text_report(cmp, opts, &meta, &mut second).unwrap();
    assert_eq!(first, second, "rendering is deterministic (INV-RPT-1)");
    first
}

#[test]
#[cfg_attr(not(unix), ignore = "the goldens hold Unix host paths")]
fn text_report_goldens() {
    let dirs = cases(&fixtures("text"));
    assert!(dirs.len() >= 25, "text report cases");
    for dir in &dirs {
        let case: Case =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        assert!(!case.description.is_empty(), "{}", dir.display());
        let cmp = compare_case(&inputs(dir, &case));
        let n = case.context;
        let base = TextReportOptions {
            layout: TextLayout::SideBySide,
            display: DisplayMode::parse(&case.display, n).unwrap(),
            ignore_unimportant: case.ignore_unimportant,
            line_numbers: case.line_numbers,
            width: case.width,
            wrap: Wrap::parse(&case.wrap).unwrap(),
            format: OutputFormat::Text { ansi: false },
        };
        let text = OutputFormat::Text { ansi: false };
        let html = OutputFormat::Html { mono: case.mono };
        let outputs = [
            (
                "expected.annotated.diff",
                TextLayout::AnnotatedUnified { lines: n },
                text,
            ),
            (
                "expected.unified.patch",
                TextLayout::Patch(PatchStyle::Unified { lines: n }),
                text,
            ),
            (
                "expected.context.patch",
                TextLayout::Patch(PatchStyle::Context { lines: n }),
                text,
            ),
            (
                "expected.normal.patch",
                TextLayout::Patch(PatchStyle::Normal),
                text,
            ),
            ("expected.sbs.txt", TextLayout::SideBySide, text),
            ("expected.sbs.html", TextLayout::SideBySide, html),
            ("expected.summary.txt", TextLayout::Summary, text),
            ("expected.summary.html", TextLayout::Summary, html),
            ("expected.stats.csv", TextLayout::Statistics, text),
        ];
        let ansi = OutputFormat::Text { ansi: true };
        let ansi_outputs = [
            (
                "expected.annotated.ansi",
                TextLayout::AnnotatedUnified { lines: n },
                ansi,
            ),
            (
                "expected.context.ansi",
                TextLayout::Patch(PatchStyle::Context { lines: n }),
                ansi,
            ),
            ("expected.sbs.ansi.txt", TextLayout::SideBySide, ansi),
        ];
        let ansi_outputs = if case.ansi { &ansi_outputs[..] } else { &[] };
        for &(file, layout, format) in outputs.iter().chain(ansi_outputs) {
            let opts = TextReportOptions {
                layout,
                format,
                ..base
            };
            let bytes = render(&cmp, &opts);
            if matches!(format, OutputFormat::Html { .. }) {
                let html = std::str::from_utf8(&bytes).unwrap();
                check_html(html).unwrap_or_else(|e| panic!("{}/{file}: {e}", dir.display()));
            }
            assert_golden(&dir.join(file), &bytes);
        }
        let view = ViewOptions {
            filter: base.display.text_filter(),
            ignore_unimportant: case.ignore_unimportant,
            expanded: Vec::new(),
        };
        let mut json = serde_json::to_string_pretty(&text_compare_json(&cmp, &view)).unwrap();
        json.push('\n');
        assert_golden(&dir.join("expected.json"), json.as_bytes());
    }
}
